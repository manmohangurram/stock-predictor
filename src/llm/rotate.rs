//! Portfolio rotation: sell something you hold to fund something better.
//!
//! One decision with two legs. The buy is only valid if the sell happens, so both are stored linked
//! and approved together — a half-applied swap would leave the book in a state neither leg intended.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::models::{AssetClass, Mode};
use crate::rules::limits::Limits;
use crate::rules::settings::Settings;

/// One line of the holdings table the model is given.
#[derive(Debug, Clone, PartialEq)]
pub struct HoldingLine {
    pub symbol: String,
    pub name: String,
    pub qty: f64,
    pub avg_cost: f64,
    pub ltp: Option<f64>,
    pub stop: Option<f64>,
    pub target: Option<f64>,
}

impl HoldingLine {
    pub fn profit_pct(&self) -> Option<f64> {
        let ltp = self.ltp?;
        if self.avg_cost <= 0.0 {
            return None;
        }
        Some((ltp - self.avg_cost) / self.avg_cost * 100.0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Leg {
    pub symbol: String,
    pub qty: f64,
    pub price: f64,
    #[serde(default)]
    pub why: Option<String>,
    #[serde(default)]
    pub rationale: Option<String>,
    #[serde(default)]
    pub stop: Option<f64>,
    #[serde(default)]
    pub target: Option<f64>,
    #[serde(default)]
    pub confidence: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Rotation {
    // Boxed: a swap is an order of magnitude larger than a pass, and it is passed by value.
    Swap(Box<Swap>),
    Pass { why: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Swap {
    pub sell: Leg,
    pub buy: Leg,
}

/// The table the model reads. Compact on purpose: one line per holding.
pub fn holdings_table(holdings: &[HoldingLine], cash: f64, currency: &str) -> String {
    let mut out = String::from("symbol | name | qty | avg cost | last | P&L % | stop | target\n");
    for h in holdings {
        let pct = h
            .profit_pct()
            .map(|p| format!("{p:+.1}%"))
            .unwrap_or_else(|| "—".into());
        out.push_str(&format!(
            "{} | {} | {} | {:.2} | {} | {} | {} | {}\n",
            h.symbol,
            h.name,
            h.qty,
            h.avg_cost,
            h.ltp.map(|v| format!("{v:.2}")).unwrap_or_else(|| "—".into()),
            pct,
            h.stop.map(|v| format!("{v:.2}")).unwrap_or_else(|| "—".into()),
            h.target.map(|v| format!("{v:.2}")).unwrap_or_else(|| "—".into()),
        ));
    }
    out.push_str(&format!("\ncash available: {cash:.2} {currency}\n"));
    out
}

/// What the model answered, before it is trusted.
pub fn parse(content: &str) -> Result<Rotation> {
    let body = extract_json(content).ok_or_else(|| anyhow!("rotation: no JSON in the reply"))?;
    let action = body
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    match action {
        "rotate" | "decide" => {
            let sell = body
                .get("sell")
                .ok_or_else(|| anyhow!("rotation: no sell leg"))?;
            let buy = body.get("buy").ok_or_else(|| anyhow!("rotation: no buy leg"))?;
            Ok(Rotation::Swap(Box::new(Swap {
                sell: serde_json::from_value(sell.clone())?,
                buy: serde_json::from_value(buy.clone())?,
            })))
        }
        "pass" | "none" => Ok(Rotation::Pass {
            why: body
                .get("why")
                .and_then(|v| v.as_str())
                .unwrap_or("nothing worth selling")
                .to_string(),
        }),
        other => Err(anyhow!("rotation: unexpected action {other:?}")),
    }
}

/// Tolerates a fence or a stray sentence around the object.
fn extract_json(content: &str) -> Option<serde_json::Value> {
    let start = content.find('{')?;
    let end = content.rfind('}')?;
    serde_json::from_str(&content[start..=end]).ok()
}

/// Why a swap is not allowed. Every message is user-facing.
pub fn validate(
    r: &Rotation,
    holdings: &[HoldingLine],
    cash: f64,
    settings: &Settings,
    mode: Mode,
) -> Result<()> {
    let Rotation::Swap(swap) = r else {
        return Ok(());
    };
    let (sell, buy) = (&swap.sell, &swap.buy);
    let limits: &Limits = match mode {
        Mode::Long => &settings.limits_long,
        Mode::Trade => &settings.limits_trade,
    };

    let held = holdings
        .iter()
        .find(|h| h.symbol.eq_ignore_ascii_case(&sell.symbol))
        .ok_or_else(|| anyhow!("rotation: {} is not held", sell.symbol))?;
    if !sell.qty.is_finite() || sell.qty <= 0.0 || sell.qty > held.qty + 1e-9 {
        return Err(anyhow!(
            "rotation: cannot sell {} of {}, only {} held",
            sell.qty,
            sell.symbol,
            held.qty
        ));
    }
    if !sell.price.is_finite() || sell.price <= 0.0 {
        return Err(anyhow!("rotation: the sell price is not a price"));
    }
    // Selling a loss to buy something else is not a rotation, it is a way to lock in damage.
    if let Some(pct) = held.profit_pct() {
        if pct < limits.min_profit_pct {
            return Err(anyhow!(
                "rotation: {} is only {pct:.1}% up, below the {:.1}% the pot expects — not a profit to take",
                held.symbol,
                limits.min_profit_pct
            ));
        }
    } else {
        return Err(anyhow!("rotation: {} has no live price", held.symbol));
    }

    if !buy.qty.is_finite() || buy.qty <= 0.0 || !buy.price.is_finite() || buy.price <= 0.0 {
        return Err(anyhow!("rotation: the buy leg is not a trade"));
    }
    let proceeds = sell.qty * sell.price;
    let cost = buy.qty * buy.price;
    if cost > proceeds + cash + 1e-9 {
        return Err(anyhow!(
            "rotation: cost {cost:.2} exceeds proceeds {proceeds:.2} plus cash {cash:.2}"
        ));
    }
    if cost > limits.max_trade_amount + 1e-9 {
        return Err(anyhow!(
            "rotation: {cost:.2} is above the pot's largest trade {:.2}",
            limits.max_trade_amount
        ));
    }
    if let (Some(stop), Some(target)) = (buy.stop, buy.target) {
        if !stop.is_finite() || stop >= buy.price {
            return Err(anyhow!("rotation: stop must sit below the buy price"));
        }
        if !target.is_finite() || target <= buy.price {
            return Err(anyhow!("rotation: target must sit above the buy price"));
        }
        let floor = buy.price * (1.0 + limits.min_profit_pct / 100.0);
        if target + 1e-9 < floor {
            return Err(anyhow!(
                "rotation: the target is below the pot's minimum profit of {:.1}%",
                limits.min_profit_pct
            ));
        }
    } else {
        return Err(anyhow!("rotation: the buy needs both a stop and a target"));
    }

    if buy.symbol.eq_ignore_ascii_case(&sell.symbol) {
        return Err(anyhow!("rotation: selling and buying the same symbol"));
    }
    Ok(())
}

/// A mutual fund has no intraday exit, so it can never be the buy leg of a trading-pot rotation.
pub fn buy_allowed(class: AssetClass, mode: Mode) -> bool {
    class.pot_allowed(mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held() -> Vec<HoldingLine> {
        vec![HoldingLine {
            symbol: "INFY.NS".into(),
            name: "Infosys".into(),
            qty: 24.0,
            avg_cost: 1042.30,
            ltp: Some(1171.60),
            stop: Some(1080.0),
            target: Some(1420.0),
        }]
    }

    fn settings() -> Settings {
        Settings {
            budget: 0.0,
            committed: 0.0,
            available: 0.0,
            currency: "INR".into(),
            currency_locked: false,
            deepseek_configured: false,
            trading_budget: 0.0,
            trading_available: 0.0,
            ntfy_url: String::new(),
            ntfy_topic: String::new(),
            ntfy_token_configured: false,
            llm_provider: "deepseek".into(),
            llm_model: String::new(),
            llm_base_url: String::new(),
            llm_key_configured: false,
            limits_long: Limits {
                min_profit_pct: 5.0,
                max_profit_pct: 50.0,
                min_trade_amount: 1_000.0,
                max_trade_amount: 1_000_000.0,
                max_instrument_pct: 25.0,
            },
            limits_trade: Limits::default(),
            runtime: crate::rules::runtime::Runtime::default().view(),
        }
    }

    fn swap(sell_qty: f64, buy_qty: f64, buy_price: f64, target: f64) -> Rotation {
        Rotation::Swap(Box::new(Swap {
            sell: Leg {
                symbol: "INFY.NS".into(),
                qty: sell_qty,
                price: 1171.60,
                why: Some("target reached".into()),
                rationale: None,
                stop: None,
                target: None,
                confidence: None,
            },
            buy: Leg {
                symbol: "HDFCBANK.NS".into(),
                qty: buy_qty,
                price: buy_price,
                why: None,
                rationale: Some("better trend".into()),
                stop: Some(buy_price * 0.92),
                target: Some(target),
                confidence: Some("medium".into()),
            },
        }))
    }

    #[test]
    fn a_sensible_swap_is_allowed() {
        let r = swap(24.0, 8.0, 784.10, 980.0);
        assert!(validate(&r, &held(), 1000.0, &settings(), Mode::Long).is_ok());
    }

    #[test]
    fn the_buy_cannot_cost_more_than_the_sale_plus_cash() {
        let r = swap(24.0, 40.0, 784.10, 980.0); // 31,364 > 28,118 + 100
        let err = validate(&r, &held(), 100.0, &settings(), Mode::Long).unwrap_err();
        assert!(err.to_string().contains("exceeds proceeds"), "{err}");
    }

    #[test]
    fn selling_a_loss_maker_is_refused() {
        let mut rows = held();
        rows[0].ltp = Some(1000.0); // 4% down on cost
        let err = validate(&swap(24.0, 8.0, 784.10, 980.0), &rows, 0.0, &settings(), Mode::Long)
            .unwrap_err();
        assert!(err.to_string().contains("below the"), "{err}");
    }

    #[test]
    fn a_buy_without_both_exits_is_refused() {
        let mut r = swap(24.0, 8.0, 784.10, 980.0);
        if let Rotation::Swap(swap) = &mut r {
            swap.buy.target = None;
        }
        let err = validate(&r, &held(), 0.0, &settings(), Mode::Long).unwrap_err();
        assert!(err.to_string().contains("stop and a target"), "{err}");
    }

    #[test]
    fn a_rotation_reply_is_parsed_from_prose_or_a_fence() {
        let raw = r#"Here you go:
```json
{"action":"rotate",
 "sell":{"symbol":"INFY.NS","qty":24,"price":1171.60,"why":"target reached"},
 "buy":{"symbol":"HDFCBANK.NS","qty":8,"price":784.10,"stop":700,"target":980,
        "rationale":"stronger trend","confidence":"medium"}}
```"#;
        match parse(raw).unwrap() {
            Rotation::Swap(swap) => {
                assert_eq!(swap.sell.symbol, "INFY.NS");
                assert_eq!(swap.buy.stop, Some(700.0));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_pass_is_a_valid_answer() {
        match parse(r#"{"action":"pass","why":"nothing worth selling"}"#).unwrap() {
            Rotation::Pass { why } => assert!(why.contains("nothing")),
            other => panic!("{other:?}"),
        }
    }
}
