use std::collections::BTreeMap;

use anyhow::{anyhow, Result};

/// Deployment configuration only: where to listen, where the database lives, and the tuning
/// knobs. Anything a user edits in the UI — budget, currency, API keys, ntfy — lives in the
/// settings table and is never read from the environment.
#[derive(Debug, Clone)]
pub struct Config {
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
            poll_secs: num("POLL_SECS", 900.0)? as u64,
            llm_daily_cap: num("LLM_DAILY_CAP", 200.0)? as u32,
            llm_min_price_move: num("LLM_MIN_PRICE_MOVE", 2.0)?,
            llm_max_silence_secs: num("LLM_MAX_SILENCE_SECS", 3600.0)? as i64,
            prompt_window_hours: num("PROMPT_WINDOW_HOURS", 4.0)? as u32,
            off_hours_secs: num("OFF_HOURS_SECS", 7200.0)? as u64,
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
