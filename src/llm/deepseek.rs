use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::Instant;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use super::audit::{self, ApiCallRow};
use crate::config::Config;

pub const MODEL: &str = "deepseek-chat";

#[derive(Debug, Clone, PartialEq)]
pub struct HttpReply {
    pub status: u16,
    pub body: String,
}

/// The one seam through which every outbound call goes, so tests never touch the network.
pub trait Transport: Send + Sync {
    fn post<'a>(
        &'a self,
        url: &'a str,
        body: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<HttpReply>> + Send + 'a>>;
}

pub struct ReqwestTransport {
    client: reqwest::Client,
    key: String,
}

impl ReqwestTransport {
    pub fn new(key: String) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .context("build deepseek client")?,
            key,
        })
    }
}

impl Transport for ReqwestTransport {
    fn post<'a>(
        &'a self,
        url: &'a str,
        body: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<HttpReply>> + Send + 'a>> {
        Box::pin(async move {
            let resp = self
                .client
                .post(url)
                .bearer_auth(&self.key)
                .header("Content-Type", "application/json")
                .body(body.to_string())
                .send()
                .await?;
            let status = resp.status().as_u16();
            Ok(HttpReply {
                status,
                body: resp.text().await?,
            })
        })
    }
}

/// A queue of canned replies, plus the urls it was asked for.
pub struct FixtureTransport {
    pub replies: Mutex<Vec<HttpReply>>,
    pub urls: Mutex<Vec<String>>,
}

impl FixtureTransport {
    pub fn new(replies: Vec<HttpReply>) -> Self {
        Self {
            replies: Mutex::new(replies),
            urls: Mutex::new(Vec::new()),
        }
    }

    pub fn ok(body: &str) -> Self {
        Self::new(vec![HttpReply {
            status: 200,
            body: body.to_string(),
        }])
    }

    pub fn calls(&self) -> usize {
        self.urls.lock().unwrap().len()
    }
}

impl Transport for FixtureTransport {
    fn post<'a>(
        &'a self,
        url: &'a str,
        _body: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<HttpReply>> + Send + 'a>> {
        self.urls.lock().unwrap().push(url.to_string());
        let next = {
            let mut q = self.replies.lock().unwrap();
            if q.is_empty() {
                None
            } else {
                Some(q.remove(0))
            }
        };
        Box::pin(async move { next.ok_or_else(|| anyhow!("fixture transport: no reply queued")) })
    }
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub conversation_id: String,
    pub round: i64,
    pub prompt: String,
    pub prompt_template: String,
    pub prompt_hash: String,
    pub purpose: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatReply {
    pub content: String,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub summary: String,
    pub date: Option<String>,
}

#[derive(Serialize)]
struct ChatBody<'a> {
    model: &'a str,
    messages: Vec<Msg<'a>>,
    response_format: ResponseFormat,
    temperature: f32,
}

#[derive(Serialize)]
struct Msg<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    kind: &'static str,
}

pub async fn chat(
    pool: &SqlitePool,
    cfg: &Config,
    transport: &dyn Transport,
    req: ChatRequest,
) -> Result<ChatReply> {
    let body = serde_json::to_string(&ChatBody {
        model: MODEL,
        messages: vec![Msg {
            role: "user",
            content: &req.prompt,
        }],
        response_format: ResponseFormat {
            kind: "json_object",
        },
        temperature: 0.2,
    })?;

    let url = format!(
        "{}/chat/completions",
        cfg.deepseek_base.trim_end_matches('/')
    );
    let started = Instant::now();
    let reply = transport.post(&url, &body).await;
    let latency = started.elapsed().as_millis() as i64;

    let mut row = ApiCallRow {
        conversation_id: Some(req.conversation_id.clone()),
        round: Some(req.round),
        endpoint: "chat".into(),
        purpose: Some(req.purpose.clone()),
        prompt_template: Some(req.prompt_template.clone()),
        prompt_hash: Some(req.prompt_hash.clone()),
        request_json: Some(body.clone()),
        latency_ms: Some(latency),
        ..Default::default()
    };

    let outcome = parse_chat(reply, &mut row);
    audit::record(pool, row).await?;
    outcome
}

fn parse_chat(reply: Result<HttpReply>, row: &mut ApiCallRow) -> Result<ChatReply> {
    let reply = match reply {
        Ok(r) => r,
        Err(e) => {
            row.error = Some(e.to_string());
            return Err(e);
        }
    };
    row.http_status = Some(reply.status as i64);
    row.response_json = Some(reply.body.clone());

    if !(200..300).contains(&reply.status) {
        row.error = Some(format!("http {}", reply.status));
        return Err(anyhow!("deepseek chat: http {}", reply.status));
    }

    let v: serde_json::Value = match serde_json::from_str(&reply.body) {
        Ok(v) => v,
        Err(e) => {
            row.error = Some(format!("malformed json: {e}"));
            return Err(anyhow!("deepseek chat: malformed json: {e}"));
        }
    };
    let content = v
        .pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .ok_or_else(|| {
            row.error = Some("missing choices[0].message.content".into());
            anyhow!("deepseek chat: missing content")
        })?
        .to_string();

    let prompt_tokens = v
        .pointer("/usage/prompt_tokens")
        .and_then(|t| t.as_i64())
        .unwrap_or(0);
    let completion_tokens = v
        .pointer("/usage/completion_tokens")
        .and_then(|t| t.as_i64())
        .unwrap_or(0);
    row.prompt_tokens = Some(prompt_tokens);
    row.completion_tokens = Some(completion_tokens);

    Ok(ChatReply {
        content,
        prompt_tokens,
        completion_tokens,
    })
}

pub async fn search(
    pool: &SqlitePool,
    cfg: &Config,
    transport: &dyn Transport,
    query: &str,
) -> Result<Vec<SearchHit>> {
    let body = serde_json::json!({
        "query": query,
        "count": 8,
        "need_content": false,
    })
    .to_string();
    let url = format!(
        "{}/v1/tools/search",
        cfg.deepseek_base.trim_end_matches('/')
    );
    let started = Instant::now();
    let reply = transport.post(&url, &body).await;
    let latency = started.elapsed().as_millis() as i64;

    let mut row = ApiCallRow {
        endpoint: "search".into(),
        purpose: Some(query.to_string()),
        request_json: Some(body),
        latency_ms: Some(latency),
        ..Default::default()
    };

    let outcome = (|| -> Result<Vec<SearchHit>> {
        let reply = match reply {
            Ok(r) => r,
            Err(e) => {
                row.error = Some(e.to_string());
                return Err(e);
            }
        };
        row.http_status = Some(reply.status as i64);
        row.response_json = Some(reply.body.clone());
        if !(200..300).contains(&reply.status) {
            row.error = Some(format!("http {}", reply.status));
            return Err(anyhow!("deepseek search: http {}", reply.status));
        }
        let v: serde_json::Value = serde_json::from_str(&reply.body)
            .map_err(|e| anyhow!("deepseek search: malformed json: {e}"))?;
        let results = v
            .get("results")
            .and_then(|r| r.as_array())
            .ok_or_else(|| anyhow!("deepseek search: missing results[]"))?;
        Ok(results
            .iter()
            .map(|r| SearchHit {
                title: r.get("title").and_then(|x| x.as_str()).unwrap_or("").into(),
                url: r.get("url").and_then(|x| x.as_str()).unwrap_or("").into(),
                summary: r
                    .get("summary")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .into(),
                date: r.get("date").and_then(|x| x.as_str()).map(str::to_string),
            })
            .collect())
    })();

    audit::record(pool, row).await?;
    outcome
}

pub async fn fetch_page(
    pool: &SqlitePool,
    cfg: &Config,
    transport: &dyn Transport,
    url_to_fetch: &str,
) -> Result<String> {
    let body = serde_json::json!({ "url": url_to_fetch }).to_string();
    let url = format!("{}/v1/tools/fetch", cfg.deepseek_base.trim_end_matches('/'));
    let started = Instant::now();
    let reply = transport.post(&url, &body).await;
    let latency = started.elapsed().as_millis() as i64;

    let mut row = ApiCallRow {
        endpoint: "fetch".into(),
        purpose: Some(url_to_fetch.to_string()),
        request_json: Some(body),
        latency_ms: Some(latency),
        ..Default::default()
    };

    let outcome = (|| -> Result<String> {
        let reply = match reply {
            Ok(r) => r,
            Err(e) => {
                row.error = Some(e.to_string());
                return Err(e);
            }
        };
        row.http_status = Some(reply.status as i64);
        row.response_json = Some(reply.body.clone());
        if !(200..300).contains(&reply.status) {
            row.error = Some(format!("http {}", reply.status));
            return Err(anyhow!("deepseek fetch: http {}", reply.status));
        }
        let v: serde_json::Value = serde_json::from_str(&reply.body)
            .map_err(|e| anyhow!("deepseek fetch: malformed json: {e}"))?;
        Ok(v.get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string())
    })();

    audit::record(pool, row).await?;
    outcome
}
