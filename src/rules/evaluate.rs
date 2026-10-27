use super::budget;
use super::indicators::Indicators;
use crate::models::{Action, AssetClass, Bar, Candidate, Instrument, Position};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RuleConfig {
    pub rsi_buy_ceiling: f64,
    pub rsi_sell_floor: f64,
    pub stop_atr: f64,
    pub target_atr: f64,
}

impl Default for RuleConfig {
    fn default() -> Self {
        Self {
            rsi_buy_ceiling: 70.0,
            rsi_sell_floor: 75.0,
            stop_atr: 2.0,
            target_atr: 3.0,
        }
    }
}

pub struct CandidateInput<'a> {
    pub instrument: &'a Instrument,
    pub bars: &'a [Bar],
    pub ind: &'a Indicators,
    pub position: Option<&'a Position>,
    pub available_cash: f64,
    pub cfg: RuleConfig,
}

/// The cheap gate. Returns `None` when no rule fires, which is what keeps the LLM off the
/// wire on an ordinary poll.
pub fn candidate(input: CandidateInput<'_>) -> Option<Candidate> {
    let close = input.bars.last()?.close;
    if !close.is_finite() || close <= 0.0 {
        return None;
    }
    let ind = input.ind;
    let class = input.instrument.class;
    let mut reasons = Vec::new();

    if let Some(pos) = input.position {
        let below_stop = pos.stop.is_some_and(|s| close < s);
        let trend_break = ind.rsi14 > input.cfg.rsi_sell_floor && close < ind.sma50;
        if below_stop {
            reasons.push(format!(
                "close {close:.2} below recorded stop {:.2}",
                pos.stop.unwrap_or_default()
            ));
        }
        if trend_break {
            reasons.push(format!(
                "RSI {:.1} overbought with close below SMA50 {:.2}",
                ind.rsi14, ind.sma50
            ));
        }
        if below_stop || trend_break {
            let stop_hit = pos.stop.is_some_and(|s| close < s);
            return Some(Candidate {
                instrument_id: input.instrument.id,
                action: Action::Sell,
                reasons,
                entry: close,
                stop: None,
                target: pos.target,
                qty: pos.qty,
                timing_note: class.timing_note().to_string(),
                urgent: stop_hit,
            });
        }
    }

    let uptrend = close > ind.sma50 && ind.sma50 > ind.sma200;
    let not_overbought = ind.rsi14 < input.cfg.rsi_buy_ceiling;
    if !(uptrend && not_overbought) {
        return None;
    }

    let qty = match class {
        AssetClass::MutualFund => budget::units_floor(input.available_cash / close),
        _ => (input.available_cash / close).floor(),
    };
    if qty < 1.0 && !(class == AssetClass::MutualFund && qty > 0.0) {
        return None;
    }

    reasons.push(format!("close {close:.2} above SMA50 {:.2} and SMA200 {:.2}", ind.sma50, ind.sma200));
    reasons.push(format!("RSI {:.1} below buy ceiling {:.1}", ind.rsi14, input.cfg.rsi_buy_ceiling));

    let (stop, target) = match class {
        AssetClass::Stock => (
            Some(close - input.cfg.stop_atr * ind.atr14),
            Some(close + input.cfg.target_atr * ind.atr14),
        ),
        // MF and IPO have no intraday stop: their execution is NAV-of-the-day or allotment.
        AssetClass::MutualFund | AssetClass::Ipo => (None, None),
    };

    Some(Candidate {
        instrument_id: input.instrument.id,
        action: Action::Buy,
        reasons,
        entry: close,
        stop,
        target,
        qty,
        timing_note: class.timing_note().to_string(),
        urgent: false,
    })
}
