//! Knobs the app reads while running, as opposed to the deployment facts in `Config`.
//!
//! These live in the `settings` table so they can be edited in the UI and take effect on the next
//! poll without a restart. Reading is deliberately forgiving: a corrupt row falls back to its
//! default, because a bad number must never stop the app from waking up.

use std::collections::BTreeMap;

use anyhow::Result;
use serde::Serialize;
use sqlx::SqlitePool;

/// What the API is allowed to say about the proxy: that it exists, and where to.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct View {
    pub poll_secs: u64,
    pub off_hours_secs: u64,
    pub prompt_window_hours: u32,
    pub min_price_move: f64,
    /// Reconsider a held position after this long **even with no price move**. 0 = never:
    /// a call is for a change, not for the passage of time.
    pub max_silence_secs: i64,
    pub daily_cap: u32,
    pub proxy_configured: bool,
    pub proxy_host: String,
    /// The username is shown back; the password never is.
    pub proxy_user: Option<String>,
    pub proxy_pass_configured: bool,
    pub proxy_bypass: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Runtime {
    /// Tick cadence while the market is open.
    pub poll_secs: u64,
    /// Tick cadence outside market hours, which only exists to pick up headlines.
    pub off_hours_secs: u64,
    /// Hours of 15-minute tape in every prompt. The biggest token lever.
    pub prompt_window_hours: u32,
    /// Skip a model call unless the price moved at least this much...
    pub min_price_move: f64,
    /// ...or this long has passed since the last one.
    /// Reconsider a held position after this long **even with no price move**. 0 = never:
    /// a call is for a change, not for the passage of time.
    pub max_silence_secs: i64,
    /// Hard ceiling on model calls per day.
    pub daily_cap: u32,
    /// All outbound traffic goes through this, when set. Never carries credentials.
    pub proxy: Option<String>,
    /// Proxy credentials, stored apart from the URL so the password can be masked like a key.
    pub proxy_user: Option<String>,
    pub proxy_pass: Option<String>,
    /// Comma-separated hosts that skip the proxy.
    pub proxy_bypass: Option<String>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            poll_secs: 900,
            off_hours_secs: 7_200,
            prompt_window_hours: 4,
            min_price_move: 2.0,
            max_silence_secs: 0,
            daily_cap: 200,
            proxy: None,
            proxy_user: None,
            proxy_pass: None,
            proxy_bypass: None,
        }
    }
}

/// `(key, default, lowest, highest)` — the table the migration and the UI both speak.
pub const KNOBS: &[(&str, f64, f64, f64)] = &[
    ("poll_secs", 900.0, 60.0, 86_400.0),
    ("off_hours_secs", 7_200.0, 60.0, 86_400.0),
    ("prompt_window_hours", 4.0, 1.0, 24.0),
    ("min_price_move", 2.0, 0.0, 1_000_000.0),
    ("max_silence_secs", 0.0, 0.0, 86_400.0),
    ("daily_cap", 200.0, 0.0, 10_000.0),
];

impl Runtime {
    pub fn from_map(m: &BTreeMap<String, String>) -> Self {
        let d = Self::default();
        let num = |key: &str, default: f64| -> f64 {
            let Some((_, _, lo, hi)) = KNOBS.iter().find(|(k, ..)| *k == key) else {
                return default;
            };
            match m.get(key) {
                Some(v) if !v.trim().is_empty() => match v.trim().parse::<f64>() {
                    Ok(n) if n.is_finite() => n.clamp(*lo, *hi),
                    _ => {
                        tracing::warn!(key, value = %v, "unreadable setting, using the default");
                        default
                    }
                },
                _ => default,
            }
        };
        let text = |key: &str| {
            m.get(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        Self {
            poll_secs: num("poll_secs", d.poll_secs as f64) as u64,
            off_hours_secs: num("off_hours_secs", d.off_hours_secs as f64) as u64,
            prompt_window_hours: num("prompt_window_hours", d.prompt_window_hours as f64) as u32,
            min_price_move: num("min_price_move", d.min_price_move),
            max_silence_secs: num("max_silence_secs", d.max_silence_secs as f64) as i64,
            daily_cap: num("daily_cap", d.daily_cap as f64) as u32,
            proxy: text("proxy").map(split_legacy_proxy).map(|p| p.0),
            proxy_user: text("proxy_user").or_else(|| text("proxy").and_then(|p| split_legacy_proxy(p).1)),
            proxy_pass: None,
            proxy_bypass: text("proxy_bypass"),
        }
    }

    pub fn view(&self) -> View {
        View {
            poll_secs: self.poll_secs,
            off_hours_secs: self.off_hours_secs,
            prompt_window_hours: self.prompt_window_hours,
            min_price_move: self.min_price_move,
            max_silence_secs: self.max_silence_secs,
            daily_cap: self.daily_cap,
            proxy_configured: self.proxy.is_some(),
            proxy_host: self.proxy.as_deref().map(crate::http::proxy_host).unwrap_or_default(),
            proxy_user: self.proxy_user.clone(),
            proxy_pass_configured: self.proxy_pass.is_some(),
            proxy_bypass: self.proxy_bypass.clone(),
        }
    }

    /// Range check used when a human writes a value: reject, never clamp. Clamping is for reads.
    pub fn check(key: &str, value: f64) -> Result<(), String> {
        let Some((_, _, lo, hi)) = KNOBS.iter().find(|(k, ..)| *k == key) else {
            return Ok(());
        };
        if !value.is_finite() || value < *lo || value > *hi {
            return Err(format!("{key} must be between {lo} and {hi}"));
        }
        Ok(())
    }
}

/// An older database stored `http://user:pass@host` in one row. Split it so both shapes work.
fn split_legacy_proxy(raw: String) -> (String, Option<String>, Option<String>) {
    let Ok(url) = reqwest::Url::parse(&raw) else {
        return (raw, None, None);
    };
    if url.username().is_empty() {
        return (raw, None, None);
    }
    let user = url.username().to_string();
    let pass = url.password().map(str::to_string);
    let mut clean = url.clone();
    let _ = clean.set_username("");
    let _ = clean.set_password(None);
    (clean.to_string(), Some(user), pass)
}

/// One query per poll. Missing keys take their defaults, so an empty table is a working app.
pub async fn load(pool: &SqlitePool) -> Result<Runtime> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT key, value FROM settings")
        .fetch_all(pool)
        .await?;
    let mut rt = Runtime::from_map(&rows.into_iter().collect());
    // The password is encrypted at rest, so it does not come back through `from_map`.
    rt.proxy_pass = crate::secrets::get_secret(pool, "proxy_pass").await?;
    Ok(rt)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn an_empty_table_is_the_default_runtime() {
        assert_eq!(Runtime::from_map(&m(&[])), Runtime::default());
    }

    #[test]
    fn values_are_clamped_into_their_range() {
        let r = Runtime::from_map(&m(&[
            ("poll_secs", "5"),
            ("prompt_window_hours", "99"),
            ("daily_cap", "-3"),
        ]));
        assert_eq!(r.poll_secs, 60);
        assert_eq!(r.prompt_window_hours, 24);
        assert_eq!(r.daily_cap, 0);
    }

    #[test]
    fn a_corrupt_row_falls_back_to_the_default() {
        let r = Runtime::from_map(&m(&[("poll_secs", "often"), ("unknown", "x")]));
        assert_eq!(r.poll_secs, 900);
        let r = Runtime::from_map(&m(&[("min_price_move", "NaN")]));
        assert_eq!(r.min_price_move, 2.0);
    }

    #[test]
    fn blank_text_means_no_proxy() {
        let r = Runtime::from_map(&m(&[("proxy", "  ")]));
        assert_eq!(r.proxy, None);
    }
}
