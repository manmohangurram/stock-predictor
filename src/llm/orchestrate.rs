use anyhow::Result;
use sqlx::SqlitePool;

use super::deepseek::{self, ChatRequest, Transport};
use super::parse::{self, LlmVerdict, Proposal};
use super::prompts::{self, PromptContext};
use super::validate::{self, ValidateCtx};
use crate::market::MarketSource;
use crate::models::{Candidate, Headline, Instrument};
use crate::rules::indicators::{atr_bands, Indicators};

pub const MAX_HEADLINES_PER_ESCALATION: usize = 3;
pub const MAX_CHARS_PER_ARTICLE: usize = 4_000;
pub const MAX_TOTAL_FETCHED_CHARS: usize = 10_000;
/// Hard ceiling on any historical window the model can trigger. Requests above this are
/// clamped, never honoured — 7 days is the most the prompt will ever carry.
pub const MAX_HISTORY_DAYS: u32 = 7;

pub fn history_range() -> String {
    format!("{MAX_HISTORY_DAYS}d")
}

pub struct LlmDeps<'a> {
    pub pool: &'a SqlitePool,
    pub runtime: crate::rules::runtime::Runtime,
    pub transport: &'a dyn Transport,
    pub market: &'a dyn MarketSource,
}

pub struct RunInput<'a> {
    /// Why the gate let this call through, carried into the audit row.
    pub call_reason: Option<String>,
    pub instrument: &'a Instrument,
    pub ind: &'a Indicators,
    pub candidate: &'a Candidate,
    pub available_cash: f64,
    pub held_qty: f64,
    /// Limits, pot and current exposure: the same numbers the validator enforces, so the
    /// model is asked for something that can actually be accepted.
    pub limits: crate::rules::limits::Limits,
    /// This stock's expectation, already defaulted and clamped to the band by the caller.
    pub required_profit_pct: f64,
    pub instrument_exposure: f64,
    pub pot: f64,
    /// Hours of intraday tape included in the prompt.
    pub window_hours: u32,
    pub headlines: &'a [Headline],
    pub price_window: &'a str,
    pub daily_summary: &'a str,
    /// Revenue, profit and valuation as a few lines; "unavailable" when unfetched.
    pub fundamentals: &'a str,
    /// The short-horizon card; empty for the long-term book.
    pub short_context: &'a str,
    /// Which book this is: it decides the playbook.
    pub mode: crate::models::Mode,
    pub positions: &'a str,
    pub recent_trades: &'a str,
    pub last_decision: &'a str,
    pub now: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Llm(Proposal),
    Rules(Candidate),
    /// The model answered, but the proposal failed validation. The caller decides the
    /// consequence: over-budget becomes BLOCKED, everything else falls back to rules.
    Rejected {
        proposal: Box<Proposal>,
        error: validate::ValidationError,
    },
    Pass {
        why: String,
    },
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
        input.now.timestamp_millis()
    );

    if daily_chat_calls(deps.pool).await? >= deps.runtime.daily_cap as i64 {
        return Ok(Decision {
            outcome: Outcome::Rules(input.candidate.clone()),
            conversation_id,
        });
    }

    let mut headlines: Vec<Headline> = input.headlines.to_vec();
    for (idx, h) in headlines.iter_mut().enumerate() {
        h.id = idx as i64 + 1;
    }
    let sent_ids: Vec<i64> = headlines.iter().map(|h| h.id).collect();

    let base = PromptContext {
        instrument: input.instrument,
        ind: input.ind,
        last_close: input.candidate.entry,
        cash_available: input.available_cash,
        target_profit_pct: Some(input.required_profit_pct),
        limits: &limits_block(input.limits, input.pot, input.instrument_exposure),
        headlines: &headlines,
        window_hours: input.window_hours,
        price_window: input.price_window,
        daily_days: 10,
        daily_summary: input.daily_summary,
        fundamentals: input.fundamentals,
        short_context: input.short_context,
        positions: input.positions,
        recent_trades: input.recent_trades,
        last_decision: input.last_decision,
        now: input.now,
        extracts: &[],
        price_history: "",
    };

    let rendered = prompts::render(prompts::decide_template(input.mode), &base)?;
    let model = crate::rules::settings::model_config(deps.pool).await?;
    let reply = deepseek::chat(
        deps.pool,
        &model,
        deps.transport,
        ChatRequest {
            conversation_id: conversation_id.clone(),
            round: 1,
            prompt: rendered.text,
            prompt_template: rendered.template,
            prompt_hash: rendered.hash,
            trigger: input.call_reason.clone(),
            purpose: format!("decide {}", input.instrument.symbol),
        },
    )
    .await;

    let first = match reply {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "phase 1 failed, using rules numbers");
            return Ok(rules_only(&input, conversation_id));
        }
    };
    let verdict = match parse::verdict(&first.content) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "phase 1 unparseable, using rules numbers");
            return Ok(rules_only(&input, conversation_id));
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
        LlmVerdict::NeedMore(more) => {
            let wanted: Vec<i64> = more
                .headlines
                .iter()
                .take(MAX_HEADLINES_PER_ESCALATION)
                .map(|r| r.headline_id)
                .filter(|id| sent_ids.contains(id))
                .collect();

            let mut extracts: Vec<(i64, String)> = Vec::new();
            let can_fetch = model.provider.supports_article_fetch();
            let mut budget_chars = MAX_TOTAL_FETCHED_CHARS;
            for id in wanted {
                let Some(h) = headlines.iter().find(|h| h.id == id) else {
                    continue;
                };
                if !can_fetch {
                    continue;
                }
                let text =
                    match deepseek::fetch_page(deps.pool, &model.base_url, deps.transport, &h.url)
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

            // The second tier of the data pyramid: hourly bars, never more than 7 days.
            let history = if more.price_history {
                match deps
                    .market
                    .intraday(&input.instrument.symbol, "60m", &history_range())
                    .await
                {
                    Ok(bars) => format_history(&bars),
                    Err(e) => {
                        tracing::warn!(error = %e, "price history unavailable");
                        "(unavailable)".to_string()
                    }
                }
            } else {
                String::new()
            };

            if extracts.is_empty() && history.is_empty() {
                return Ok(Decision {
                    outcome: Outcome::Pass {
                        why: "asked for more, then received nothing usable".into(),
                    },
                    conversation_id,
                });
            }

            let ctx2 = PromptContext {
                extracts: &extracts,
                price_history: &history,
                ..base
            };
            let rendered2 = prompts::render("escalate_v1", &ctx2)?;
            let reply2 = deepseek::chat(
                deps.pool,
                &model,
                deps.transport,
                ChatRequest {
                    conversation_id: conversation_id.clone(),
                    round: 2,
                    prompt: rendered2.text,
                    prompt_template: rendered2.template,
                    prompt_hash: rendered2.hash,
                    trigger: Some("the first answer asked for more data".into()),
                    purpose: format!("escalate {}", input.instrument.symbol),
                },
            )
            .await;

            match reply2.ok().and_then(|r| parse::verdict(&r.content).ok()) {
                Some(LlmVerdict::Decide(p)) => LlmVerdict::Decide(p),
                // One escalation only; a second request is treated as a pass.
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
                // The trade book's horizon forces an exit, so it demands high confidence.
                requires_high_confidence: input.mode == crate::models::Mode::Trade,
                bands: atr_bands(input.candidate.entry, input.ind.atr14),
                atr: input.ind.atr14,
                available_cash: input.available_cash,
                qty: input.candidate.qty,
                held_qty: input.held_qty,
                sent_headline_ids: &sent_ids,
                requires_exit: input.instrument.class == crate::models::AssetClass::Stock,
                limits: input.limits,
                required_profit_pct: input.required_profit_pct,
                instrument_exposure: input.instrument_exposure,
                pot: input.pot,
                now: input.now,
            };
            match validate::check(&p, &ctx) {
                Ok(()) => Ok(Decision {
                    outcome: Outcome::Llm(*p),
                    conversation_id,
                }),
                Err(error) => {
                    tracing::warn!(%error, "llm proposal rejected");
                    Ok(Decision {
                        outcome: Outcome::Rejected { proposal: p, error },
                        conversation_id,
                    })
                }
            }
        }
        LlmVerdict::Pass { why } => Ok(Decision {
            outcome: Outcome::Pass { why },
            conversation_id,
        }),
        LlmVerdict::NeedMore(_) => Ok(rules_only(&input, conversation_id)),
    }
}

fn rules_only(input: &RunInput<'_>, conversation_id: String) -> Decision {
    Decision {
        outcome: Outcome::Rules(input.candidate.clone()),
        conversation_id,
    }
}

/// Hourly bars rendered compactly: one line per bar, oldest first.
pub fn format_history(bars: &[crate::models::Bar]) -> String {
    bars.iter()
        .map(|b| {
            let ts = chrono::DateTime::parse_from_rfc3339(&b.ts)
                .map(|d| {
                    d.with_timezone(&chrono::FixedOffset::east_opt(330 * 60).unwrap())
                        .format("%m-%d %H:%M")
                        .to_string()
                })
                .unwrap_or_else(|_| b.ts.clone());
            format!(
                "{} {:.2} {:.2} {:.2} {:.2} {}",
                ts, b.open, b.high, b.low, b.close, b.volume
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The limits as one readable paragraph, with the room that is actually left.
fn limits_block(limits: crate::rules::limits::Limits, pot: f64, already_spent: f64) -> String {
    let cap = pot * limits.max_instrument_pct / 100.0;
    let room = (cap - already_spent).max(0.0);
    format!(
        "trade size {:.2} to {:.2} per fill; at most {:.2} in this instrument ({:.0}% of the pot), \
         of which {:.2} is already committed, leaving {:.2} of room; take-profit between {:.1}% \
         and {:.1}% above entry",
        limits.min_trade_amount,
        limits.max_trade_amount,
        cap,
        limits.max_instrument_pct,
        already_spent,
        room,
        limits.min_profit_pct,
        limits.max_profit_pct
    )
}

pub async fn daily_chat_calls(pool: &SqlitePool) -> Result<i64> {
    let start = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let row: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM api_calls WHERE endpoint = 'chat' AND ts >= ?")
            .bind(start)
            .fetch_one(pool)
            .await?;
    Ok(row.0)
}
