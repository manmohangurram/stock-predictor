use anyhow::{Context, Result};
use sqlx::SqlitePool;

use crate::models::{AssetClass, Mode};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CashKind {
    Budget,
    Buy,
    Sell,
    Reserve,
    Release,
}

impl CashKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CashKind::Budget => "budget",
            CashKind::Buy => "buy",
            CashKind::Sell => "sell",
            CashKind::Reserve => "reserve",
            CashKind::Release => "release",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewCashEntry {
    pub kind: CashKind,
    /// Signed. A budget delta, a negative buy, a positive sell, and so on.
    pub amount: f64,
    pub reference: String,
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

/// The ceiling itself: the sum of every `budget` ledger row. One source of truth — the
/// ledger — rather than a settings value that could drift away from its own history.
pub async fn budget_total(pool: &SqlitePool) -> Result<f64> {
    budget_total_in(pool, Mode::Long).await
}

pub async fn budget_total_in(pool: &SqlitePool, mode: Mode) -> Result<f64> {
    let row: (Option<f64>,) = sqlx::query_as(
        "SELECT SUM(amount) FROM cash_ledger WHERE kind = 'budget' AND mode = ?",
    )
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

pub async fn available_in(pool: &SqlitePool, mode: Mode) -> Result<f64> {
    Ok(budget_total_in(pool, mode).await? - committed_in(pool, mode).await?)
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
    sqlx::query("INSERT INTO cash_ledger (ts, kind, amount, ref, mode) VALUES (?, ?, ?, ?, ?)")
        .bind(ts)
        .bind(entry.kind.as_str())
        .bind(entry.amount)
        .bind(entry.reference)
        .bind(mode.as_str())
        .execute(pool)
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
