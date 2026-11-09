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
    /// Units for a stock plan; a fund plan sizes in rupees instead, because units arrive at NAV.
    #[serde(default)]
    pub qty: Option<f64>,
    #[serde(default)]
    pub price: Option<f64>,
    /// Rupees, for a fund plan. The purse is denominated in money, not units.
    #[serde(default)]
    pub amount: Option<f64>,
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

/// What a plan call may look at. Two prompts, two purses, two books: a stock plan cannot name a
/// fund, and a fund plan cannot name a stock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Stock,
    Fund,
}

impl Scope {
    pub fn template(self) -> &'static str {
        match self {
            Scope::Stock => "plan_stock_v1",
            Scope::Fund => "plan_mf_v1",
        }
    }

    pub fn is_fund(self) -> bool {
        self == Scope::Fund
    }
}

/// Which purse a move draws on.
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
#[allow(clippy::too_many_arguments)]
pub fn validate(
    plan: &Plan,
    book: &[HoldingLine],
    cash: Cash,
    limits: &Limits,
    currency: &str,
    now: DateTime<Utc>,
    // Symbols that are mutual funds: a stock plan may not name one, a fund plan may name nothing else.
    funds: &[String],
    scope: Scope,
) -> Result<()> {
    // A plan has one purse. The other is reported as zero, so a mixed plan cannot balance.
    let purse = if scope.is_fund() {
        Purse::Fund
    } else {
        Purse::Stock
    };
    let mut freed = 0.0;
    let mut spent = 0.0;
    let mut seen: Vec<&str> = Vec::new();
    let _ = now;

    for m in &plan.moves {
        let is_fund = funds.iter().any(|f| f == &m.symbol);
        match (scope, is_fund) {
            (Scope::Stock, true) => {
                return Err(anyhow!(
                    "{} is a mutual fund: it belongs to the fund plan, not the stock plan",
                    m.symbol
                ))
            }
            (Scope::Fund, false) => {
                return Err(anyhow!(
                    "{} is a stock: the fund plan may only move mutual funds",
                    m.symbol
                ))
            }
            _ => {}
        }
        if seen.contains(&m.symbol.as_str()) {
            return Err(anyhow!("plan moves {} twice", m.symbol));
        }
        seen.push(&m.symbol);

        // Money is the unit of account: stock plans price it as qty × price, fund plans take rupees.
        let value = match scope {
            Scope::Fund => m
                .amount
                .filter(|a| a.is_finite() && *a > 0.0)
                .ok_or_else(|| {
                    anyhow!(
                        "{} has no amount — a fund move is sized in rupees",
                        m.symbol
                    )
                })?,
            Scope::Stock => {
                let qty = m
                    .qty
                    .filter(|q| q.is_finite() && *q > 0.0)
                    .ok_or_else(|| anyhow!("{} has no usable quantity", m.symbol))?;
                let price = m
                    .price
                    .filter(|p| p.is_finite() && *p > 0.0)
                    .ok_or_else(|| anyhow!("{} has no usable price", m.symbol))?;
                qty * price
            }
        };

        match m.action {
            Action::Sell => {
                let held = book
                    .iter()
                    .find(|h| h.symbol == m.symbol)
                    .ok_or_else(|| anyhow!("{} is not held", m.symbol))?;
                if let Some(qty) = m.qty {
                    if qty > held.qty + 1e-9 {
                        return Err(anyhow!(
                            "{} sells {:.3} but only {:.3} is held",
                            m.symbol,
                            qty,
                            held.qty
                        ));
                    }
                }
                // A fund is sized in rupees, so its sale is checked against what the holding is
                // worth. A stock's units are checked above, and its price is the model's own.
                if scope.is_fund() {
                    let held_value = held.qty * held.ltp.unwrap_or(held.avg_cost);
                    if value > held_value + 1e-6 {
                        return Err(anyhow!(
                            "{} sells {value:.2} but only {held_value:.2} is held",
                            m.symbol
                        ));
                    }
                }
                freed += value;
            }
            Action::Buy => {
                // A stock buy without an exit is a position with no way out.
                if !scope.is_fund() {
                    let stop = m
                        .stop
                        .ok_or_else(|| anyhow!("{} needs a stop and a target", m.symbol))?;
                    let target = m
                        .target
                        .ok_or_else(|| anyhow!("{} needs a stop and a target", m.symbol))?;
                    let price = m.price.unwrap_or_default();
                    if !stop.is_finite() || stop >= price {
                        return Err(anyhow!("{}: stop on the wrong side of entry", m.symbol));
                    }
                    if !target.is_finite() || target <= price {
                        return Err(anyhow!("{}: target on the wrong side of entry", m.symbol));
                    }
                }
                spent += value
            }
            Action::Hold | Action::None => {}
        }
    }

    if spent > freed + cash.get(purse) + 1e-6 {
        return Err(anyhow!(
            "the {} purse does not balance: buys cost {:.2} but the plan frees {:.2} plus {:.2} {currency} of {} cash",
            if scope.is_fund() { "fund" } else { "stock" },
            spent,
            freed,
            cash.get(purse),
            if scope.is_fund() { "fund" } else { "stock" }
        ));
    }
    if spent > limits.max_trade_amount + freed + 1e-6 {
        return Err(anyhow!(
            "the plan spends {spent:.2}, above the {:.2} ceiling for one plan",
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
