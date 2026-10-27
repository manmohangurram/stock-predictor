use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::config::Config;
use crate::llm::deepseek::Transport;
use crate::llm::orchestrate::{self, LlmDeps, Outcome, RunInput};
use crate::llm::parse::Proposal;
use crate::models::{Action, AssetClass, Bar, Candidate, Headline, Instrument, Position};
use crate::notify::{self, Notification, Notifier};
use crate::rules::budget;
use crate::rules::evaluate::{self, CandidateInput, RuleConfig};
use crate::rules::indicators::{self, atr_bands};

pub trait MarketSource: Send + Sync {
    fn bars<'a>(
        &'a self,
        symbol: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Bar>>> + Send + 'a>>;
    fn news<'a>(
        &'a self,
        symbol: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Headline>>> + Send + 'a>>;
}

pub struct YahooSource {
    client: reqwest::Client,
}

impl YahooSource {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: crate::market::yahoo::client()?,
        })
    }
}

impl MarketSource for YahooSource {
    fn bars<'a>(
        &'a self,
        symbol: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Bar>>> + Send + 'a>> {
        Box::pin(async move { crate::market::yahoo::fetch_bars(&self.client, symbol).await })
    }

    fn news<'a>(
        &'a self,
        symbol: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Headline>>> + Send + 'a>> {
        Box::pin(async move { crate::market::yahoo::fetch_news(&self.client, symbol).await })
    }
}

pub struct EngineDeps<'a> {
    pub pool: &'a SqlitePool,
    pub cfg: &'a Config,
    pub market: &'a dyn MarketSource,
    pub transport: &'a dyn Transport,
    pub notifier: &'a dyn Notifier,
    pub rules: RuleConfig,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PollReport {
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
    for h in &news {
        sqlx::query(
            "INSERT OR IGNORE INTO news (instrument_id, published_at, title, publisher, url)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(instrument.id)
        .bind(&h.published_at)
        .bind(&h.title)
        .bind(&h.publisher)
        .bind(&h.url)
        .execute(deps.pool)
        .await?;
    }

    // Bars are stored so the UI can chart what the decision was made from, and so a
    // backtest can be run later on exactly what the engine saw.
    for b in &bars {
        sqlx::query(
            "INSERT INTO bars (instrument_id, ts, open, high, low, close, volume)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(instrument_id, ts) DO UPDATE SET
               open = excluded.open, high = excluded.high, low = excluded.low,
               close = excluded.close, volume = excluded.volume",
        )
        .bind(instrument.id)
        .bind(&b.ts)
        .bind(b.open)
        .bind(b.high)
        .bind(b.low)
        .bind(b.close)
        .bind(b.volume)
        .execute(deps.pool)
        .await?;
    }

    let ind = match indicators::compute(&bars) {
        Ok(i) => i,
        Err(e) => {
            tracing::debug!(symbol = %instrument.symbol, error = %e, "no indicators, skipping");
            return Ok(());
        }
    };

    let position = load_position(deps.pool, instrument.id).await?;
    let available = budget::available(deps.pool).await?;

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

    // Cost gate: if the rules layer proposes exactly what it proposed last time, the model
    // would be asked the same question again for the same answer. Skip the call.
    if is_same_candidate(deps.pool, instrument.id, &candidate).await? {
        return Ok(());
    }

    let llm_deps = LlmDeps {
        pool: deps.pool,
        cfg: deps.cfg,
        transport: deps.transport,
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
            search_summaries: &[],
        },
    )
    .await?;

    let Some(resolved) = resolve(&candidate, decision.outcome, now) else {
        return Ok(());
    };

    // The budget gate is the last word, above the LLM: a proposal that does not fit is
    // recorded as BLOCKED with the shortfall, and is never pushed.
    if resolved.action == Action::Buy {
        let cost = resolved.qty * resolved.entry;
        if !budget::fits(deps.pool, cost).await? {
            record_blocked(deps.pool, instrument, &resolved, &candidate, available, now).await?;
            report.blocked += 1;
            return Ok(());
        }
    }

    if is_duplicate(deps.pool, instrument.id, &resolved).await? {
        return Ok(());
    }

    let signal_id = insert_signal(deps.pool, instrument.id, &resolved, &candidate, now).await?;
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

async fn is_duplicate(pool: &SqlitePool, instrument_id: i64, r: &Resolved) -> Result<bool> {
    let last: Option<(String, Option<f64>, Option<f64>)> = sqlx::query_as(
        "SELECT action, entry, stop FROM signals WHERE instrument_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(instrument_id)
    .fetch_optional(pool)
    .await?;

    Ok(match last {
        Some((action, entry, stop)) => {
            action == r.action.as_str()
                && entry.is_some_and(|e| (e - r.entry).abs() < 0.005)
                && stop_round(stop) == stop_round(r.stop)
        }
        None => false,
    })
}

fn stop_round(v: Option<f64>) -> Option<i64> {
    v.map(|x| (x * 100.0).round() as i64)
}

fn fingerprint(c: &Candidate) -> serde_json::Value {
    serde_json::json!({
        "rules_action": c.action.as_str(),
        "rules_entry": (c.entry * 100.0).round() / 100.0,
        "rules_stop": c.stop.map(|s| (s * 100.0).round() / 100.0),
    })
}

async fn is_same_candidate(
    pool: &SqlitePool,
    instrument_id: i64,
    c: &Candidate,
) -> Result<bool> {
    let last: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT reason_json FROM signals WHERE instrument_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(instrument_id)
    .fetch_optional(pool)
    .await?;
    let Some((Some(json),)) = last else {
        return Ok(false);
    };
    let prev: serde_json::Value = serde_json::from_str(&json).unwrap_or_default();
    let want = fingerprint(c);
    Ok(prev.get("rules_action") == want.get("rules_action")
        && prev.get("rules_entry") == want.get("rules_entry")
        && prev.get("rules_stop") == want.get("rules_stop"))
}

async fn insert_signal(
    pool: &SqlitePool,
    instrument_id: i64,
    r: &Resolved,
    candidate: &Candidate,
    now: DateTime<Utc>,
) -> Result<i64> {
    let mut reason = serde_json::json!({
        "reasons": r.reasons,
        "rationale": r.rationale,
    });
    if let (Some(dst), Some(src)) = (reason.as_object_mut(), fingerprint(candidate).as_object()) {
        for (k, v) in src {
            dst.insert(k.clone(), v.clone());
        }
    }
    let reason = reason.to_string();
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
    let mut reason = serde_json::json!({
        "blocked": "insufficient_cash",
        "cost": cost,
        "available": available,
        "shortfall": shortfall,
        "reasons": r.reasons,
    });
    if let (Some(dst), Some(src)) = (reason.as_object_mut(), fingerprint(candidate).as_object()) {
        for (k, v) in src {
            dst.insert(k.clone(), v.clone());
        }
    }
    let reason = reason.to_string();
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
