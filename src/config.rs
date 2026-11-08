use std::collections::BTreeMap;

use anyhow::{anyhow, Result};

/// Deployment facts only: where to listen, where the database lives, and the address the app is
/// reachable at. Everything a user edits in the UI — budget, currency, limits, API keys, ntfy,
/// cadence, the proxy — lives in the settings table and is never read from the environment.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub db_path: String,
    pub bind: String,
    /// Used only to build notification deep links.
    pub public_url: String,
    /// 32 bytes, from `SECRET_KEY` (64 hex chars or base64). `None` falls back to a key file
    /// beside the database, created on first boot.
    pub secret_key: Option<Vec<u8>>,
}

/// Variables that used to configure the app and are now settings. Boot warns when one is still
/// set, because a silently ignored `POLL_SECS=600` is an evening lost to wondering.
pub const RETIRED: &[&str] = &[
    "POLL_SECS",
    "OFF_HOURS_SECS",
    "LLM_DAILY_CAP",
    "LLM_MIN_PRICE_MOVE",
    "LLM_MAX_SILENCE_SECS",
    "PROMPT_WINDOW_HOURS",
    "DEEPSEEK_BASE_URL",
];

/// 32 bytes as 64 hex characters, or base64 (44 with padding, 43 without).
fn parse_key(raw: &str) -> Result<Vec<u8>> {
    let hex: Option<Vec<u8>> = if raw.len() == 64 && raw.chars().all(|c| c.is_ascii_hexdigit()) {
        (0..64)
            .step_by(2)
            .map(|i| u8::from_str_radix(&raw[i..i + 2], 16).ok())
            .collect()
    } else {
        None
    };
    if let Some(bytes) = hex.filter(|b| b.len() == 32) {
        return Ok(bytes);
    }
    use base64::Engine;
    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(raw) {
        if bytes.len() == 32 {
            return Ok(bytes);
        }
    }
    Err(anyhow!(
        "SECRET_KEY must be 32 bytes: 64 hex characters or base64 — generate one with \
         `openssl rand -hex 32`"
    ))
}

impl Config {
    pub fn from_map(m: &BTreeMap<String, String>) -> Result<Self> {
        let get = |k: &str| m.get(k).map(|s| s.as_str());
        let bind = get("BIND").unwrap_or("127.0.0.1:8080").to_string();
        if bind.parse::<std::net::SocketAddr>().is_err() {
            return Err(anyhow!("BIND must be host:port, got {bind:?}"));
        }
        let secret_key = match get("SECRET_KEY") {
            Some(raw) if !raw.trim().is_empty() => Some(parse_key(raw.trim())?),
            _ => None,
        };
        Ok(Self {
            db_path: get("DB_PATH")
                .unwrap_or("data/stock-predictor.db")
                .to_string(),
            bind,
            public_url: get("PUBLIC_URL").unwrap_or_default().trim_end_matches('/').to_string(),
            secret_key,
        })
    }

    pub fn from_env() -> Result<Self> {
        Self::from_map(&std::env::vars().collect())
    }

    /// Retired variable names still present in the environment. Non-empty means the operator is
    /// configuring something that no longer exists.
    pub fn retired_warnings() -> Vec<&'static str> {
        RETIRED
            .iter()
            .copied()
            .filter(|k| std::env::var(k).is_ok())
            .collect()
    }
}
