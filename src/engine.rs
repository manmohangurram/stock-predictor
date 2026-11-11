use anyhow::Result;
use chrono::{DateTime, Datelike, Utc};
use sqlx::SqlitePool;

use crate::llm::deepseek::Transport;
use crate::llm::orchestrate::{self, LlmDeps, Outcome, RunInput};
use crate::llm::parse::Proposal;
use crate::models::{Action, AssetClass, Bar, Candidate, Headline, Instrument, Position};
use crate::notify::{self, Notification, Notifier};
use crate::rules::budget;
use crate::rules::evaluate::{self, CandidateInput, RuleConfig};
use crate::rules::indicators::{self, atr_bands, Indicators};

// Re-exported so tests and callers keep importing them from the engine.
pub use crate::market::{MarketSource, YahooSource};

pub struct EngineDeps<'a> {
    pub pool: &'a SqlitePool,
    /// Read at the top of every tick, so an edit in the UI applies without a restart.
    pub runtime: crate::rules::runtime::Runtime,
    pub market: &'a dyn MarketSource,
    pub transport: &'a dyn Transport,
    pub notifier: &'a dyn Notifier,
    pub rules: RuleConfig,
    /// Skip the quiet-tick dedupe. Used by \`--predict\`, where a human asked for an answer
    /// right now and "nothing changed since last time" is not an answer.
    pub force: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PollReport {
    /// One human-readable line per decision, for \`--predict\`.
    pub notes: Vec<String>,
    pub candidates: usize,
    pub llm_calls: i64,
    pub signals: usize,
    pub notifications: usize,
    pub blocked: usize,
    /// Instruments this tick looked at: the denominator for "2 of 6 instruments asked".
    pub polled: usize,
    /// Instruments the model was actually asked about this tick (ruling and re-priced alike).
    pub asked: usize,
    /// Instruments the gate left alone this tick: no price move, no new headline.
    pub not_asked: Vec<String>,
    /// Plan calls that produced a plan. Zero is the normal answer.
    pub plans: usize,
    pub errors: Vec<String>,
}

/// The resolved decision for one instrument, whichever layer produced it.
struct Resolved {
    action: Action,
    entry: f64,
    stop: Option<f64>,
    target: Option<f64>,
    valid_until: Option<String>,
    qty: f64,
    reasons: Vec<String>,
    rationale: Option<String>,
    /// None when the rules produced the signal with no model judgement behind it.
    confidence: Option<String>,
}

pub async fn poll_once(deps: &EngineDeps<'_>, now: DateTime<Utc>) -> Result<PollReport> {
    let mut report = PollReport::default();
    let instruments = load_instruments(deps.pool).await?;

    for instrument in instruments {
        match poll_instrument(deps, &instrument, now, &mut report).await {
            Ok(()) => {}
            Err(e) => report.errors.push(format!("{}: {e:#}", instrument.symbol)),
        }
    }

    // The book call runs after the per-instrument passes, and only if one of them actually spent
    // a call this tick — the same "something moved" gate the rest of the poll obeys.
    let calls_before = report.llm_calls;
    let calls_after = orchestrate::daily_chat_calls(deps.pool).await.unwrap_or(0);
    if calls_after > calls_before {
        match plan_book(deps, now, crate::llm::plan::Scope::Stock, None).await {
            Ok(Some(())) => report.plans += 1,
            Ok(None) => {}
            Err(e) => report.errors.push(format!("plan: {e:#}")),
        }
    }
    report.llm_calls = orchestrate::daily_chat_calls(deps.pool).await.unwrap_or(0);
    Ok(report)
}

/// Everything one instrument's decision needs, gathered once. \`poll_instrument\` and \`advise\`
/// shared this logic by copy before, and had already drifted apart.
/// One book call: the model sees every holding, the cash and today's list, and may answer with up
/// to three moves that stand or fall together. Returns `Some(())` only when a plan was stored.
async fn plan_book(
    deps: &EngineDeps<'_>,
    now: DateTime<Utc>,
    scope: crate::llm::plan::Scope,
    // A manual run already has its row; a scheduled one opens its own.
    run: Option<i64>,
) -> Result<Option<()>> {
    let fund = scope.is_fund();
    // A book decision, not an intraday one: asked once per IST day, on the first tick that can
    // ask it. Per-stock calls already cover what moves inside the day.
    let today = ist_day(now);
    let marker = if fund {
        "mf_plan_last"
    } else {
        "stock_plan_last"
    };
    let last: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?")
        .bind(marker)
        .fetch_optional(deps.pool)
        .await?;
    if last.map(|r| r.0).as_deref() == Some(today.as_str()) {
        return Ok(None);
    }
    let funds = fund_symbols(deps.pool).await?;
    // Two books, two prompts: this call only ever sees its own half.
    let book: Vec<crate::llm::rotate::HoldingLine> =
        load_holdings(deps.pool, crate::models::Mode::Long)
            .await?
            .into_iter()
            .filter(|h| funds.contains(&h.symbol) == fund)
            .collect();
    if book.is_empty() {
        return Ok(None); // nothing to reason about, and nothing to pay a call for
    }
    let settings = crate::rules::settings::get(deps.pool).await?;
    // A fund plan draws on the fund purse; a stock plan on the stock purse. Never both.
    let available = if fund {
        budget::available_mf(deps.pool).await?
    } else {
        budget::available_in(deps.pool, crate::models::Mode::Long).await?
    };
    if orchestrate::daily_chat_calls(deps.pool).await? >= deps.runtime.daily_cap as i64 {
        tracing::info!("daily call cap reached — plan not attempted");
        return Ok(None);
    }

    let candidates = candidates_for_plan(deps.pool, &settings.currency, scope).await?;
    let limits = if fund {
        format!(
            "the whole plan may spend at most {:.2} {currency}, which is everything the fund purse holds; there is no per-trade ceiling for funds",
            available, currency = settings.currency
        )
    } else {
        format!(
            "spend at most {:.2} {currency} across the plan; no single buy above {:.2}; a take-profit between {:.0}% and {:.0}%",
            settings.limits_long.max_trade_amount,
            settings.limits_long.max_trade_amount,
            settings.limits_long.min_profit_pct,
            settings.limits_long.max_profit_pct,
            currency = settings.currency
        )
    };
    let rendered = crate::llm::prompts::render_plan(
        scope,
        &crate::llm::plan::book_block(&book, &settings.currency),
        &crate::llm::prompts::cap_line(available, &settings.currency),
        available,
        &settings.currency,
        &limits,
        &candidates,
    )?;
    let model = crate::rules::settings::model_config(deps.pool).await?;
    let run = match run {
        Some(id) => Some(id),
        None => Some(
            crate::runs::start(
                deps.pool,
                if fund {
                    crate::runs::Prompt::PlanFund
                } else {
                    crate::runs::Prompt::PlanStock
                },
                crate::runs::Trigger::Schedule,
                None,
            )
            .await?,
        ),
    };
    let conversation = format!("plan-{}", now.timestamp_millis());
    let reply = match crate::llm::deepseek::chat(
        deps.pool,
        &model,
        deps.transport,
        crate::llm::deepseek::ChatRequest {
            conversation_id: conversation.clone(),
            round: 1,
            prompt: rendered.text,
            prompt_template: rendered.template,
            prompt_hash: rendered.hash,
            trigger: Some(format!(
                "{} plan: {} holdings, {:.2} {} free — looked for moves that fit together",
                if fund { "fund" } else { "stock" },
                book.len(),
                available,
                settings.currency
            )),
            purpose: if fund {
                "plan funds".to_string()
            } else {
                "plan stocks".to_string()
            },
        },
    )
    .await
    {
        Ok(reply) => reply,
        Err(e) => {
            // A failed call must close its run, or the page shows a spinner that will not stop.
            if let Some(id) = run {
                crate::runs::fail(deps.pool, id, &format!("{e:#}")).await?;
            }
            return Err(e);
        }
    };

    sqlx::query(
        "INSERT INTO settings (key, value, updated_at) VALUES (?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(marker)
    .bind(&today)
    .bind(now.to_rfc3339())
    .execute(deps.pool)
    .await?;

    let answer = match crate::llm::plan::parse(&reply.content) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(error = %e, "plan reply unreadable");
            if let Some(id) = run {
                crate::runs::fail(deps.pool, id, &format!("the reply did not parse: {e}")).await?;
            }
            return Ok(None);
        }
    };
    let crate::llm::plan::Answer::Plan(plan) = answer else {
        if let Some(id) = run {
            crate::runs::finish(
                deps.pool,
                id,
                "nothing worth doing — the model passed",
                Some(&conversation),
            )
            .await?;
        }
        return Ok(None);
    };

    // One purse per plan: the other is reported as zero, so a mixed plan cannot balance.
    let cash = if fund {
        crate::llm::plan::Cash {
            stock: 0.0,
            fund: available,
        }
    } else {
        crate::llm::plan::Cash {
            stock: available,
            fund: 0.0,
        }
    };
    if let Err(e) = crate::llm::plan::validate(
        &plan,
        &book,
        cash,
        &settings.limits_long,
        &settings.currency,
        now,
        &funds,
        scope,
    ) {
        tracing::warn!(error = %e, "plan refused by the validator");
        record_plan_refusal(deps.pool, &e.to_string(), now, Some(&conversation), scope).await?;
        if let Some(id) = run {
            crate::runs::finish(deps.pool, id, &format!("refused: {e}"), Some(&conversation))
                .await?;
        }
        return Ok(None);
    }

    let mut plan_id: Option<i64> = None;
    for m in &plan.moves {
        let id: Option<(i64,)> = sqlx::query_as(
            "INSERT INTO signals (instrument_id, action, qty, entry, stop, target, valid_until,
                reason_json, created_at, confidence, mode, kind, status, plan_id, conversation_id)
             SELECT id, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'long', 'plan', 'pending', ?, ?
             FROM instruments WHERE symbol = ? RETURNING id",
        )
        .bind(m.action.as_str())
        .bind(m.qty)
        .bind(m.price)
        .bind(m.stop)
        .bind(m.target)
        .bind((now + chrono::Duration::hours(18)).to_rfc3339())
        .bind(
            serde_json::json!({
                "reasons": [],
                "rationale": m.why,
                "plan_rationale": plan.rationale,
                "plan": true,
                "scope": if fund { "fund" } else { "stock" },
            })
            .to_string(),
        )
        .bind(now.to_rfc3339())
        .bind(Option::<String>::None)
        .bind(plan_id)
        .bind(&conversation)
        .bind(&m.symbol)
        .fetch_optional(deps.pool)
        .await?;
        let Some((id,)) = id else {
            // A move on a symbol that is not an instrument cannot be stored; the plan is refused
            // whole rather than half-recorded.
            record_plan_refusal(
                deps.pool,
                &format!(
                    "{} is not in the book, so the plan cannot be stored",
                    m.symbol
                ),
                now,
                Some(&conversation),
                scope,
            )
            .await?;
            if let Some(id) = run {
                crate::runs::fail(
                    deps.pool,
                    id,
                    &format!(
                        "{} is not in the book, so the plan cannot be stored",
                        m.symbol
                    ),
                )
                .await?;
            }
            return Ok(None);
        };
        // The first leg is its own group: every later leg points at it, and so does it.
        if plan_id.is_none() {
            sqlx::query("UPDATE signals SET plan_id = ? WHERE id = ?")
                .bind(id)
                .bind(id)
                .execute(deps.pool)
                .await?;
            plan_id = Some(id);
        }
    }
    if let Some(id) = run {
        crate::runs::finish(
            deps.pool,
            id,
            &format!(
                "{} move{} stored",
                plan.moves.len(),
                if plan.moves.len() == 1 { "" } else { "s" }
            ),
            Some(&conversation),
        )
        .await?;
    }
    Ok(Some(()))
}

/// A refused plan is a decision too: it is recorded with the reason, so the ledger can say why
/// nothing happened, the way a blocked single proposal already does.
async fn record_plan_refusal(
    pool: &SqlitePool,
    why: &str,
    now: DateTime<Utc>,
    conversation_id: Option<&str>,
    scope: crate::llm::plan::Scope,
) -> Result<()> {
    let reason = serde_json::json!({
        "blocked": "plan_refused",
        "reasons": [why],
    })
    .to_string();
    sqlx::query(
        "INSERT INTO signals (instrument_id, action, qty, entry, valid_until, reason_json,
            created_at, mode, kind, status, conversation_id)
         SELECT id, 'BLOCKED', 0, 0, ?, ?, ?, 'long', 'plan', 'pending', ?
         FROM instruments WHERE (class = 'mutual_fund') = ? AND active = 1
         ORDER BY id LIMIT 1",
    )
    .bind((now + chrono::Duration::hours(18)).to_rfc3339())
    .bind(reason)
    .bind(now.to_rfc3339())
    .bind(conversation_id)
    .bind(scope.is_fund())
    .execute(pool)
    .await?;
    Ok(())
}

/// The stock plan on its own, for a manual run: the same call the poll makes, asked now.
pub async fn plan_stock(
    deps: &EngineDeps<'_>,
    now: DateTime<Utc>,
    run: Option<i64>,
) -> Result<bool> {
    Ok(plan_book(deps, now, crate::llm::plan::Scope::Stock, run)
        .await?
        .is_some())
}

/// One instrument, asked on its own — the precise version of a poll. Returns the decision as one
/// line, or None when the rules refuse to propose anything.
pub async fn decide_one(
    deps: &EngineDeps<'_>,
    symbol: &str,
    now: DateTime<Utc>,
) -> Result<Option<String>> {
    let instrument = load_instruments(deps.pool)
        .await?
        .into_iter()
        .find(|i| i.symbol == symbol)
        .ok_or_else(|| anyhow::anyhow!("{symbol} is not an active instrument"))?;
    advise(deps, &instrument, now).await
}

/// The fund plan, run once a day after the arithmetic scan has ranked today's funds. Separate
/// entry point because it is a different question, not a second pass over the same one.
pub async fn plan_funds(
    deps: &EngineDeps<'_>,
    now: DateTime<Utc>,
    run: Option<i64>,
) -> Result<bool> {
    Ok(plan_book(deps, now, crate::llm::plan::Scope::Fund, run)
        .await?
        .is_some())
}

/// `YYYY-MM-DD` of the current IST day — the unit the daily gates are written in.
fn ist_day(now: DateTime<Utc>) -> String {
    let ist = now + chrono::Duration::minutes(330);
    format!("{:04}-{:02}-{:02}", ist.year(), ist.month(), ist.day())
}

/// The symbols that are mutual funds, so the plan's money can be counted per purse.
async fn fund_symbols(pool: &SqlitePool) -> Result<Vec<String>> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT symbol FROM instruments WHERE class = 'mutual_fund'")
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

/// Today's list for the book call: the active instruments and their last two closes, so the model
/// can weigh a holding against what else is on offer.
async fn candidates_for_plan(
    pool: &SqlitePool,
    currency: &str,
    scope: crate::llm::plan::Scope,
) -> Result<String> {
    #[derive(sqlx::FromRow)]
    struct Line {
        symbol: String,
        name: String,
        last: f64,
        prev: Option<f64>,
    }
    let rows: Vec<Line> = sqlx::query_as(
        "SELECT i.symbol AS symbol, i.name AS name,
                (SELECT b.close FROM bars b WHERE b.instrument_id = i.id
                 ORDER BY b.ts DESC LIMIT 1) AS last,
                (SELECT b.close FROM bars b WHERE b.instrument_id = i.id
                 ORDER BY b.ts DESC LIMIT 1 OFFSET 1) AS prev
         FROM instruments i
         WHERE i.active = 1 AND (i.class = 'mutual_fund') = ?
         ORDER BY i.symbol",
    )
    .bind(scope.is_fund())
    .fetch_all(pool)
    .await?;
    let mut out = String::new();
    for r in rows {
        let change = r
            .prev
            .filter(|p| *p > 0.0)
            .map(|p| format!("{:+.2}% today", (r.last - p) / p * 100.0))
            .unwrap_or_else(|| "no prior close".into());
        out.push_str(&format!(
            "{} {} at {:.2} {currency}, {change}\n",
            r.symbol, r.name, r.last
        ));
    }
    if out.is_empty() {
        out.push_str("nothing on offer today\n");
    }
    Ok(out)
}

struct Gathered {
    bars: Vec<Bar>,
    news: Vec<Headline>,
    ind: Indicators,
    position: Option<Position>,
    available: f64,
    limits: crate::rules::limits::Limits,
    required_profit_pct: f64,
    pot: f64,
    instrument_exposure: f64,
    price_window: String,
    /// The raw intraday tape, kept for the short-horizon card (the text is for the prompt).
    tape: Vec<Bar>,
    daily_summary: String,
    positions_text: String,
    recent_trades: String,
    last_decision: String,
}

impl Gathered {
    fn last_close(&self) -> f64 {
        self.bars.last().map(|b| b.close).unwrap_or_default()
    }
}

async fn gather(
    deps: &EngineDeps<'_>,
    instrument: &Instrument,
    now: DateTime<Utc>,
) -> Result<Option<Gathered>> {
    let bars = deps.market.bars(&instrument.symbol).await?;
    let Ok(ind) = indicators::compute(&bars) else {
        tracing::debug!(symbol = %instrument.symbol, "not enough history for indicators");
        return Ok(None);
    };
    let news = deps
        .market
        .news(&instrument.symbol)
        .await
        .unwrap_or_default();
    crate::store::bars(deps.pool, instrument.id, &bars).await?;
    crate::store::headlines(deps.pool, instrument.id, &news).await?;

    let position = load_position(deps.pool, instrument.id).await?;
    let available = budget::available_in(deps.pool, crate::models::Mode::Long).await?;
    let limits = crate::rules::settings::limits(deps.pool, crate::models::Mode::Long).await?;
    let pot = budget::budget_total_in(deps.pool, crate::models::Mode::Long).await?;
    let instrument_exposure = load_exposure(deps.pool, instrument.id).await?;
    // No per-stock value means "the pot's minimum"; a value above the ceiling is clamped, so a
    // lowered max cannot make every trade on this stock impossible.
    let required_profit_pct = load_target_profit(deps.pool, instrument.id)
        .await?
        .unwrap_or(limits.min_profit_pct)
        .clamp(limits.min_profit_pct, limits.max_profit_pct);
    let (price_window, tape) = match deps.market.intraday(&instrument.symbol, "15m", "5d").await {
        Ok(tape) => (
            format_window(&tape, deps.runtime.prompt_window_hours, now),
            tape,
        ),
        Err(e) => {
            tracing::debug!(symbol = %instrument.symbol, error = %e, "no intraday tape");
            ("(intraday tape unavailable)".to_string(), Vec::new())
        }
    };

    Ok(Some(Gathered {
        tape,
        daily_summary: format_daily(&bars, 10),
        positions_text: format_book(
            &load_holdings(deps.pool, crate::models::Mode::Long).await?,
            &instrument.symbol,
        ),
        recent_trades: load_recent_trades(deps.pool, instrument.id, now).await?,
        last_decision: load_last_decision(deps.pool, instrument.id).await?,
        limits,
        required_profit_pct,
        pot,
        instrument_exposure,
        bars,
        news,
        ind,
        position,
        available,
        price_window,
    }))
}

async fn poll_instrument(
    deps: &EngineDeps<'_>,
    instrument: &Instrument,
    now: DateTime<Utc>,
    report: &mut PollReport,
) -> Result<()> {
    // An AMFI scheme code (135762) has no price series anywhere: it can be held, but it cannot be
    // charted, so polling it only produces a 404 every fifteen minutes. Say so once per poll.
    if instrument.class == crate::models::AssetClass::MutualFund && !instrument.symbol.contains('.')
    {
        report.notes.push(format!(
            "{}: AMFI scheme code, no price series — hold it, but add the Yahoo code to get advice",
            instrument.symbol
        ));
        return Ok(());
    }
    let Some(g) = gather(deps, instrument, now).await? else {
        return Ok(());
    };
    report.polled += 1;
    let (bars, news, ind) = (&g.bars, &g.news, &g.ind);
    let (position, available) = (&g.position, g.available);

    let rules_candidate = evaluate::candidate(CandidateInput {
        instrument,
        bars,
        ind,
        position: position.as_ref(),
        available_cash: available,
        cfg: deps.rules,
    });

    // The rules no longer decide whether the model is asked — they describe the setup. A held
    // position with no fresh headlines still deserves a view on prices alone.
    let had_rules_candidate = rules_candidate.is_some();
    let candidate = match rules_candidate {
        Some(c) => c,
        None => {
            // Say why, per instrument: "the ledger is empty" is not an answer on its own.
            let failed: Vec<String> = evaluate::diagnose(&CandidateInput {
                instrument,
                bars,
                ind,
                position: position.as_ref(),
                available_cash: available,
                cfg: deps.rules,
            })
            .into_iter()
            .filter(|c| !c.passed)
            .map(|c| format!("{} — {}", c.name, c.detail))
            .collect();
            report.notes.push(format!(
                "{}: rules see no entry ({}) — asking the model on prices alone",
                instrument.symbol,
                if failed.is_empty() {
                    "no rule fired".to_string()
                } else {
                    failed.join("; ")
                }
            ));
            Candidate {
                instrument_id: instrument.id,
                action: Action::Hold,
                reasons: Vec::new(),
                entry: bars.last().map(|b| b.close).unwrap_or_default(),
                stop: None,
                target: None,
                qty: (available / bars.last().map(|b| b.close).unwrap_or(1.0).max(1e-9)).floor(),
                timing_note: instrument.class.timing_note().into(),
                urgent: false,
            }
        }
    };
    // Limits bind the rules too, not just the model: a 200k fill against a 50k ceiling is
    // not acceptable because a rule suggested it.
    let candidate = match clamp_to_limits(candidate, g.limits, g.pot, g.instrument_exposure) {
        Some(c) => c,
        None => {
            report.notes.push(format!(
                "{}: no affordable size inside the limits ({:.2} to {:.2} per trade)",
                instrument.symbol, g.limits.min_trade_amount, g.limits.max_trade_amount
            ));
            return Ok(());
        }
    };
    if had_rules_candidate {
        report.candidates += 1;
    }

    // The model is paid for judgement about news, so news is what re-triggers it. Prices can
    // move all day without another call; the rules refresh them.
    let headline_fp = crate::llm::memo::fingerprint(news);
    let headlines_changed = !crate::llm::memo::unchanged(deps.pool, instrument.id, &headline_fp)
        .await
        .unwrap_or(true);

    // Reported financials, fetched only when a call is actually going to happen — this is one
    // extra request per call, not per tick.
    let fundamentals = fetch_fundamentals(deps, &instrument.symbol, now).await;

    // The short-horizon card exists for one reason: a trade that must finish inside a week needs the
    // session clock, relative volume and the level it is trading against — numbered, not inferred.
    let short_context = short_card(deps, instrument, &g.tape, now).await;

    // Why this call is being made, in words a person can read later in the audit trail.
    let previous_price: Option<(Option<f64>,)> =
        sqlx::query_as("SELECT last_price FROM llm_memos WHERE instrument_id = ?")
            .bind(instrument.id)
            .fetch_optional(deps.pool)
            .await?;
    let gate_reason = if deps.force {
        Some("asked by hand".to_string())
    } else if headlines_changed {
        Some(format!("new headline on {}", instrument.symbol))
    } else {
        match previous_price.and_then(|r| r.0) {
            Some(prev) if (prev - candidate.entry).abs() >= deps.runtime.min_price_move => {
                Some(format!("price moved {prev:.2} → {:.2}", candidate.entry))
            }
            None => Some("never asked about this instrument".to_string()),
            _ => None,
        }
    };

    // Do not spend a call on a price that has not moved, and keep quiet otherwise.
    if !deps.force {
        let gate = crate::llm::memo::CallGate {
            min_move: deps.runtime.min_price_move,
        };
        if !crate::llm::memo::should_call(
            deps.pool,
            instrument.id,
            candidate.entry,
            now,
            gate,
            headlines_changed,
        )
        .await?
        {
            report.notes.push(format!(
                "{}: price moved less than {:.2} and no new headlines — model not asked",
                instrument.symbol, gate.min_move
            ));
            report.not_asked.push(instrument.symbol.clone());
            // The decision stands, but its price should not age: refresh the entry only.
            let _ = reuse_model_signal(deps.pool, instrument.id, candidate.entry, ind.atr14, now)
                .await?;
            return Ok(());
        }
    }

    // No key configured means no model layer at all — not a failed call on every tick.
    let llm_available = crate::rules::settings::deepseek_key(deps.pool)
        .await?
        .is_some();
    if !llm_available {
        // A key that cannot be decrypted is a different problem from no key at all: the value is
        // there, the SECRET_KEY is not the one it was written with.
        let broken = crate::secrets::unreadable(deps.pool, "deepseek_key")
            .await
            .unwrap_or(false);
        report.notes.push(if broken {
            format!(
                "{}: the stored key cannot be opened with the current SECRET_KEY — re-enter it in Settings",
                instrument.symbol
            )
        } else {
            format!("{}: no DeepSeek key — rules only", instrument.symbol)
        });
        let resolved = Resolved {
            action: candidate.action,
            entry: candidate.entry,
            stop: candidate.stop,
            target: candidate.target,
            valid_until: Some((now + chrono::Duration::hours(18)).to_rfc3339()),
            qty: candidate.qty,
            reasons: candidate.reasons.clone(),
            rationale: None,
            confidence: None,
        };
        return finish(
            deps,
            instrument,
            Finished {
                resolved,
                candidate: &candidate,
                sizing: Sizing {
                    available,
                    atr: ind.atr14,
                },
                now,
                conversation_id: None,
            },
            report,
        )
        .await;
    }

    // A buy that the cash cannot fund is where a rotation belongs: one call that names what to
    // sell to pay for it. If nothing is in profit, this returns immediately without a call.
    if !deps.force {
        match rotate_for_cash(deps, instrument, &candidate, now).await {
            Ok(Some((sell_id, buy_id))) => {
                report.signals += 2;
                report.notes.push(format!(
                    "{}: swap proposed — sell (decision {sell_id}) to fund this (decision {buy_id})",
                    instrument.symbol
                ));
                return Ok(());
            }
            Ok(None) => {}
            Err(e) => tracing::warn!(error = %e, "rotation attempt failed"),
        }
    }

    let llm_deps = LlmDeps {
        pool: deps.pool,
        runtime: deps.runtime.clone(),
        transport: deps.transport,
        market: deps.market,
    };
    let decision = orchestrate::run(
        &llm_deps,
        RunInput {
            instrument,
            ind,
            candidate: &candidate,
            call_reason: gate_reason.clone(),
            fundamentals: &fundamentals,
            short_context: &short_context,
            mode: crate::models::Mode::Long,
            available_cash: available,
            held_qty: position.as_ref().map(|p| p.qty).unwrap_or(0.0),
            limits: g.limits,
            required_profit_pct: g.required_profit_pct,
            instrument_exposure: g.instrument_exposure,
            pot: g.pot,
            window_hours: deps.runtime.prompt_window_hours,
            headlines: news,
            price_window: &g.price_window,
            daily_summary: &g.daily_summary,
            positions: &g.positions_text,
            recent_trades: &g.recent_trades,
            last_decision: &g.last_decision,
            now,
        },
    )
    .await?;

    // The model answered for this headline set; do not ask it about the same news again.
    if !matches!(decision.outcome, Outcome::Rules(_)) {
        crate::llm::memo::remember(deps.pool, instrument.id, &headline_fp).await?;
    }

    // Recorded whatever the answer was, so a failing key cannot loop on every tick.
    crate::llm::memo::record_call(deps.pool, instrument.id, candidate.entry, now, &headline_fp)
        .await?;

    report.asked += 1;
    // A proposal the validator refused is still a decision the owner should be able to see: the
    // model said "buy at X", and the answer was no, with a reason. Without this it vanished — the
    // rules' fallback is usually a HOLD, and `finish` records nothing for a HOLD.
    let refused = match &decision.outcome {
        Outcome::Rejected { proposal, error }
            if !matches!(
                error,
                crate::llm::validate::ValidationError::InsufficientCash
            ) =>
        {
            Some((proposal.clone(), error.to_string()))
        }
        _ => None,
    };
    let Some(resolved) = resolve(&candidate, decision.outcome, now) else {
        return Ok(());
    };
    if !matches!(resolved.action, Action::Buy | Action::Sell) {
        if let Some((proposal, why)) = refused {
            record_refused(
                deps.pool,
                instrument,
                &proposal,
                &why,
                now,
                Some(&decision.conversation_id),
            )
            .await?;
            report.blocked += 1;
            report.notes.push(format!(
                "{}: model proposed {:?} at {:.2} — refused: {why}",
                instrument.symbol, proposal.action, proposal.entry
            ));
            return Ok(());
        }
    }

    finish(
        deps,
        instrument,
        Finished {
            resolved,
            candidate: &candidate,
            sizing: Sizing {
                available,
                atr: ind.atr14,
            },
            now,
            conversation_id: Some(&decision.conversation_id),
        },
        report,
    )
    .await
}

/// Ask the model about one instrument on demand, bypassing the rules gate, and write nothing.
/// This is the "second opinion" path: the rules may see no setup, but a human still wants the
/// model's read before deciding to do nothing.
pub async fn advise(
    deps: &EngineDeps<'_>,
    instrument: &Instrument,
    now: DateTime<Utc>,
) -> Result<Option<String>> {
    let Some(g) = gather(deps, instrument, now).await? else {
        return Ok(Some("not enough stored history for indicators".to_string()));
    };

    // A neutral stand-in: the prompt uses entry and quantity, not the rules' opinion.
    let neutral = Candidate {
        instrument_id: instrument.id,
        action: Action::Hold,
        reasons: Vec::new(),
        entry: g.last_close(),
        stop: None,
        target: None,
        qty: (g.available / g.last_close().max(1e-9)).floor().max(0.0),
        timing_note: instrument.class.timing_note().into(),
        urgent: false,
    };

    let llm_deps = LlmDeps {
        pool: deps.pool,
        runtime: deps.runtime.clone(),
        transport: deps.transport,
        market: deps.market,
    };
    let decision = orchestrate::run(
        &llm_deps,
        RunInput {
            call_reason: None,
            fundamentals: "",
            short_context: "",
            mode: crate::models::Mode::Long,
            instrument,
            ind: &g.ind,
            candidate: &neutral,
            available_cash: g.available,
            held_qty: g.position.as_ref().map(|p| p.qty).unwrap_or(0.0),
            limits: g.limits,
            required_profit_pct: g.required_profit_pct,
            instrument_exposure: g.instrument_exposure,
            pot: g.pot,
            window_hours: deps.runtime.prompt_window_hours,
            headlines: &g.news,
            price_window: &g.price_window,
            daily_summary: &g.daily_summary,
            positions: &g.positions_text,
            recent_trades: &g.recent_trades,
            last_decision: &g.last_decision,
            now,
        },
    )
    .await?;

    Ok(Some(match decision.outcome {
        Outcome::Llm(p) => format!(
            "{} {:?} entry {:.2} stop {} target {} confidence {} · {}",
            instrument.symbol,
            p.action,
            p.entry,
            p.stop
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "—".into()),
            p.target
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "—".into()),
            p.confidence.as_str(),
            p.rationale
        ),
        Outcome::Pass { why } => format!("{} PASS · {why}", instrument.symbol),
        Outcome::Rejected { proposal, error } => format!(
            "{} model proposed {:?} entry {:.2} stop {} target {} but it was rejected: {error}",
            instrument.symbol,
            proposal.action,
            proposal.entry,
            proposal
                .stop
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "—".into()),
            proposal
                .target
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "—".into()),
        ),
        Outcome::Rules(_) => format!(
            "{} no usable model answer, rules unchanged",
            instrument.symbol
        ),
    }))
}

/// Signal persistence, the budget gate and the push — shared by the LLM and rules-only paths.
/// Price context the gate and the risk checks both need.
#[derive(Debug, Clone, Copy)]
struct Sizing {
    available: f64,
    atr: f64,
}

/// What a finished decision needs to be recorded. Bundled so `finish` stays readable.
struct Finished<'a> {
    resolved: Resolved,
    candidate: &'a Candidate,
    sizing: Sizing,
    now: DateTime<Utc>,
    /// The model conversation this came from, so Audit can show what happened to it.
    conversation_id: Option<&'a str>,
}

async fn finish(
    deps: &EngineDeps<'_>,
    instrument: &Instrument,
    done: Finished<'_>,
    report: &mut PollReport,
) -> Result<()> {
    let Finished {
        resolved,
        candidate,
        sizing,
        now,
        conversation_id,
    } = done;
    // A HOLD is the absence of a decision, not a decision to record.
    if !matches!(resolved.action, Action::Buy | Action::Sell) {
        return Ok(());
    }

    let available = sizing.available;

    // The budget gate is the last word, above the LLM: a proposal that does not fit is
    // recorded as BLOCKED with the shortfall, and is never pushed.
    if resolved.action == Action::Buy {
        let cost = resolved.qty * resolved.entry;
        if !budget::fits(deps.pool, cost).await? {
            record_blocked(
                deps.pool,
                instrument,
                &resolved,
                candidate,
                available,
                now,
                conversation_id,
            )
            .await?;
            report.blocked += 1;
            return Ok(());
        }
    }

    report.notes.push(format!(
        "{} {} qty {} entry {:.2} stop {} target {} until {} :: {}",
        instrument.symbol,
        resolved.action.as_str(),
        resolved.qty,
        resolved.entry,
        resolved
            .stop
            .map(|v| format!("{v:.2}"))
            .unwrap_or_else(|| "-".into()),
        resolved
            .target
            .map(|v| format!("{v:.2}"))
            .unwrap_or_else(|| "-".into()),
        resolved.valid_until.clone().unwrap_or_else(|| "-".into()),
        resolved
            .rationale
            .clone()
            .unwrap_or_else(|| "rules only".into()),
    ));

    // A model-derived signal that is still valid and still inside its band survives a failed
    // model call: the rules refresh the price but must not overwrite a view they did not form.
    if resolved.rationale.is_none()
        && reuse_model_signal(deps.pool, instrument.id, resolved.entry, sizing.atr, now).await?
    {
        return Ok(());
    }

    // "Prices refresh freely" means the entry may drift without a new row or a push: the
    // latest signal is updated in place. A different action, stop or target is new information
    // and gets its own row.
    if !deps.force {
        if let Some(prev) = latest_signal(deps.pool, instrument.id).await? {
            let same_shape = prev.action == resolved.action.as_str()
                && stop_round(prev.stop) == stop_round(resolved.stop)
                && stop_round(prev.target) == stop_round(resolved.target);
            if same_shape {
                if prev
                    .entry
                    .is_some_and(|e| (e - resolved.entry).abs() >= 0.005)
                {
                    sqlx::query("UPDATE signals SET entry = ? WHERE id = ?")
                        .bind(resolved.entry)
                        .bind(prev.id)
                        .execute(deps.pool)
                        .await?;
                }
                return Ok(());
            }
        }
    }

    let signal_id = insert_signal(
        deps.pool,
        instrument.id,
        &resolved,
        candidate,
        now,
        conversation_id,
    )
    .await?;
    report.signals += 1;

    let mut as_candidate = candidate.clone();
    as_candidate.action = resolved.action;
    as_candidate.entry = resolved.entry;
    as_candidate.stop = resolved.stop;
    as_candidate.target = resolved.target;
    as_candidate.qty = resolved.qty;

    if let Some(mut n) = Notification::from_candidate(
        &instrument.symbol,
        &instrument.name,
        &instrument.currency,
        &as_candidate,
    ) {
        n.signal_id = Some(signal_id);
        match notify::dispatch(deps.pool, deps.notifier, &n).await {
            Ok(true) => report.notifications += 1,
            Ok(false) if !deps.notifier.configured() => report
                .notes
                .push(format!("{}: no ntfy topic — recorded without a push", instrument.symbol)),
            Ok(false) => {}
            // A push that fails must not lose the decision: the row is already written, and the
            // notification is retried on the next tick because no dedupe row was kept.
            Err(e) => report
                .errors
                .push(format!("{}: push failed: {e}", instrument.symbol)),
        }
    }
    Ok(())
}

/// Rules candidate is the spine; the LLM may change direction and prices, never quantity.
fn resolve(candidate: &Candidate, outcome: Outcome, now: DateTime<Utc>) -> Option<Resolved> {
    let valid_until = Some((now + chrono::Duration::hours(18)).to_rfc3339());
    match outcome {
        Outcome::Rules(c) => Some(Resolved {
            action: c.action,
            entry: c.entry,
            stop: c.stop,
            target: c.target,
            valid_until,
            qty: c.qty,
            reasons: c.reasons,
            rationale: None,
            confidence: None,
        }),
        Outcome::Llm(p) => {
            let mut resolved = from_proposal(candidate, &p, valid_until);
            // A breached stop or a reached target is already decided. The model may explain it,
            // not relitigate it — otherwise "HOLD" would swallow the exit signal and its push.
            if candidate.urgent {
                resolved.action = candidate.action;
                resolved.entry = candidate.entry;
                resolved.qty = candidate.qty;
                resolved.stop = None;
                resolved.target = candidate.target;
            }
            Some(resolved)
        }
        // An over-budget proposal is NOT re-sized down to something that fits: it is carried
        // through to the budget gate, which records it as BLOCKED with the shortfall.
        Outcome::Rejected {
            proposal,
            error: crate::llm::validate::ValidationError::InsufficientCash,
        } => Some(from_proposal(candidate, &proposal, valid_until)),
        Outcome::Rejected { .. } => Some(Resolved {
            action: candidate.action,
            entry: candidate.entry,
            stop: candidate.stop,
            target: candidate.target,
            valid_until,
            qty: candidate.qty,
            reasons: candidate.reasons.clone(),
            rationale: None,
            confidence: None,
        }),
        Outcome::Pass { .. } => None,
    }
}

fn from_proposal(candidate: &Candidate, p: &Proposal, fallback_until: Option<String>) -> Resolved {
    Resolved {
        action: p.action,
        // The model may size the trade; otherwise the rules' sizing stands.
        qty: p.qty.unwrap_or(candidate.qty),
        entry: p.entry,
        stop: p.stop,
        target: p.target,
        valid_until: Some(p.valid_until.clone()).or(fallback_until),
        reasons: candidate.reasons.clone(),
        rationale: Some(p.rationale.clone()),
        confidence: Some(p.confidence.as_str().to_string()),
    }
}

async fn load_instruments(pool: &SqlitePool) -> Result<Vec<Instrument>> {
    let rows: Vec<(i64, String, String, String, String, i64)> = sqlx::query_as(
        "SELECT id, symbol, class, name, currency, active FROM instruments WHERE active = 1 ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|(id, symbol, class, name, currency, active)| {
            Ok(Instrument {
                id,
                symbol,
                class: AssetClass::parse(&class)?,
                name,
                currency,
                active: active != 0,
            })
        })
        .collect()
}

/// Cap a candidate's size to the pot's limits, or drop it if nothing usable is left.
fn clamp_to_limits(
    mut c: Candidate,
    limits: crate::rules::limits::Limits,
    pot: f64,
    exposure: f64,
) -> Option<Candidate> {
    if !matches!(c.action, Action::Buy | Action::Sell) {
        return Some(c);
    }
    let max_fill = limits.max_fill(pot, exposure);
    if max_fill <= 0.0 {
        return None;
    }
    let max_units = max_fill / c.entry.max(1e-9);
    if c.qty > max_units {
        c.qty = match c.action {
            Action::Sell => c.qty.min(max_units),
            _ => max_units.floor(),
        };
    }
    if c.qty <= 0.0 {
        return None;
    }
    // A buy below the minimum is not worth its fees; a sale of a tiny remainder still is.
    if c.action == Action::Buy && c.qty * c.entry < limits.min_trade_amount {
        return None;
    }
    Some(c)
}

async fn load_exposure(pool: &SqlitePool, instrument_id: i64) -> Result<f64> {
    let row: (Option<f64>,) = sqlx::query_as(
        "SELECT SUM(qty * avg_cost) FROM positions
         WHERE instrument_id = ? AND mode = 'long' AND state <> 'closed'",
    )
    .bind(instrument_id)
    .fetch_one(pool)
    .await?;
    Ok(row.0.unwrap_or(0.0))
}

async fn load_target_profit(pool: &SqlitePool, instrument_id: i64) -> Result<Option<f64>> {
    let row: Option<(Option<f64>,)> =
        sqlx::query_as("SELECT target_profit_pct FROM instruments WHERE id = ?")
            .bind(instrument_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.and_then(|r| r.0))
}

#[derive(sqlx::FromRow)]
struct PositionDbRow {
    id: i64,
    instrument_id: i64,
    qty: f64,
    avg_cost: f64,
    stop: Option<f64>,
    target: Option<f64>,
    state: String,
}

async fn load_position(pool: &SqlitePool, instrument_id: i64) -> Result<Option<Position>> {
    let row: Option<PositionDbRow> = sqlx::query_as(
        "SELECT id AS id, instrument_id AS instrument_id, qty AS qty, avg_cost AS avg_cost,
                stop AS stop, target AS target, state AS state
         FROM positions WHERE instrument_id = ? AND state = 'open' ORDER BY id DESC LIMIT 1",
    )
    .bind(instrument_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| Position {
        id: r.id,
        instrument_id: r.instrument_id,
        qty: r.qty,
        avg_cost: r.avg_cost,
        stop: r.stop,
        target: r.target,
        state: r.state,
    }))
}

/// Refresh the entry on the last signal when it is still a live model decision. Returns
/// false when the stored signal was rules-only, has expired, or the price left the band.
async fn reuse_model_signal(
    pool: &SqlitePool,
    instrument_id: i64,
    price: f64,
    atr: f64,
    now: DateTime<Utc>,
) -> Result<bool> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: i64,
        entry: Option<f64>,
        valid_until: Option<String>,
        reason_json: Option<String>,
    }
    let row: Option<Row> = sqlx::query_as(
        "SELECT id AS id, entry AS entry, valid_until AS valid_until,
                reason_json AS reason_json FROM signals
         WHERE instrument_id = ? AND action <> 'BLOCKED' ORDER BY id DESC LIMIT 1",
    )
    .bind(instrument_id)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Ok(false);
    };
    let (id, entry, valid_until) = (row.id, row.entry, row.valid_until);
    let from_model = row
        .reason_json
        .as_deref()
        .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
        .and_then(|v| v.get("rationale").cloned())
        .is_some();
    let still_valid = valid_until
        .as_deref()
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
        .map(|d| d.with_timezone(&Utc) > now)
        .unwrap_or(false);
    let inside_band = entry
        .map(|e| (price - e).abs() <= atr.abs().max(1e-9))
        .unwrap_or(false);

    if from_model && still_valid && inside_band {
        sqlx::query("UPDATE signals SET entry = ? WHERE id = ?")
            .bind(price)
            .bind(id)
            .execute(pool)
            .await?;
        return Ok(true);
    }
    Ok(false)
}

fn stop_round(v: Option<f64>) -> Option<i64> {
    v.map(|x| (x * 100.0).round() as i64)
}

#[derive(sqlx::FromRow)]
struct LastSignal {
    id: i64,
    action: String,
    entry: Option<f64>,
    stop: Option<f64>,
    target: Option<f64>,
}

async fn latest_signal(pool: &SqlitePool, instrument_id: i64) -> Result<Option<LastSignal>> {
    let row: Option<LastSignal> = sqlx::query_as(
        "SELECT id AS id, action AS action, entry AS entry, stop AS stop, target AS target
         FROM signals WHERE instrument_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(instrument_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

async fn insert_signal(
    pool: &SqlitePool,
    instrument_id: i64,
    r: &Resolved,
    candidate: &Candidate,
    now: DateTime<Utc>,
    conversation_id: Option<&str>,
) -> Result<i64> {
    let _ = candidate;
    let reason = serde_json::json!({
        "reasons": r.reasons,
        "rationale": r.rationale,
    })
    .to_string();
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO signals (instrument_id, action, qty, entry, stop, target, valid_until,
            reason_json, created_at, confidence, conversation_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(instrument_id)
    .bind(r.action.as_str())
    .bind(r.qty)
    .bind(r.entry)
    .bind(r.stop)
    .bind(r.target)
    .bind(&r.valid_until)
    .bind(reason)
    .bind(now.to_rfc3339())
    .bind(&r.confidence)
    .bind(conversation_id)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

async fn record_blocked(
    pool: &SqlitePool,
    instrument: &Instrument,
    r: &Resolved,
    candidate: &Candidate,
    available: f64,
    now: DateTime<Utc>,
    // The model conversation that proposed it, so Audit can show it beside the call.
    conversation_id: Option<&str>,
) -> Result<()> {
    let cost = r.qty * r.entry;
    let shortfall = cost - available;
    let _ = candidate;
    let reason = serde_json::json!({
        "blocked": "insufficient_cash",
        "cost": cost,
        "available": available,
        "shortfall": shortfall,
        "reasons": r.reasons,
    })
    .to_string();
    sqlx::query(
        "INSERT INTO signals (instrument_id, action, qty, entry, stop, target, valid_until,
            reason_json, created_at, conversation_id)
         VALUES (?, 'BLOCKED', ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(instrument.id)
    .bind(r.qty)
    .bind(r.entry)
    .bind(r.stop)
    .bind(r.target)
    .bind(&r.valid_until)
    .bind(reason)
    .bind(now.to_rfc3339())
    .bind(conversation_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// A proposal the validator refused: recorded as not-actionable with the reason the model was told,
/// so Audit and the ledger agree about what happened. No push — there is nothing to approve.
async fn record_refused(
    pool: &SqlitePool,
    instrument: &Instrument,
    p: &crate::llm::parse::Proposal,
    why: &str,
    now: DateTime<Utc>,
    conversation_id: Option<&str>,
) -> Result<()> {
    let reason = serde_json::json!({
        "blocked": "the model's proposal was refused",
        "detail": why,
        "proposed": {
            "action": p.action.as_str(),
            "qty": p.qty,
            "entry": p.entry,
            "stop": p.stop,
            "target": p.target,
            "rationale": p.rationale,
        },
    })
    .to_string();
    sqlx::query(
        "INSERT INTO signals (instrument_id, action, qty, entry, stop, target, valid_until,
            reason_json, created_at, confidence, conversation_id)
         VALUES (?, 'BLOCKED', ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(instrument.id)
    .bind(p.qty)
    .bind(p.entry)
    .bind(p.stop)
    .bind(p.target)
    .bind(&p.valid_until)
    .bind(reason)
    .bind(now.to_rfc3339())
    .bind(p.confidence.as_str())
    .bind(conversation_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Half-width of the validator band, surfaced for the UI's chart rules.
pub fn band_for(last_close: f64, atr14: f64) -> indicators::Bands {
    atr_bands(last_close, atr14)
}

/// 15-minute tape, oldest first, timestamps in IST.
fn format_window(bars: &[Bar], hours: u32, now: DateTime<Utc>) -> String {
    let cutoff = now - chrono::Duration::hours(hours as i64);
    let ist = chrono::FixedOffset::east_opt(330 * 60).unwrap();
    let lines: Vec<String> = bars
        .iter()
        .filter(|b| {
            chrono::DateTime::parse_from_rfc3339(&b.ts)
                .map(|d| d.with_timezone(&Utc) >= cutoff)
                .unwrap_or(false)
        })
        .map(|b| {
            let t = chrono::DateTime::parse_from_rfc3339(&b.ts)
                .map(|d| d.with_timezone(&ist).format("%d %H:%M").to_string())
                .unwrap_or_else(|_| b.ts.clone());
            format!(
                "{} {:.2} {:.2} {:.2} {:.2} {}",
                t, b.open, b.high, b.low, b.close, b.volume
            )
        })
        .collect();
    if lines.is_empty() {
        "(no bars in the last window)".into()
    } else {
        lines.join("\n")
    }
}

fn format_daily(bars: &[Bar], days: usize) -> String {
    let tail: Vec<String> = bars
        .iter()
        .rev()
        .take(days)
        .rev()
        .map(|b| format!("{} {:.2}", &b.ts[..10.min(b.ts.len())], b.close))
        .collect();
    if tail.is_empty() {
        "(none)".into()
    } else {
        tail.join(", ")
    }
}

/// How many holdings a prompt will enumerate before summarising the rest. A prompt that grows
/// with the portfolio is a bill that grows with the portfolio.
const BOOK_LINES: usize = 12;

/// One line per holding, richest first, with the instrument under review marked. The model is
/// being asked about the whole book, not one stock in a vacuum: "sell this to buy that" is only
/// a thinkable answer if it can see both.
pub fn format_book(rows: &[crate::llm::rotate::HoldingLine], current: &str) -> String {
    if rows.is_empty() {
        return "empty — nothing is held".to_string();
    }
    let mut sorted: Vec<&crate::llm::rotate::HoldingLine> = rows.iter().collect();
    sorted.sort_by(|a, b| {
        let v = |h: &crate::llm::rotate::HoldingLine| h.qty * h.ltp.unwrap_or(h.avg_cost);
        v(b).partial_cmp(&v(a)).unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut out = String::new();
    for h in sorted.iter().take(BOOK_LINES) {
        out.push_str(&format!(
            "{} {}: {:.3} units at {:.2}, now {}, {}, stop {}, target {}",
            h.symbol,
            h.name,
            h.qty,
            h.avg_cost,
            h.ltp
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "—".into()),
            h.profit_pct()
                .map(|p| format!("{p:+.1}%"))
                .unwrap_or_else(|| "P&L unknown".into()),
            h.stop
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "none".into()),
            h.target
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "none".into()),
        ));
        if h.symbol == current {
            out.push_str("  ← this one");
        }
        out.push('\n');
    }
    if rows.len() > BOOK_LINES {
        out.push_str(&format!("+{} more not shown\n", rows.len() - BOOK_LINES));
    }
    out
}

async fn load_recent_trades(
    pool: &SqlitePool,
    instrument_id: i64,
    now: DateTime<Utc>,
) -> Result<String> {
    let cutoff = (now - chrono::Duration::days(7)).to_rfc3339();
    let rows: Vec<(f64, f64, String, String)> = sqlx::query_as(
        "SELECT qty, avg_cost, opened_at, state FROM positions
         WHERE instrument_id = ? AND opened_at >= ? ORDER BY opened_at DESC",
    )
    .bind(instrument_id)
    .bind(cutoff)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok("none in the last 7 days".into());
    }
    Ok(rows
        .into_iter()
        .map(|(qty, cost, at, state)| {
            format!(
                "{} BUY {:.3} @ {:.2} ({})",
                &at[..10.min(at.len())],
                qty,
                cost,
                state
            )
        })
        .collect::<Vec<_>>()
        .join("; "))
}

async fn load_last_decision(pool: &SqlitePool, instrument_id: i64) -> Result<String> {
    let row: Option<(String, Option<f64>, String, Option<String>)> = sqlx::query_as(
        "SELECT action, entry, created_at, reason_json FROM signals
         WHERE instrument_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(instrument_id)
    .fetch_optional(pool)
    .await?;
    Ok(match row {
        None => "none".into(),
        Some((action, entry, at, reason)) => {
            let rationale = reason
                .as_deref()
                .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
                .and_then(|v| {
                    v.get("rationale")
                        .and_then(|r| r.as_str())
                        .map(str::to_string)
                });
            format!(
                "{} {} @ {} — {}",
                &at[..10.min(at.len())],
                action,
                entry
                    .map(|e| format!("{e:.2}"))
                    .unwrap_or_else(|| "-".into()),
                rationale.unwrap_or_else(|| "rules only".into())
            )
        }
    })
}

/// Everything held in this pot, with the latest close as its price — the table a rotation reads.
/// The short-horizon card: computed from the daily bars and today's tape we already hold.
async fn short_card(
    deps: &EngineDeps<'_>,
    instrument: &Instrument,
    intraday: &[crate::models::Bar],
    now: DateTime<Utc>,
) -> String {
    if instrument.class != crate::models::AssetClass::Stock {
        return String::new(); // a fund has no intraday horizon
    }
    let daily = match deps.market.bars(&instrument.symbol).await {
        Ok(b) => b,
        Err(e) => {
            tracing::debug!(error = %e, symbol = %instrument.symbol, "no daily bars for the card");
            return String::new();
        }
    };
    crate::rules::context::compute(&daily, intraday, now).render()
}

/// Revenue, profit and valuation, or "unavailable" — never a guess, and never a failed call.
async fn fetch_fundamentals(deps: &EngineDeps<'_>, symbol: &str, now: DateTime<Utc>) -> String {
    let Ok(client) =
        crate::http::client_with(&deps.runtime, 15, Some("Mozilla/5.0 stock-predictor"))
    else {
        return "unavailable".into();
    };
    match crate::market::fundamentals::fetch(&client, symbol, now.year()).await {
        Ok(f) => {
            let text = f.summary();
            if text.trim().is_empty() {
                "unavailable".into()
            } else {
                text
            }
        }
        Err(e) => {
            tracing::debug!(error = %e, symbol, "financials unavailable");
            "unavailable".into()
        }
    }
}

async fn load_holdings(
    pool: &SqlitePool,
    mode: crate::models::Mode,
) -> Result<Vec<crate::llm::rotate::HoldingLine>> {
    #[derive(sqlx::FromRow)]
    struct HeldRow {
        symbol: String,
        name: String,
        qty: f64,
        avg_cost: f64,
        stop: Option<f64>,
        target: Option<f64>,
        ltp: Option<f64>,
    }
    let rows: Vec<HeldRow> = sqlx::query_as(
        "SELECT i.symbol, i.name, p.qty, p.avg_cost, p.stop, p.target,
                    (SELECT b.close FROM bars b WHERE b.instrument_id = p.instrument_id
                     ORDER BY b.ts DESC LIMIT 1) AS ltp
             FROM positions p JOIN instruments i ON i.id = p.instrument_id
             WHERE p.state = 'open' AND p.mode = ?",
    )
    .bind(mode.as_str())
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| crate::llm::rotate::HoldingLine {
            symbol: r.symbol,
            name: r.name,
            qty: r.qty,
            avg_cost: r.avg_cost,
            ltp: r.ltp,
            stop: r.stop,
            target: r.target,
        })
        .collect())
}

/// Ask for a swap when a buy is right but the cash is not there.
///
/// Returns the two signal ids (primary sell, paired buy) when a swap was proposed and stored.
async fn rotate_for_cash(
    deps: &EngineDeps<'_>,
    instrument: &Instrument,
    candidate: &crate::models::Candidate,
    now: DateTime<Utc>,
) -> Result<Option<(i64, i64)>> {
    if candidate.action != Action::Buy {
        return Ok(None);
    }
    let available = budget::available_in(deps.pool, crate::models::Mode::Long).await?;
    let cost = budget::cost(candidate.qty, candidate.entry, instrument.class);
    if cost <= available + 1e-9 {
        return Ok(None); // it fits; nothing to sell
    }

    let holdings = load_holdings(deps.pool, crate::models::Mode::Long).await?;
    let settings = crate::rules::settings::get(deps.pool).await?;
    let profitable = holdings.iter().any(|h| {
        h.profit_pct()
            .is_some_and(|p| p >= settings.limits_long.min_profit_pct)
    });
    if !profitable {
        return Ok(None); // nothing worth selling, so do not spend a call on it
    }

    let table = crate::llm::rotate::holdings_table(&holdings, available, &settings.currency);
    let wanted = format!(
        "{} ({}) at {:.2}, {} units = {:.2} {}",
        instrument.symbol, instrument.name, candidate.entry, candidate.qty, cost, settings.currency
    );
    let rendered = crate::llm::prompts::render_rotation(&table, &wanted)?;
    let model = crate::rules::settings::model_config(deps.pool).await?;
    if crate::llm::orchestrate::daily_chat_calls(deps.pool).await? >= deps.runtime.daily_cap as i64
    {
        tracing::info!("daily call cap reached — rotation not attempted");
        return Ok(None);
    }
    let reply = crate::llm::deepseek::chat(
        deps.pool,
        &model,
        deps.transport,
        crate::llm::deepseek::ChatRequest {
            conversation_id: format!("rotate-{}-{}", instrument.symbol, now.timestamp_millis()),
            round: 1,
            prompt: rendered.text,
            prompt_template: rendered.template,
            prompt_hash: rendered.hash,
            trigger: Some(format!(
                "rotation: {} units at {:.2} costs {:.2}, cash {:.2} — looked for a profit to sell",
                candidate.qty, candidate.entry, cost, available
            )),
            purpose: format!("rotate {}", instrument.symbol),
        },
    )
    .await?;

    let rotation = match crate::llm::rotate::parse(&reply.content) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "rotation reply unreadable");
            return Ok(None);
        }
    };
    if let crate::llm::rotate::Rotation::Pass { why } = &rotation {
        tracing::info!(%why, "rotation declined");
        return Ok(None);
    }
    if let Err(e) = crate::llm::rotate::validate(
        &rotation,
        &holdings,
        available,
        &settings,
        crate::models::Mode::Long,
    ) {
        tracing::warn!(error = %e, "rotation refused by the validator");
        return Ok(None);
    }

    let crate::llm::rotate::Rotation::Swap(swap) = rotation else {
        return Ok(None);
    };
    let (sell, buy) = (swap.sell, swap.buy);
    let sell_id: (i64,) = sqlx::query_as(
        "INSERT INTO signals (instrument_id, action, qty, entry, valid_until, reason_json,
            created_at, confidence, mode, kind, status)
         SELECT id, 'SELL', ?, ?, ?, ?, ?, ?, 'long', 'rotation', 'pending'
         FROM instruments WHERE symbol = ? RETURNING id",
    )
    .bind(sell.qty)
    .bind(sell.price)
    .bind((now + chrono::Duration::hours(18)).to_rfc3339())
    .bind(serde_json::json!({ "reasons": [sell.why.clone().unwrap_or_default()] }).to_string())
    .bind(now.to_rfc3339())
    .bind(sell.confidence.clone())
    .bind(&sell.symbol)
    .fetch_one(deps.pool)
    .await?;

    let buy_id: (i64,) = sqlx::query_as(
        "INSERT INTO signals (instrument_id, action, qty, entry, stop, target, valid_until,
            reason_json, created_at, confidence, mode, kind, status, pair_id)
         SELECT id, 'BUY', ?, ?, ?, ?, ?, ?, ?, ?, 'long', 'rotation', 'pending', ?
         FROM instruments WHERE symbol = ? RETURNING id",
    )
    .bind(buy.qty)
    .bind(buy.price)
    .bind(buy.stop)
    .bind(buy.target)
    .bind((now + chrono::Duration::hours(18)).to_rfc3339())
    .bind(
        serde_json::json!({
            "reasons": [],
            "rationale": buy.rationale,
            "paired_with": sell.symbol,
        })
        .to_string(),
    )
    .bind(now.to_rfc3339())
    .bind(buy.confidence.clone())
    .bind(sell_id.0)
    .bind(&buy.symbol)
    .fetch_one(deps.pool)
    .await?;

    Ok(Some((sell_id.0, buy_id.0)))
}
