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
    /// Which chat API decisions come from, and the model name.
    pub llm_provider: String,
    pub llm_model: String,
    pub llm_base_url: String,
    /// True when a key is stored for the selected provider.
    pub llm_key_configured: bool,
    /// Risk limits, one set per pot.
    pub limits_long: crate::rules::limits::Limits,
    pub limits_trade: crate::rules::limits::Limits,
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
    pub llm_provider: Option<String>,
    pub llm_model: Option<String>,
    pub llm_base_url: Option<String>,
    /// Stored under the selected provider's own key name.
    pub llm_key: Option<String>,
    pub limits_long: Option<crate::rules::limits::Limits>,
    pub limits_trade: Option<crate::rules::limits::Limits>,
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
    let provider = crate::llm::provider::Provider::parse(
        &setting(pool, "llm_provider").await?.unwrap_or_default(),
    )
    .unwrap_or(crate::llm::provider::Provider::Deepseek);
    let llm_model = setting(pool, "llm_model")
        .await?
        .unwrap_or_else(|| provider.default_model().to_string());
    let llm_base_url = setting(pool, "llm_base_url")
        .await?
        .unwrap_or_else(|| provider.default_base_url().to_string());
    let llm_key_configured = key_for(pool, provider).await?.is_some();
    let limits_long = limits(pool, crate::models::Mode::Long).await?;
    let limits_trade = limits(pool, crate::models::Mode::Trade).await?;
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
        llm_provider: provider.as_str().to_string(),
        llm_model,
        llm_base_url,
        llm_key_configured,
        limits_long,
        limits_trade,
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
    ] {
        if let Some(v) = value {
            // An empty string is a deliberate clear for these; the token is different.
            put_setting(pool, name, v.trim())
                .await
                .map_err(|_| SettingsError::InvalidAmount)?;
        }
    }

    for (mode, limits) in [
        (crate::models::Mode::Long, patch.limits_long),
        (crate::models::Mode::Trade, patch.limits_trade),
    ] {
        if let Some(limits) = limits {
            if !limits.min_profit_pct.is_finite()
                || !limits.max_profit_pct.is_finite()
                || limits.min_profit_pct < 0.0
                || limits.max_profit_pct < limits.min_profit_pct
                || limits.min_trade_amount < 0.0
                || limits.max_trade_amount < limits.min_trade_amount
                || !(0.0..=100.0).contains(&limits.max_instrument_pct)
            {
                return Err(SettingsError::InvalidAmount);
            }
            for name in [
                "min_profit_pct",
                "max_profit_pct",
                "min_trade_amount",
                "max_trade_amount",
                "max_instrument_pct",
            ] {
                if let Some(value) = limits.field(name) {
                    put_setting(
                        pool,
                        &format!("{}_{name}", crate::rules::limits::Limits::key_prefix(mode)),
                        &value.to_string(),
                    )
                    .await
                    .map_err(|_| SettingsError::InvalidAmount)?;
                }
            }
        }
    }

    if let Some(provider) = patch.llm_provider.as_deref() {
        let p = crate::llm::provider::Provider::parse(provider)
            .map_err(|_| SettingsError::InvalidAmount)?;
        put_setting(pool, "llm_provider", p.as_str())
            .await
            .map_err(|_| SettingsError::InvalidAmount)?;
    }
    for (name, value) in [
        ("llm_model", patch.llm_model),
        ("llm_base_url", patch.llm_base_url),
    ] {
        if let Some(v) = value {
            put_setting(pool, name, v.trim())
                .await
                .map_err(|_| SettingsError::InvalidAmount)?;
        }
    }
    if let Some(key) = patch.llm_key {
        let key = key.trim();
        if !key.is_empty() {
            let provider = crate::llm::provider::Provider::parse(
                &setting(pool, "llm_provider")
                    .await
                    .map_err(|_| SettingsError::InvalidAmount)?
                    .unwrap_or_default(),
            )
            .unwrap_or(crate::llm::provider::Provider::Deepseek);
            put_setting(pool, key_name(provider), key)
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

/// The knobs the app reads while running. Read per tick so edits apply without a restart.
pub use super::runtime::load as runtime;

/// The key itself, for the client that actually calls DeepSeek. Never exposed over HTTP.
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

fn key_name(provider: crate::llm::provider::Provider) -> &'static str {
    match provider {
        crate::llm::provider::Provider::Deepseek => "deepseek_key",
        crate::llm::provider::Provider::Openai => "openai_key",
        crate::llm::provider::Provider::Gemini => "gemini_key",
        crate::llm::provider::Provider::Custom => "custom_key",
    }
}

/// The key for one provider, whatever is currently selected.
pub async fn key_for(
    pool: &SqlitePool,
    provider: crate::llm::provider::Provider,
) -> Result<Option<String>> {
    setting(pool, key_name(provider)).await
}

/// The limits for one pot, defaulted field by field so a partial configuration still works.
pub async fn limits(
    pool: &SqlitePool,
    mode: crate::models::Mode,
) -> Result<crate::rules::limits::Limits> {
    let mut limits = crate::rules::limits::Limits::default();
    for name in [
        "min_profit_pct",
        "max_profit_pct",
        "min_trade_amount",
        "max_trade_amount",
        "max_instrument_pct",
    ] {
        let key = format!("{}_{name}", crate::rules::limits::Limits::key_prefix(mode));
        if let Some(v) = setting(pool, &key).await? {
            if let Ok(value) = v.parse::<f64>() {
                if let Some(next) = limits.with(name, value) {
                    limits = next;
                }
            }
        }
    }
    Ok(limits)
}

/// Everything a chat call needs, resolved from the settings table.
pub async fn model_config(pool: &SqlitePool) -> Result<crate::llm::provider::ModelConfig> {
    let provider = crate::llm::provider::Provider::parse(
        &setting(pool, "llm_provider").await?.unwrap_or_default(),
    )
    .unwrap_or(crate::llm::provider::Provider::Deepseek);
    let model = setting(pool, "llm_model")
        .await?
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| provider.default_model().to_string());
    let base_url = setting(pool, "llm_base_url")
        .await?
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| provider.default_base_url().to_string());
    Ok(crate::llm::provider::ModelConfig {
        provider,
        model,
        base_url,
        api_key: key_for(pool, provider).await?.unwrap_or_default(),
    })
}

pub async fn deepseek_key(pool: &SqlitePool) -> Result<Option<String>> {
    key_for(pool, crate::llm::provider::Provider::Deepseek).await
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

/// First boot only: make sure the currency row exists. Everything else — budget, trading pot,
/// keys, ntfy — starts empty and is set in the UI, so there is exactly one source of truth.
pub async fn seed(pool: &SqlitePool) -> Result<()> {
    let rows: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM settings WHERE key = 'currency'")
        .fetch_one(pool)
        .await?;
    if rows.0 == 0 {
        put_setting(pool, "currency", "INR").await?;
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
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM cash_ledger WHERE kind <> 'budget'")
        .fetch_one(pool)
        .await?;
    Ok(row.0 > 0)
}
