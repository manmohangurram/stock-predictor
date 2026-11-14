use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::models::Mode;
use crate::rules::budget::{self, bucket_for, CashKind, NewCashEntry};

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
    pub mode: String,
    /// The close on the day the fill happened, so the fill can be read against the market.
    pub market_close: Option<f64>,
}

/// Records a real fill: the history row, the position projection, and the cash movement all
/// move together or not at all.
/// When a fill happened. A bare `YYYY-MM-DD` means that trading day, stamped at the 15:30 IST
/// close, so a backfilled record sorts after the morning and before the next day.
pub fn parse_when(
    input: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<chrono::DateTime<chrono::Utc>> {
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

#[allow(clippy::too_many_arguments)]
pub async fn record(
    pool: &SqlitePool,
    instrument_id: i64,
    side: Side,
    qty: f64,
    price: f64,
    note: Option<&str>,
    at: Option<chrono::DateTime<chrono::Utc>>,
    mode: Mode,
) -> Result<()> {
    // The position, the trade row and the money move together or not at all.
    let mut tx = pool.begin().await?;
    record_on(&mut tx, instrument_id, side, qty, price, note, at, mode).await?;
    tx.commit().await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn record_on(
    conn: &mut sqlx::SqliteConnection,
    instrument_id: i64,
    side: Side,
    qty: f64,
    price: f64,
    note: Option<&str>,
    at: Option<chrono::DateTime<chrono::Utc>>,
    mode: Mode,
) -> Result<()> {
    ensure!(qty.is_finite() && qty > 0.0, "qty must be positive");
    ensure!(price.is_finite() && price > 0.0, "price must be positive");

    // A mutual fund has no intraday exit, so it cannot be a trading-pot fill.
    let class: (String,) = sqlx::query_as("SELECT class FROM instruments WHERE id = ?")
        .bind(instrument_id)
        .fetch_one(&mut *conn)
        .await?;
    let class = crate::models::AssetClass::parse(&class.0)?;
    ensure!(
        class.pot_allowed(mode),
        "{} cannot be held in the trading pot — mutual funds are long-term only",
        class.as_str()
    );

    let when = at.unwrap_or_else(chrono::Utc::now);
    let now = when.to_rfc3339();
    let open: Option<(i64, f64, f64, String)> = sqlx::query_as(
        "SELECT id, qty, avg_cost, opened_at FROM positions
         WHERE instrument_id = ? AND state = 'open' AND mode = ?
         ORDER BY id DESC LIMIT 1",
    )
    .bind(instrument_id)
    .bind(mode.as_str())
    .fetch_optional(&mut *conn)
    .await?;

    match side {
        Side::Buy => {
            let value = qty * price;
            let purse = budget::bucket_for(class);
            let available = budget::cash_in_bucket_on(&mut *conn, mode, purse).await?;
            ensure!(
                value <= available + 1e-9,
                "cost {value:.2} exceeds available cash {available:.2}"
            );

            match open {
                Some((id, held, cost, opened_at)) => {
                    let new_qty = held + qty;
                    let new_cost = (held * cost + value) / new_qty;
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
                    .execute(&mut *conn)
                    .await?;
                }
                None => {
                    sqlx::query(
                        "INSERT INTO positions (instrument_id, qty, avg_cost, opened_at, state, mode)
                         VALUES (?, ?, ?, ?, 'open', ?)",
                    )
                    .bind(instrument_id)
                    .bind(qty)
                    .bind(price)
                    .bind(&now)
                    .bind(mode.as_str())
                    .execute(&mut *conn)
                    .await?;
                }
            }

            insert_trade(
                &mut *conn,
                instrument_id,
                side,
                qty,
                price,
                None,
                &now,
                note,
                mode,
            )
            .await?;
            budget::record_on(
                &mut *conn,
                NewCashEntry {
                    kind: CashKind::Buy,
                    amount: -value,
                    reference: format!("buy {qty} @ {price}"),
                    bucket: bucket_for(class).map(str::to_string),
                },
                &now,
                mode,
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
                    .execute(&mut *conn)
                    .await?;
            } else {
                sqlx::query("UPDATE positions SET qty = ? WHERE id = ?")
                    .bind(remaining)
                    .bind(id)
                    .execute(&mut *conn)
                    .await?;
            }

            insert_trade(
                &mut *conn,
                instrument_id,
                side,
                qty,
                price,
                Some(realised),
                &now,
                note,
                mode,
            )
            .await?;
            budget::record_on(
                &mut *conn,
                NewCashEntry {
                    kind: CashKind::Sell,
                    amount: qty * price,
                    reference: format!("sell {qty} @ {price}"),
                    bucket: bucket_for(class).map(str::to_string),
                },
                &now,
                mode,
            )
            .await?;
        }
    }
    Ok(())
}

/// Correct a recorded fill.
///
/// The trade row is rewritten, the position moves by the difference, and the cash difference is
/// posted as its own `adjust` entry — the ledger stays append-only, so a correction is a visible
/// event rather than a silent rewrite of history.
pub async fn edit(
    pool: &SqlitePool,
    id: i64,
    qty: Option<f64>,
    price: Option<f64>,
    at: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<()> {
    // The trade row, the position and the correction entry move together.
    let mut tx = pool.begin().await?;
    edit_on(&mut tx, id, qty, price, at).await?;
    tx.commit().await?;
    Ok(())
}

/// The body of an edit, on a connection so it can join a caller's transaction.
pub async fn edit_on(
    conn: &mut sqlx::SqliteConnection,
    id: i64,
    qty: Option<f64>,
    price: Option<f64>,
    at: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<()> {
    #[derive(sqlx::FromRow)]
    struct Row {
        instrument_id: i64,
        side: String,
        qty: f64,
        price: f64,
        traded_at: String,
        mode: String,
    }
    let row: Row = sqlx::query_as(
        "SELECT instrument_id, side, qty, price, traded_at, mode FROM trades WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| anyhow::anyhow!("no fill {id}"))?;
    let mode = Mode::parse(&row.mode)?;
    let side = Side::parse(&row.side)?;
    let new_qty = qty.unwrap_or(row.qty);
    let new_price = price.unwrap_or(row.price);
    ensure!(new_qty.is_finite() && new_qty > 0.0, "qty must be positive");
    ensure!(
        new_price.is_finite() && new_price > 0.0,
        "price must be positive"
    );
    let when = at.unwrap_or_else(|| {
        chrono::DateTime::parse_from_rfc3339(&row.traded_at)
            .map(|d| d.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now())
    });

    let open: Option<(i64, f64, f64)> = sqlx::query_as(
        "SELECT id, qty, avg_cost FROM positions
         WHERE instrument_id = ? AND mode = ? ORDER BY state = 'open' DESC, id DESC LIMIT 1",
    )
    .bind(row.instrument_id)
    .bind(mode.as_str())
    .fetch_optional(&mut *conn)
    .await?;
    let (position_id, held, cost) = open.unwrap_or((0, 0.0, 0.0));

    let old_value = row.qty * row.price;
    let new_value = new_qty * new_price;
    // A fill is a movement of units and of money; an edit moves the difference.
    let delta_units = match side {
        Side::Buy => new_qty - row.qty,
        Side::Sell => row.qty - new_qty,
    };
    let delta_cash = match side {
        Side::Buy => old_value - new_value,
        Side::Sell => new_value - old_value,
    };
    ensure!(
        held + delta_units >= -1e-9,
        "that edit would leave {:.3} units held, which is less than nothing",
        held + delta_units
    );

    if position_id != 0 {
        let new_held = held + delta_units;
        let basis = held * cost
            + match side {
                Side::Buy => new_value - old_value,
                Side::Sell => 0.0,
            };
        let new_cost = if new_held > 1e-9 {
            basis / new_held
        } else {
            cost
        };
        sqlx::query("UPDATE positions SET qty = ?, avg_cost = ?, state = ? WHERE id = ?")
            .bind(new_held)
            .bind(new_cost)
            .bind(if new_held > 1e-9 { "open" } else { "closed" })
            .bind(position_id)
            .execute(&mut *conn)
            .await?;
    }

    let realised = match side {
        Side::Sell => Some(new_qty * (new_price - cost)),
        Side::Buy => None,
    };
    sqlx::query("UPDATE trades SET qty = ?, price = ?, realised = ?, traded_at = ? WHERE id = ?")
        .bind(new_qty)
        .bind(new_price)
        .bind(realised)
        .bind(when.to_rfc3339())
        .bind(id)
        .execute(&mut *conn)
        .await?;

    if delta_cash.abs() > 1e-9 {
        // Money back when the fill was overstated, more when it was understated.
        budget::record_on(
            &mut *conn,
            NewCashEntry {
                kind: CashKind::Adjust,
                amount: delta_cash,
                reference: format!("correct fill {id}"),
                bucket: None,
            },
            &when.to_rfc3339(),
            mode,
        )
        .await?;
    }
    Ok(())
}

/// Remove a fill entirely.
///
/// The trade row goes, the position moves back by exactly what the fill moved it, and the money
/// difference is posted as its own `adjust` entry — the ledger stays append-only. A fill that came
/// from an approved decision puts that decision back to `pending`, so the record never shows an
/// approval with no fill behind it.
pub async fn delete(pool: &SqlitePool, id: i64) -> Result<()> {
    let mut tx = pool.begin().await?;
    delete_on(&mut tx, id).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn delete_on(conn: &mut sqlx::SqliteConnection, id: i64) -> Result<()> {
    #[derive(sqlx::FromRow)]
    struct Row {
        instrument_id: i64,
        side: String,
        qty: f64,
        price: f64,
        note: Option<String>,
        mode: String,
    }
    let row: Row = sqlx::query_as(
        "SELECT instrument_id, side, qty, price, note, mode FROM trades WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| anyhow::anyhow!("no fill {id}"))?;
    let mode = Mode::parse(&row.mode)?;
    let side = Side::parse(&row.side)?;
    let value = row.qty * row.price;

    let open: Option<(i64, f64, f64)> = sqlx::query_as(
        "SELECT id, qty, avg_cost FROM positions
         WHERE instrument_id = ? AND mode = ? ORDER BY state = 'open' DESC, id DESC LIMIT 1",
    )
    .bind(row.instrument_id)
    .bind(mode.as_str())
    .fetch_optional(&mut *conn)
    .await?;
    let (position_id, held, cost) = open.unwrap_or((0, 0.0, 0.0));

    // Undo: a buy gives its units back, a sell returns the units it took.
    let delta_units = match side {
        Side::Buy => -row.qty,
        Side::Sell => row.qty,
    };
    ensure!(
        held + delta_units >= -1e-9,
        "deleting this fill would leave {:.3} units held — sell or correct the later fills first",
        held + delta_units
    );

    if position_id != 0 {
        let new_held = held + delta_units;
        let basis = held * cost - if side == Side::Buy { value } else { 0.0 };
        let new_cost = if new_held > 1e-9 {
            (basis / new_held).max(0.0)
        } else {
            cost
        };
        sqlx::query("UPDATE positions SET qty = ?, avg_cost = ?, state = ? WHERE id = ?")
            .bind(new_held)
            .bind(new_cost)
            .bind(if new_held > 1e-9 { "open" } else { "closed" })
            .bind(position_id)
            .execute(&mut *conn)
            .await?;
    }

    // The money comes back the way it went out: a deleted buy refunds, a deleted sell charges.
    let cash = match side {
        Side::Buy => value,
        Side::Sell => -value,
    };
    if cash.abs() > 1e-9 {
        budget::record_on(
            conn,
            NewCashEntry {
                kind: CashKind::Adjust,
                amount: cash,
                reference: format!("delete fill {id}"),
                bucket: None,
            },
            &chrono::Utc::now().to_rfc3339(),
            mode,
        )
        .await?;
    }

    sqlx::query("DELETE FROM trades WHERE id = ?")
        .bind(id)
        .execute(&mut *conn)
        .await?;

    // A fill that satisfied a decision no longer exists, so the decision is pending again.
    if let Some(decision) = row
        .note
        .as_deref()
        .and_then(|n| n.strip_prefix("decision "))
        .and_then(|n| n.trim().parse::<i64>().ok())
    {
        sqlx::query(
            "UPDATE signals SET status = 'pending', status_at = NULL, filled_qty = NULL,
                filled_price = NULL
             WHERE id = ?",
        )
        .bind(decision)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn insert_trade(
    conn: &mut sqlx::SqliteConnection,
    instrument_id: i64,
    side: Side,
    qty: f64,
    price: f64,
    realised: Option<f64>,
    at: &str,
    note: Option<&str>,
    mode: Mode,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO trades (instrument_id, side, qty, price, realised, traded_at, note, mode)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(instrument_id)
    .bind(side.as_str())
    .bind(qty)
    .bind(price)
    .bind(realised)
    .bind(at)
    .bind(note)
    .bind(mode.as_str())
    .execute(&mut *conn)
    .await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct TradeRow {
    id: i64,
    symbol: String,
    side: String,
    qty: f64,
    price: f64,
    realised: Option<f64>,
    traded_at: String,
    note: Option<String>,
    mode: String,
    market_close: Option<f64>,
}

pub async fn history(
    pool: &SqlitePool,
    instrument_id: Option<i64>,
    limit: i64,
) -> Result<Vec<Trade>> {
    const SELECT: &str = "SELECT t.id AS id, i.symbol AS symbol, t.side AS side, t.qty AS qty,
            t.price AS price, t.realised AS realised, t.traded_at AS traded_at, t.note AS note,
            t.mode AS mode,
            (SELECT b.close FROM bars b
             WHERE b.instrument_id = t.instrument_id
               AND substr(b.ts, 1, 10) = substr(t.traded_at, 1, 10)
             ORDER BY b.ts DESC LIMIT 1) AS market_close
        FROM trades t JOIN instruments i ON i.id = t.instrument_id";

    let rows: Vec<TradeRow> = match instrument_id {
        Some(id) => {
            sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "{SELECT} WHERE t.instrument_id = ? ORDER BY t.traded_at DESC LIMIT ?"
            )))
            .bind(id)
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "{SELECT} ORDER BY t.traded_at DESC LIMIT ?"
            )))
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
    };

    Ok(rows
        .into_iter()
        .map(|r| Trade {
            id: r.id,
            symbol: r.symbol,
            side: r.side,
            qty: r.qty,
            price: r.price,
            realised: r.realised,
            traded_at: r.traded_at,
            note: r.note,
            mode: r.mode,
            market_close: r.market_close,
        })
        .collect())
}

pub async fn realised_in(pool: &SqlitePool, mode: Mode) -> Result<f64> {
    let row: (Option<f64>,) =
        sqlx::query_as("SELECT SUM(realised) FROM trades WHERE side = 'sell' AND mode = ?")
            .bind(mode.as_str())
            .fetch_one(pool)
            .await?;
    Ok(row.0.unwrap_or(0.0))
}
