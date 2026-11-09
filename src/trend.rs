//! Short-term mode: what moved today, and whether it keeps moving.
//!
//! Runs on every poll tick and asks about each of the day's top gainers **only when that symbol has
//! actually changed** — its price moved, or its headlines changed — using the same gate as the
//! long-term path. A flat name costs nothing however often this runs.
//!
//! Discovery never starts polling on its own: a new symbol is stored inactive, and approving its
//! decision activates it.

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::llm::{memo, parse, validate};
use crate::models::{Action, Bar, Mode};

const TOP_N: usize = 10;
/// The book is short-term: nothing is proposed beyond a week out.
const MAX_HORIZON_HOURS: i64 = 24 * 7;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct TrendReport {
    pub examined: usize,
    pub called: usize,
    pub proposed: usize,
    pub notes: Vec<String>,
}

/// The decision row a trend proposal becomes.
struct Proposal {
    instrument_id: i64,
    entry: f64,
    stop: f64,
    target: f64,
    qty: f64,
    confidence: String,
    rationale: String,
    expires: String,
}

pub async fn run(
    pool: &SqlitePool,
    market: &dyn crate::market::MarketSource,
    transport: &dyn crate::llm::deepseek::Transport,
    runtime: &crate::rules::runtime::Runtime,
    now: DateTime<Utc>,
) -> Result<TrendReport> {
    let mut report = TrendReport::default();

    let client = crate::http::client_with(runtime, 20, Some("Mozilla/5.0 stock-predictor"))?;
    let gainers = crate::market::nse::fetch_gainers(&client, TOP_N).await?;
    report.examined = gainers.len();
    if gainers.is_empty() {
        report.notes.push("no gainers returned by NSE".into());
        return Ok(report);
    }

    let settings = crate::rules::settings::get(pool).await?;
    let limits = settings.limits_trade;
    let cash = crate::rules::budget::available_in(pool, Mode::Trade).await?;
    let pot = crate::rules::budget::cash_in(pool, Mode::Trade).await?;
    let model = crate::rules::settings::model_config(pool).await?;

    for g in gainers {
        let symbol = g.yahoo_symbol();
        let Some(id) = ensure_instrument(pool, &symbol, &g.symbol).await? else {
            report
                .notes
                .push(format!("{symbol}: already held in the trading pot, skipped"));
            continue;
        };

        // Gate first, network second: ten refresh pairs per tick is the expensive part, and most
        // of these names have not moved since the last pass.
        let headlines_changed = news_newer_than_last_call(pool, id).await?;
        let gate = memo::CallGate {
            min_move: runtime.min_price_move,
            max_interval_secs: runtime.max_silence_secs,
        };
        if !memo::should_call(pool, id, g.ltp, now, gate, headlines_changed).await? {
            continue;
        }
        // Only now is the tape worth fetching.
        if let Err(e) = crate::store::refresh(pool, id, &symbol, market).await {
            report.notes.push(format!("{symbol}: {e}"));
            continue;
        }

        let bars = load_bars(pool, id).await?;
        if bars.len() < 30 {
            continue;
        }
        let headlines = recent_headlines(pool, id).await?;
        let prompt = crate::llm::prompts::render_trend(
            &format!(
                "{} · {:.2} now, {:+.2}% today, previous close {:.2}, turnover {:.0}",
                symbol,
                g.ltp,
                g.per_change,
                g.previous_close,
                g.turnover.unwrap_or(0.0)
            ),
            &tape(&bars),
            &headlines,
            &format!("cash {} {:.2}, allowance {:.2}", settings.currency, cash, pot),
        )?;

        report.called += 1;
        let reply = match crate::llm::deepseek::chat(
            pool,
            &model,
            transport,
            crate::llm::deepseek::ChatRequest {
                conversation_id: format!("trend-{symbol}-{}", now.timestamp_millis()),
                round: 1,
                prompt: prompt.text,
                prompt_template: prompt.template,
                prompt_hash: prompt.hash,
                trigger: Some(if headlines_changed {
                    format!(
                        "short-term scan: {:+.2}% today, new headlines since the last call",
                        g.per_change
                    )
                } else {
                    format!("short-term scan: {:+.2}% today, price moved", g.per_change)
                }),
                purpose: format!("trend {symbol}"),
            },
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                report.notes.push(format!("{symbol}: call failed: {e}"));
                continue;
            }
        };

        let proposal = match parse::verdict(&reply.content) {
            Ok(parse::LlmVerdict::Decide(p)) => p,
            Ok(parse::LlmVerdict::Pass { why }) => {
                report.notes.push(format!("{symbol}: pass — {why}"));
                continue;
            }
            Ok(parse::LlmVerdict::NeedMore(_)) => {
                report.notes.push(format!("{symbol}: asked for more data"));
                continue;
            }
            Err(e) => {
                report.notes.push(format!("{symbol}: unreadable reply: {e}"));
                continue;
            }
        };
        if proposal.action != Action::Buy {
            report.notes.push(format!("{symbol}: {:?} is not a short-term entry", proposal.action));
            continue;
        }

        let (Some(stop), Some(target)) = (proposal.stop, proposal.target) else {
            report.notes.push(format!("{symbol}: no exit plan"));
            continue;
        };
        let ind = match crate::rules::indicators::compute(&bars) {
            Ok(i) => i,
            Err(e) => {
                report.notes.push(format!("{symbol}: {e}"));
                continue;
            }
        };
        let qty = proposal
            .qty
            .filter(|q| q.is_finite() && *q > 0.0)
            .unwrap_or_else(|| (cash / proposal.entry).floor());
        let check = validate::check(
            &proposal,
            &validate::ValidateCtx {
                bands: crate::engine::band_for(bars[bars.len() - 1].close, ind.atr14),
                atr: ind.atr14,
                available_cash: cash,
                qty,
                held_qty: 0.0,
                sent_headline_ids: &[],
                requires_exit: true,
                limits,
                required_profit_pct: limits.min_profit_pct,
                instrument_exposure: 0.0,
                pot,
                now,
            },
        );
        if let Err(e) = check {
            report.notes.push(format!("{symbol}: blocked — {e}"));
            continue;
        }

        let expires = clamp_expiry(&proposal.valid_until, now)
            .unwrap_or_else(|| (now + chrono::Duration::hours(MAX_HORIZON_HOURS)).to_rfc3339());
        let row = Proposal {
            instrument_id: id,
            entry: proposal.entry,
            stop,
            target,
            qty,
            confidence: proposal.confidence.as_str().to_string(),
            rationale: proposal.rationale.clone(),
            expires,
        };
        store(pool, &row, now).await?;
        report.proposed += 1;
        report
            .notes
            .push(format!("{symbol}: proposed {qty:.0} units at {:.2}", row.entry));
    }

    Ok(report)
}

/// The instrument for a discovered symbol, created inactive so discovery never starts polling.
async fn ensure_instrument(
    pool: &SqlitePool,
    yahoo_symbol: &str,
    short_name: &str,
) -> Result<Option<i64>> {
    let existing: Option<(i64, i64)> =
        sqlx::query_as("SELECT id, active FROM instruments WHERE symbol = ?")
            .bind(yahoo_symbol)
            .fetch_optional(pool)
            .await?;
    if let Some((id, _)) = existing {
        let held: (f64,) = sqlx::query_as(
            "SELECT CAST(COALESCE(SUM(qty),0) AS REAL) FROM positions
             WHERE instrument_id = ? AND state <> 'closed' AND mode = 'trade'",
        )
        .bind(id)
        .fetch_one(pool)
        .await?;
        return Ok(if held.0 > 0.0 { None } else { Some(id) });
    }
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO instruments (symbol, class, name, currency, active)
         VALUES (?, 'stock', ?, 'INR', 0) RETURNING id",
    )
    .bind(yahoo_symbol)
    .bind(short_name)
    .fetch_one(pool)
    .await?;
    Ok(Some(row.0))
}

async fn news_newer_than_last_call(pool: &SqlitePool, instrument_id: i64) -> Result<bool> {
    let last: Option<(Option<String>,)> =
        sqlx::query_as("SELECT last_call_at FROM llm_memos WHERE instrument_id = ?")
            .bind(instrument_id)
            .fetch_optional(pool)
            .await?;
    let Some((Some(at),)) = last else {
        return Ok(true);
    };
    let fresh: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM news WHERE instrument_id = ? AND published_at > ?",
    )
    .bind(instrument_id)
    .bind(at)
    .fetch_one(pool)
    .await?;
    Ok(fresh.0 > 0)
}

async fn load_bars(pool: &SqlitePool, instrument_id: i64) -> Result<Vec<Bar>> {
    let rows: Vec<(String, f64, f64, f64, f64, i64)> = sqlx::query_as(
        "SELECT ts, open, high, low, close, volume FROM bars WHERE instrument_id = ?
         ORDER BY ts DESC LIMIT 60",
    )
    .bind(instrument_id)
    .fetch_all(pool)
    .await?;
    let mut bars: Vec<Bar> = rows
        .into_iter()
        .map(|(ts, open, high, low, close, volume)| Bar {
            ts,
            open,
            high,
            low,
            close,
            volume,
        })
        .collect();
    bars.reverse();
    Ok(bars)
}

async fn recent_headlines(pool: &SqlitePool, instrument_id: i64) -> Result<String> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT title, published_at FROM news WHERE instrument_id = ?
         ORDER BY published_at DESC LIMIT 4",
    )
    .bind(instrument_id)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok("(no headlines)".into());
    }
    Ok(rows
        .into_iter()
        .map(|(t, at)| format!("[{}] {t}", &at[..10.min(at.len())]))
        .collect::<Vec<_>>()
        .join("\n"))
}

fn tape(bars: &[Bar]) -> String {
    bars.iter()
        .rev()
        .take(8)
        .map(|b| format!("{} close {:.2}", &b.ts[..10.min(b.ts.len())], b.close))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A proposal is never allowed past the book's horizon, whatever date it asked for.
fn clamp_expiry(asked: &str, now: DateTime<Utc>) -> Option<String> {
    let parsed = chrono::DateTime::parse_from_rfc3339(asked)
        .ok()?
        .with_timezone(&Utc);
    let cap = now + chrono::Duration::hours(MAX_HORIZON_HOURS);
    Some(parsed.min(cap).to_rfc3339())
}

async fn store(pool: &SqlitePool, p: &Proposal, now: DateTime<Utc>) -> Result<()> {
    let reason = serde_json::json!({
        "reasons": [],
        "rationale": p.rationale,
        "source": "nse gainers",
    })
    .to_string();
    sqlx::query(
        "INSERT INTO signals (instrument_id, action, qty, entry, stop, target, valid_until,
            reason_json, created_at, confidence, mode, kind, status)
         VALUES (?, 'BUY', ?, ?, ?, ?, ?, ?, ?, ?, 'trade', 'trend', 'pending')",
    )
    .bind(p.instrument_id)
    .bind(p.qty)
    .bind(p.entry)
    .bind(p.stop)
    .bind(p.target)
    .bind(&p.expires)
    .bind(reason)
    .bind(now.to_rfc3339())
    .bind(&p.confidence)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(ts: &str, close: f64) -> Bar {
        Bar {
            ts: ts.into(),
            open: close,
            high: close,
            low: close,
            close,
            volume: 1,
        }
    }

    #[test]
    fn a_proposal_is_never_allowed_past_a_week() {
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 11, 9, 10, 0, 0).unwrap();
        let asked = (now + chrono::Duration::days(30)).to_rfc3339();
        let got = clamp_expiry(&asked, now).unwrap();
        let parsed = chrono::DateTime::parse_from_rfc3339(&got).unwrap();
        assert!(parsed <= now + chrono::Duration::days(7));
    }

    #[test]
    fn an_earlier_expiry_is_kept() {
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 11, 9, 10, 0, 0).unwrap();
        let asked = (now + chrono::Duration::hours(6)).to_rfc3339();
        assert_eq!(clamp_expiry(&asked, now).unwrap(), asked);
    }

    #[test]
    fn the_tape_is_newest_first_and_short() {
        let bars = vec![
            bar("2026-11-07", 100.0),
            bar("2026-11-08", 101.0),
            bar("2026-11-09", 102.0),
        ];
        let t = tape(&bars);
        assert!(t.starts_with("2026-11-09"), "{t}");
        assert_eq!(t.lines().count(), 3);
    }
}
