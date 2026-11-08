//! Knobs the app reads while running, as opposed to the deployment facts in `Config`.
//!
//! These live in the `settings` table so they can be edited in the UI and take effect on the next
//! poll without a restart. Reading is deliberately forgiving: a corrupt row falls back to its
//! default, because a bad number must never stop the app from waking up.

use std::collections::BTreeMap;

use anyhow::Result;
use sqlx::SqlitePool;

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
    pub max_silence_secs: i64,
    /// Hard ceiling on model calls per day.
    pub daily_cap: u32,
    /// All outbound traffic goes through this, when set.
    pub proxy: Option<String>,
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
            max_silence_secs: 3_600,
            daily_cap: 200,
            proxy: None,
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
    ("max_silence_secs", 3_600.0, 60.0, 86_400.0),
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
            proxy: text("proxy"),
            proxy_bypass: text("proxy_bypass"),
        }
    }

    /// Every key this struct owns, with the value the UI should show for it.
    pub fn rows(&self) -> Vec<(&'static str, String)> {
        vec![
            ("poll_secs", self.poll_secs.to_string()),
            ("off_hours_secs", self.off_hours_secs.to_string()),
            ("prompt_window_hours", self.prompt_window_hours.to_string()),
            ("min_price_move", self.min_price_move.to_string()),
            ("max_silence_secs", self.max_silence_secs.to_string()),
            ("daily_cap", self.daily_cap.to_string()),
        ]
    }
}

/// One query per poll. Missing keys take their defaults, so an empty table is a working app.
pub async fn load(pool: &SqlitePool) -> Result<Runtime> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT key, value FROM settings")
        .fetch_all(pool)
        .await?;
    Ok(Runtime::from_map(&rows.into_iter().collect()))
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
