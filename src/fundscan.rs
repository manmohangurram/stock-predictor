//! Best-fund discovery: a curated list of large equity funds, ranked by what they actually returned.
//!
//! Unlike stocks, there is no "gainers" feed for funds. The list is shipped in the repo; each fund's
//! NAV history is fetched from mfapi.in (the AMFI mirror), and the top names by trailing 3-year and
//! 1-year return are proposed against the mutual-fund earmark. Runs once a day — a fund is a
//! six-month-plus commitment, so an hourly scan would be noise.

use anyhow::Result;
use chrono::{DateTime, Datelike, Utc};
use sqlx::SqlitePool;

use crate::market::mfapi::FundHistory;
use crate::models::{Bar, Mode};

/// Curated large-cap / flexi / ELSS equity funds, by AMFI scheme code. Extend this list to widen
/// the universe; nothing else needs to change.
const FUNDS: &[(&str, &str)] = &[
    ("122639", "Parag Parikh Flexi Cap Fund"),
    ("118955", "HDFC Flexi Cap Fund"),
    ("119723", "SBI ELSS Tax Saver Fund"),
    ("129046", "Motilal Oswal Flexi Cap Fund"),
    ("120843", "Quant Flexi Cap Fund"),
    ("119076", "DSP Flexi Cap Fund"),
    ("120503", "Axis ELSS Tax Saver Fund"),
    ("119292", "Union Flexi Cap Fund"),
    ("144546", "Tata Flexi Cap Fund"),
    ("118535", "Franklin India Flexi Cap Fund"),
];

const TOP_N: usize = 3;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct FundScanReport {
    pub examined: usize,
    pub ranked: usize,
    pub proposed: usize,
    pub notes: Vec<String>,
}

/// `YYYY-MM-DD` of the last IST day; the scan runs once per day.
fn ist_day(now: DateTime<Utc>) -> String {
    let ist = now + chrono::Duration::minutes(330);
    format!("{:04}-{:02}-{:02}", ist.year(), ist.month(), ist.day())
}

pub async fn run(
    pool: &SqlitePool,
    runtime: &crate::rules::runtime::Runtime,
    now: DateTime<Utc>,
) -> Result<FundScanReport> {
    let mut report = FundScanReport::default();

    let today = ist_day(now);
    let last: Option<(String,)> =
        sqlx::query_as("SELECT value FROM settings WHERE key = 'fundscan_last'")
            .fetch_optional(pool)
            .await?;
    if last.map(|r| r.0).as_deref() == Some(today.as_str()) {
        return Ok(report); // already ran today
    }

    let client = crate::http::client_with(runtime, 20, Some("Mozilla/5.0 stock-predictor"))?;

    let mut histories = Vec::new();
    for (code, _name) in FUNDS {
        report.examined += 1;
        match crate::market::mfapi::fetch(&client, code).await {
            Ok(h) if h.navs.len() >= 30 => {
                // Rank by 3-year, falling back to 1-year when the scheme is younger.
                if let Some(r3) = h.trailing_return(3.0) {
                    histories.push((code.to_string(), h.name.clone(), r3, h));
                } else if let Some(r1) = h.trailing_return(1.0) {
                    histories.push((code.to_string(), h.name.clone(), r1, h));
                }
            }
            Ok(h) => report
                .notes
                .push(format!("{}: only {} NAV points", code, h.navs.len())),
            Err(e) => report.notes.push(format!("{code}: {e}")),
        }
    }

    // Highest return first; the score is a blend of 1y and 3y so a one-year wonder is not picked.
    histories.sort_by(|a, b| b.2.total_cmp(&a.2));

    // Any curated fund already in the book gets its chart, whether or not it is proposed today.
    for (code, _, _, history) in &histories {
        if let Some(id) = instrument_in_book(pool, code).await? {
            save_nav_history(pool, id, history).await?;
        }
    }
    report.ranked = histories.len();

    let earmark = crate::rules::settings::mf_allowance(pool).await?;
    if earmark <= 0.0 {
        report
            .notes
            .push("no mutual-fund allowance configured — the scan is not proposing".into());
        return Ok(report);
    }

    let held_categories: Vec<String> = sqlx::query_as::<_, (String,)>(
        "SELECT i.name FROM positions p JOIN instruments i ON i.id = p.instrument_id
         WHERE p.mode = 'long' AND p.state <> 'closed' AND i.class = 'mutual_fund'",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| r.0)
    .collect();

    let mut proposed_categories: Vec<String> = Vec::new();
    for (code, name, score, history) in histories.into_iter().take(TOP_N) {
        // Overlap is the cheapest real filter a fund can have: a second flexi cap is the same bet.
        let category = history.category.clone();
        let clashes = |existing: &[String]| {
            !category.is_empty()
                && existing.iter().any(|n| {
                    let e = n.to_lowercase();
                    let c = category.to_lowercase();
                    (c.contains("flexi") && e.contains("flexi"))
                        || (c.contains("elss") && e.contains("elss"))
                        || (c.contains("large cap") && e.contains("large cap"))
                        || (c.contains("mid cap") && e.contains("mid cap"))
                        || (c.contains("small cap") && e.contains("small cap"))
                })
        };
        if clashes(&held_categories) || clashes(&proposed_categories) {
            report.notes.push(format!(
                "{name}: skipped — you already hold a {} fund",
                if category.is_empty() {
                    "similar"
                } else {
                    &category
                }
            ));
            continue;
        }
        proposed_categories.push(name.clone());
        // The latest NAV is the entry; a fund decision has no stop or target (NAV execution).
        let Some((_, nav)) = history.navs.last() else {
            continue;
        };
        let already = sqlx::query_as::<_, (i64,)>(
            "SELECT COUNT(*) FROM signals WHERE kind = 'fundscan' AND reason_json LIKE ? AND created_at >= ?",
        )
        .bind(format!("%{code}%"))
        .bind(now.to_rfc3339())
        .fetch_one(pool)
        .await?;
        if already.0 > 0 {
            continue;
        }
        let id = ensure_instrument(pool, &code, &name).await?;
        // The scan already holds the whole NAV series; storing it gives the fund a chart.
        save_nav_history(pool, id, &history).await?;
        store(pool, id, &name, &code, *nav, score, now).await?;
        report.proposed += 1;
        report.notes.push(format!(
            "{name}: +{score:.1}% over the window, NAV {nav:.2} — proposed"
        ));
    }

    sqlx::query(
        "INSERT INTO settings (key, value, updated_at) VALUES ('fundscan_last', ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(&today)
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;

    Ok(report)
}

/// The fund's instrument row, created inactive so the scan never starts polling it.
async fn ensure_instrument(pool: &SqlitePool, code: &str, name: &str) -> Result<i64> {
    let symbol = format!("AMFI:{code}");
    if let Some((id,)) = sqlx::query_as::<_, (i64,)>("SELECT id FROM instruments WHERE symbol = ?")
        .bind(&symbol)
        .fetch_optional(pool)
        .await?
    {
        return Ok(id);
    }
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO instruments (symbol, class, name, currency, active)
         VALUES (?, 'mutual_fund', ?, 'INR', 0) RETURNING id",
    )
    .bind(&symbol)
    .bind(name)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

async fn instrument_in_book(pool: &SqlitePool, code: &str) -> Result<Option<i64>> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT id FROM instruments WHERE symbol = ? AND active = 1")
            .bind(code)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|r| r.0))
}

/// The NAV series as daily bars. A fund has no OHLC, so every price is the NAV and volume is 0.
async fn save_nav_history(pool: &SqlitePool, instrument_id: i64, h: &FundHistory) -> Result<()> {
    crate::store::bars(pool, instrument_id, &nav_bars(h)).await
}

pub fn nav_bars(h: &FundHistory) -> Vec<Bar> {
    h.navs
        .iter()
        .map(|(date, nav)| Bar {
            ts: format!("{date}T00:00:00Z"),
            open: *nav,
            high: *nav,
            low: *nav,
            close: *nav,
            volume: 0,
        })
        .collect()
}

async fn store(
    pool: &SqlitePool,
    instrument_id: i64,
    name: &str,
    code: &str,
    nav: f64,
    score: f64,
    now: DateTime<Utc>,
) -> Result<()> {
    let reason = serde_json::json!({
        "reasons": [format!("{name} returned +{score:.1}% over the trailing window")],
        "rationale": "A six-month-plus commitment: judged on trailing returns, with the fund earmark in mind.",
        "source": "mfapi",
        "scheme_code": code,
    })
    .to_string();
    sqlx::query(
        "INSERT INTO signals (instrument_id, action, qty, entry, valid_until, reason_json,
            created_at, confidence, mode, kind, status)
         VALUES (?, 'BUY', ?, ?, ?, ?, ?, NULL, 'long', 'fundscan', 'pending')",
    )
    .bind(instrument_id)
    .bind(0.0) // quantity is not sized until you approve: a fund is bought for a rupee amount, not units
    .bind(nav)
    .bind((now + chrono::Duration::days(7)).to_rfc3339())
    .bind(reason)
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;
    let _ = Mode::Long;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_nav_series_becomes_daily_bars() {
        let h = FundHistory {
            code: "120503".into(),
            name: "Axis ELSS".into(),
            fund_house: "Axis".into(),
            category: "ELSS".into(),
            navs: vec![("2026-11-12".into(), 74.05), ("2026-11-13".into(), 74.62)],
        };
        let bars = nav_bars(&h);
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[1].ts, "2026-11-13T00:00:00Z");
        assert_eq!(bars[1].close, 74.62);
        assert_eq!(bars[1].open, bars[1].high);
        assert_eq!(bars[1].volume, 0);
    }

    #[test]
    fn the_day_is_ist() {
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 11, 9, 20, 0, 0).unwrap();
        // 20:00 UTC is 01:30 IST the next day.
        assert_eq!(ist_day(now), "2026-11-10");
    }
}
