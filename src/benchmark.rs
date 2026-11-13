//! The long-term book's candidate list: the Nifty 100, ranked by their last five sessions.
//!
//! NSE refuses `equity-stockIndices` to a non-browser client, so the index cannot be asked for its
//! own performance. What is available is the constituent CSV and one daily chart per symbol — a
//! hundred requests once an IST day, kept in `plan_candidates` so a retry costs nothing.
//!
//! Ranking by the last five sessions, keeping only the positive ones, and taking ten: a top-ten
//! drawn from a market that fell 6% would otherwise be the four names that only fell 2%.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::market::MarketSource;

/// The sessions the ranking is measured over. Five is a week of Indian trading, long enough that a
/// single gap-up cannot carry a name into the list on its own.
pub const SESSIONS: usize = 5;
/// How many names the plan is offered.
pub const TOP: usize = 10;

#[derive(Debug, Clone, PartialEq)]
pub struct Pick {
    /// Yahoo form: `ABB.NS`.
    pub symbol: String,
    pub name: String,
    pub close: f64,
    /// Five-session return as a fraction: 0.0612 is +6.12%.
    pub ret5: f64,
}

impl Pick {
    /// The one line the plan prompt reads.
    pub fn line(&self, currency: &str) -> String {
        format!(
            "{} {} at {:.2} {currency}, {:+.2}% over {SESSIONS} sessions",
            self.symbol,
            self.name,
            self.close,
            self.ret5 * 100.0
        )
    }
}

/// `Company Name,Industry,Symbol,Series,ISIN Code` — the symbol is bare (`ABB`), Yahoo wants `.NS`.
pub fn parse_constituents(csv: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in csv.lines().skip(1) {
        let cells: Vec<&str> = line.split(',').collect();
        if cells.len() < 3 {
            continue;
        }
        let (name, symbol) = (cells[0].trim(), cells[2].trim());
        if symbol.is_empty() || symbol.eq_ignore_ascii_case("symbol") {
            continue;
        }
        out.push((format!("{symbol}.NS"), name.to_string()));
    }
    out
}

/// Return over the last `sessions` closes, or `None` when there is not enough history. A name with
/// four closes is not ranked on three of them and called a trend.
pub fn return_over(closes: &[f64], sessions: usize) -> Option<f64> {
    if closes.len() <= sessions {
        return None;
    }
    let last = *closes.last()?;
    let window = &closes[closes.len() - 1 - sessions..];
    if !last.is_finite() || window.iter().any(|c| !c.is_finite() || *c <= 0.0) {
        return None;
    }
    Some(last / window[0] - 1.0)
}

/// Fetch the closes and compute the return. A symbol whose chart fails is skipped, not fatal: one
/// suspended name must not cost the whole day's list.
pub async fn measure(market: &dyn MarketSource, symbol: &str, name: &str) -> Option<Pick> {
    let bars = market.bars(symbol).await.ok()?;
    let closes: Vec<f64> = bars.iter().map(|b| b.close).collect();
    let ret5 = return_over(&closes, SESSIONS)?;
    Some(Pick {
        symbol: symbol.to_string(),
        name: name.to_string(),
        close: *closes.last()?,
        ret5,
    })
}

/// Positive first, then ranked, then cut to `top`.
pub fn rank(mut picks: Vec<Pick>, top: usize) -> Vec<Pick> {
    picks.retain(|p| p.ret5 > 0.0 && p.close > 0.0);
    picks.sort_by(|a, b| b.ret5.total_cmp(&a.ret5));
    picks.truncate(top);
    picks
}

/// Today's list: from the table if it was built, otherwise built and stored.
pub async fn daily_list(
    pool: &SqlitePool,
    market: &dyn MarketSource,
    now: DateTime<Utc>,
) -> Result<Vec<Pick>> {
    let day = ist_day(now);
    let stored = load(pool, &day).await?;
    if !stored.is_empty() {
        return Ok(stored);
    }

    let mut measured = Vec::new();
    for (symbol, name) in market.constituents().await.context("nifty 100 list")? {
        if let Some(p) = measure(market, &symbol, &name).await {
            measured.push(p);
        }
    }
    let picks = rank(measured, TOP);
    for p in &picks {
        store(pool, &day, p, now).await?;
    }
    // Set even when empty: an empty day is an answer, and a retry must not pay for it again.
    set_marker(pool, &day, now).await?;
    Ok(picks)
}

async fn load(pool: &SqlitePool, day: &str) -> Result<Vec<Pick>> {
    let rows: Vec<(String, String, f64, f64)> = sqlx::query_as(
        "SELECT symbol, name, close, ret5 FROM plan_candidates WHERE day = ? ORDER BY ret5 DESC",
    )
    .bind(day)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(symbol, name, close, ret5)| Pick {
            symbol,
            name,
            close,
            ret5,
        })
        .collect())
}

async fn store(pool: &SqlitePool, day: &str, p: &Pick, now: DateTime<Utc>) -> Result<()> {
    sqlx::query(
        "INSERT INTO plan_candidates (day, symbol, name, close, ret5, added_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(day, symbol) DO NOTHING",
    )
    .bind(day)
    .bind(&p.symbol)
    .bind(&p.name)
    .bind(p.close)
    .bind(p.ret5)
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

async fn set_marker(pool: &SqlitePool, day: &str, now: DateTime<Utc>) -> Result<()> {
    sqlx::query(
        "INSERT INTO settings (key, value, updated_at) VALUES ('plan_candidates_last', ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(day)
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

fn ist_day(now: DateTime<Utc>) -> String {
    (now + chrono::Duration::minutes(330))
        .format("%Y-%m-%d")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_csv_gives_bare_tickers_and_names() {
        let csv = "Company Name,Industry,Symbol,Series,ISIN Code\n\
                   ABB India Ltd.,Capital Goods,ABB,EQ,INE117A01022\n\
                   Adani Energy Solutions Ltd.,Power,ADANIENSOL,EQ,INE931S01010\n";
        let rows = parse_constituents(csv);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], ("ABB.NS".into(), "ABB India Ltd.".into()));
        assert_eq!(rows[1].0, "ADANIENSOL.NS");
    }

    #[test]
    fn a_header_row_is_not_a_constituent() {
        assert!(parse_constituents("Company Name,Industry,Symbol,Series\n").is_empty());
    }

    /// Five sessions needs six closes: the return is last against the close *before* the window.
    #[test]
    fn the_return_is_measured_over_five_sessions() {
        let closes = [100.0, 101.0, 102.0, 103.0, 104.0, 110.0];
        let r = return_over(&closes, 5).unwrap();
        assert!((r - 0.10).abs() < 1e-9, "{r}");
        assert!(return_over(&closes[..5], 5).is_none(), "five closes is not enough");
        assert!(return_over(&[100.0, 0.0, 1.0, 2.0, 3.0, 4.0], 5).is_none());
    }

    /// Positive only, ranked, cut: a market that fell must offer nothing rather than its least-bad.
    #[test]
    fn only_positive_returns_are_offered_best_first() {
        let pick = |s: &str, r: f64| Pick {
            symbol: format!("{s}.NS"),
            name: s.into(),
            close: 100.0,
            ret5: r,
        };
        let out = rank(vec![pick("A", 0.02), pick("B", -0.03), pick("C", 0.08)], 10);
        assert_eq!(
            out.iter().map(|p| p.symbol.as_str()).collect::<Vec<_>>(),
            vec!["C.NS", "A.NS"]
        );
        assert!(rank(vec![pick("A", -0.01), pick("B", -0.02)], 10).is_empty());
    }

    #[test]
    fn the_cap_is_ten() {
        let many: Vec<Pick> = (0..25)
            .map(|i| Pick {
                symbol: format!("S{i}.NS"),
                name: format!("S{i}"),
                close: 100.0,
                ret5: 0.001 * i as f64,
            })
            .collect();
        assert_eq!(rank(many, TOP).len(), TOP);
    }

    #[test]
    fn the_line_the_model_reads_names_the_window() {
        let p = Pick {
            symbol: "ABB.NS".into(),
            name: "ABB India".into(),
            close: 7400.5,
            ret5: 0.0612,
        };
        let line = p.line("INR");
        assert!(line.contains("7400.50 INR"), "{line}");
        assert!(line.contains("+6.12% over 5 sessions"), "{line}");
    }
}
