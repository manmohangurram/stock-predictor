use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use anyhow::{Context, Result};
use serde::Serialize;
use sqlx::SqlitePool;

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
    /// Used for the tap-through link; empty when there is nothing to open.
    pub symbol: String,
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
            symbol: symbol.to_string(),
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
    crate::hex(&h.finalize())
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
    token: String,
    public_url: String,
}

impl Ntfy {
    pub fn new(
        base_url: &str,
        topic: &str,
        token: &str,
        public_url: &str,
    ) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .context("build ntfy client")?,
            url: format!("{}/{}", base_url.trim_end_matches('/'), topic.trim_start_matches('/')),
            token: token.to_string(),
            public_url: public_url.trim_end_matches('/').to_string(),
        })
    }

    /// Build from the settings table, which is what the UI edits.
    pub async fn from_settings(pool: &SqlitePool, public_url: &str) -> Result<Option<Self>> {
        let s = crate::rules::settings::get(pool).await?;
        if s.ntfy_topic.trim().is_empty() {
            return Ok(None);
        }
        let token = crate::rules::settings::ntfy_token(pool)
            .await?
            .unwrap_or_default();
        Ok(Some(Self::new(&s.ntfy_url, &s.ntfy_topic, &token, public_url)?))
    }
}

/// ntfy headers are latin-1; the copy uses non-ASCII separators, so fold them down rather
/// than letting reqwest reject the request.
fn encode_header(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii() { c } else { '-' })
        .collect::<String>()
}

/// The headers and body ntfy expects, built separately so they can be asserted without a
/// network round trip. Kept in one place: a header typo used to mean a silently missing alert.
pub fn payload(n: &Notification, public_url: &str) -> (Vec<(String, String)>, String) {
    let mut headers = vec![
        ("Title".to_string(), encode_header(&n.title)),
        ("Priority".to_string(), n.priority.as_ntfy().to_string()),
        (
            "Tags".to_string(),
            if n.priority == Priority::High {
                "rotating_light".to_string()
            } else {
                "chart_with_upwards_trend".to_string()
            },
        ),
    ];
    if !public_url.is_empty() && !n.symbol.is_empty() {
        headers.push((
            "Click".to_string(),
            format!("{public_url}/i/{}", n.symbol),
        ));
    }
    (headers, n.body.clone())
}

impl Ntfy {
    /// The same send path, exposed for `--test-notify`.
    pub async fn send_test(&self, n: &Notification) -> Result<()> {
        Notifier::send(self, n).await
    }
}

impl Notifier for Ntfy {
    fn send<'a>(
        &'a self,
        n: &'a Notification,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let (headers, body) = payload(n, &self.public_url);
            let mut request = self.client.post(&self.url).body(body);
            for (k, v) in headers {
                request = request.header(k, v);
            }
            // A protected topic rejects anonymous posts; open topics ignore the header.
            if !self.token.is_empty() {
                request = request.bearer_auth(&self.token);
            }

            let resp = request.send().await.context("ntfy: request")?;
            let status = resp.status();
            if !status.is_success() {
                let detail = resp.text().await.unwrap_or_default();
                anyhow::bail!("ntfy: http {status} {}", detail.trim());
            }
            Ok(())
        })
    }
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
