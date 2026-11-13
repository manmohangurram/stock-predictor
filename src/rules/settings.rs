use anyhow::{Context, Result};
use serde::Serialize;
use sqlx::SqlitePool;
use thiserror::Error;

use super::budget;

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
    /// Long-term is split: this is the monthly allowance earmarked for mutual funds. The pot's
    /// `budget` is the stocks-and-cash allowance; the MF money is tracked against this line.
    pub mf_budget: f64,
    pub mf_available: f64,
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
    /// Cadence, gate thresholds and the proxy — read by the scheduler, edited on its own tabs.
    pub runtime: super::runtime::View,
}

#[derive(Debug, Clone, Default)]
pub struct SettingsPatch {
    pub budget: Option<f64>,
    pub currency: Option<String>,
    /// Empty or whitespace is ignored rather than clearing: a masked field saves the form
    /// without wiping a working key.
    pub deepseek_key: Option<String>,
    pub trading_budget: Option<f64>,
    pub mf_budget: Option<f64>,
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
    pub poll_secs: Option<u64>,
    pub off_hours_secs: Option<u64>,
    pub prompt_window_hours: Option<u32>,
    pub min_price_move: Option<f64>,
    pub daily_cap: Option<u32>,
    /// The short-term scan's filters: price floor, turnover floor, locked-band cut-off.
    pub scan_min_price: Option<f64>,
    pub scan_min_turnover: Option<f64>,
    pub scan_locked_pct: Option<f64>,
    /// Empty string clears it; absent leaves it alone.
    pub proxy: Option<String>,
    pub proxy_bypass: Option<String>,
    pub proxy_user: Option<String>,
    /// Empty or whitespace keeps the stored password, like every other secret field.
    pub proxy_pass: Option<String>,
}

#[derive(Debug, Error, PartialEq)]
pub enum SettingsError {
    #[error("currency is fixed once the ledger has entries")]
    CurrencyLocked,
    #[error("invalid amount")]
    InvalidAmount,
    #[error("{0}")]
    BadRequest(String),
}

/// `900.0` shows as `900`, so the settings table stays readable in a query.
fn trim_number(v: f64) -> String {
    if (v.fract()).abs() < 1e-9 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

pub async fn get(pool: &SqlitePool) -> Result<Settings> {
    let budget = allowance_setting(pool, crate::models::Mode::Long).await?;
    let committed = budget::committed(pool).await?;
    let currency = currency(pool).await?;
    let locked = ledger_has_entries(pool).await?;
    let deepseek_configured = deepseek_key(pool).await?.is_some();
    let trading_budget = allowance_setting(pool, crate::models::Mode::Trade).await?;
    let mf_budget = mf_allowance(pool).await?;
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
        // The money that is actually there, from the ledger — not the allowance minus holdings.
        available: budget::available_in(pool, crate::models::Mode::Long).await?,
        currency,
        currency_locked: locked,
        deepseek_configured,
        trading_budget,
        trading_available,
        mf_budget,
        mf_available: budget::available_mf(pool).await.unwrap_or(0.0),
        ntfy_url,
        ntfy_topic,
        ntfy_token_configured,
        llm_provider: provider.as_str().to_string(),
        llm_model,
        llm_base_url,
        llm_key_configured,
        limits_long,
        limits_trade,
        runtime: runtime(pool)
            .await
            .map_err(|_| SettingsError::InvalidAmount)?
            .view(),
    })
}

pub async fn set(pool: &SqlitePool, patch: SettingsPatch) -> Result<(), SettingsError> {
    for (key, value) in [
        ("poll_secs", patch.poll_secs.map(|v| v as f64)),
        ("off_hours_secs", patch.off_hours_secs.map(|v| v as f64)),
        (
            "prompt_window_hours",
            patch.prompt_window_hours.map(|v| v as f64),
        ),
        ("min_price_move", patch.min_price_move),
        ("daily_cap", patch.daily_cap.map(|v| v as f64)),
        ("scan_min_price", patch.scan_min_price),
        ("scan_min_turnover", patch.scan_min_turnover),
        ("scan_locked_pct", patch.scan_locked_pct),
    ] {
        if let Some(v) = value {
            super::runtime::Runtime::check(key, v).map_err(SettingsError::BadRequest)?;
            put_setting(pool, key, &trim_number(v))
                .await
                .map_err(|_| SettingsError::InvalidAmount)?;
        }
    }

    if let Some(proxy) = patch.proxy.as_deref() {
        let proxy = proxy.trim();
        if proxy.is_empty() {
            for key in ["proxy", "proxy_user", "proxy_pass"] {
                sqlx::query("DELETE FROM settings WHERE key = ?")
                    .bind(key)
                    .execute(pool)
                    .await
                    .map_err(|_| SettingsError::InvalidAmount)?;
            }
        } else {
            crate::http::validate_proxy(proxy)
                .map_err(|e| SettingsError::BadRequest(e.to_string()))?;
            put_setting(pool, "proxy", proxy)
                .await
                .map_err(|_| SettingsError::InvalidAmount)?;
        }
    }

    if let Some(user) = patch.proxy_user.as_deref() {
        put_setting(pool, "proxy_user", user.trim())
            .await
            .map_err(|_| SettingsError::InvalidAmount)?;
    }

    if let Some(pass) = patch.proxy_pass.as_deref() {
        let pass = pass.trim();
        if !pass.is_empty() {
            crate::secrets::put_secret(pool, "proxy_pass", pass)
                .await
                .map_err(|_| SettingsError::InvalidAmount)?;
        }
    }

    if let Some(bypass) = patch.proxy_bypass.as_deref() {
        put_setting(pool, "proxy_bypass", bypass.trim())
            .await
            .map_err(|_| SettingsError::InvalidAmount)?;
    }

    if let Some(budget) = patch.budget {
        if !budget.is_finite() || budget < 0.0 {
            return Err(SettingsError::InvalidAmount);
        }
        put_setting(pool, "allowance_long", &trim_number(budget))
            .await
            .map_err(|_| SettingsError::InvalidAmount)?;
        super::pot::ensure_month(
            pool,
            crate::models::Mode::Long,
            None,
            budget,
            chrono::Utc::now(),
        )
        .await
        .map_err(|_| SettingsError::InvalidAmount)?;
    }
    if let Some(budget) = patch.mf_budget {
        if !budget.is_finite() || budget < 0.0 {
            return Err(SettingsError::InvalidAmount);
        }
        put_setting(pool, "allowance_long_mf", &trim_number(budget))
            .await
            .map_err(|_| SettingsError::InvalidAmount)?;
        super::pot::ensure_month(
            pool,
            crate::models::Mode::Long,
            Some("mf"),
            budget,
            chrono::Utc::now(),
        )
        .await
        .map_err(|_| SettingsError::InvalidAmount)?;
    }

    if let Some(budget) = patch.trading_budget {
        if !budget.is_finite() || budget < 0.0 {
            return Err(SettingsError::InvalidAmount);
        }
        put_setting(pool, "allowance_trade", &trim_number(budget))
            .await
            .map_err(|_| SettingsError::InvalidAmount)?;
        // Apply it now, not on the 1st: setting an allowance should make it spendable.
        super::pot::ensure_month(
            pool,
            crate::models::Mode::Trade,
            None,
            budget,
            chrono::Utc::now(),
        )
        .await
        .map_err(|_| SettingsError::InvalidAmount)?;
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
            crate::secrets::put_secret(pool, key_name(provider), key)
                .await
                .map_err(|_| SettingsError::InvalidAmount)?;
        }
    }

    if let Some(token) = patch.ntfy_token {
        let token = token.trim();
        if !token.is_empty() {
            crate::secrets::put_secret(pool, "ntfy_token", token)
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

/// The configured monthly allowance: what the month reset grants on the 1st.
pub async fn allowance(pool: &SqlitePool, mode: crate::models::Mode) -> Result<f64> {
    allowance_setting(pool, mode).await
}

async fn allowance_setting(pool: &SqlitePool, mode: crate::models::Mode) -> Result<f64> {
    let key = match mode {
        crate::models::Mode::Long => "allowance_long",
        crate::models::Mode::Trade => "allowance_trade",
    };
    Ok(setting(pool, key)
        .await?
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v >= 0.0)
        .unwrap_or(0.0))
}

/// The monthly allowance earmarked for mutual funds, inside the long-term pot.
pub async fn mf_allowance(pool: &SqlitePool) -> Result<f64> {
    Ok(setting(pool, "allowance_long_mf")
        .await?
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v >= 0.0)
        .unwrap_or(0.0))
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
    crate::secrets::get_secret(pool, "ntfy_token").await
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
/// The key for one provider. The database wins; a developer's `.env` is the fallback, so a prompt
/// can be exercised locally without storing a key at rest. Nothing read from the environment is
/// ever written back to a setting.
pub async fn key_for(
    pool: &SqlitePool,
    provider: crate::llm::provider::Provider,
) -> Result<Option<String>> {
    if let Some(stored) = crate::secrets::get_secret(pool, key_name(provider)).await? {
        if !stored.trim().is_empty() {
            return Ok(Some(stored));
        }
    }
    Ok(std::env::var(crate::config::key_env_name(provider))
        .ok()
        .filter(|k| !k.trim().is_empty()))
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
