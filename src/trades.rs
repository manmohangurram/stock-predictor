use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::rules::budget::{self, CashKind, NewCashEntry};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub fn as_str(self) -> &'static str {
        match self {
            Side::Buy => "buy",
            Side::Sell => "sell",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "buy" => Ok(Side::Buy),
            "sell" => Ok(Side::Sell),
            other => anyhow::bail!("side must be buy or sell, got {other:?}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Trade {
    pub id: i64,
    pub symbol: String,
    pub side: String,
    pub qty: f64,
    pub price: f64,
    pub realised: Option<f64>,
    pub traded_at: String,
    pub note: Option<String>,
    /// The close on the day the fill happened, so the fill can be read against the market.
    pub market_close: Option<f64>,
}

/// Records a real fill: the history row, the position projection, and the cash movement all
/// move together or not at all.
/// When a fill happened. A bare `YYYY-MM-DD` means that trading day, stamped at the 15:30 IST
/// close, so a backfilled record sorts after the morning and before the next day.
pub fn parse_when(input: &str, now: chrono::DateTime<chrono::Utc>) -> Result<chrono::DateTime<chrono::Utc>> {
    let raw = input.trim();
    if raw.is_empty() {
        return Ok(now);
    }

    let parsed = if let Ok(full) = chrono::DateTime::parse_from_rfc3339(raw) {
        full.with_timezone(&chrono::Utc)
    } else {
        let date = chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d")
            .map_err(|_| anyhow::anyhow!("date must be YYYY-MM-DD or RFC3339, got {raw:?}"))?;
        let close = date
            .and_hms_opt(15, 30, 0)
            .ok_or_else(|| anyhow::anyhow!("bad time"))?;
        // 15:30 IST == 10:00 UTC; NSE has no DST, so a fixed offset is exact.
        chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
            close - chrono::Duration::minutes(330),
            chrono::Utc,
        )
    };

    if parsed > now + chrono::Duration::days(1) {
        anyhow::bail!("cannot record a fill in the future");
    }
    if parsed < chrono::DateTime::parse_from_rfc3339("1990-01-01T00:00:00Z").unwrap() {
        anyhow::bail!("that date is before any market this app knows about");
    }
    Ok(parsed)
}

pub async fn record(
    pool: &SqlitePool,
    instrument_id: i64,
    side: Side,
    qty: f64,
    price: f64,
    note: Option<&str>,
    at: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<()> {
    ensure!(qty.is_finite() && qty > 0.0, "qty must be positive");
    ensure!(price.is_finite() && price > 0.0, "price must be positive");

    let when = at.unwrap_or_else(chrono::Utc::now);
    let now = when.to_rfc3339();
    let open: Option<(i64, f64, f64, String)> = sqlx::query_as(
        "SELECT id, qty, avg_cost, opened_at FROM positions
         WHERE instrument_id = ? AND state = 'open' ORDER BY id DESC LIMIT 1",
    )
    .bind(instrument_id)
    .fetch_optional(pool)
    .await?;

    match side {
        Side::Buy => {
            let value = qty * price;
            let available = budget::available(pool).await?;
            ensure!(
                value <= available + 1e-9,
                "cost {value:.2} exceeds available cash {available:.2}"
            );

            match open {
                Some((id, held, cost, opened_at)) => {
                    let new_qty = held + qty;
                    let new_cost = (held * cost + value) / new_qty;
                    // A backfilled buy moves the position's start date backwards, otherwise a
                    // later-dated sale of it would be refused as "before the position opened".
                    let opened = chrono::DateTime::parse_from_rfc3339(&opened_at)
                        .map(|d| d.with_timezone(&chrono::Utc))
                        .unwrap_or(when);
                    let started = opened.min(when).to_rfc3339();
                    sqlx::query(
                        "UPDATE positions SET qty = ?, avg_cost = ?, opened_at = ? WHERE id = ?",
                    )
                    .bind(new_qty)
                    .bind(new_cost)
                    .bind(started)
                    .bind(id)
                    .execute(pool)
                    .await?;
                }
                None => {
                    sqlx::query(
                        "INSERT INTO positions (instrument_id, qty, avg_cost, opened_at, state)
                         VALUES (?, ?, ?, ?, 'open')",
                    )
                    .bind(instrument_id)
                    .bind(qty)
                    .bind(price)
                    .bind(&now)
                    .execute(pool)
                    .await?;
                }
            }

            insert_trade(pool, instrument_id, side, qty, price, None, &now, note).await?;
            budget::record_at(
                pool,
                NewCashEntry {
                    kind: CashKind::Buy,
                    amount: -value,
                    reference: format!("buy {qty} @ {price}"),
                },
                &now,
            )
            .await?;
        }
        Side::Sell => {
            let (id, held, cost, opened_at) =
                open.ok_or_else(|| anyhow::anyhow!("nothing is held to sell"))?;
            ensure!(qty <= held + 1e-9, "cannot sell {qty}, only {held} held");
            if let Ok(opened) = chrono::DateTime::parse_from_rfc3339(&opened_at) {
                ensure!(
                    when >= opened.with_timezone(&chrono::Utc),
                    "sell date is before the position was opened ({})",
                    opened_at
                );
            }

            let realised = qty * (price - cost);
            let remaining = held - qty;
            if remaining <= 1e-9 {
                sqlx::query("UPDATE positions SET qty = 0, state = 'closed' WHERE id = ?")
                    .bind(id)
                    .execute(pool)
                    .await?;
            } else {
                sqlx::query("UPDATE positions SET qty = ? WHERE id = ?")
                    .bind(remaining)
                    .bind(id)
                    .execute(pool)
                    .await?;
            }

            insert_trade(
                pool,
                instrument_id,
                side,
                qty,
                price,
                Some(realised),
                &now,
                note,
            )
            .await?;
            budget::record_at(
                pool,
                NewCashEntry {
                    kind: CashKind::Sell,
                    amount: qty * price,
                    reference: format!("sell {qty} @ {price}"),
                },
                &now,
            )
            .await?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn insert_trade(
    pool: &SqlitePool,
    instrument_id: i64,
    side: Side,
    qty: f64,
    price: f64,
    realised: Option<f64>,
    at: &str,
    note: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO trades (instrument_id, side, qty, price, realised, traded_at, note)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(instrument_id)
    .bind(side.as_str())
    .bind(qty)
    .bind(price)
    .bind(realised)
    .bind(at)
    .bind(note)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn history(pool: &SqlitePool, instrument_id: Option<i64>, limit: i64) -> Result<Vec<Trade>> {
    let select = "SELECT t.id, i.symbol, t.side, t.qty, t.price, t.realised, t.traded_at, t.note,
                         (SELECT b.close FROM bars b
                          WHERE b.instrument_id = t.instrument_id
                            AND substr(b.ts, 1, 10) = substr(t.traded_at, 1, 10)
                          ORDER BY b.ts DESC LIMIT 1)
                  FROM trades t JOIN instruments i ON i.id = t.instrument_id";
    let rows: Vec<(
        i64,
        String,
        String,
        f64,
        f64,
        Option<f64>,
        String,
        Option<String>,
        Option<f64>,
    )> = match instrument_id {
        Some(id) => {
            sqlx::query_as(&format!(
                "{select} WHERE t.instrument_id = ? ORDER BY t.traded_at DESC LIMIT ?"
            ))
            .bind(id)
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_as(&format!("{select} ORDER BY t.traded_at DESC LIMIT ?"))
                .bind(limit)
                .fetch_all(pool)
                .await?
        }
    };

    Ok(rows
        .into_iter()
        .map(|r| Trade {
            id: r.0,
            symbol: r.1,
            side: r.2,
            qty: r.3,
            price: r.4,
            realised: r.5,
            traded_at: r.6,
            note: r.7,
            market_close: r.8,
        })
        .collect())
}

pub async fn realised_total(pool: &SqlitePool) -> Result<f64> {
    let row: (Option<f64>,) = sqlx::query_as("SELECT SUM(realised) FROM trades WHERE side = 'sell'")
        .fetch_one(pool)
        .await?;
    Ok(row.0.unwrap_or(0.0))
}
