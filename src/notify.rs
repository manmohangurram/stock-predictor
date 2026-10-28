use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use anyhow::{Context, Result};
use serde::Serialize;
use sqlx::SqlitePool;

use crate::config::Config;
use crate::models::{Action, Candidate};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    High,
    Default,
}

impl Priority {
    pub fn as_ntfy(self) -> &'static str {
        match self {
            Priority::High => "high",
            Priority::Default => "default",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Notification {
    pub title: String,
    pub body: String,
    pub priority: Priority,
    pub dedupe_key: String,
    pub signal_id: Option<i64>,
}

impl Notification {
    /// Copy follows the design system: one line, figures in mono, no adjectives.
    /// BLOCKED never produces a Notification — there is nothing to do about it.
    pub fn from_candidate(symbol: &str, currency: &str, c: &Candidate) -> Option<Self> {
        if !matches!(c.action, Action::Buy | Action::Sell) {
            return None;
        }
        let (title, priority) = match c.action {
            Action::Sell => (
                format!("SELL · {symbol} {:.0} units", c.qty),
                Priority::High,
            ),
            Action::Buy => (
                format!(
                    "BUY · {symbol} {} units · limit {:.2}",
                    fmt_qty(c.qty),
                    c.entry
                ),
                Priority::Default,
            ),
            _ => unreachable!(),
        };
        let body = match c.action {
            Action::Sell => {
                let stop = c
                    .stop
                    .map(|s| format!("stop {s:.2} hit, "))
                    .unwrap_or_default();
                format!("{stop}now {:.2} · {}", c.entry, c.timing_note)
            }
            _ => {
                // A buy is only actionable if it says where to get out on both sides.
                let exits = match (c.stop, c.target) {
                    (Some(s), Some(t)) => {
                        format!("sell if ≤ {s:.2} · take profit ≥ {t:.2} · ")
                    }
                    _ => String::new(),
                };
                format!(
                    "cost {:.2} {currency} · {exits}{}",
                    c.qty * c.entry,
                    c.timing_note
                )
            }
        };
        Some(Self {
            title,
            body,
            priority,
            dedupe_key: dedupe_key(symbol, c),
            signal_id: None,
        })
    }
}

fn fmt_qty(q: f64) -> String {
    if (q - q.round()).abs() < 1e-9 {
        format!("{q:.0}")
    } else {
        format!("{q:.3}")
    }
}

pub fn dedupe_key(symbol: &str, c: &Candidate) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(symbol.as_bytes());
    h.update(c.action.as_str().as_bytes());
    h.update(format!("{:.2}", c.entry).as_bytes());
    h.update(
        c.stop
            .map(|s| format!("{s:.2}"))
            .unwrap_or_default()
            .as_bytes(),
    );
    format!("{:x}", h.finalize())
}

pub trait Notifier: Send + Sync {
    fn send<'a>(
        &'a self,
        n: &'a Notification,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
}

pub struct Ntfy {
    client: reqwest::Client,
    url: String,
}

impl Ntfy {
    pub fn new(cfg: &Config) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .context("build ntfy client")?,
            url: format!(
                "{}/{}",
                cfg.ntfy_url.trim_end_matches('/'),
                cfg.ntfy_topic.trim_start_matches('/')
            ),
        })
    }
}

impl Notifier for Ntfy {
    fn send<'a>(
        &'a self,
        n: &'a Notification,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let resp = self
                .client
                .post(&self.url)
                .header("Title", encode_header(&n.title))
                .header("Priority", n.priority.as_ntfy())
                .header(
                    "Tags",
                    if n.priority == Priority::High {
                        "rotating_light"
                    } else {
                        "chart_with_upwards_trend"
                    },
                )
                .body(n.body.clone())
                .send()
                .await
                .context("ntfy: request")?;
            let status = resp.status();
            if !status.is_success() {
                anyhow::bail!("ntfy: http {status}");
            }
            Ok(())
        })
    }
}

/// ntfy headers are latin-1; the copy uses non-ASCII separators, so fold them down rather
/// than letting reqwest reject the request.
fn encode_header(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii() { c } else { '-' })
        .collect::<String>()
}

#[derive(Default)]
pub struct FakeNotifier {
    pub sent: Mutex<Vec<Notification>>,
    pub fail: bool,
}

impl FakeNotifier {
    pub fn failing() -> Self {
        Self {
            sent: Mutex::new(Vec::new()),
            fail: true,
        }
    }

    pub fn count(&self) -> usize {
        self.sent.lock().unwrap().len()
    }
}

impl Notifier for FakeNotifier {
    fn send<'a>(
        &'a self,
        n: &'a Notification,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if self.fail {
                anyhow::bail!("ntfy: simulated outage");
            }
            self.sent.lock().unwrap().push(n.clone());
            Ok(())
        })
    }
}

/// The row is written only after a successful send, so a failed push is retried on the next
/// tick instead of being silently swallowed.
pub async fn dispatch(
    pool: &SqlitePool,
    notifier: &dyn Notifier,
    n: &Notification,
) -> Result<bool> {
    let existing: Option<(i64,)> =
        sqlx::query_as("SELECT id FROM notifications WHERE dedupe_key = ?")
            .bind(&n.dedupe_key)
            .fetch_optional(pool)
            .await?;
    if existing.is_some() {
        return Ok(false);
    }

    notifier.send(n).await?;

    sqlx::query(
        "INSERT INTO notifications (signal_id, sent_at, channel, dedupe_key) VALUES (?, ?, 'ntfy', ?)",
    )
    .bind(n.signal_id)
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(&n.dedupe_key)
    .execute(pool)
    .await?;
    Ok(true)
}
