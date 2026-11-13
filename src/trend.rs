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

use crate::models::Bar;

const TOP_N: usize = 10;
/// How old a search result may be and still count as *news*. The API has no date filter — the only
/// lever is the date on each result — so this is where "today's reason" is separated from a page
/// that merely has the stock's name in its title.
const NEWS_WINDOW_DAYS: i64 = 3;
/// The one query the daily scan searches. Broad on purpose — the model reads the results and picks.
const TRENDING_QUERY: &str = "Indian stock market biggest movers news";
#[derive(Debug, Default, Clone, PartialEq)]
pub struct TrendReport {
    pub examined: usize,
    pub called: usize,
    pub proposed: usize,
    /// Names the model looked at and refused. A decision with nothing to approve.
    pub passed: usize,
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

/// What the scan may offer, before the model is asked anything. Three numbers from Settings:
/// a price floor (a paise tick must not be a large part of the move), a turnover floor (this pot's
/// order has to be a rounding error), and the locked-band cut-off (a stock at its ceiling cannot be
/// bought at any price you name).
fn worth_offering(c: &Candidate, min_price: f64, min_turnover: f64, locked_pct: f64) -> bool {
    // A name with no exchange price yet is left to the model: it has no numbers to judge by here.
    let (Some(ltp), Some(turnover), Some(change)) = (c.ltp, c.turnover, c.change_pct) else {
        return true;
    };
    ltp >= min_price && turnover >= min_turnover && change.abs() < locked_pct
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

/// Remove the names a plan call looked at and never used: inactive, never archived, and with no
/// decision, no fill and no position. The plan may look at twenty tickers and buy one; the other
/// nineteen must not stay in the book.
pub async fn sweep_glances(pool: &SqlitePool) -> Result<usize> {
    let rows: Vec<(i64,)> = sqlx::query_as(
        "SELECT i.id FROM instruments i
          WHERE i.active = 0
            AND i.archived_at IS NULL
            AND NOT EXISTS (SELECT 1 FROM signals   s WHERE s.instrument_id = i.id)
            AND NOT EXISTS (SELECT 1 FROM trades    t WHERE t.instrument_id = i.id)
            AND NOT EXISTS (SELECT 1 FROM positions p WHERE p.instrument_id = i.id)",
    )
    .fetch_all(pool)
    .await?;
    let mut dropped = 0;
    for (id,) in rows {
        let mut tx = pool.begin().await?;
        for sql in [
            "DELETE FROM bars WHERE instrument_id = ?",
            "DELETE FROM news WHERE instrument_id = ?",
            "DELETE FROM llm_memos WHERE instrument_id = ?",
            "DELETE FROM trend_candidates WHERE symbol = (SELECT symbol FROM instruments WHERE id = ?)",
            "DELETE FROM instruments WHERE id = ?",
        ] {
            sqlx::query(sql).bind(id).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        dropped += 1;
    }
    Ok(dropped)
}

/// The trade plan's list: what moved today, with the tape and today's dated web evidence for the
/// few names worth a look. One call, one book — the shape in which "sell A to buy B" is possible.
///
/// Evidence for four names, not twenty: the model can only act on two moves, so twenty names' worth
/// of tape and headlines is prompt spent on names nobody will buy.
pub async fn candidate_block(
    pool: &SqlitePool,
    market: &dyn crate::market::MarketSource,
    transport: &dyn crate::llm::deepseek::Transport,
    now: DateTime<Utc>,
) -> String {
    const EVIDENCE: usize = 4;
    let Ok(client) = crate::http::client_with(
        &crate::rules::settings::runtime(pool).await.unwrap_or_default(),
        20,
        Some("Mozilla/5.0 stock-predictor"),
    ) else {
        return "no list today — the exchange could not be reached\n".into();
    };
    let mut report = TrendReport::default();
    let filters = crate::rules::runtime::load(pool).await.unwrap_or_default();
    let list: Vec<Candidate> = daily_list(pool, &client, transport, now, &mut report)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|c| {
            let keep = worth_offering(
                c,
                filters.scan_min_price,
                filters.scan_min_turnover,
                filters.scan_locked_pct,
            );
            if !keep {
                report.notes.push(format!(
                    "{} left out: under {:.0} rupees, under {:.0} turnover, or locked at the band",
                    c.symbol, filters.scan_min_price, filters.scan_min_turnover
                ));
            }
            keep
        })
        .collect();
    if list.is_empty() {
        return format!(
            "no candidates today{}\n",
            if report.notes.is_empty() {
                String::new()
            } else {
                format!(" ({})", report.notes.join("; "))
            }
        );
    }
    let mut out = String::new();
    for (i, g) in list.iter().enumerate() {
        out.push_str(&format!(
            "{}. {} · {} · {}\n",
            i + 1,
            g.symbol,
            g.describe(),
            scan_note(&g.why)
        ));
        if i >= EVIDENCE {
            continue;
        }
        // The evidence only for the ones the model might actually act on.
        let Ok(Some((id, _))) = ensure_instrument(pool, &g.symbol, g.short_name()).await else {
            continue;
        };
        if crate::store::refresh(pool, id, &g.symbol, market).await.is_err() {
            continue;
        }
        let known_name: Option<String> = sqlx::query_as("SELECT name FROM instruments WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten()
            .map(|(n,): (String,)| n)
            .filter(|n| !n.trim().is_empty() && !g.short_name().eq_ignore_ascii_case(n.trim()));
        let Ok(bars) = load_bars(pool, id).await else {
            continue;
        };
        out.push_str(&format!("   tape: {}\n", tape(&bars).replace('\n', "; ")));
        if let Some(warn) = g.disagrees_with(bars.last().map(|b| b.close)) {
            out.push_str(&format!("   feed vs tape: {warn}\n"));
        }
        let news = recent_headlines(pool, id).await.unwrap_or_default();
        if !news.is_empty() {
            out.push_str(&format!("   headlines: {}\n", render_headlines(&news).replace('\n', " | ")));
        }
        if let Ok(model) = crate::rules::settings::model_config(pool).await {
            if let Ok(hits) = crate::llm::deepseek::search_for(
                pool,
                &model.base_url,
                transport,
                &dated_query(&news_query(g.short_name(), known_name.as_deref()), now),
                &format!("search {}", g.symbol),
            )
            .await
            {
                let recent = relevant(recent_hits(hits, now), g.short_name(), known_name.as_deref());
                if !recent.is_empty() {
                    out.push_str(&format!("   web: {}\n", crate::llm::prompts::render_web_hits(&recent).replace('\n', " ")));
                }
            }
        }
    }
    out.push_str(&format!(
        "\n(evidence supplied for the first {EVIDENCE}; the rest are prices only)\n"
    ));
    out
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
    let hits = recent_hits(
        crate::llm::deepseek::search_for(
            pool,
            &model.base_url,
            transport,
            &dated_query(TRENDING_QUERY, now),
            "search the market",
        )
        .await?,
        now,
    );
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

/// Keep only results dated inside the window. Undated pages go too: a quote page or a filings
/// index has no date because it is not an article, and a blank date reads as "current".
fn recent_hits(
    hits: Vec<crate::llm::deepseek::SearchHit>,
    now: DateTime<Utc>,
) -> Vec<crate::llm::deepseek::SearchHit> {
    let cutoff = now.date_naive() - chrono::Duration::days(NEWS_WINDOW_DAYS);
    hits.into_iter()
        .filter(|h| {
            h.date
                .as_deref()
                .and_then(day_of)
                .is_some_and(|d| d >= cutoff)
        })
        .collect()
}

/// What to search for, given a ticker and (when known) the company's name.
///
/// The ticker alone is not enough: search engines split `ANTGRAPHIC` into `ant` and hand back
/// Anthropic's IPO. The name, quoted, is what a news page actually writes.
fn news_query(ticker: &str, name: Option<&str>) -> String {
    match name {
        Some(name) => format!("\"{}\" OR {} share news", name.trim(), ticker),
        None => format!("{ticker} share news"),
    }
}

/// Keep only the hits that are about *this* stock: the ticker or the company's name must appear in
/// the title or the snippet. A result that mentions neither is a different company — it costs tokens
/// and it lies to the model about what the catalyst is.
fn relevant(
    hits: Vec<crate::llm::deepseek::SearchHit>,
    ticker: &str,
    name: Option<&str>,
) -> Vec<crate::llm::deepseek::SearchHit> {
    let needle = |haystack: &str| {
        let lower = haystack.to_lowercase();
        let ticker_hit = lower.contains(&ticker.to_lowercase());
        let name_hit = name.is_some_and(|n| {
            n.split_whitespace()
                .take(2)
                .any(|word| word.len() >= 4 && lower.contains(&word.to_lowercase()))
        });
        ticker_hit || name_hit
    };
    hits.into_iter()
        .filter(|h| needle(&h.title) || needle(&h.snippet))
        .collect()
}

/// The window, pushed into the query itself: `after:` and `before:` are real operators, and a search
/// that is asked for three days comes back ranked for three days. `lang:en` because a source that is
/// not in English is not one this book can read; the `date` filter below still decides what arrives.
fn dated_query(query: &str, now: DateTime<Utc>) -> String {
    let from = (now.date_naive() - chrono::Duration::days(NEWS_WINDOW_DAYS)).format("%Y-%m-%d");
    format!("{query} after:{from} lang:en")
}

/// `2026-11-08T16:00:00+00:00` → 2026-11-08. The time of day is not the question; the day is.
fn day_of(raw: &str) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(raw.get(..10)?, "%Y-%m-%d").ok()
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

/// `YYYY-MM-DD` in IST — the day the tape belongs to, used for dedupe.
fn ist_day(now: DateTime<Utc>) -> String {
    (now + chrono::Duration::minutes(330))
        .format("%Y-%m-%d")
        .to_string()
}


/// The instrument for a discovered symbol: found, or created **for this tick only**. The second
/// element says the row is new, so the tick can take it back if nothing came of it.
async fn ensure_instrument(
    pool: &SqlitePool,
    yahoo_symbol: &str,
    short_name: &str,
) -> Result<Option<(i64, bool)>> {
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
        return Ok(if held.0 > 0.0 { None } else { Some((id, false)) });
    }
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO instruments (symbol, class, name, currency, active)
         VALUES (?, 'stock', ?, 'INR', 0) RETURNING id",
    )
    .bind(yahoo_symbol)
    .bind(short_name)
    .fetch_one(pool)
    .await?;
    Ok(Some((row.0, true)))
}

/// Take back the rows this tick created and never used: no decision, no fill, no position. The
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

#[cfg(test)]
mod news_window_tests {
    use super::*;
    use crate::llm::deepseek::SearchHit;

    fn hit(title: &str, date: Option<&str>) -> SearchHit {
        SearchHit {
            title: title.into(),
            site: "Wire".into(),
            url: "https://example.com".into(),
            snippet: "s".into(),
            summary: "m".into(),
            date: date.map(str::to_string),
        }
    }

    #[test]
    fn only_the_last_three_days_survive() {
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 11, 11, 6, 0, 0).unwrap();
        let kept = recent_hits(
            vec![
                hit("today", Some("2026-11-11T04:00:00+00:00")),
                hit("yesterday", Some("2026-11-10T16:00:00+00:00")),
                hit("three days", Some("2026-11-08T16:00:00+00:00")),
                hit("a week old", Some("2026-11-04T16:00:00+00:00")),
                hit("a March transcript", Some("2026-03-10T16:00:00+00:00")),
                hit("undated quote page", None),
            ],
            now,
        );
        let titles: Vec<&str> = kept.iter().map(|h| h.title.as_str()).collect();
        assert_eq!(titles, vec!["today", "yesterday", "three days"]);
    }

    #[test]
    fn a_date_is_read_as_a_day_not_a_timestamp() {
        assert_eq!(
            day_of("2026-11-08T16:00:00+00:00"),
            Some(chrono::NaiveDate::from_ymd_opt(2026, 11, 8).unwrap())
        );
        assert_eq!(day_of("yesterday"), None);
        assert_eq!(day_of(""), None);
    }
}

#[cfg(test)]
mod dated_query_tests {
    use super::*;

    #[test]
    fn the_query_asks_for_the_window_and_english() {
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 11, 11, 6, 0, 0).unwrap();
        let q = dated_query("CHOLAFIN news today", now);
        assert_eq!(q, "CHOLAFIN news today after:2026-11-08 lang:en");
    }

    #[test]
    fn the_scan_query_carries_the_same_window() {
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 12, 1, 6, 0, 0).unwrap();
        assert!(dated_query(TRENDING_QUERY, now).ends_with("after:2026-11-28 lang:en"));
    }
}

#[cfg(test)]
mod row_hygiene_tests {
    use super::*;

    /// A name the tick created is taken back when nothing came of it — no signal, no fill, no
    /// position. The scan may look at a hundred tickers; the book must not remember them.
    #[tokio::test]
    async fn a_created_row_with_no_decision_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
            .await
            .unwrap();
        let now = chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 11, 11, 6, 0, 0).unwrap();

        let (looked_at, fresh) = ensure_instrument(&pool, "MAKERSL.NS", "MAKERSL")
            .await
            .unwrap()
            .unwrap();
        assert!(fresh, "a name not in the book is created for the tick");
        sqlx::query("INSERT INTO bars (instrument_id, ts, close) VALUES (?, '2026-11-10', 100.0)")
            .bind(looked_at)
            .execute(&pool)
            .await
            .unwrap();

        let dropped = sweep_glances(&pool).await.unwrap();
        assert_eq!(dropped, 1);
        let left: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM instruments WHERE symbol = 'MAKERSL.NS'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(left.0, 0, "no row, and no orphan bars");
        let bars: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM bars")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(bars.0, 0);

        // And asking again makes it fresh again, which is the point: nothing was remembered.
        let (_, again) = ensure_instrument(&pool, "MAKERSL.NS", "MAKERSL")
            .await
            .unwrap()
            .unwrap();
        assert!(again);
        let _ = now;
    }

    /// A name that produced a proposal keeps its row: that is the decision the reader can act on.
    #[tokio::test]
    async fn a_created_row_that_proposed_something_stays() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
            .await
            .unwrap();
        let (id, _) = ensure_instrument(&pool, "MAKERSL.NS", "MAKERSL")
            .await
            .unwrap()
            .unwrap();
        sqlx::query(
            "INSERT INTO signals (instrument_id, action, qty, entry, valid_until, reason_json,
                created_at, mode, status, kind)
             VALUES (?, 'BUY', 10, 268.34, '2026-11-12T15:30:00+05:30', '{}',
                     '2026-11-11T06:00:00Z', 'trade', 'pending', 'trend')",
        )
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();

        let dropped = sweep_glances(&pool).await.unwrap();
        assert_eq!(dropped, 0, "a name with a decision keeps its row");
        let left: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM instruments WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(left.0, 1);
    }
}

#[cfg(test)]
mod relevance_tests {
    use super::*;
    use crate::llm::deepseek::SearchHit;

    fn hit(title: &str, snippet: &str) -> SearchHit {
        SearchHit {
            title: title.into(),
            site: "Wire".into(),
            url: "https://example.com".into(),
            snippet: snippet.into(),
            summary: String::new(),
            date: Some("2026-11-11".into()),
        }
    }

    /// The real failure: `ANTGRAPHIC` was tokenised as "ant", and the prompt got Anthropic's IPO.
    #[test]
    fn a_hit_about_a_different_company_is_dropped() {
        let kept = relevant(
            vec![
                hit("Anthropic retracts its $70–80bn IPO range", "$ANT listing expected next week"),
                hit("Antarctica Ltd jumps 19.67% on the BSE", "Antarctica Ltd closed at Rs 0.73"),
            ],
            "ANTGRAPHIC",
            Some("Antarctica"),
        );
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert!(kept[0].title.contains("Antarctica"));
    }

    #[test]
    fn the_name_is_what_makes_a_hit_relevant() {
        let kept = relevant(
            vec![hit("Prozone Realty Q2 profit jumps to ₹555 cr", "revenue rises 7%")],
            "PROZONER",
            Some("Prozone Realty Limited"),
        );
        assert_eq!(kept.len(), 1);
        // Without the name, the ticker is all we have, and this hit does not carry it.
        assert!(relevant(
            vec![hit("Prozone Realty Q2 profit jumps", "revenue rises 7%")],
            "PROZONER",
            None
        )
        .is_empty());
    }

    #[test]
    fn the_query_asks_for_the_name_when_there_is_one() {
        assert_eq!(
            news_query("ANTGRAPHIC", Some("Antarctica")),
            "\"Antarctica\" OR ANTGRAPHIC share news"
        );
        assert_eq!(news_query("IDEA", None), "IDEA share news");
    }
}

#[cfg(test)]
mod filter_tests {
    use super::*;

    fn c(ltp: Option<f64>, turnover: Option<f64>, pct: Option<f64>) -> Candidate {
        Candidate {
            symbol: "X.NS".into(),
            source: "gainers".into(),
            why: String::new(),
            ltp,
            change_pct: pct,
            previous_close: ltp.map(|p| p / (1.0 + pct.unwrap_or(0.0) / 100.0)),
            turnover,
        }
    }

    /// The four micro-caps from the operator's own list, which the model refused three times in a
    /// row in its own words. The floor does the refusing now, before a token is spent.
    #[test]
    fn the_micro_caps_at_the_band_are_not_offered() {
        let (price, turnover, locked) = (10.0, 10_000_000.0, 19.5);
        // ORCHASP 1.38, 51.9 lakh, +20.00
        assert!(!worth_offering(&c(Some(1.38), Some(5_195_000.0), Some(20.0)), price, turnover, locked));
        // ANTGRAPHIC 0.73, 27.7 lakh, +19.67
        assert!(!worth_offering(&c(Some(0.73), Some(2_773_000.0), Some(19.67)), price, turnover, locked));
        // FLEXITUFF 3.85, 10.6 lakh, +19.94
        assert!(!worth_offering(&c(Some(3.85), Some(1_056_000.0), Some(19.94)), price, turnover, locked));
        // PROZONER 63.76, 13.5 Cr, +19.98 — liquid and dear enough, but locked at the ceiling.
        assert!(!worth_offering(&c(Some(63.76), Some(135_500_000.0), Some(19.98)), price, turnover, locked));
    }

    /// A real mover that is not locked survives, which is the point of a floor.
    #[test]
    fn a_liquid_mover_is_still_offered() {
        let (price, turnover, locked) = (10.0, 10_000_000.0, 19.5);
        // GABRIEL 1619, 1289 Cr, +14.95
        assert!(worth_offering(&c(Some(1619.0), Some(12_897_300_000.0), Some(14.95)), price, turnover, locked));
        // JARO 404, 361 Cr, +16.65
        assert!(worth_offering(&c(Some(404.0), Some(361_040_000.0), Some(16.65)), price, turnover, locked));
    }

    /// A name the search found has no exchange numbers yet — the model judges it, not this filter.
    #[test]
    fn a_name_without_numbers_is_left_to_the_model() {
        assert!(worth_offering(&c(None, None, None), 10.0, 10_000_000.0, 19.5));
    }

    /// The thresholds are settings: a looser floor lets the same name through.
    #[test]
    fn the_floors_are_what_settings_say_they_are() {
        let flexituff = c(Some(3.85), Some(1_056_000.0), Some(19.94));
        assert!(!worth_offering(&flexituff, 10.0, 10_000_000.0, 19.5));
        assert!(worth_offering(&flexituff, 1.0, 100_000.0, 20.5), "a looser set of numbers keeps it");
    }
}
