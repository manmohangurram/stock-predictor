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
}

/// Records a real fill: the history row, the position projection, and the cash movement all
/// move together or not at all.
pub async fn record(
    pool: &SqlitePool,
    instrument_id: i64,
    side: Side,
    qty: f64,
    price: f64,
    note: Option<&str>,
) -> Result<()> {
    ensure!(qty.is_finite() && qty > 0.0, "qty must be positive");
    ensure!(price.is_finite() && price > 0.0, "price must be positive");

    let now = chrono::Utc::now().to_rfc3339();
    let open: Option<(i64, f64, f64)> = sqlx::query_as(
        "SELECT id, qty, avg_cost FROM positions
         WHERE instrument_id = ? AND state = 'open' ORDER BY id DESC LIMIT 1",
    )
    .bind(instrument_id)
    .fetch_optional(pool)
    .await?;

    match side {
        Side::Buy => {
            let value = qty * price;
            ensure!(
                budget::fits(pool, value).await?,
                "this buy does not fit the budget"
            );

            match open {
                Some((id, held, cost)) => {
                    let new_qty = held + qty;
                    let new_cost = (held * cost + value) / new_qty;
                    sqlx::query("UPDATE positions SET qty = ?, avg_cost = ? WHERE id = ?")
                        .bind(new_qty)
                        .bind(new_cost)
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
            budget::record(
                pool,
                NewCashEntry {
                    kind: CashKind::Buy,
                    amount: -value,
                    reference: format!("buy {qty} @ {price}"),
                },
            )
            .await?;
        }
        Side::Sell => {
            let (id, held, cost) =
                open.ok_or_else(|| anyhow::anyhow!("nothing is held to sell"))?;
            ensure!(qty <= held + 1e-9, "cannot sell {qty}, only {held} held");

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
            budget::record(
                pool,
                NewCashEntry {
                    kind: CashKind::Sell,
                    amount: qty * price,
                    reference: format!("sell {qty} @ {price}"),
                },
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
    let rows: Vec<(i64, String, String, f64, f64, Option<f64>, String, Option<String>)> =
        match instrument_id {
            Some(id) => {
                sqlx::query_as(
                    "SELECT t.id, i.symbol, t.side, t.qty, t.price, t.realised, t.traded_at, t.note
                     FROM trades t JOIN instruments i ON i.id = t.instrument_id
                     WHERE t.instrument_id = ? ORDER BY t.traded_at DESC LIMIT ?",
                )
                .bind(id)
                .bind(limit)
                .fetch_all(pool)
                .await?
            }
            None => {
                sqlx::query_as(
                    "SELECT t.id, i.symbol, t.side, t.qty, t.price, t.realised, t.traded_at, t.note
                     FROM trades t JOIN instruments i ON i.id = t.instrument_id
                     ORDER BY t.traded_at DESC LIMIT ?",
                )
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
        })
        .collect())
}

pub async fn realised_total(pool: &SqlitePool) -> Result<f64> {
    let row: (Option<f64>,) = sqlx::query_as("SELECT SUM(realised) FROM trades WHERE side = 'sell'")
        .fetch_one(pool)
        .await?;
    Ok(row.0.unwrap_or(0.0))
}
