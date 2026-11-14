use anyhow::{Context, Result};
use sqlx::SqlitePool;

use crate::models::{AssetClass, Mode};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CashKind {
    Budget,
    Buy,
    Sell,
    /// Unspent allowance removed at a month boundary. Not a trading loss.
    Expiry,
    /// The cash difference when a recorded fill is corrected. Never a trade of its own.
    Adjust,
}

impl CashKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CashKind::Budget => "budget",
            CashKind::Buy => "buy",
            CashKind::Sell => "sell",
            CashKind::Expiry => "expiry",
            CashKind::Adjust => "adjust",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewCashEntry {
    pub kind: CashKind,
    /// Signed. A budget delta, a negative buy, a positive sell, and so on.
    pub amount: f64,
    pub reference: String,
    /// Which purse inside the pot: `Some("mf")` for mutual funds, `None` for the pot's main one.
    pub bucket: Option<String>,
}

/// The purse a purchase draws on. A fund and a stock in the same pot are different money.
pub fn bucket_for(class: AssetClass) -> Option<&'static str> {
    match class {
        AssetClass::MutualFund => Some("mf"),
        _ => None,
    }
}

/// Units round **down** to 3 decimals before costing, so rounding can only under-spend.
pub fn units_floor(qty: f64) -> f64 {
    (qty * 1000.0).floor() / 1000.0
}

pub fn cost(qty: f64, price: f64, class: AssetClass) -> f64 {
    let q = match class {
        AssetClass::MutualFund => units_floor(qty),
        _ => qty,
    };
    if q <= 0.0 {
        0.0
    } else {
        q * price
    }
}

pub async fn budget_total_in(pool: &SqlitePool, mode: Mode) -> Result<f64> {
    let row: (Option<f64>,) =
        sqlx::query_as("SELECT SUM(amount) FROM cash_ledger WHERE kind = 'budget' AND mode = ?")
            .bind(mode.as_str())
            .fetch_one(pool)
            .await
            .context("sum budget")?;
    Ok(row.0.unwrap_or(0.0))
}

/// Open positions at cost, plus cash locked in pending IPO applications.
pub async fn committed(pool: &SqlitePool) -> Result<f64> {
    committed_in(pool, Mode::Long).await
}

pub async fn committed_in(pool: &SqlitePool, mode: Mode) -> Result<f64> {
    let pos: (Option<f64>,) = sqlx::query_as(
        "SELECT SUM(qty * avg_cost) FROM positions WHERE state <> 'closed' AND mode = ?",
    )
    .bind(mode.as_str())
    .fetch_one(pool)
    .await
    .context("sum positions")?;

    // IPO applications are a long-term instrument; trading never reserves for them.
    let ipo: (Option<f64>,) = match mode {
        Mode::Long => {
            sqlx::query_as("SELECT SUM(amount) FROM ipo_applications WHERE status = 'pending'")
                .fetch_one(pool)
                .await
                .context("sum ipo reservations")?
        }
        Mode::Trade => (None,),
    };

    Ok(pos.0.unwrap_or(0.0) + ipo.0.unwrap_or(0.0))
}

pub async fn available(pool: &SqlitePool) -> Result<f64> {
    available_in(pool, Mode::Long).await
}

/// The money that is actually there: every ledger row, not just the allowances. That is what
/// makes a sold profit spendable and an expired allowance gone.
pub async fn cash_in(pool: &SqlitePool, mode: Mode) -> Result<f64> {
    cash_in_bucket(pool, mode, None).await
}

/// Cash in one purse. `None` is the pot's main purse; `Some("mf")` is the mutual-fund purse.
pub async fn cash_in_bucket(pool: &SqlitePool, mode: Mode, bucket: Option<&str>) -> Result<f64> {
    let row: (Option<f64>,) = sqlx::query_as(
        "SELECT SUM(amount) FROM cash_ledger
         WHERE mode = ? AND COALESCE(bucket, '') = ?",
    )
    .bind(mode.as_str())
    .bind(bucket.unwrap_or(""))
    .fetch_one(pool)
    .await
    .context("sum cash")?;
    Ok(row.0.unwrap_or(0.0))
}

/// The same sum on an existing connection, so a fill checks the purse inside its own transaction.
pub async fn cash_in_bucket_on(
    conn: &mut sqlx::SqliteConnection,
    mode: Mode,
    bucket: Option<&str>,
) -> Result<f64> {
    let row: (Option<f64>,) = sqlx::query_as(
        "SELECT SUM(amount) FROM cash_ledger
         WHERE mode = ? AND COALESCE(bucket, '') = ?",
    )
    .bind(mode.as_str())
    .bind(bucket.unwrap_or(""))
    .fetch_one(&mut *conn)
    .await
    .context("sum cash")?;
    Ok(row.0.unwrap_or(0.0))
}

/// Free money in the mutual-fund purse of the long-term pot.
pub async fn available_mf(pool: &SqlitePool) -> Result<f64> {
    cash_in_bucket(pool, Mode::Long, Some("mf")).await
}

pub async fn available_in(pool: &SqlitePool, mode: Mode) -> Result<f64> {
    let ipo: (Option<f64>,) = match mode {
        Mode::Long => {
            sqlx::query_as("SELECT SUM(amount) FROM ipo_applications WHERE status = 'pending'")
                .fetch_one(pool)
                .await
                .context("sum ipo reservations")?
        }
        Mode::Trade => (None,),
    };
    Ok(cash_in(pool, mode).await? - ipo.0.unwrap_or(0.0))
}

pub async fn fits(pool: &SqlitePool, cost: f64) -> Result<bool> {
    fits_in(pool, cost, Mode::Long).await
}

pub async fn fits_in(pool: &SqlitePool, cost: f64, mode: Mode) -> Result<bool> {
    Ok(cost <= available_in(pool, mode).await? + 1e-9)
}

pub async fn record(pool: &SqlitePool, entry: NewCashEntry) -> Result<()> {
    record_at(pool, entry, &chrono::Utc::now().to_rfc3339()).await
}

/// The same row, stamped with when it actually happened. Backfilled trades must not land at
/// the top of the ledger just because they were entered today.
pub async fn record_at(pool: &SqlitePool, entry: NewCashEntry, ts: &str) -> Result<()> {
    record_at_mode(pool, entry, ts, Mode::Long).await
}

pub async fn record_at_mode(
    pool: &SqlitePool,
    entry: NewCashEntry,
    ts: &str,
    mode: Mode,
) -> Result<()> {
    insert_entry(pool, entry, ts, mode).await
}

/// The same insert on a connection, so a fill can post its money in the same transaction as the
/// position and the trade row. Half a fill is a corrupted book.
pub async fn record_on(
    conn: &mut sqlx::SqliteConnection,
    entry: NewCashEntry,
    ts: &str,
    mode: Mode,
) -> Result<()> {
    insert_entry(&mut *conn, entry, ts, mode).await
}

async fn insert_entry<'e, E>(executor: E, entry: NewCashEntry, ts: &str, mode: Mode) -> Result<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "INSERT INTO cash_ledger (ts, kind, amount, ref, mode, bucket) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(ts)
    .bind(entry.kind.as_str())
    .bind(entry.amount)
    .bind(entry.reference)
    .bind(mode.as_str())
    .bind(entry.bucket.as_deref())
    .execute(executor)
    .await
    .context("insert cash ledger")?;
    Ok(())
}

pub async fn reserve_ipo(
    pool: &SqlitePool,
    instrument_id: i64,
    qty: f64,
    price: f64,
) -> Result<()> {
    let amount = cost(qty, price, AssetClass::Ipo);
    sqlx::query(
        "INSERT INTO ipo_applications (instrument_id, qty, amount, status, applied_at)
         VALUES (?, ?, ?, 'pending', ?)",
    )
    .bind(instrument_id)
    .bind(qty)
    .bind(amount)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .context("insert ipo application")?;
    Ok(())
}
