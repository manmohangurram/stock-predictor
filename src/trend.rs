//! Short-term mode: what moved today, and whether it keeps moving.
//!
//! Runs on every poll tick and asks about each of the day's top gainers **only when that symbol has
//! actually changed** — its price moved, or its headlines changed — using the same gate as the
//! long-term path. A flat name costs nothing however often this runs.
//!
//! The list itself is asked **once per IST day** from three places and stored in `trend_candidates`:
//! the exchange's gainers, the Nifty 50's own movers, and the model's reading of the web. A tick
//! reads that list rather than paying for the search again. Duplicates are one row — the first
//! source that names a symbol keeps it, and the exchange's row wins because it carries a price.
//!
//! Discovery never starts polling on its own: a new symbol is stored inactive, and approving its
//! decision activates it.

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::llm::{memo, parse, validate};
use crate::models::{Action, Bar, Mode};

const TOP_N: usize = 10;
/// The one query the daily scan searches. Broad on purpose — the model reads the results and picks.
const TRENDING_QUERY: &str = "Indian stock market today biggest movers news";
/// The book is short-term: nothing is proposed beyond a week out.
const MAX_HORIZON_HOURS: i64 = 24 * 7;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct TrendReport {
    pub examined: usize,
    pub called: usize,
    pub proposed: usize,
    /// One line per proposal, ready to put in a notification.
    pub ideas: Vec<String>,
    pub notes: Vec<String>,
    /// 1 when a summary notification went out.
    pub notified: usize,
}

/// One name on today's list, from whichever of the three sources named it first.
#[derive(Debug, Clone, PartialEq)]
struct Candidate {
    /// Yahoo form: `TRANSWORLD.NS`.
    symbol: String,
    source: String,
    /// One line: why it is on the list. The exchange's sources have none, the search has.
    why: String,
    ltp: Option<f64>,
    change_pct: Option<f64>,
    previous_close: Option<f64>,
    turnover: Option<f64>,
}

impl Candidate {
    fn from_gainer(g: &crate::market::nse::Gainer, source: &str) -> Self {
        Self {
            symbol: g.yahoo_symbol(),
            source: source.into(),
            why: String::new(),
            ltp: Some(g.ltp),
            change_pct: Some(g.per_change),
            previous_close: Some(g.previous_close),
            turnover: g.turnover,
        }
    }

    /// The bare NSE ticker: what NSE writes, and what a search query wants.
    fn short_name(&self) -> &str {
        self.symbol.split('.').next().unwrap_or(&self.symbol)
    }

    /// The one line the prompt opens with. Exchange names have prices; discovered ones do not.
    fn describe(&self) -> String {
        match (self.ltp, self.change_pct, self.previous_close) {
            (Some(ltp), Some(pct), Some(prev)) => format!(
                "{ltp:.2} on the exchange feed, {pct:+.2}% today, previous close {prev:.2}, \
                 turnover {:.0}",
                self.turnover.unwrap_or(0.0)
            ),
            _ => "on today's scan; no exchange price for it, so the tape below is the price".into(),
        }
    }

    /// True when the feed's price is not in the same world as our own last close. A stale snapshot
    /// and a genuine move look identical to a model reading two numbers, so the prompt says which
    /// of the two is suspect instead of letting it conclude the stock "is" that price.
    fn disagrees_with(&self, last_close: Option<f64>) -> Option<String> {
        let (feed, close) = (self.ltp?, last_close?);
        if close <= 0.0 {
            return None;
        }
        let gap = (feed - close).abs() / close;
        (gap > 0.05).then(|| {
            format!(
                "the feed says {feed:.2} while our last close was {close:.2} ({:+.1}%) — one of the \
                 two is stale; trust the tape, and pass if the setup depends on the feed's number",
                (feed / close - 1.0) * 100.0
            )
        })
    }
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
    notifier: &dyn crate::notify::Notifier,
    runtime: &crate::rules::runtime::Runtime,
    now: DateTime<Utc>,
) -> Result<TrendReport> {
    let mut report = TrendReport::default();

    let client = crate::http::client_with(runtime, 20, Some("Mozilla/5.0 stock-predictor"))?;
    let candidates = daily_list(pool, &client, transport, now, &mut report).await?;
    report.examined = candidates.len();
    if candidates.is_empty() {
        report.notes.push("no candidates on today's list".into());
        return Ok(report);
    }

    // The daily cap is a ceiling on *all* model calls, not just the long-term path.
    let cap = runtime.daily_cap as i64;
    let mut used = crate::llm::orchestrate::daily_chat_calls(pool).await?;
    if used >= cap {
        report
            .notes
            .push(format!("daily call cap reached ({used}/{cap})"));
        return Ok(report);
    }

    let settings = crate::rules::settings::get(pool).await?;
    let limits = settings.limits_trade;
    let cash = crate::rules::budget::available_in(pool, Mode::Trade).await?;
    let pot = crate::rules::budget::cash_in(pool, Mode::Trade).await?;
    let model = crate::rules::settings::model_config(pool).await?;

    for g in candidates {
        let symbol = g.symbol.clone();
        let Some(id) = ensure_instrument(pool, &symbol, g.short_name()).await? else {
            report.notes.push(format!(
                "{symbol}: already held in the trading pot, skipped"
            ));
            continue;
        };

        // Gate first, network second: ten refresh pairs per tick is the expensive part, and most
        // of these names have not moved since the last pass.
        let headlines_changed = news_newer_than_last_call(pool, id).await?;
        let gate = memo::CallGate {
            min_move: runtime.min_price_move,
        };
        // A name discovered by the search has no exchange price yet, so the bar close stands in.
        let price = match g.ltp {
            Some(ltp) => ltp,
            None => last_close(pool, id).await?.unwrap_or(0.0),
        };
        if !memo::should_call(pool, id, price, now, gate, headlines_changed).await? {
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
        let news = recent_headlines(pool, id).await?;
        let headlines = render_headlines(&news);
        let headline_fp = memo::fingerprint(
            &news
                .iter()
                .map(|(title, at)| crate::models::Headline {
                    id: 0,
                    title: title.clone(),
                    publisher: String::new(),
                    published_at: at.clone(),
                    url: String::new(),
                })
                .collect::<Vec<_>>(),
        );
        // The web is asked about this one name, so the model reads today's reason and not only
        // what our own poller happened to store. A failed search costs a note, not the call.
        let web = match crate::llm::deepseek::search_for(
            pool,
            &model.base_url,
            transport,
            &format!("{} news today", g.short_name()),
            // The audit row names the holding: "search TATAMOTORS.NS".
            &format!("search {symbol}"),
        )
        .await
        {
            Ok(hits) => crate::llm::prompts::render_web_hits(&hits),
            Err(e) => {
                report.notes.push(format!("{symbol}: search failed: {e}"));
                crate::llm::prompts::render_web_hits(&[])
            }
        };
        // The feed and our own tape are compared here, not by the model: two prices from two
        // worlds look like a contradiction to it, and a contradiction is a `pass`.
        let last_close = bars.last().map(|b| b.close);
        let warn = g
            .disagrees_with(last_close)
            .map(|w| format!(" · {w}"))
            .unwrap_or_default();
        let prompt = crate::llm::prompts::render_trend(
            &format!("{} · {} · {}{warn}", symbol, g.describe(), scan_note(&g.why)),
            &tape(&bars),
            &headlines,
            &web,
            &format!(
                "cash {} {:.2}, allowance {:.2}",
                settings.currency, cash, pot
            ),
        )?;

        if used >= cap {
            report
                .notes
                .push(format!("daily call cap reached ({used}/{cap})"));
            break;
        }
        report.called += 1;
        used += 1;
        let reply = crate::llm::deepseek::chat(
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
                        g.change_pct.unwrap_or(0.0)
                    )
                } else {
                    format!(
                        "short-term scan: {:+.2}% today, price moved",
                        g.change_pct.unwrap_or(0.0)
                    )
                }),
                purpose: format!("trend {symbol}"),
            },
        )
        .await;
        // Recorded before the outcome is judged, success or failure: a failing call that is not
        // remembered would be paid for again on the next tick.
        memo::record_call(pool, id, price, now, &headline_fp).await?;
        let reply = match reply {
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
                report
                    .notes
                    .push(format!("{symbol}: unreadable reply: {e}"));
                continue;
            }
        };
        if proposal.action != Action::Buy {
            report.notes.push(format!(
                "{symbol}: {:?} is not a short-term entry",
                proposal.action
            ));
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
                requires_high_confidence: true, // short-term: high confidence or nothing
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
        report.notes.push(format!(
            "{symbol}: proposed {qty:.0} units at {:.2}",
            row.entry
        ));
    }

    if !report.ideas.is_empty() {
        // One notification for the pass, not one per name: the ledger is where you act on them.
        let n = crate::notify::Notification {
            title: format!(
                "Short-term · {} idea{}",
                report.ideas.len(),
                plural(report.ideas.len())
            ),
            body: report.ideas.join("\n"),
            priority: crate::notify::Priority::Default,
            dedupe_key: crate::llm::memo::fingerprint(
                &report
                    .ideas
                    .iter()
                    .map(|i| crate::models::Headline {
                        id: 0,
                        title: i.clone(),
                        publisher: String::new(),
                        published_at: ist_day(now),
                        url: String::new(),
                    })
                    .collect::<Vec<_>>(),
            ),
            signal_id: None,
            symbol: report
                .ideas
                .first()
                .and_then(|i| i.split(' ').next())
                .unwrap_or("")
                .to_string(),
        };
        if crate::notify::dispatch(pool, notifier, &n).await? {
            report.notified = 1;
        }
    }

    Ok(report)
}

/// Today's list: read from the table if it was asked, otherwise asked and stored.
///
/// Once per IST day, and only once: three sources cost two exchange calls, one search and one chat,
/// and a tick every fifteen minutes must not pay that repeatedly. A failed source is a note, never a
/// reason to lose the whole list — a holiday with no search results still has an exchange list.
async fn daily_list(
    pool: &SqlitePool,
    client: &reqwest::Client,
    transport: &dyn crate::llm::deepseek::Transport,
    now: DateTime<Utc>,
    report: &mut TrendReport,
) -> Result<Vec<Candidate>> {
    let today = ist_day(now);
    let stored = load_candidates(pool, &today).await?;
    if !stored.is_empty() {
        return Ok(stored);
    }

    let mut out: Vec<Candidate> = Vec::new();
    // The exchange first: its rows carry a price, so they win a duplicate. One request returns
    // both lists — the whole market's gainers and the index's own movers.
    match crate::market::nse::fetch_today(client, TOP_N).await {
        Ok((gainers, index)) => {
            out.extend(gainers.iter().map(|g| Candidate::from_gainer(g, "gainers")));
            out.extend(index.iter().map(|g| Candidate::from_gainer(g, "nifty50")));
        }
        Err(e) => report
            .notes
            .push(format!("exchange lists unavailable: {e}")),
    }
    match searched_candidates(pool, transport, now).await {
        Ok(rows) => out.extend(rows),
        Err(e) => report.notes.push(format!("trending search failed: {e}")),
    }

    let out = dedupe(out);
    for c in &out {
        store_candidate(pool, &today, c, now).await?;
    }
    // Set even when the list came back empty: an empty day is an answer, and ninety-six ticks must
    // not each pay for the same silence. A person asking again clears it (see `forget_today`).
    set_marker(pool, "trend_list_last", &today).await?;
    Ok(out)
}

/// One row per symbol, first source wins: the exchange's rows carry a price, so they are pushed
/// first, and a search hit that the exchange already named adds nothing.
fn dedupe(rows: Vec<Candidate>) -> Vec<Candidate> {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<Candidate> = rows
        .into_iter()
        .filter(|c| seen.insert(c.symbol.clone()))
        .collect();
    out.sort_by(|a, b| b.change_pct.unwrap_or(-999.0).total_cmp(&a.change_pct.unwrap_or(-999.0)));
    out.truncate(TOP_N * 3);
    out
}

/// What the web says is moving today, as the model reads it. One search, one call, one list.
async fn searched_candidates(
    pool: &SqlitePool,
    transport: &dyn crate::llm::deepseek::Transport,
    now: DateTime<Utc>,
) -> Result<Vec<Candidate>> {
    let model = crate::rules::settings::model_config(pool).await?;
    let hits = crate::llm::deepseek::search_for(
        pool,
        &model.base_url,
        transport,
        TRENDING_QUERY,
        "search the market",
    )
    .await?;
    let prompt =
        crate::llm::prompts::render_trending(&ist_day(now), &crate::llm::prompts::render_web_hits(&hits))?;
    let reply = crate::llm::deepseek::chat(
        pool,
        &model,
        transport,
        crate::llm::deepseek::ChatRequest {
            conversation_id: format!("trending-{}", now.timestamp_millis()),
            round: 1,
            prompt: prompt.text,
            prompt_template: prompt.template,
            prompt_hash: prompt.hash,
            trigger: Some("daily scan: what the web says is moving in India".into()),
            purpose: "trending list".into(),
        },
    )
    .await?;
    Ok(parse_trending(&reply.content)
        .into_iter()
        .map(|(symbol, why)| Candidate {
            symbol,
            source: "search".into(),
            why,
            ltp: None,
            change_pct: None,
            previous_close: None,
            turnover: None,
        })
        .collect())
}

/// `{"symbols":[{"symbol":"TATAMOTORS","why":"…"}]}` — a bare ticker becomes a Yahoo symbol.
pub fn parse_trending(content: &str) -> Vec<(String, String)> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(content) else {
        return Vec::new();
    };
    let Some(list) = v.get("symbols").and_then(|s| s.as_array()) else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = Vec::new();
    for row in list {
        let raw = match row {
            serde_json::Value::String(s) => s.clone(),
            other => other.get("symbol").and_then(|s| s.as_str()).unwrap_or("").to_string(),
        };
        let symbol = raw.trim().to_uppercase();
        if symbol.len() < 2 || symbol.len() > 20 || !symbol
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '&' | '-' | '.')) {
            continue;
        }
        let why = row
            .get("why")
            .and_then(|w| w.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        // `TATAMOTORS` on its own is what the prompt asks for; `.NS` is accepted, not required.
        let yahoo = if symbol.contains('.') { symbol } else { format!("{symbol}.NS") };
        if !out.iter().any(|(s, _)| s == &yahoo) {
            out.push((yahoo, why));
        }
    }
    out.truncate(TOP_N);
    out
}

/// The scan's line, marked as what it is: a reading of the web by another model, which may quote a
/// price that is days old. The tape below it is ours and is the only price that counts.
fn scan_note(why: &str) -> String {
    if why.trim().is_empty() {
        "no note from the scan".into()
    } else {
        format!("scan's note, unverified — do not take a price from it: {why}")
    }
}

/// One stored row: symbol, source, why, ltp, change, previous close, turnover.
type CandidateRow = (String, String, String, Option<f64>, Option<f64>, Option<f64>, Option<f64>);

async fn load_candidates(pool: &SqlitePool, day: &str) -> Result<Vec<Candidate>> {
    let rows: Vec<CandidateRow> =
        sqlx::query_as(
            "SELECT symbol, source, why, ltp, change_pct, previous_close, turnover
             FROM trend_candidates WHERE day = ?
             ORDER BY COALESCE(change_pct, -999) DESC, symbol",
        )
        .bind(day)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(symbol, source, why, ltp, change_pct, previous_close, turnover)| Candidate {
            symbol,
            source,
            why,
            ltp,
            change_pct,
            previous_close,
            turnover,
        })
        .collect())
}

async fn store_candidate(
    pool: &SqlitePool,
    day: &str,
    c: &Candidate,
    now: DateTime<Utc>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO trend_candidates (day, symbol, source, why, ltp, change_pct, previous_close, turnover, added_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(day, symbol) DO NOTHING",
    )
    .bind(day)
    .bind(&c.symbol)
    .bind(&c.source)
    .bind(&c.why)
    .bind(c.ltp)
    .bind(c.change_pct)
    .bind(c.previous_close)
    .bind(c.turnover)
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// Start today's list from nothing — what "Run again" means for the short-term pass.
pub async fn forget_today(pool: &SqlitePool, now: DateTime<Utc>) -> Result<()> {
    sqlx::query("DELETE FROM trend_candidates WHERE day = ?")
        .bind(ist_day(now))
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM settings WHERE key = 'trend_list_last'")
        .execute(pool)
        .await?;
    Ok(())
}

async fn set_marker(pool: &SqlitePool, key: &str, value: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO settings (key, value, updated_at) VALUES (?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(key)
    .bind(value)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// The last close we have stored, for a name the exchange has not priced for us yet.
async fn last_close(pool: &SqlitePool, instrument_id: i64) -> Result<Option<f64>> {
    let row: Option<(f64,)> = sqlx::query_as(
        "SELECT close FROM bars WHERE instrument_id = ? ORDER BY ts DESC LIMIT 1",
    )
    .bind(instrument_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.0))
}

/// `YYYY-MM-DD` in IST — the day the tape belongs to, used for dedupe.
fn ist_day(now: DateTime<Utc>) -> String {
    (now + chrono::Duration::minutes(330))
        .format("%Y-%m-%d")
        .to_string()
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
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
    let fresh: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM news WHERE instrument_id = ? AND published_at > ?")
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

async fn recent_headlines(pool: &SqlitePool, instrument_id: i64) -> Result<Vec<(String, String)>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT title, published_at FROM news WHERE instrument_id = ?
         ORDER BY published_at DESC LIMIT 4",
    )
    .bind(instrument_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

fn render_headlines(rows: &[(String, String)]) -> String {
    if rows.is_empty() {
        return "(no headlines)".into();
    }
    rows.iter()
        .map(|(t, at)| format!("[{}] {t}", &at[..10.min(at.len())]))
        .collect::<Vec<_>>()
        .join("\n")
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

#[cfg(test)]
mod list_tests {
    use super::*;

    fn cand(symbol: &str, source: &str, pct: Option<f64>) -> Candidate {
        Candidate {
            symbol: symbol.into(),
            source: source.into(),
            why: String::new(),
            ltp: pct.map(|_| 100.0),
            change_pct: pct,
            previous_close: pct.map(|_| 99.0),
            turnover: None,
        }
    }

    #[test]
    fn a_symbol_named_twice_is_one_row_and_the_exchange_keeps_it() {
        let out = dedupe(vec![
            cand("TATAMOTORS.NS", "gainers", Some(5.1)),
            cand("TATAMOTORS.NS", "search", None),
            cand("IDEA.NS", "search", None),
        ]);
        assert_eq!(out.len(), 2);
        let tata = out.iter().find(|c| c.symbol == "TATAMOTORS.NS").unwrap();
        assert_eq!(tata.source, "gainers", "the row with a price wins");
        assert!(out[0].symbol == "TATAMOTORS.NS", "ranked by change, unpriced last");
    }

    #[test]
    fn a_bare_ticker_from_the_model_becomes_a_yahoo_symbol() {
        let rows = parse_trending(
            r#"{"symbols":[{"symbol":"tatamotors","why":"Q2 beat"},{"symbol":"IDEA.NS","why":"fund raise"}]}"#,
        );
        assert_eq!(rows[0].0, "TATAMOTORS.NS");
        assert_eq!(rows[0].1, "Q2 beat");
        assert_eq!(rows[1].0, "IDEA.NS");
    }

    #[test]
    fn an_unreadable_or_empty_scan_is_an_empty_list_not_a_panic() {
        assert!(parse_trending("not json").is_empty());
        assert!(parse_trending(r#"{"symbols":[]}"#).is_empty());
        assert!(parse_trending(r#"{"symbols":[{"symbol":"..!.."}]}"#).is_empty());
    }

    #[test]
    fn the_scan_is_capped_at_ten_names() {
        let symbols: Vec<String> = (0..15).map(|i| format!(r#"{{"symbol":"S{i}"}}"#)).collect();
        let body = format!(r#"{{"symbols":[{}]}}"#, symbols.join(","));
        assert_eq!(parse_trending(&body).len(), TOP_N);
    }

    #[test]
    fn an_exchange_row_says_its_price_and_a_discovered_one_does_not_pretend_to() {
        let priced = Candidate::from_gainer(
            &crate::market::nse::Gainer {
                symbol: "TATAMOTORS".into(),
                ltp: 780.5,
                previous_close: 742.1,
                per_change: 5.17,
                turnover: Some(1200.0),
            },
            "gainers",
        );
        assert!(priced.describe().contains("780.50 on the exchange feed"));
        assert_eq!(priced.short_name(), "TATAMOTORS");
        assert!(cand("IDEA.NS", "search", None).describe().contains("tape"));
    }
}

#[cfg(test)]
mod feed_tests {
    use super::*;

    fn priced(ltp: f64, pct: f64) -> Candidate {
        Candidate {
            symbol: "CHOLAFIN.NS".into(),
            source: "search".into(),
            why: "Up 3.82% to ₹1604.2".into(),
            ltp: Some(ltp),
            change_pct: Some(pct),
            previous_close: Some(ltp / 1.0382),
            turnover: None,
        }
    }

    /// The failure that produced a `pass`: a feed price a month behind our own tape.
    #[test]
    fn a_feed_price_far_from_our_close_is_called_out_before_the_model_sees_it() {
        let warn = priced(1604.2, 3.82).disagrees_with(Some(1790.80)).unwrap();
        assert!(warn.contains("1790.80"), "{warn}");
        assert!(warn.contains("stale"), "{warn}");
    }

    #[test]
    fn a_feed_price_near_our_close_says_nothing() {
        assert!(priced(1795.0, 1.2).disagrees_with(Some(1790.80)).is_none());
        // No price of our own to compare with is not a disagreement.
        assert!(priced(1795.0, 1.2).disagrees_with(None).is_none());
    }

    /// The scan's line is another model's reading of the web, and it may quote a stale price.
    #[test]
    fn the_scans_note_says_that_it_is_unverified() {
        let note = scan_note("Up 3.82% to ₹1604.2");
        assert!(note.contains("unverified"), "{note}");
        assert!(note.contains("do not take a price"), "{note}");
        assert_eq!(scan_note("  "), "no note from the scan");
    }

    #[test]
    fn an_unpriced_candidate_points_the_model_at_the_tape() {
        let line = Candidate {
            ltp: None,
            change_pct: None,
            previous_close: None,
            ..priced(0.0, 0.0)
        }
        .describe();
        assert!(line.contains("tape below is the price"), "{line}");
    }
}
