//! The month boundary of a pot.
//!
//! Money arrives once a month and does not roll over: on the first tick of a new month the previous
//! balance expires as its own ledger kind, then the allowance is granted. Both are inserts — the
//! ledger stays append-only — and the expiry is never counted as a trading result.

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Utc};
use sqlx::SqlitePool;

use super::budget::{self, CashKind, NewCashEntry};
use crate::models::Mode;

#[derive(Debug, Clone, PartialEq)]
pub struct MonthReport {
    pub month: String,
    /// Cash removed because the month ended. Zero when there was nothing left.
    pub expired: f64,
    /// The allowance granted for the new month. Zero when the month was already current.
    pub granted: f64,
}

const IST_OFFSET_MINUTES: i64 = 330;

/// `YYYY-MM` in IST — the month the market is in, not the server's month.
pub fn ist_month(now: DateTime<Utc>) -> String {
    let ist = now + chrono::Duration::minutes(IST_OFFSET_MINUTES);
    format!("{:04}-{:02}", ist.year(), ist.month())
}

fn month_key(mode: Mode) -> String {
    format!("pot_month_{}", mode.as_str())
}

/// Idempotent: a second call in the same month does nothing.
pub async fn ensure_month(
    pool: &SqlitePool,
    mode: Mode,
    allowance: f64,
    now: DateTime<Utc>,
) -> Result<MonthReport> {
    let month = ist_month(now);

    let seen: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?")
        .bind(month_key(mode))
        .fetch_optional(pool)
        .await?;
    if seen.map(|r| r.0).as_deref() == Some(month.as_str()) {
        return Ok(MonthReport {
            month,
            expired: 0.0,
            granted: 0.0,
        });
    }

    let existing = budget::cash_in(pool, mode).await?;
    let expired = if existing > 0.0 { existing } else { 0.0 };

    let mut tx = pool.begin().await.context("begin the month reset")?;
    if expired > 0.0 {
        sqlx::query("INSERT INTO cash_ledger (ts, kind, amount, ref, mode) VALUES (?, ?, ?, ?, ?)")
            .bind(now.to_rfc3339())
            .bind(CashKind::Expiry.as_str())
            .bind(-expired)
            .bind(format!("unspent allowance expired {}", month))
            .bind(mode.as_str())
            .execute(&mut *tx)
            .await?;
    }
    if allowance > 0.0 {
        sqlx::query("INSERT INTO cash_ledger (ts, kind, amount, ref, mode) VALUES (?, ?, ?, ?, ?)")
            .bind(now.to_rfc3339())
            .bind(CashKind::Budget.as_str())
            .bind(allowance)
            .bind(format!("allowance {}", month))
            .bind(mode.as_str())
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query(
        "INSERT INTO settings (key, value, updated_at) VALUES (?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(month_key(mode))
    .bind(&month)
    .bind(now.to_rfc3339())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(MonthReport {
        month,
        expired,
        granted: allowance,
    })
}

/// What expires at the end of the month, for the UI, without writing anything.
pub async fn month_state(pool: &SqlitePool, mode: Mode, now: DateTime<Utc>) -> Result<(String, f64)> {
    Ok((ist_month(now), budget::cash_in(pool, mode).await?))
}

/// A reference string the ledger and the tests can both recognise.
pub fn allowance_reference(month: &str) -> String {
    format!("allowance {month}")
}

#[allow(dead_code)]
fn _entry_shape(e: NewCashEntry) -> NewCashEntry {
    e
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    async fn pool() -> (tempfile::TempDir, SqlitePool) {
        let dir = tempfile::tempdir().unwrap();
        let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
            .await
            .unwrap();
        (dir, pool)
    }

    fn at(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        chrono::TimeZone::with_ymd_and_hms(&Utc, y, m, d, 6, 0, 0).unwrap()
    }

    #[tokio::test]
    async fn a_new_month_grants_the_allowance() {
        let (_d, pool) = pool().await;
        let r = ensure_month(&pool, Mode::Long, 50_000.0, at(2026, 11, 2)).await.unwrap();
        assert_eq!(r.granted, 50_000.0);
        assert_eq!(r.expired, 0.0);
        assert_eq!(budget::cash_in(&pool, Mode::Long).await.unwrap(), 50_000.0);
    }

    #[tokio::test]
    async fn the_same_month_is_never_granted_twice() {
        let (_d, pool) = pool().await;
        ensure_month(&pool, Mode::Long, 50_000.0, at(2026, 11, 2)).await.unwrap();
        let second = ensure_month(&pool, Mode::Long, 50_000.0, at(2026, 11, 20)).await.unwrap();
        assert_eq!(second.granted, 0.0);
        assert_eq!(budget::cash_in(&pool, Mode::Long).await.unwrap(), 50_000.0);
    }

    #[tokio::test]
    async fn unspent_cash_expires_and_is_not_a_trading_loss() {
        let (_d, pool) = pool().await;
        ensure_month(&pool, Mode::Long, 50_000.0, at(2026, 11, 2)).await.unwrap();
        let december = ensure_month(&pool, Mode::Long, 50_000.0, at(2026, 12, 2)).await.unwrap();
        assert_eq!(december.expired, 50_000.0, "November's remainder should have expired");
        assert_eq!(budget::cash_in(&pool, Mode::Long).await.unwrap(), 50_000.0);

        // Realised P&L is untouched by an expiry.
        let realised: (Option<f64>,) =
            sqlx::query_as("SELECT SUM(realised) FROM trades WHERE mode = 'long'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(realised.0, None);
    }

    #[tokio::test]
    async fn a_month_with_no_cash_writes_no_expiry() {
        let (_d, pool) = pool().await;
        let r = ensure_month(&pool, Mode::Trade, 0.0, at(2026, 11, 2)).await.unwrap();
        assert_eq!(r.expired, 0.0);
        let kinds: Vec<(String,)> = sqlx::query_as("SELECT kind FROM cash_ledger")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert!(kinds.iter().all(|k| k.0 != "expiry"), "{kinds:?}");
    }
}
