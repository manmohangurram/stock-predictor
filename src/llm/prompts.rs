use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use crate::models::{Headline, Instrument};
use crate::rules::indicators::Indicators;

pub const DECIDE_V1: &str = include_str!("../../prompts/decide_v1.md");
pub const ESCALATE_V1: &str = include_str!("../../prompts/escalate_v1.md");
pub const ROTATE_V1: &str = include_str!("../../prompts/rotate_v1.md");
pub const TREND_V1: &str = include_str!("../../prompts/trend_v1.md");

#[derive(Debug, Clone)]
pub struct RenderedPrompt {
    pub text: String,
    pub hash: String,
    pub template: String,
}

/// Everything the model is allowed to see, assembled by the caller. Nothing reaches the
/// prompt that is not in here — which is what keeps article bodies off the wire until asked.
pub struct PromptContext<'a> {
    pub instrument: &'a Instrument,
    pub ind: &'a Indicators,
    pub last_close: f64,
    pub cash_available: f64,
    /// The owner's profit expectation for this instrument, if any.
    pub target_profit_pct: Option<f64>,
    /// The pot's limits, already rendered.
    pub limits: &'a str,
    pub headlines: &'a [Headline],
    pub window_hours: u32,
    pub price_window: &'a str,
    pub daily_days: u32,
    pub daily_summary: &'a str,
    pub positions: &'a str,
    pub recent_trades: &'a str,
    pub last_decision: &'a str,
    pub now: DateTime<Utc>,
    /// Phase 2 only.
    pub extracts: &'a [(i64, String)],
    /// Phase 2 only: 7-day hourly bars, already clamped.
    pub price_history: &'a str,
}

pub fn template(name: &str) -> Result<&'static str> {
    match name {
        "decide_v1" => Ok(DECIDE_V1),
        "escalate_v1" => Ok(ESCALATE_V1),
        "rotate_v1" => Ok(ROTATE_V1),
        "trend_v1" => Ok(TREND_V1),
        other => Err(anyhow!("unknown prompt template {other:?}")),
    }
}

/// SHA-256 of the template file — not of the rendered text — so a signal can be traced to
/// the prompt version that produced it.
pub fn template_hash(name: &str) -> Result<String> {
    let mut h = Sha256::new();
    h.update(template(name)?.as_bytes());
    Ok(crate::hex(&h.finalize()))
}

/// The rotation prompt: a holdings table and one candidate, no market context.
pub fn render_rotation(holdings: &str, candidate: &str) -> Result<RenderedPrompt> {
    let template = template("rotate_v1")?;
    Ok(RenderedPrompt {
        template: "rotate_v1".into(),
        hash: template_hash("rotate_v1")?,
        text: template
            .replace("{{holdings}}", holdings)
            .replace("{{candidate}}", candidate),
    })
}

/// The short-term prompt: one mover, its tape, its headlines, and the book's cash.
pub fn render_trend(
    candidate: &str,
    tape: &str,
    headlines: &str,
    book: &str,
) -> Result<RenderedPrompt> {
    let template = template("trend_v1")?;
    Ok(RenderedPrompt {
        template: "trend_v1".into(),
        hash: template_hash("trend_v1")?,
        text: template
            .replace("{{candidate}}", candidate)
            .replace("{{tape}}", tape)
            .replace("{{headlines}}", headlines)
            .replace("{{book}}", book),
    })
}

pub fn render(name: &str, ctx: &PromptContext<'_>) -> Result<RenderedPrompt> {
    let headline_block = if ctx.headlines.is_empty() {
        "(no headlines in the last 7 days)".to_string()
    } else {
        ctx.headlines
            .iter()
            .map(|h| {
                format!(
                    "[{}] {} — {}, {}",
                    h.id,
                    h.title,
                    if h.publisher.is_empty() {
                        "unknown"
                    } else {
                        &h.publisher
                    },
                    &h.published_at[..10.min(h.published_at.len())]
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let extract_block = if ctx.extracts.is_empty() {
        "(none requested)".to_string()
    } else {
        ctx.extracts
            .iter()
            .map(|(id, text)| format!("### headline [{id}]\n{text}"))
            .collect::<Vec<_>>()
            .join("\n\n")
    };

    let history_block = if ctx.price_history.trim().is_empty() {
        "(not requested)".to_string()
    } else {
        ctx.price_history.to_string()
    };

    let text = template(name)?
        .replace("{{symbol}}", &ctx.instrument.symbol)
        .replace("{{class}}", ctx.instrument.class.as_str())
        .replace("{{currency}}", &ctx.instrument.currency)
        .replace("{{last_close}}", &format!("{:.2}", ctx.last_close))
        .replace(
            "{{now}}",
            &ctx.now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        )
        .replace(
            "{{indicators}}",
            &format!(
                "sma50={:.2}  sma200={:.2}  rsi14={:.1}  atr14={:.2}  52w low={:.2}  high={:.2}",
                ctx.ind.sma50,
                ctx.ind.sma200,
                ctx.ind.rsi14,
                ctx.ind.atr14,
                ctx.ind.low52,
                ctx.ind.high52
            ),
        )
        .replace("{{window_hours}}", &ctx.window_hours.to_string())
        .replace("{{price_window}}", ctx.price_window)
        .replace("{{daily_days}}", &ctx.daily_days.to_string())
        .replace("{{daily_summary}}", ctx.daily_summary)
        .replace("{{positions}}", ctx.positions)
        .replace("{{recent_trades}}", ctx.recent_trades)
        .replace("{{last_decision}}", ctx.last_decision)
        .replace("{{cash}}", &format!("{:.2}", ctx.cash_available))
        .replace("{{limits}}", ctx.limits)
        .replace(
            "{{target_profit}}",
            &match ctx.target_profit_pct {
                Some(p) => format!(
                    "at least {p:.1}% (a take-profit below {:.2} is rejected)",
                    ctx.last_close * (1.0 + p / 100.0)
                ),
                None => "no minimum set — choose what the setup justifies".to_string(),
            },
        )
        .replace("{{class_rules}}", ctx.instrument.class.timing_note())
        .replace("{{headlines}}", &headline_block)
        .replace("{{extracts}}", &extract_block)
        .replace("{{price_history}}", &history_block);

    Ok(RenderedPrompt {
        text,
        hash: template_hash(name)?,
        template: name.to_string(),
    })
}
