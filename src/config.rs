use std::collections::BTreeMap;

use anyhow::{anyhow, Result};

/// Runtime configuration.
///
/// `seed_budget` and `seed_currency` are **first-boot seeds only**: they are written
/// into the `settings` table on first run and never read again. The authoritative
/// values live in the database and are edited from the UI.
#[derive(Debug, Clone)]
pub struct Config {
    pub seed_budget: f64,
    pub seed_currency: String,
    pub poll_secs: u64,
    pub llm_daily_cap: u32,
    /// Skip a model call unless the price moved at least this much...
    pub llm_min_price_move: f64,
    /// ...or this long has passed since the last one.
    pub llm_max_silence_secs: i64,
    /// How many hours of 15-minute tape go into every prompt. The biggest token lever.
    pub prompt_window_hours: u32,
    /// Poll cadence outside market hours, which only exists to pick up headlines.
    pub off_hours_secs: u64,
    pub ntfy_url: String,
    pub ntfy_topic: String,
    pub deepseek_key: String,
    pub deepseek_base: String,
    pub db_path: String,
    pub bind: String,
}

impl Config {
    pub fn from_map(m: &BTreeMap<String, String>) -> Result<Self> {
        let get = |k: &str| m.get(k).map(|s| s.as_str());
        let num = |k: &str, default: f64| -> Result<f64> {
            match get(k) {
                Some(v) => v
                    .parse::<f64>()
                    .map_err(|_| anyhow!("{k} must be a number, got {v:?}")),
                None => Ok(default),
            }
        };

        Ok(Self {
            seed_budget: num("BUDGET", 0.0)?,
            seed_currency: get("CURRENCY").unwrap_or("INR").to_string(),
            poll_secs: num("POLL_SECS", 900.0)? as u64,
            llm_daily_cap: num("LLM_DAILY_CAP", 200.0)? as u32,
            llm_min_price_move: num("LLM_MIN_PRICE_MOVE", 2.0)?,
            llm_max_silence_secs: num("LLM_MAX_SILENCE_SECS", 3600.0)? as i64,
            prompt_window_hours: num("PROMPT_WINDOW_HOURS", 4.0)? as u32,
            off_hours_secs: num("OFF_HOURS_SECS", 7200.0)? as u64,
            ntfy_url: get("NTFY_BASE_URL")
                .unwrap_or("https://ntfy.sh")
                .to_string(),
            ntfy_topic: get("NTFY_TOPIC").unwrap_or_default().to_string(),
            deepseek_key: get("DEEPSEEK_API_KEY").unwrap_or_default().to_string(),
            deepseek_base: get("DEEPSEEK_BASE_URL")
                .unwrap_or("https://api.deepseek.com")
                .to_string(),
            db_path: get("DB_PATH")
                .unwrap_or("data/stock-predicter.db")
                .to_string(),
            bind: get("BIND").unwrap_or("127.0.0.1:8080").to_string(),
        })
    }

    pub fn from_env() -> Result<Self> {
        Self::from_map(&std::env::vars().collect())
    }
}
