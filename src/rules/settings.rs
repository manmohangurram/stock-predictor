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
    /// Only ever a boolean. The key itself is never serialised — this server has no auth,
    /// so anything returned here is readable by anyone on the network.
    pub deepseek_configured: bool,
    /// The second pot. Kept separate on purpose: a swing trade must not starve a long-term buy.
    pub trading_budget: f64,
    pub trading_available: f64,
    /// Push channel. The topic is a name the user must be able to read back; the token never
    /// leaves the server.
    pub ntfy_url: String,
    pub ntfy_topic: String,
    pub ntfy_token_configured: bool,
    pub public_url: String,
}

#[derive(Debug, Clone, Default)]
pub struct SettingsPatch {
    pub budget: Option<f64>,
    pub currency: Option<String>,
    /// Empty or whitespace is ignored rather than clearing: a masked field saves the form
    /// without wiping a working key.
    pub deepseek_key: Option<String>,
    pub trading_budget: Option<f64>,
    pub ntfy_url: Option<String>,
    pub ntfy_topic: Option<String>,
    pub ntfy_token: Option<String>,
    pub public_url: Option<String>,
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
    let deepseek_configured = deepseek_key(pool).await?.is_some();
    let trading_budget = budget::budget_total_in(pool, crate::models::Mode::Trade).await?;
    let trading_available = budget::available_in(pool, crate::models::Mode::Trade).await?;
    let ntfy_topic = setting(pool, "ntfy_topic").await?.unwrap_or_default();
    let ntfy_url = setting(pool, "ntfy_url")
        .await?
        .unwrap_or_else(|| "https://ntfy.sh".to_string());
    let ntfy_token_configured = setting(pool, "ntfy_token")
        .await?
        .is_some_and(|t| !t.trim().is_empty());
    let public_url = setting(pool, "public_url").await?.unwrap_or_default();
    Ok(Settings {
        budget,
        committed,
        available: budget - committed,
        currency,
        currency_locked: locked,
        deepseek_configured,
        trading_budget,
        trading_available,
        ntfy_url,
        ntfy_topic,
        ntfy_token_configured,
        public_url,
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

    if let Some(trading) = patch.trading_budget {
        if !trading.is_finite() || trading < 0.0 {
            return Err(SettingsError::InvalidAmount);
        }
        let committed = budget::committed_in(pool, crate::models::Mode::Trade)
            .await
            .map_err(|_| SettingsError::InvalidAmount)?;
        if trading < committed - 1e-9 {
            return Err(SettingsError::BelowCommitted { committed });
        }
        let current = budget::budget_total_in(pool, crate::models::Mode::Trade)
            .await
            .map_err(|_| SettingsError::InvalidAmount)?;
        if (trading - current).abs() > 1e-9 {
            budget::record_at_mode(
                pool,
                NewCashEntry {
                    kind: CashKind::Budget,
                    amount: trading - current,
                    reference: format!("trading budget set to {trading}"),
                },
                &chrono::Utc::now().to_rfc3339(),
                crate::models::Mode::Trade,
            )
            .await
            .map_err(|_| SettingsError::InvalidAmount)?;
        }
    }

    for (name, value) in [
        ("ntfy_url", patch.ntfy_url),
        ("ntfy_topic", patch.ntfy_topic),
        ("public_url", patch.public_url),
    ] {
        if let Some(v) = value {
            // An empty string is a deliberate clear for these three; the token is different.
            put_setting(pool, name, v.trim())
                .await
                .map_err(|_| SettingsError::InvalidAmount)?;
        }
    }

    if let Some(token) = patch.ntfy_token {
        let token = token.trim();
        if !token.is_empty() {
            put_setting(pool, "ntfy_token", token)
                .await
                .map_err(|_| SettingsError::InvalidAmount)?;
        }
    }

    if let Some(key) = patch.deepseek_key {
        let key = key.trim();
        if !key.is_empty() {
            put_setting(pool, "deepseek_key", key)
                .await
                .map_err(|_| SettingsError::InvalidAmount)?;
        }
    }

    Ok(())
}

/// The key itself, for the client that actually calls DeepSeek. Never exposed over HTTP.
pub async fn deepseek_key(pool: &SqlitePool) -> Result<Option<String>> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT value FROM settings WHERE key = 'deepseek_key'")
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|r| r.0).filter(|k| !k.trim().is_empty()))
}

async fn put_setting(pool: &SqlitePool, key: &str, value: &str) -> Result<()> {
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

/// First boot only: seed the ceiling and the currency from env, then never read env again.
pub async fn seed(
    pool: &SqlitePool,
    seed_budget: f64,
    seed_currency: &str,
    seed_deepseek_key: &str,
) -> Result<()> {
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

    if !seed_currency.trim().is_empty() {
        let rows: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM settings WHERE key = 'currency'")
            .fetch_one(pool)
            .await?;
        if rows.0 == 0 {
            put_setting(pool, "currency", seed_currency).await?;
        }
    }
    if !seed_deepseek_key.trim().is_empty() {
        let rows: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM settings WHERE key = 'deepseek_key'")
                .fetch_one(pool)
                .await?;
        if rows.0 == 0 {
            put_setting(pool, "deepseek_key", seed_deepseek_key.trim()).await?;
        }
    }
    Ok(())
}

pub async fn setting(pool: &SqlitePool, key: &str) -> Result<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.0).filter(|v| !v.is_empty()))
}

/// The token itself, for the notifier. Never serialised.
pub async fn ntfy_token(pool: &SqlitePool) -> Result<Option<String>> {
    setting(pool, "ntfy_token").await
}

async fn currency(pool: &SqlitePool) -> Result<String> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT value FROM settings WHERE key = 'currency'")
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|r| r.0).unwrap_or_else(|| "INR".to_string()))
}

async fn ledger_has_entries(pool: &SqlitePool) -> Result<bool> {
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM cash_ledger WHERE kind <> 'budget'")
        .fetch_one(pool)
        .await?;
    Ok(row.0 > 0)
}
