use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::llm::deepseek::Transport;
use crate::llm::orchestrate::{self, LlmDeps, Outcome, RunInput};
use crate::llm::parse::Proposal;
use crate::models::{Action, AssetClass, Bar, Candidate, Headline, Instrument, Position};
use crate::notify::{self, Notification, Notifier};
use crate::rules::budget;
use crate::rules::evaluate::{self, CandidateInput, RuleConfig};
use crate::rules::indicators::{self, atr_bands, Indicators};

// Re-exported so tests and callers keep importing them from the engine.
pub use crate::market::{MarketSource, YahooSource};

pub struct EngineDeps<'a> {
    pub pool: &'a SqlitePool,
    /// Read at the top of every tick, so an edit in the UI applies without a restart.
    pub runtime: crate::rules::runtime::Runtime,
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
    /// None when the rules produced the signal with no model judgement behind it.
    confidence: Option<String>,
}

pub async fn poll_once(deps: &EngineDeps<'_>, now: DateTime<Utc>) -> Result<PollReport> {
    let mut report = PollReport::default();
    let instruments = load_instruments(deps.pool).await?;

    for instrument in instruments {
        match poll_instrument(deps, &instrument, now, &mut report).await {
            Ok(()) => {}
            Err(e) => report.errors.push(format!("{}: {e:#}", instrument.symbol)),
        }
    }

    report.llm_calls = orchestrate::daily_chat_calls(deps.pool).await.unwrap_or(0);
    Ok(report)
}

/// Everything one instrument's decision needs, gathered once. \`poll_instrument\` and \`advise\`
/// shared this logic by copy before, and had already drifted apart.
struct Gathered {
    bars: Vec<Bar>,
    news: Vec<Headline>,
    ind: Indicators,
    position: Option<Position>,
    available: f64,
    limits: crate::rules::limits::Limits,
    required_profit_pct: f64,
    pot: f64,
    instrument_exposure: f64,
    price_window: String,
    daily_summary: String,
    positions_text: String,
    recent_trades: String,
    last_decision: String,
}

impl Gathered {
    fn last_close(&self) -> f64 {
        self.bars.last().map(|b| b.close).unwrap_or_default()
    }
}

async fn gather(
    deps: &EngineDeps<'_>,
    instrument: &Instrument,
    now: DateTime<Utc>,
) -> Result<Option<Gathered>> {
    let bars = deps.market.bars(&instrument.symbol).await?;
    let Ok(ind) = indicators::compute(&bars) else {
        tracing::debug!(symbol = %instrument.symbol, "not enough history for indicators");
        return Ok(None);
    };
    let news = deps
        .market
        .news(&instrument.symbol)
        .await
        .unwrap_or_default();
    crate::store::bars(deps.pool, instrument.id, &bars).await?;
    crate::store::headlines(deps.pool, instrument.id, &news).await?;

    let position = load_position(deps.pool, instrument.id).await?;
    let available = budget::available_in(deps.pool, crate::models::Mode::Long).await?;
    let limits = crate::rules::settings::limits(deps.pool, crate::models::Mode::Long).await?;
    let pot = budget::budget_total_in(deps.pool, crate::models::Mode::Long).await?;
    let instrument_exposure = load_exposure(deps.pool, instrument.id).await?;
    // No per-stock value means "the pot's minimum"; a value above the ceiling is clamped, so a
    // lowered max cannot make every trade on this stock impossible.
    let required_profit_pct = load_target_profit(deps.pool, instrument.id)
        .await?
        .unwrap_or(limits.min_profit_pct)
        .clamp(limits.min_profit_pct, limits.max_profit_pct);
    let price_window = match deps.market.intraday(&instrument.symbol, "15m", "5d").await {
        Ok(tape) => format_window(&tape, deps.runtime.prompt_window_hours, now),
        Err(e) => {
            tracing::debug!(symbol = %instrument.symbol, error = %e, "no intraday tape");
            "(intraday tape unavailable)".to_string()
        }
    };

    Ok(Some(Gathered {
        daily_summary: format_daily(&bars, 10),
        positions_text: format_positions(position.as_ref(), &instrument.symbol),
        recent_trades: load_recent_trades(deps.pool, instrument.id, now).await?,
        last_decision: load_last_decision(deps.pool, instrument.id).await?,
        limits,
        required_profit_pct,
        pot,
        instrument_exposure,
        bars,
        news,
        ind,
        position,
        available,
        price_window,
    }))
}

async fn poll_instrument(
    deps: &EngineDeps<'_>,
    instrument: &Instrument,
    now: DateTime<Utc>,
    report: &mut PollReport,
) -> Result<()> {
    let Some(g) = gather(deps, instrument, now).await? else {
        return Ok(());
    };
    let (bars, news, ind) = (&g.bars, &g.news, &g.ind);
    let (position, available) = (&g.position, g.available);

    let rules_candidate = evaluate::candidate(CandidateInput {
        instrument,
        bars,
        ind,
        position: position.as_ref(),
        available_cash: available,
        cfg: deps.rules,
    });

    // The rules no longer decide whether the model is asked — they describe the setup. A held
    // position with no fresh headlines still deserves a view on prices alone.
    let had_rules_candidate = rules_candidate.is_some();
    let candidate = match rules_candidate {
        Some(c) => c,
        None => {
            // Say why, per instrument: "the ledger is empty" is not an answer on its own.
            let failed: Vec<String> = evaluate::diagnose(&CandidateInput {
                instrument,
                bars,
                ind,
                position: position.as_ref(),
                available_cash: available,
                cfg: deps.rules,
            })
            .into_iter()
            .filter(|c| !c.passed)
            .map(|c| format!("{} — {}", c.name, c.detail))
            .collect();
            report.notes.push(format!(
                "{}: rules see no entry ({}) — asking the model on prices alone",
                instrument.symbol,
                if failed.is_empty() {
                    "no rule fired".to_string()
                } else {
                    failed.join("; ")
                }
            ));
            Candidate {
                instrument_id: instrument.id,
                action: Action::Hold,
                reasons: Vec::new(),
                entry: bars.last().map(|b| b.close).unwrap_or_default(),
                stop: None,
                target: None,
                qty: (available / bars.last().map(|b| b.close).unwrap_or(1.0).max(1e-9)).floor(),
                timing_note: instrument.class.timing_note().into(),
                urgent: false,
            }
        }
    };
    // Limits bind the rules too, not just the model: a 200k fill against a 50k ceiling is
    // not acceptable because a rule suggested it.
    let candidate = match clamp_to_limits(candidate, g.limits, g.pot, g.instrument_exposure) {
        Some(c) => c,
        None => {
            report.notes.push(format!(
                "{}: no affordable size inside the limits ({:.2} to {:.2} per trade)",
                instrument.symbol, g.limits.min_trade_amount, g.limits.max_trade_amount
            ));
            return Ok(());
        }
    };
    if had_rules_candidate {
        report.candidates += 1;
    }

    // The model is paid for judgement about news, so news is what re-triggers it. Prices can
    // move all day without another call; the rules refresh them.
    let headline_fp = crate::llm::memo::fingerprint(news);
    let headlines_changed = !crate::llm::memo::unchanged(deps.pool, instrument.id, &headline_fp)
        .await
        .unwrap_or(true);

    // Do not spend a call on a price that has not moved, but never leave a stock unwatched
    // for more than the gate's interval.
    if !deps.force {
        let gate = crate::llm::memo::CallGate {
            min_move: deps.runtime.min_price_move,
            max_interval_secs: deps.runtime.max_silence_secs,
        };
        if !crate::llm::memo::should_call(
            deps.pool,
            instrument.id,
            candidate.entry,
            now,
            gate,
            headlines_changed,
        )
        .await?
        {
            report.notes.push(format!(
                "{}: price moved less than {:.2} and no new headlines — model not asked",
                instrument.symbol, gate.min_move
            ));
            // The decision stands, but its price should not age: refresh the entry only.
            let _ = reuse_model_signal(deps.pool, instrument.id, candidate.entry, ind.atr14, now)
                .await?;
            return Ok(());
        }
    }

    // No key configured means no model layer at all — not a failed call on every tick.
    let llm_available = crate::rules::settings::deepseek_key(deps.pool)
        .await?
        .is_some();
    if !llm_available {
        report.notes.push(format!(
            "{}: no DeepSeek key — rules only",
            instrument.symbol
        ));
        let resolved = Resolved {
            action: candidate.action,
            entry: candidate.entry,
            stop: candidate.stop,
            target: candidate.target,
            valid_until: Some((now + chrono::Duration::hours(18)).to_rfc3339()),
            qty: candidate.qty,
            reasons: candidate.reasons.clone(),
            rationale: None,
            confidence: None,
        };
        return finish(
            deps,
            instrument,
            resolved,
            &candidate,
            Sizing {
                available,
                atr: ind.atr14,
            },
            now,
            report,
        )
        .await;
    }

    let llm_deps = LlmDeps {
        pool: deps.pool,
        runtime: deps.runtime.clone(),
        transport: deps.transport,
        market: deps.market,
    };
    let decision = orchestrate::run(
        &llm_deps,
        RunInput {
            instrument,
            ind,
            candidate: &candidate,
            available_cash: available,
            held_qty: position.as_ref().map(|p| p.qty).unwrap_or(0.0),
            limits: g.limits,
            required_profit_pct: g.required_profit_pct,
            instrument_exposure: g.instrument_exposure,
            pot: g.pot,
            window_hours: deps.runtime.prompt_window_hours,
            headlines: news,
            price_window: &g.price_window,
            daily_summary: &g.daily_summary,
            positions: &g.positions_text,
            recent_trades: &g.recent_trades,
            last_decision: &g.last_decision,
            now,
        },
    )
    .await?;

    // The model answered for this headline set; do not ask it about the same news again.
    if !matches!(decision.outcome, Outcome::Rules(_)) {
        crate::llm::memo::remember(deps.pool, instrument.id, &headline_fp).await?;
    }

    // Recorded whatever the answer was, so a failing key cannot loop on every tick.
    crate::llm::memo::record_call(deps.pool, instrument.id, candidate.entry, now, &headline_fp)
        .await?;

    let Some(resolved) = resolve(&candidate, decision.outcome, now) else {
        return Ok(());
    };

    finish(
        deps,
        instrument,
        resolved,
        &candidate,
        Sizing {
            available,
            atr: ind.atr14,
        },
        now,
        report,
    )
    .await
}

/// Ask the model about one instrument on demand, bypassing the rules gate, and write nothing.
/// This is the "second opinion" path: the rules may see no setup, but a human still wants the
/// model's read before deciding to do nothing.
pub async fn advise(
    deps: &EngineDeps<'_>,
    instrument: &Instrument,
    now: DateTime<Utc>,
) -> Result<Option<String>> {
    let Some(g) = gather(deps, instrument, now).await? else {
        return Ok(Some("not enough stored history for indicators".to_string()));
    };

    // A neutral stand-in: the prompt uses entry and quantity, not the rules' opinion.
    let neutral = Candidate {
        instrument_id: instrument.id,
        action: Action::Hold,
        reasons: Vec::new(),
        entry: g.last_close(),
        stop: None,
        target: None,
        qty: (g.available / g.last_close().max(1e-9)).floor().max(0.0),
        timing_note: instrument.class.timing_note().into(),
        urgent: false,
    };

    let llm_deps = LlmDeps {
        pool: deps.pool,
        runtime: deps.runtime.clone(),
        transport: deps.transport,
        market: deps.market,
    };
    let decision = orchestrate::run(
        &llm_deps,
        RunInput {
            instrument,
            ind: &g.ind,
            candidate: &neutral,
            available_cash: g.available,
            held_qty: g.position.as_ref().map(|p| p.qty).unwrap_or(0.0),
            limits: g.limits,
            required_profit_pct: g.required_profit_pct,
            instrument_exposure: g.instrument_exposure,
            pot: g.pot,
            window_hours: deps.runtime.prompt_window_hours,
            headlines: &g.news,
            price_window: &g.price_window,
            daily_summary: &g.daily_summary,
            positions: &g.positions_text,
            recent_trades: &g.recent_trades,
            last_decision: &g.last_decision,
            now,
        },
    )
    .await?;

    Ok(Some(match decision.outcome {
        Outcome::Llm(p) => format!(
            "{} {:?} entry {:.2} stop {} target {} confidence {} · {}",
            instrument.symbol,
            p.action,
            p.entry,
            p.stop
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "—".into()),
            p.target
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "—".into()),
            p.confidence.as_str(),
            p.rationale
        ),
        Outcome::Pass { why } => format!("{} PASS · {why}", instrument.symbol),
        Outcome::Rejected { proposal, error } => format!(
            "{} model proposed {:?} entry {:.2} stop {} target {} but it was rejected: {error}",
            instrument.symbol,
            proposal.action,
            proposal.entry,
            proposal
                .stop
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "—".into()),
            proposal
                .target
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "—".into()),
        ),
        Outcome::Rules(_) => format!(
            "{} no usable model answer, rules unchanged",
            instrument.symbol
        ),
    }))
}

/// Signal persistence, the budget gate and the push — shared by the LLM and rules-only paths.
/// Price context the gate and the risk checks both need.
#[derive(Debug, Clone, Copy)]
struct Sizing {
    available: f64,
    atr: f64,
}

async fn finish(
    deps: &EngineDeps<'_>,
    instrument: &Instrument,
    resolved: Resolved,
    candidate: &Candidate,
    sizing: Sizing,
    now: DateTime<Utc>,
    report: &mut PollReport,
) -> Result<()> {
    // A HOLD is the absence of a decision, not a decision to record.
    if !matches!(resolved.action, Action::Buy | Action::Sell) {
        return Ok(());
    }

    let available = sizing.available;

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
        resolved
            .stop
            .map(|v| format!("{v:.2}"))
            .unwrap_or_else(|| "-".into()),
        resolved
            .target
            .map(|v| format!("{v:.2}"))
            .unwrap_or_else(|| "-".into()),
        resolved.valid_until.clone().unwrap_or_else(|| "-".into()),
        resolved
            .rationale
            .clone()
            .unwrap_or_else(|| "rules only".into()),
    ));

    // A model-derived signal that is still valid and still inside its band survives a failed
    // model call: the rules refresh the price but must not overwrite a view they did not form.
    if resolved.rationale.is_none()
        && reuse_model_signal(deps.pool, instrument.id, resolved.entry, sizing.atr, now).await?
    {
        return Ok(());
    }

    // "Prices refresh freely" means the entry may drift without a new row or a push: the
    // latest signal is updated in place. A different action, stop or target is new information
    // and gets its own row.
    if !deps.force {
        if let Some(prev) = latest_signal(deps.pool, instrument.id).await? {
            let same_shape = prev.action == resolved.action.as_str()
                && stop_round(prev.stop) == stop_round(resolved.stop)
                && stop_round(prev.target) == stop_round(resolved.target);
            if same_shape {
                if prev
                    .entry
                    .is_some_and(|e| (e - resolved.entry).abs() >= 0.005)
                {
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

    if let Some(mut n) =
        Notification::from_candidate(&instrument.symbol, &instrument.currency, &as_candidate)
    {
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
            confidence: None,
        }),
        Outcome::Llm(p) => Some(from_proposal(candidate, &p, valid_until)),
        // An over-budget proposal is NOT re-sized down to something that fits: it is carried
        // through to the budget gate, which records it as BLOCKED with the shortfall.
        Outcome::Rejected {
            proposal,
            error: crate::llm::validate::ValidationError::InsufficientCash,
        } => Some(from_proposal(candidate, &proposal, valid_until)),
        Outcome::Rejected { .. } => Some(Resolved {
            action: candidate.action,
            entry: candidate.entry,
            stop: candidate.stop,
            target: candidate.target,
            valid_until,
            qty: candidate.qty,
            reasons: candidate.reasons.clone(),
            rationale: None,
            confidence: None,
        }),
        Outcome::Pass { .. } => None,
    }
}

fn from_proposal(candidate: &Candidate, p: &Proposal, fallback_until: Option<String>) -> Resolved {
    Resolved {
        action: p.action,
        // The model may size the trade; otherwise the rules' sizing stands.
        qty: p.qty.unwrap_or(candidate.qty),
        entry: p.entry,
        stop: p.stop,
        target: p.target,
        valid_until: Some(p.valid_until.clone()).or(fallback_until),
        reasons: candidate.reasons.clone(),
        rationale: Some(p.rationale.clone()),
        confidence: Some(p.confidence.as_str().to_string()),
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

/// Cap a candidate's size to the pot's limits, or drop it if nothing usable is left.
fn clamp_to_limits(
    mut c: Candidate,
    limits: crate::rules::limits::Limits,
    pot: f64,
    exposure: f64,
) -> Option<Candidate> {
    if !matches!(c.action, Action::Buy | Action::Sell) {
        return Some(c);
    }
    let max_fill = limits.max_fill(pot, exposure);
    if max_fill <= 0.0 {
        return None;
    }
    let max_units = max_fill / c.entry.max(1e-9);
    if c.qty > max_units {
        c.qty = match c.action {
            Action::Sell => c.qty.min(max_units),
            _ => max_units.floor(),
        };
    }
    if c.qty <= 0.0 {
        return None;
    }
    // A buy below the minimum is not worth its fees; a sale of a tiny remainder still is.
    if c.action == Action::Buy && c.qty * c.entry < limits.min_trade_amount {
        return None;
    }
    Some(c)
}

async fn load_exposure(pool: &SqlitePool, instrument_id: i64) -> Result<f64> {
    let row: (Option<f64>,) = sqlx::query_as(
        "SELECT SUM(qty * avg_cost) FROM positions
         WHERE instrument_id = ? AND mode = 'long' AND state <> 'closed'",
    )
    .bind(instrument_id)
    .fetch_one(pool)
    .await?;
    Ok(row.0.unwrap_or(0.0))
}

async fn load_target_profit(pool: &SqlitePool, instrument_id: i64) -> Result<Option<f64>> {
    let row: Option<(Option<f64>,)> =
        sqlx::query_as("SELECT target_profit_pct FROM instruments WHERE id = ?")
            .bind(instrument_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.and_then(|r| r.0))
}

#[derive(sqlx::FromRow)]
struct PositionDbRow {
    id: i64,
    instrument_id: i64,
    qty: f64,
    avg_cost: f64,
    stop: Option<f64>,
    target: Option<f64>,
    state: String,
}

async fn load_position(pool: &SqlitePool, instrument_id: i64) -> Result<Option<Position>> {
    let row: Option<PositionDbRow> = sqlx::query_as(
        "SELECT id AS id, instrument_id AS instrument_id, qty AS qty, avg_cost AS avg_cost,
                stop AS stop, target AS target, state AS state
         FROM positions WHERE instrument_id = ? AND state = 'open' ORDER BY id DESC LIMIT 1",
    )
    .bind(instrument_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| Position {
        id: r.id,
        instrument_id: r.instrument_id,
        qty: r.qty,
        avg_cost: r.avg_cost,
        stop: r.stop,
        target: r.target,
        state: r.state,
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
    #[derive(sqlx::FromRow)]
    struct Row {
        id: i64,
        entry: Option<f64>,
        valid_until: Option<String>,
        reason_json: Option<String>,
    }
    let row: Option<Row> = sqlx::query_as(
        "SELECT id AS id, entry AS entry, valid_until AS valid_until,
                reason_json AS reason_json FROM signals
         WHERE instrument_id = ? AND action <> 'BLOCKED' ORDER BY id DESC LIMIT 1",
    )
    .bind(instrument_id)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Ok(false);
    };
    let (id, entry, valid_until) = (row.id, row.entry, row.valid_until);
    let from_model = row
        .reason_json
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

#[derive(sqlx::FromRow)]
struct LastSignal {
    id: i64,
    action: String,
    entry: Option<f64>,
    stop: Option<f64>,
    target: Option<f64>,
}

async fn latest_signal(pool: &SqlitePool, instrument_id: i64) -> Result<Option<LastSignal>> {
    let row: Option<LastSignal> = sqlx::query_as(
        "SELECT id AS id, action AS action, entry AS entry, stop AS stop, target AS target
         FROM signals WHERE instrument_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(instrument_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
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
        "INSERT INTO signals (instrument_id, action, qty, entry, stop, target, valid_until,
            reason_json, created_at, confidence)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
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
    .bind(&r.confidence)
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
        "INSERT INTO signals (instrument_id, action, qty, entry, stop, target, valid_until,
            reason_json, created_at)
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
            p.stop
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "none".into()),
            p.target
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "none".into()),
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
                .and_then(|v| {
                    v.get("rationale")
                        .and_then(|r| r.as_str())
                        .map(str::to_string)
                });
            format!(
                "{} {} @ {} — {}",
                &at[..10.min(at.len())],
                action,
                entry
                    .map(|e| format!("{e:.2}"))
                    .unwrap_or_else(|| "-".into()),
                rationale.unwrap_or_else(|| "rules only".into())
            )
        }
    })
}
