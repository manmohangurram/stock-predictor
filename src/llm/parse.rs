use anyhow::{anyhow, Result};
use serde::Deserialize;

use crate::models::Action;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct HeadlineRequest {
    pub headline_id: i64,
    pub why: String,
}

/// What the model asked for in its one escalation: specific article bodies, the coarser price
/// history, or both.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MoreRequest {
    pub headlines: Vec<HeadlineRequest>,
    pub price_history: bool,
}

impl MoreRequest {
    pub fn is_empty(&self) -> bool {
        self.headlines.is_empty() && !self.price_history
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    pub action: Action,
    pub entry: f64,
    pub stop: Option<f64>,
    pub target: Option<f64>,
    pub valid_until: String,
    pub confidence: Confidence,
    pub rationale: String,
    pub cites: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LlmVerdict {
    Decide(Box<Proposal>),
    NeedMore(MoreRequest),
    Pass { why: String },
}

#[derive(Deserialize)]
struct Envelope {
    action: String,
    signal: Option<SignalJson>,
    /// `headlines` is what the prompt asks for; `requests` is accepted for older transcripts.
    #[serde(alias = "requests")]
    headlines: Option<Vec<HeadlineRequest>>,
    #[serde(default)]
    price_history: Option<serde_json::Value>,
    why: Option<String>,
}

#[derive(Deserialize)]
struct SignalJson {
    action: String,
    entry: f64,
    stop: Option<f64>,
    target: Option<f64>,
    valid_until: String,
    confidence: Confidence,
    rationale: String,
    #[serde(default)]
    cites: Vec<i64>,
}

/// Anything that fails the shape is an error: the caller degrades to the rules candidate
/// rather than guessing what a truncated response meant.
pub fn verdict(content: &str) -> Result<LlmVerdict> {
    let env: Envelope = serde_json::from_str(content)
        .map_err(|e| anyhow!("llm response is not the agreed shape: {e}"))?;

    match env.action.as_str() {
        "decide" => {
            let s = env.signal.ok_or_else(|| anyhow!("decide without signal"))?;
            Ok(LlmVerdict::Decide(Box::new(Proposal {
                action: Action::parse(&s.action)?,
                entry: s.entry,
                stop: s.stop,
                target: s.target,
                valid_until: s.valid_until,
                confidence: s.confidence,
                rationale: s.rationale,
                cites: s.cites,
            })))
        }
        "need_more" => {
            let more = MoreRequest {
                headlines: env.headlines.unwrap_or_default(),
                // Models like to send `true`, `"7d"` or 7 here; any truthy value means yes.
                price_history: match env.price_history {
                    Some(serde_json::Value::Bool(b)) => b,
                    Some(serde_json::Value::String(s)) => !s.is_empty() && s != "false",
                    Some(serde_json::Value::Number(n)) => n.as_f64().unwrap_or(0.0) > 0.0,
                    _ => false,
                },
            };
            if more.is_empty() {
                return Err(anyhow!("need_more with nothing requested"));
            }
            Ok(LlmVerdict::NeedMore(more))
        }
        "pass" => Ok(LlmVerdict::Pass {
            why: env.why.unwrap_or_else(|| "no reason given".into()),
        }),
        other => Err(anyhow!("unknown verdict action {other:?}")),
    }
}
