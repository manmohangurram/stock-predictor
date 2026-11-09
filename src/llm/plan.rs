//! The book call: the model sees every holding and the cash, and may answer with a plan of up to
//! three moves that stand or fall together.
//!
//! The parse is forgiving about the wrapper and strict about the content; the validation refuses
//! the whole plan rather than repairing it, for the same reason every other validator here does —
//! a half-executed swap is worse than none.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::rotate::HoldingLine;
use crate::models::Action;
use crate::rules::limits::Limits;

/// More than this is not a plan, it is a wish list.
pub const MAX_MOVES: usize = 3;

#[derive(Debug, Clone, Deserialize)]
pub struct Move {
    #[serde(deserialize_with = "action_of")]
    pub action: Action,
    pub symbol: String,
    pub qty: f64,
    pub price: f64,
    #[serde(default)]
    pub stop: Option<f64>,
    #[serde(default)]
    pub target: Option<f64>,
    #[serde(default)]
    pub why: String,
}

fn action_of<'de, D>(d: D) -> std::result::Result<Action, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = String::deserialize(d)?;
    match raw.trim().to_ascii_uppercase().as_str() {
        "BUY" => Ok(Action::Buy),
        "SELL" => Ok(Action::Sell),
        other => Err(serde::de::Error::custom(format!(
            "a plan moves BUY or SELL, not {other}"
        ))),
    }
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub moves: Vec<Move>,
    pub rationale: String,
}

#[derive(Debug, Clone)]
pub enum Answer {
    Plan(Plan),
    Pass { why: String },
}

/// The model's reply, before it is trusted.
pub fn parse(content: &str) -> Result<Answer> {
    let body = extract_json(content).ok_or_else(|| anyhow!("plan: no JSON in the reply"))?;
    let raw = body
        .get("moves")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if raw.is_empty() {
        return Ok(Answer::Pass {
            why: body
                .get("pass_reason")
                .and_then(|v| v.as_str())
                .unwrap_or("nothing worth doing today")
                .to_string(),
        });
    }
    if raw.len() > MAX_MOVES {
        return Err(anyhow!(
            "plan: {} moves is more than the {MAX_MOVES} allowed",
            raw.len()
        ));
    }
    let moves: Vec<Move> = raw
        .into_iter()
        .map(serde_json::from_value)
        .collect::<std::result::Result<_, _>>()?;
    if let Some(first_buy) = moves.iter().position(|m| m.action == Action::Buy) {
        if moves[first_buy..].iter().any(|m| m.action == Action::Sell) {
            return Err(anyhow!("plan: sells first, then buys"));
        }
    }
    Ok(Answer::Plan(Plan {
        moves,
        rationale: body
            .get("rationale")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
    }))
}

/// Tolerates a fence or a stray sentence around the object.
fn extract_json(content: &str) -> Option<serde_json::Value> {
    let start = content.find('{')?;
    let end = content.rfind('}')?;
    serde_json::from_str(&content[start..=end]).ok()
}

/// Which purse a move draws on. A plan is stocks-only, so this exists to *refuse* funds rather
/// than to balance them: a fund belongs to the daily arithmetic scan, which needs no model call.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Purse {
    Stock,
    Fund,
}

/// Cash free in each purse, before this plan's own sales.
#[derive(Debug, Clone, Copy, Default)]
pub struct Cash {
    pub stock: f64,
    pub fund: f64,
}

impl Cash {
    fn get(self, purse: Purse) -> f64 {
        match purse {
            Purse::Stock => self.stock,
            Purse::Fund => self.fund,
        }
    }
}

/// Why a plan is not allowed. Every message is user-facing.
///
/// Leg by leg first — a symbol that is not held, a stop on the wrong side, a quantity that is not a
/// number — then the plan as a whole: buys may not cost more than cash plus what the plan's own
/// sells free, and no symbol may appear twice.
pub fn validate(
    plan: &Plan,
    book: &[HoldingLine],
    cash: Cash,
    limits: &Limits,
    currency: &str,
    now: DateTime<Utc>,
    // Symbols that are mutual funds: they draw on the fund purse, never on stock money.
    funds: &[String],
) -> Result<()> {
    // Money is counted per purse. A plan is stocks-only, so only the stock purse is ever spent;
    // the fund purse is kept in the check so a mixed plan is refused for a stated reason.
    let purse_of = |symbol: &str| {
        if funds.iter().any(|f| f == symbol) {
            Purse::Fund
        } else {
            Purse::Stock
        }
    };
    let mut freed = [0.0f64; 2];
    let mut spent = [0.0f64; 2];
    let slot = |p: Purse| match p {
        Purse::Stock => 0,
        Purse::Fund => 1,
    };
    let mut seen: Vec<&str> = Vec::new();

    for m in &plan.moves {
        if seen.contains(&m.symbol.as_str()) {
            return Err(anyhow!("plan moves {} twice", m.symbol));
        }
        seen.push(&m.symbol);
        if funds.iter().any(|f| f == &m.symbol) {
            return Err(anyhow!(
                "{} is a mutual fund: funds are chosen by the daily scan from their own purse, not in a stock plan",
                m.symbol
            ));
        }
        if !m.qty.is_finite() || m.qty <= 0.0 {
            return Err(anyhow!("{} has no usable quantity", m.symbol));
        }
        if !m.price.is_finite() || m.price <= 0.0 {
            return Err(anyhow!("{} has no usable price", m.symbol));
        }
        let _ = now;
        match m.action {
            Action::Sell => {
                let held = book
                    .iter()
                    .find(|h| h.symbol == m.symbol)
                    .ok_or_else(|| anyhow!("{} is not held", m.symbol))?;
                if m.qty > held.qty + 1e-9 {
                    return Err(anyhow!(
                        "{} sells {:.3} but only {:.3} is held",
                        m.symbol,
                        m.qty,
                        held.qty
                    ));
                }
                freed[slot(purse_of(&m.symbol))] += m.qty * m.price;
            }
            Action::Buy => {
                if m.stop.is_none() || m.target.is_none() {
                    return Err(anyhow!("{} needs a stop and a target", m.symbol));
                }
                if let Some(stop) = m.stop {
                    if !stop.is_finite() || stop >= m.price {
                        return Err(anyhow!("{}: stop on the wrong side of entry", m.symbol));
                    }
                }
                if let Some(target) = m.target {
                    if !target.is_finite() || target <= m.price {
                        return Err(anyhow!(
                            "{}: target on the wrong side of entry",
                            m.symbol
                        ));
                    }
                }
                spent[slot(purse_of(&m.symbol))] += m.qty * m.price;
            }
            Action::Hold | Action::None => {}
        }
    }

    for (purse, name) in [(Purse::Stock, "stock"), (Purse::Fund, "fund")] {
        let i = slot(purse);
        if spent[i] > freed[i] + cash.get(purse) + 1e-6 {
            return Err(anyhow!(
                "the {name} purse does not balance: buys cost {:.2} but the plan frees {:.2} plus {:.2} {currency} of {name} cash — the two purses never lend to each other",
                spent[i],
                freed[i],
                cash.get(purse)
            ));
        }
    }
    let total_spent = spent[0] + spent[1];
    let total_freed = freed[0] + freed[1];
    if total_spent > limits.max_trade_amount + total_freed + 1e-6 {
        return Err(anyhow!(
            "the plan spends {total_spent:.2}, above the {:.2} ceiling for one plan",
            limits.max_trade_amount
        ));
    }
    Ok(())
}

/// The book as the prompt reads it: one line per holding, richest first.
pub fn book_block(book: &[HoldingLine], currency: &str) -> String {
    let mut out = String::from("symbol | name | qty | avg cost | last | P&L % | stop | target\n");
    for h in book {
        let pct = h
            .profit_pct()
            .map(|p| format!("{p:+.1}%"))
            .unwrap_or_else(|| "—".into());
        out.push_str(&format!(
            "{} | {} | {:.3} | {:.2} | {} | {} | {} | {}\n",
            h.symbol,
            h.name,
            h.qty,
            h.avg_cost,
            h.ltp.map(|v| format!("{v:.2}")).unwrap_or("—".into()),
            pct,
            h.stop.map(|v| format!("{v:.2}")).unwrap_or("—".into()),
            h.target.map(|v| format!("{v:.2}")).unwrap_or("—".into()),
        ));
    }
    out.push_str(&format!(
        "\ntotal invested: {:.2} {currency}\n",
        book.iter().map(|h| h.qty * h.avg_cost).sum::<f64>()
    ));
    out
}
