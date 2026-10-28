use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

/// The chat APIs this app can talk to. Everything except Gemini is OpenAI-compatible, so the
/// difference is the base URL, the model name and — for Gemini — the request shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Deepseek,
    Openai,
    Gemini,
    /// Any OpenAI-compatible endpoint: OpenRouter, Groq, Ollama, a proxy.
    Custom,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Deepseek => "deepseek",
            Provider::Openai => "openai",
            Provider::Gemini => "gemini",
            Provider::Custom => "custom",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "deepseek" => Ok(Provider::Deepseek),
            "openai" => Ok(Provider::Openai),
            "gemini" | "google" => Ok(Provider::Gemini),
            "custom" | "openai-compatible" => Ok(Provider::Custom),
            other => Err(anyhow!("unknown model provider {other:?}")),
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            Provider::Deepseek => "deepseek-chat",
            Provider::Openai => "gpt-4o-mini",
            Provider::Gemini => "gemini-2.0-flash",
            Provider::Custom => "",
        }
    }

    pub fn default_base_url(self) -> &'static str {
        match self {
            Provider::Deepseek => "https://api.deepseek.com",
            Provider::Openai => "https://api.openai.com/v1",
            Provider::Gemini => "https://generativelanguage.googleapis.com/v1beta",
            Provider::Custom => "",
        }
    }

    /// Only DeepSeek offers server-side page fetch, which the escalation step uses to read an
    /// article the model asked for. The others decide from the headlines as given.
    pub fn supports_article_fetch(self) -> bool {
        matches!(self, Provider::Deepseek)
    }
}

/// The resolved settings for one call.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelConfig {
    pub provider: Provider,
    pub model: String,
    pub base_url: String,
    pub api_key: String,
}

impl ModelConfig {
    pub fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url.trim_end_matches('/'), path.trim_start_matches('/'))
    }

    /// A Gemini key travels in the query string, not in an Authorization header.
    pub fn chat_url(&self) -> String {
        match self.provider {
            Provider::Gemini => format!(
                "{}?key={}",
                self.url(&format!("models/{}:generateContent", self.model)),
                self.api_key
            ),
            _ => self.url("chat/completions"),
        }
    }

    pub fn request_body(&self, prompt: &str) -> serde_json::Value {
        match self.provider {
            Provider::Gemini => serde_json::json!({
                "contents": [{ "role": "user", "parts": [{ "text": prompt }] }],
                "generationConfig": { "responseMimeType": "application/json", "temperature": 0.2 }
            }),
            _ => serde_json::json!({
                "model": self.model,
                "messages": [{ "role": "user", "content": prompt }],
                "response_format": { "type": "json_object" },
                "temperature": 0.2
            }),
        }
    }

    /// Pull the assistant text out of whichever envelope came back.
    pub fn extract_content(&self, body: &serde_json::Value) -> Option<String> {
        match self.provider {
            Provider::Gemini => body
                .pointer("/candidates/0/content/parts/0/text")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            _ => body
                .pointer("/choices/0/message/content")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        }
    }

    pub fn usage(&self, body: &serde_json::Value) -> (i64, i64) {
        match self.provider {
            Provider::Gemini => (
                body.pointer("/usageMetadata/promptTokenCount")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
                body.pointer("/usageMetadata/candidatesTokenCount")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
            ),
            _ => (
                body.pointer("/usage/prompt_tokens")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
                body.pointer("/usage/completion_tokens")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
            ),
        }
    }
}
