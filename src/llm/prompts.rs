use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};

use crate::models::{Action, Candidate, Headline, Instrument};
use crate::rules::indicators::Indicators;

pub const DECIDE_V1: &str = include_str!("../../prompts/decide_v1.md");
pub const ESCALATE_V1: &str = include_str!("../../prompts/escalate_v1.md");

#[derive(Debug, Clone)]
pub struct RenderedPrompt {
    pub text: String,
    pub hash: String,
    pub template: String,
}

pub struct PromptContext<'a> {
    pub instrument: &'a Instrument,
    pub ind: &'a Indicators,
    pub headlines: &'a [Headline],
    pub candidate: &'a Candidate,
    pub cash_available: f64,
    pub class_rules: &'a str,
    pub search_summaries: &'a [String],
    /// Phase 2 only: extracted article text, already truncated by the caller.
    pub extracts: &'a [(i64, String)],
}

pub fn template(name: &str) -> Result<&'static str> {
    match name {
        "decide_v1" => Ok(DECIDE_V1),
        "escalate_v1" => Ok(ESCALATE_V1),
        other => Err(anyhow!("unknown prompt template {other:?}")),
    }
}

/// SHA-256 of the template file — not of the rendered text — so a signal can be traced to
/// the prompt version that produced it.
pub fn template_hash(name: &str) -> Result<String> {
    let t = template(name)?;
    let mut h = Sha256::new();
    h.update(t.as_bytes());
    Ok(format!("{:x}", h.finalize()))
}

pub fn render(name: &str, ctx: &PromptContext<'_>) -> Result<RenderedPrompt> {
    let tpl = template(name)?;
    let headline_block = ctx
        .headlines
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
                h.published_at
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let search_block = if ctx.search_summaries.is_empty() {
        "(none)".to_string()
    } else {
        ctx.search_summaries.join("\n---\n")
    };

    let extract_block = if ctx.extracts.is_empty() {
        "(none)".to_string()
    } else {
        ctx.extracts
            .iter()
            .map(|(id, text)| format!("### headline [{id}]\n{text}"))
            .collect::<Vec<_>>()
            .join("\n\n")
    };

    let candidate_block = format!(
        "action={}, entry={:.2}, stop={}, target={}, qty={}, urgent={}",
        ctx.candidate.action.as_str(),
        ctx.candidate.entry,
        ctx.candidate
            .stop
            .map(|v| format!("{v:.2}"))
            .unwrap_or_else(|| "n/a".into()),
        ctx.candidate
            .target
            .map(|v| format!("{v:.2}"))
            .unwrap_or_else(|| "n/a".into()),
        ctx.candidate.qty,
        ctx.candidate.urgent
    );

    let mut text = tpl
        .replace("{{symbol}}", &ctx.instrument.symbol)
        .replace("{{class}}", ctx.instrument.class.as_str())
        .replace(
            "{{last_close}}",
            &format!("{:.2}", ctx.candidate.entry),
        )
        .replace(
            "{{indicators}}",
            &format!(
                "sma50={:.2} sma200={:.2} rsi14={:.1} atr14={:.2} high52={:.2} low52={:.2}",
                ctx.ind.sma50, ctx.ind.sma200, ctx.ind.rsi14, ctx.ind.atr14, ctx.ind.high52, ctx.ind.low52
            ),
        )
        .replace("{{headlines}}", &headline_block)
        .replace("{{candidate}}", &candidate_block)
        .replace("{{cash}}", &format!("{:.2}", ctx.cash_available))
        .replace("{{class_rules}}", ctx.class_rules)
        .replace("{{search_summaries}}", &search_block)
        .replace("{{extracts}}", &extract_block);

    text.push_str(&format!("\n\n[reasons: {}]", ctx.candidate.reasons.join("; ")));

    Ok(RenderedPrompt {
        text,
        hash: template_hash(name)?,
        template: name.to_string(),
    })
}

/// Kept so the caller can render an empty-headline prompt without constructing a Vec.
pub fn empty_reasons(action: Action) -> String {
    format!("no rule reasons (action={})", action.as_str())
}
