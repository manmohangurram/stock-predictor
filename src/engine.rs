use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::config::Config;
use crate::llm::deepseek::Transport;
use crate::llm::orchestrate::{self, LlmDeps, Outcome, RunInput};
use crate::llm::parse::Proposal;
use crate::models::{Action, AssetClass, Bar, Candidate, Instrument, Position};
use crate::notify::{self, Notification, Notifier};
use crate::rules::budget;
use crate::rules::evaluate::{self, CandidateInput, RuleConfig};
use crate::rules::indicators::{self, atr_bands};

// Re-exported so tests and callers keep importing them from the engine.
pub use crate::market::{MarketSource, YahooSource};

pub struct EngineDeps<'a> {
    pub pool: &'a SqlitePool,
    pub cfg: &'a Config,
    pub market: &'a dyn MarketSource,
    pub transport: &'a dyn Transport,
    pub notifier: &'a dyn Notifier,
    pub rules: RuleConfig,
    /// Skip the quiet-tick dedupe. Used by \`--predict\`, where a human asked for an answer
    /// right now and "nothing changed since last time" is not an answer.
    pub force: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PollReport {
    /// One human-readable line per decision, for \`--predict\`.
    pub notes: Vec<String>,
    pub candidates: usize,
    pub llm_calls: i64,
    pub signals: usize,
    pub notifications: usize,
    pub blocked: usize,
    pub errors: Vec<String>,
}

/// The resolved decision for one instrument, whichever layer produced it.
struct Resolved {
    action: Action,
    entry: f64,
    stop: Option<f64>,
    target: Option<f64>,
    valid_until: Option<String>,
    qty: f64,
    reasons: Vec<String>,
    rationale: Option<String>,
}

pub async fn poll_once(deps: &EngineDeps<'_>, now: DateTime<Utc>) -> Result<PollReport> {
    let mut report = PollReport::default();
    let instruments = load_instruments(deps.pool).await?;

    for instrument in instruments {
        match poll_instrument(deps, &instrument, now, &mut report).await {
            Ok(()) => {}
            Err(e) => report
                .errors
                .push(format!("{}: {e:#}", instrument.symbol)),
        }
    }

    report.llm_calls = orchestrate::daily_chat_calls(deps.pool).await.unwrap_or(0);
    Ok(report)
}

async fn poll_instrument(
    deps: &EngineDeps<'_>,
    instrument: &Instrument,
    now: DateTime<Utc>,
    report: &mut PollReport,
) -> Result<()> {
    let bars = deps.market.bars(&instrument.symbol).await?;
    let news = deps.market.news(&instrument.symbol).await.unwrap_or_default();
    crate::store::headlines(deps.pool, instrument.id, &news).await?;

    crate::store::bars(deps.pool, instrument.id, &bars).await?;

    let ind = match indicators::compute(&bars) {
        Ok(i) => i,
        Err(e) => {
            tracing::debug!(symbol = %instrument.symbol, error = %e, "no indicators, skipping");
            return Ok(());
        }
    };

    let position = load_position(deps.pool, instrument.id).await?;
    let available = budget::available(deps.pool).await?;

    // Context for the model, assembled once. Prices first, then the tape, then the news.
    let price_window = match deps.market.intraday(&instrument.symbol, "15m", "5d").await {
        Ok(bars) => format_window(&bars, 6, now),
        Err(e) => {
            tracing::debug!(symbol = %instrument.symbol, error = %e, "no intraday tape");
            "(intraday tape unavailable)".to_string()
        }
    };
    let daily_summary = format_daily(&bars, 10);
    let positions_text = format_positions(position.as_ref(), &instrument.symbol);
    let recent_trades = load_recent_trades(deps.pool, instrument.id, now).await?;
    let last_decision = load_last_decision(deps.pool, instrument.id).await?;

    let Some(candidate) = evaluate::candidate(CandidateInput {
        instrument,
        bars: &bars,
        ind: &ind,
        position: position.as_ref(),
        available_cash: available,
        cfg: deps.rules,
    }) else {
        return Ok(());
    };
    report.candidates += 1;

    // The model is paid for judgement about news, so news is what re-triggers it. Prices can
    // move all day without another call; the rules refresh them.
    let headline_fp = crate::llm::memo::fingerprint(&news);
    if !deps.force && crate::llm::memo::unchanged(deps.pool, instrument.id, &headline_fp).await? {
        // Same news: keep the model's decision (action, stop, target) and refresh only the
        // entry, as long as the price is still inside the band the model's view was valid in.
        // Outside that band the view has expired and the rules take over.
        if reuse_model_signal(deps.pool, instrument.id, candidate.entry, ind.atr14, now).await? {
            return Ok(());
        }
        let resolved = Resolved {
            action: candidate.action,
            entry: candidate.entry,
            stop: candidate.stop,
            target: candidate.target,
            valid_until: Some((now + chrono::Duration::hours(18)).to_rfc3339()),
            qty: candidate.qty,
            reasons: candidate.reasons.clone(),
            rationale: None,
        };
        return finish(deps, instrument, resolved, &candidate, available, now, report).await;
    }

    // No key configured means no model layer at all — not a failed call on every tick.
    let llm_available = crate::rules::settings::deepseek_key(deps.pool).await?.is_some();
    if !llm_available {
        let resolved = Resolved {
            action: candidate.action,
            entry: candidate.entry,
            stop: candidate.stop,
            target: candidate.target,
            valid_until: Some((now + chrono::Duration::hours(18)).to_rfc3339()),
            qty: candidate.qty,
            reasons: candidate.reasons.clone(),
            rationale: None,
        };
        return finish(deps, instrument, resolved, &candidate, available, now, report).await;
    }

    let llm_deps = LlmDeps {
        pool: deps.pool,
        cfg: deps.cfg,
        transport: deps.transport,
        market: deps.market,
    };
    let decision = orchestrate::run(
        &llm_deps,
        RunInput {
            instrument,
            ind: &ind,
            candidate: &candidate,
            available_cash: available,
            held_qty: position.as_ref().map(|p| p.qty).unwrap_or(0.0),
            headlines: &news,
            price_window: &price_window,
            daily_summary: &daily_summary,
            positions: &positions_text,
            recent_trades: &recent_trades,
            last_decision: &last_decision,
            now,
        },
    )
    .await?;

    // The model answered for this headline set; do not ask it about the same news again.
    if !matches!(decision.outcome, Outcome::Rules(_)) {
        crate::llm::memo::remember(deps.pool, instrument.id, &headline_fp).await?;
    }

    let Some(resolved) = resolve(&candidate, decision.outcome, now) else {
        return Ok(());
    };

    finish(deps, instrument, resolved, &candidate, available, now, report).await
}

/// Signal persistence, the budget gate and the push — shared by the LLM and rules-only paths.
async fn finish(
    deps: &EngineDeps<'_>,
    instrument: &Instrument,
    resolved: Resolved,
    candidate: &Candidate,
    available: f64,
    now: DateTime<Utc>,
    report: &mut PollReport,
) -> Result<()> {
    // The budget gate is the last word, above the LLM: a proposal that does not fit is
    // recorded as BLOCKED with the shortfall, and is never pushed.
    if resolved.action == Action::Buy {
        let cost = resolved.qty * resolved.entry;
        if !budget::fits(deps.pool, cost).await? {
            record_blocked(deps.pool, instrument, &resolved, candidate, available, now).await?;
            report.blocked += 1;
            return Ok(());
        }
    }

    report.notes.push(format!(
        "{} {} qty {} entry {:.2} stop {} target {} until {} :: {}",
        instrument.symbol,
        resolved.action.as_str(),
        resolved.qty,
        resolved.entry,
        resolved.stop.map(|v| format!("{v:.2}")).unwrap_or_else(|| "-".into()),
        resolved.target.map(|v| format!("{v:.2}")).unwrap_or_else(|| "-".into()),
        resolved.valid_until.clone().unwrap_or_else(|| "-".into()),
        resolved.rationale.clone().unwrap_or_else(|| "rules only".into()),
    ));

    // "Prices refresh freely" means the entry may drift without a new row or a push: the
    // latest signal is updated in place. A different action, stop or target is new information
    // and gets its own row.
    if !deps.force {
        if let Some(prev) = latest_signal(deps.pool, instrument.id).await? {
            let same_shape = prev.action == resolved.action.as_str()
                && stop_round(prev.stop) == stop_round(resolved.stop)
                && stop_round(prev.target) == stop_round(resolved.target);
            if same_shape {
                if prev.entry.is_some_and(|e| (e - resolved.entry).abs() >= 0.005) {
                    sqlx::query("UPDATE signals SET entry = ? WHERE id = ?")
                        .bind(resolved.entry)
                        .bind(prev.id)
                        .execute(deps.pool)
                        .await?;
                }
                return Ok(());
            }
        }
    }

    let signal_id = insert_signal(deps.pool, instrument.id, &resolved, candidate, now).await?;
    report.signals += 1;

    let mut as_candidate = candidate.clone();
    as_candidate.action = resolved.action;
    as_candidate.entry = resolved.entry;
    as_candidate.stop = resolved.stop;
    as_candidate.target = resolved.target;
    as_candidate.qty = resolved.qty;

    // No topic configured means no push channel; the signal is still recorded, and not
    // attempting the call keeps the poll's error list about the market, not the config.
    if deps.cfg.ntfy_topic.is_empty() {
        return Ok(());
    }
    if let Some(mut n) = Notification::from_candidate(
        &instrument.symbol,
        &instrument.currency,
        &as_candidate,
    ) {
        n.signal_id = Some(signal_id);
        if notify::dispatch(deps.pool, deps.notifier, &n).await? {
            report.notifications += 1;
        }
    }
    Ok(())
}

/// Rules candidate is the spine; the LLM may change direction and prices, never quantity.
fn resolve(candidate: &Candidate, outcome: Outcome, now: DateTime<Utc>) -> Option<Resolved> {
    let valid_until = Some((now + chrono::Duration::hours(18)).to_rfc3339());
    match outcome {
        Outcome::Rules(c) => Some(Resolved {
            action: c.action,
            entry: c.entry,
            stop: c.stop,
            target: c.target,
            valid_until,
            qty: c.qty,
            reasons: c.reasons,
            rationale: None,
        }),
        Outcome::Llm(p) => Some(from_proposal(candidate, &p, valid_until)),
        // An over-budget proposal is NOT re-sized down to something that fits: it is carried
        // through to the budget gate, which records it as BLOCKED with the shortfall.
        Outcome::Rejected { proposal, error }
            if error == crate::llm::validate::ValidationError::InsufficientCash =>
        {
            Some(from_proposal(candidate, &proposal, valid_until))
        }
        Outcome::Rejected { .. } => Some(Resolved {
            action: candidate.action,
            entry: candidate.entry,
            stop: candidate.stop,
            target: candidate.target,
            valid_until,
            qty: candidate.qty,
            reasons: candidate.reasons.clone(),
            rationale: None,
        }),
        Outcome::Pass { .. } => None,
    }
}

fn from_proposal(candidate: &Candidate, p: &Proposal, fallback_until: Option<String>) -> Resolved {
    Resolved {
        action: p.action,
        entry: p.entry,
        stop: p.stop,
        target: p.target,
        valid_until: Some(p.valid_until.clone()).or(fallback_until),
        qty: candidate.qty,
        reasons: candidate.reasons.clone(),
        rationale: Some(p.rationale.clone()),
    }
}

async fn load_instruments(pool: &SqlitePool) -> Result<Vec<Instrument>> {
    let rows: Vec<(i64, String, String, String, String, i64)> = sqlx::query_as(
        "SELECT id, symbol, class, name, currency, active FROM instruments WHERE active = 1 ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|(id, symbol, class, name, currency, active)| {
            Ok(Instrument {
                id,
                symbol,
                class: AssetClass::parse(&class)?,
                name,
                currency,
                active: active != 0,
            })
        })
        .collect()
}

async fn load_position(pool: &SqlitePool, instrument_id: i64) -> Result<Option<Position>> {
    let row: Option<(i64, i64, f64, f64, Option<f64>, Option<f64>, String)> = sqlx::query_as(
        "SELECT id, instrument_id, qty, avg_cost, stop, target, state
         FROM positions WHERE instrument_id = ? AND state = 'open' ORDER BY id DESC LIMIT 1",
    )
    .bind(instrument_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| Position {
        id: r.0,
        instrument_id: r.1,
        qty: r.2,
        avg_cost: r.3,
        stop: r.4,
        target: r.5,
        state: r.6,
    }))
}

/// Refresh the entry on the last signal when it is still a live model decision. Returns
/// false when the stored signal was rules-only, has expired, or the price left the band.
async fn reuse_model_signal(
    pool: &SqlitePool,
    instrument_id: i64,
    price: f64,
    atr: f64,
    now: DateTime<Utc>,
) -> Result<bool> {
    let row: Option<(i64, Option<f64>, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT id, entry, valid_until, reason_json FROM signals
         WHERE instrument_id = ? AND action <> 'BLOCKED' ORDER BY id DESC LIMIT 1",
    )
    .bind(instrument_id)
    .fetch_optional(pool)
    .await?;

    let Some((id, entry, valid_until, reason)) = row else {
        return Ok(false);
    };
    let from_model = reason
        .as_deref()
        .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
        .and_then(|v| v.get("rationale").cloned())
        .is_some();
    let still_valid = valid_until
        .as_deref()
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
        .map(|d| d.with_timezone(&Utc) > now)
        .unwrap_or(false);
    let inside_band = entry
        .map(|e| (price - e).abs() <= atr.abs().max(1e-9))
        .unwrap_or(false);

    if from_model && still_valid && inside_band {
        sqlx::query("UPDATE signals SET entry = ? WHERE id = ?")
            .bind(price)
            .bind(id)
            .execute(pool)
            .await?;
        return Ok(true);
    }
    Ok(false)
}

fn stop_round(v: Option<f64>) -> Option<i64> {
    v.map(|x| (x * 100.0).round() as i64)
}

struct LastSignal {
    id: i64,
    action: String,
    entry: Option<f64>,
    stop: Option<f64>,
    target: Option<f64>,
}

async fn latest_signal(pool: &SqlitePool, instrument_id: i64) -> Result<Option<LastSignal>> {
    let row: Option<(i64, String, Option<f64>, Option<f64>, Option<f64>)> = sqlx::query_as(
        "SELECT id, action, entry, stop, target FROM signals
         WHERE instrument_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(instrument_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(id, action, entry, stop, target)| LastSignal {
        id,
        action,
        entry,
        stop,
        target,
    }))
}

async fn insert_signal(
    pool: &SqlitePool,
    instrument_id: i64,
    r: &Resolved,
    candidate: &Candidate,
    now: DateTime<Utc>,
) -> Result<i64> {
    let _ = candidate;
    let reason = serde_json::json!({
        "reasons": r.reasons,
        "rationale": r.rationale,
    })
    .to_string();
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO signals (instrument_id, action, qty, entry, stop, target, valid_until, reason_json, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(instrument_id)
    .bind(r.action.as_str())
    .bind(r.qty)
    .bind(r.entry)
    .bind(r.stop)
    .bind(r.target)
    .bind(&r.valid_until)
    .bind(reason)
    .bind(now.to_rfc3339())
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

async fn record_blocked(
    pool: &SqlitePool,
    instrument: &Instrument,
    r: &Resolved,
    candidate: &Candidate,
    available: f64,
    now: DateTime<Utc>,
) -> Result<()> {
    let cost = r.qty * r.entry;
    let shortfall = cost - available;
    let _ = candidate;
    let reason = serde_json::json!({
        "blocked": "insufficient_cash",
        "cost": cost,
        "available": available,
        "shortfall": shortfall,
        "reasons": r.reasons,
    })
    .to_string();
    sqlx::query(
        "INSERT INTO signals (instrument_id, action, qty, entry, stop, target, valid_until, reason_json, created_at)
         VALUES (?, 'BLOCKED', ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(instrument.id)
    .bind(r.qty)
    .bind(r.entry)
    .bind(r.stop)
    .bind(r.target)
    .bind(&r.valid_until)
    .bind(reason)
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// Half-width of the validator band, surfaced for the UI's chart rules.
pub fn band_for(last_close: f64, atr14: f64) -> indicators::Bands {
    atr_bands(last_close, atr14)
}

/// 15-minute tape, oldest first, timestamps in IST.
fn format_window(bars: &[Bar], hours: u32, now: DateTime<Utc>) -> String {
    let cutoff = now - chrono::Duration::hours(hours as i64);
    let ist = chrono::FixedOffset::east_opt(330 * 60).unwrap();
    let lines: Vec<String> = bars
        .iter()
        .filter(|b| {
            chrono::DateTime::parse_from_rfc3339(&b.ts)
                .map(|d| d.with_timezone(&Utc) >= cutoff)
                .unwrap_or(false)
        })
        .map(|b| {
            let t = chrono::DateTime::parse_from_rfc3339(&b.ts)
                .map(|d| d.with_timezone(&ist).format("%d %H:%M").to_string())
                .unwrap_or_else(|_| b.ts.clone());
            format!(
                "{} {:.2} {:.2} {:.2} {:.2} {}",
                t, b.open, b.high, b.low, b.close, b.volume
            )
        })
        .collect();
    if lines.is_empty() {
        "(no bars in the last window)".into()
    } else {
        lines.join("\n")
    }
}

fn format_daily(bars: &[Bar], days: usize) -> String {
    let tail: Vec<String> = bars
        .iter()
        .rev()
        .take(days)
        .rev()
        .map(|b| format!("{} {:.2}", &b.ts[..10.min(b.ts.len())], b.close))
        .collect();
    if tail.is_empty() {
        "(none)".into()
    } else {
        tail.join(", ")
    }
}

fn format_positions(position: Option<&Position>, symbol: &str) -> String {
    match position {
        None => format!("{symbol}: not held"),
        Some(p) => format!(
            "{} qty {:.3} at cost {:.2}, stop {}, target {}",
            symbol,
            p.qty,
            p.avg_cost,
            p.stop.map(|v| format!("{v:.2}")).unwrap_or_else(|| "none".into()),
            p.target.map(|v| format!("{v:.2}")).unwrap_or_else(|| "none".into()),
        ),
    }
}

async fn load_recent_trades(
    pool: &SqlitePool,
    instrument_id: i64,
    now: DateTime<Utc>,
) -> Result<String> {
    let cutoff = (now - chrono::Duration::days(7)).to_rfc3339();
    let rows: Vec<(f64, f64, String, String)> = sqlx::query_as(
        "SELECT qty, avg_cost, opened_at, state FROM positions
         WHERE instrument_id = ? AND opened_at >= ? ORDER BY opened_at DESC",
    )
    .bind(instrument_id)
    .bind(cutoff)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok("none in the last 7 days".into());
    }
    Ok(rows
        .into_iter()
        .map(|(qty, cost, at, state)| {
            format!(
                "{} BUY {:.3} @ {:.2} ({})",
                &at[..10.min(at.len())],
                qty,
                cost,
                state
            )
        })
        .collect::<Vec<_>>()
        .join("; "))
}

async fn load_last_decision(pool: &SqlitePool, instrument_id: i64) -> Result<String> {
    let row: Option<(String, Option<f64>, String, Option<String>)> = sqlx::query_as(
        "SELECT action, entry, created_at, reason_json FROM signals
         WHERE instrument_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(instrument_id)
    .fetch_optional(pool)
    .await?;
    Ok(match row {
        None => "none".into(),
        Some((action, entry, at, reason)) => {
            let rationale = reason
                .as_deref()
                .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
                .and_then(|v| v.get("rationale").and_then(|r| r.as_str()).map(str::to_string));
            format!(
                "{} {} @ {} — {}",
                &at[..10.min(at.len())],
                action,
                entry.map(|e| format!("{e:.2}")).unwrap_or_else(|| "-".into()),
                rationale.unwrap_or_else(|| "rules only".into())
            )
        }
    })
}
