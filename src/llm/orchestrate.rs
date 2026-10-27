use anyhow::Result;
use sqlx::SqlitePool;

use super::deepseek::{self, ChatRequest};
use super::parse::{self, LlmVerdict, Proposal};
use super::prompts::{self, PromptContext};
use super::validate::{self, ValidateCtx};
use super::deepseek::Transport;
use crate::config::Config;
use crate::models::{Candidate, Headline, Instrument};
use crate::rules::indicators::{atr_bands, Indicators};

pub const MAX_HEADLINES_PER_ESCALATION: usize = 3;
pub const MAX_CHARS_PER_ARTICLE: usize = 4_000;
pub const MAX_TOTAL_FETCHED_CHARS: usize = 10_000;

pub struct LlmDeps<'a> {
    pub pool: &'a SqlitePool,
    pub cfg: &'a Config,
    pub transport: &'a dyn Transport,
}

pub struct RunInput<'a> {
    pub instrument: &'a Instrument,
    pub ind: &'a Indicators,
    pub candidate: &'a Candidate,
    pub available_cash: f64,
    pub held_qty: f64,
    pub headlines: &'a [Headline],
    pub search_summaries: &'a [String],
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Llm(Proposal),
    Rules(Candidate),
    Pass { why: String },
}

#[derive(Debug, Clone)]
pub struct Decision {
    pub outcome: Outcome,
    pub conversation_id: String,
}

pub async fn run(deps: &LlmDeps<'_>, input: RunInput<'_>) -> Result<Decision> {
    let conversation_id = format!(
        "{}-{}",
        input.instrument.symbol,
        chrono::Utc::now().timestamp_millis()
    );

    if daily_chat_calls(deps.pool).await? >= deps.cfg.llm_daily_cap as i64 {
        return Ok(Decision {
            outcome: Outcome::Rules(input.candidate.clone()),
            conversation_id,
        });
    }

    // Stable ids for this conversation: 1..=n in list order.
    let mut headlines: Vec<Headline> = input.headlines.to_vec();
    for (idx, h) in headlines.iter_mut().enumerate() {
        h.id = idx as i64 + 1;
    }
    let sent_ids: Vec<i64> = headlines.iter().map(|h| h.id).collect();

    let ctx = PromptContext {
        instrument: input.instrument,
        ind: input.ind,
        headlines: &headlines,
        candidate: input.candidate,
        cash_available: input.available_cash,
        class_rules: input.instrument.class.timing_note(),
        search_summaries: input.search_summaries,
        extracts: &[],
    };
    let rendered = prompts::render("decide_v1", &ctx)?;

    let reply = deepseek::chat(
        deps.pool,
        deps.cfg,
        deps.transport,
        ChatRequest {
            conversation_id: conversation_id.clone(),
            round: 1,
            prompt: rendered.text,
            prompt_template: rendered.template,
            prompt_hash: rendered.hash,
            purpose: format!("decide {}", input.instrument.symbol),
        },
    )
    .await;

    let first = match reply {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "phase 1 failed, using rules numbers");
            return Ok(Decision {
                outcome: Outcome::Rules(input.candidate.clone()),
                conversation_id,
            });
        }
    };

    let verdict = match parse::verdict(&first.content) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "phase 1 unparseable, using rules numbers");
            return Ok(Decision {
                outcome: Outcome::Rules(input.candidate.clone()),
                conversation_id,
            });
        }
    };

    let final_verdict = match verdict {
        LlmVerdict::Decide(p) => LlmVerdict::Decide(p),
        LlmVerdict::Pass { why } => {
            return Ok(Decision {
                outcome: Outcome::Pass { why },
                conversation_id,
            })
        }
        LlmVerdict::NeedMore(requests) => {
            if requests.is_empty() {
                return Ok(Decision {
                    outcome: Outcome::Pass {
                        why: "asked for more, then asked for nothing".into(),
                    },
                    conversation_id,
                });
            }
            let wanted: Vec<i64> = requests
                .iter()
                .take(MAX_HEADLINES_PER_ESCALATION)
                .map(|r| r.headline_id)
                .filter(|id| sent_ids.contains(id))
                .collect();
            if wanted.is_empty() {
                return Ok(Decision {
                    outcome: Outcome::Rules(input.candidate.clone()),
                    conversation_id,
                });
            }

            let mut extracts: Vec<(i64, String)> = Vec::new();
            let mut budget_chars = MAX_TOTAL_FETCHED_CHARS;
            for id in wanted {
                let Some(h) = headlines.iter().find(|h| h.id == id) else {
                    continue;
                };
                let text = match deepseek::fetch_page(deps.pool, deps.cfg, deps.transport, &h.url)
                    .await
                {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::warn!(error = %e, headline = id, "fetch failed");
                        continue;
                    }
                };
                let cap = MAX_CHARS_PER_ARTICLE.min(budget_chars);
                if cap == 0 {
                    break;
                }
                let truncated: String = text.chars().take(cap).collect();
                budget_chars -= truncated.chars().count();
                extracts.push((id, truncated));
            }

            if extracts.is_empty() {
                return Ok(Decision {
                    outcome: Outcome::Rules(input.candidate.clone()),
                    conversation_id,
                });
            }

            let ctx2 = PromptContext {
                headlines: &headlines,
                extracts: &extracts,
                ..ctx
            };
            let rendered2 = prompts::render("escalate_v1", &ctx2)?;
            let reply2 = deepseek::chat(
                deps.pool,
                deps.cfg,
                deps.transport,
                ChatRequest {
                    conversation_id: conversation_id.clone(),
                    round: 2,
                    prompt: rendered2.text,
                    prompt_template: rendered2.template,
                    prompt_hash: rendered2.hash,
                    purpose: format!("escalate {}", input.instrument.symbol),
                },
            )
            .await;

            match reply2.ok().and_then(|r| parse::verdict(&r.content).ok()) {
                Some(LlmVerdict::Decide(p)) => LlmVerdict::Decide(p),
                // A second request for articles is treated as a pass: one escalation only.
                Some(LlmVerdict::NeedMore(_)) | None => LlmVerdict::Pass {
                    why: "escalation did not resolve to a decision".into(),
                },
                Some(LlmVerdict::Pass { why }) => LlmVerdict::Pass { why },
            }
        }
    };

    match final_verdict {
        LlmVerdict::Decide(p) => {
            let ctx = ValidateCtx {
                bands: atr_bands(input.candidate.entry, input.ind.atr14),
                atr: input.ind.atr14,
                available_cash: input.available_cash,
                qty: input.candidate.qty,
                held_qty: input.held_qty,
                sent_headline_ids: &sent_ids,
                now: chrono::Utc::now(),
            };
            match validate::check(&p, &ctx) {
                Ok(()) => Ok(Decision {
                    outcome: Outcome::Llm(*p),
                    conversation_id,
                }),
                Err(e) => {
                    tracing::warn!(error = %e, "llm proposal rejected, using rules numbers");
                    Ok(Decision {
                        outcome: Outcome::Rules(input.candidate.clone()),
                        conversation_id,
                    })
                }
            }
        }
        LlmVerdict::Pass { why } => Ok(Decision {
            outcome: Outcome::Pass { why },
            conversation_id,
        }),
        LlmVerdict::NeedMore(_) => Ok(Decision {
            outcome: Outcome::Rules(input.candidate.clone()),
            conversation_id,
        }),
    }
}

pub async fn daily_chat_calls(pool: &SqlitePool) -> Result<i64> {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM api_calls WHERE endpoint = 'chat' AND substr(ts, 1, 10) = ?",
    )
    .bind(today)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}
