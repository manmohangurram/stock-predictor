use anyhow::{Context, Result};
use serde::Serialize;
use sqlx::SqlitePool;
use thiserror::Error;

use super::budget::{self, CashKind, NewCashEntry};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Settings {
    pub budget: f64,
    pub committed: f64,
    /// Serialised too: the UI and the API both need it, and deriving it twice invites drift.
    pub available: f64,
    pub currency: String,
    pub currency_locked: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SettingsPatch {
    pub budget: Option<f64>,
    pub currency: Option<String>,
}

#[derive(Debug, Error, PartialEq)]
pub enum SettingsError {
    #[error("committed is {committed}, budget cannot be lower")]
    BelowCommitted { committed: f64 },
    #[error("currency is fixed once the ledger has entries")]
    CurrencyLocked,
    #[error("invalid amount")]
    InvalidAmount,
}

pub async fn get(pool: &SqlitePool) -> Result<Settings> {
    let budget = budget::budget_total(pool).await?;
    let committed = budget::committed(pool).await?;
    let currency = currency(pool).await?;
    let locked = ledger_has_entries(pool).await?;
    Ok(Settings {
        budget,
        committed,
        available: budget - committed,
        currency,
        currency_locked: locked,
    })
}

pub async fn set(pool: &SqlitePool, patch: SettingsPatch) -> Result<(), SettingsError> {
    if let Some(budget) = patch.budget {
        if !budget.is_finite() || budget < 0.0 {
            return Err(SettingsError::InvalidAmount);
        }
        let committed = budget::committed(pool)
            .await
            .map_err(|_| SettingsError::InvalidAmount)?;
        if budget < committed - 1e-9 {
            return Err(SettingsError::BelowCommitted { committed });
        }
        let current = budget::budget_total(pool)
            .await
            .map_err(|_| SettingsError::InvalidAmount)?;
        if (budget - current).abs() > 1e-9 {
            budget::record(
                pool,
                NewCashEntry {
                    kind: CashKind::Budget,
                    amount: budget - current,
                    reference: format!("budget set to {budget}"),
                },
            )
            .await
            .map_err(|_| SettingsError::InvalidAmount)?;
        }
    }

    if let Some(currency) = patch.currency {
        if ledger_has_entries(pool)
            .await
            .map_err(|_| SettingsError::InvalidAmount)?
        {
            return Err(SettingsError::CurrencyLocked);
        }
        sqlx::query(
            "INSERT INTO settings (key, value, updated_at) VALUES ('currency', ?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(&currency)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(pool)
        .await
        .context("set currency")
        .map_err(|_| SettingsError::InvalidAmount)?;
    }

    Ok(())
}

/// First boot only: seed the ceiling and the currency from env, then never read env again.
pub async fn seed(pool: &SqlitePool, seed_budget: f64, seed_currency: &str) -> Result<()> {
    let budget_rows: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM cash_ledger WHERE kind = 'budget'")
            .fetch_one(pool)
            .await?;
    if budget_rows.0 == 0 && seed_budget > 0.0 {
        budget::record(
            pool,
            NewCashEntry {
                kind: CashKind::Budget,
                amount: seed_budget,
                reference: "seed from BUDGET".into(),
            },
        )
        .await?;
    }

    let currency_rows: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM settings WHERE key = 'currency'")
            .fetch_one(pool)
            .await?;
    if currency_rows.0 == 0 {
        sqlx::query("INSERT INTO settings (key, value, updated_at) VALUES ('currency', ?, ?)")
            .bind(seed_currency)
            .bind(chrono::Utc::now().to_rfc3339())
            .execute(pool)
            .await?;
    }
    Ok(())
}

async fn currency(pool: &SqlitePool) -> Result<String> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT value FROM settings WHERE key = 'currency'")
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|r| r.0).unwrap_or_else(|| "INR".to_string()))
}

async fn ledger_has_entries(pool: &SqlitePool) -> Result<bool> {
    let row: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM cash_ledger WHERE kind <> 'budget'")
            .fetch_one(pool)
            .await?;
    Ok(row.0 > 0)
}
