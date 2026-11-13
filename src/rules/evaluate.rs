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
        // The other half of a trade: the target is a sell order that already exists, not a wish.
        let at_target = pos.target.is_some_and(|t| close >= t);
        let trend_break = ind.rsi14 > input.cfg.rsi_sell_floor && close < ind.sma50;
        if below_stop {
            reasons.push(format!(
                "close {close:.2} below recorded stop {:.2}",
                pos.stop.unwrap_or_default()
            ));
        }
        if at_target {
            reasons.push(format!(
                "close {close:.2} at or above recorded target {:.2} — take the profit",
                pos.target.unwrap_or_default()
            ));
        }
        if trend_break {
            reasons.push(format!(
                "RSI {:.1} overbought with close below SMA50 {:.2}",
                ind.rsi14, ind.sma50
            ));
        }
        if below_stop || trend_break || at_target {
            let stop_hit = below_stop || at_target;
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

    reasons.push(format!(
        "close {close:.2} above SMA50 {:.2} and SMA200 {:.2}",
        ind.sma50, ind.sma200
    ));
    reasons.push(format!(
        "RSI {:.1} below buy ceiling {:.1}",
        ind.rsi14, input.cfg.rsi_buy_ceiling
    ));

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

/// One named rule and whether it passed, with the numbers that decided it.
#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub name: &'static str,
    pub passed: bool,
    pub detail: String,
}

/// The same predicates `candidate` applies, but reported instead of collapsed into
/// `Option`. This is what lets the UI answer "why is the ledger empty?".
pub fn diagnose(input: &CandidateInput<'_>) -> Vec<Check> {
    let mut checks = Vec::new();
    let Some(last) = input.bars.last() else {
        return vec![Check {
            name: "price data",
            passed: false,
            detail: "no bars stored".into(),
        }];
    };
    let close = last.close;
    let ind = input.ind;

    checks.push(Check {
        name: "price data",
        passed: true,
        detail: format!("{} bars, last close {close:.2}", input.bars.len()),
    });
    checks.push(Check {
        name: "trend",
        passed: close > ind.sma50 && ind.sma50 > ind.sma200,
        detail: format!(
            "close {close:.2} vs SMA50 {:.2} vs SMA200 {:.2}",
            ind.sma50, ind.sma200
        ),
    });
    checks.push(Check {
        name: "not overbought",
        passed: ind.rsi14 < input.cfg.rsi_buy_ceiling,
        detail: format!(
            "RSI {:.1} must be below {:.0}",
            ind.rsi14, input.cfg.rsi_buy_ceiling
        ),
    });
    let qty = match input.instrument.class {
        AssetClass::MutualFund => budget::units_floor(input.available_cash / close.max(1e-9)),
        _ => (input.available_cash / close.max(1e-9)).floor(),
    };
    checks.push(Check {
        name: "affordable",
        passed: qty >= 1.0 && input.instrument.class != AssetClass::MutualFund || qty > 0.0,
        detail: format!(
            "{qty} units at {close:.2} against {:.2} available",
            input.available_cash
        ),
    });
    if let Some(pos) = input.position {
        let stop_hit = pos.stop.is_some_and(|s| close < s);
        checks.push(Check {
            name: "stop not breached",
            passed: !stop_hit,
            detail: match pos.stop {
                Some(s) => format!("close {close:.2} vs stop {s:.2}"),
                None => "no stop recorded".into(),
            },
        });
    }

    checks
}

/// Why a holding has left its band, if it has. `None` means the rules already cover it or it is
/// inside the band.
pub fn band_reason(
    stop: Option<f64>,
    avg_cost: f64,
    close: f64,
    max_profit_pct: f64,
) -> Option<&'static str> {
    if avg_cost > 0.0 && close >= avg_cost * (1.0 + max_profit_pct / 100.0) {
        return Some("ceiling");
    }
    if stop.is_some_and(|s| close <= s) {
        return Some("broken");
    }
    None
}

/// Where a stop should sit given the price now, for a position already held.
///
/// Two ATRs under the close, never under what was paid once the price is above it, never below the
/// stop already recorded. Raise only.
pub fn trailed_stop(current: Option<f64>, avg_cost: f64, close: f64, atr: f64) -> Option<f64> {
    if close <= 0.0 || avg_cost <= 0.0 || !atr.is_finite() || atr <= 0.0 {
        return current;
    }
    // Below what was paid there is nothing to protect: the stop is the owner's own.
    if close <= avg_cost {
        return current;
    }
    let mut next = (close - 2.0 * atr).max(avg_cost);
    if let Some(held) = current {
        next = next.max(held);
    }
    Some(next)
}

#[cfg(test)]
mod trail_tests {
    use super::{band_reason, trailed_stop};

    #[test]
    fn the_band_speaks_only_when_an_edge_is_crossed() {
        // Paid 100, ceiling 20%: 130 is past it, 110 is not.
        assert_eq!(band_reason(Some(80.0), 100.0, 130.0, 20.0), Some("ceiling"));
        assert_eq!(band_reason(Some(80.0), 100.0, 110.0, 20.0), None);
        // Through the stop is the other edge.
        assert_eq!(band_reason(Some(80.0), 100.0, 75.0, 20.0), Some("broken"));
        // No stop, no ceiling reached: nothing to say.
        assert_eq!(band_reason(None, 100.0, 90.0, 20.0), None);
    }

    #[test]
    fn a_winner_never_trails_below_its_own_cost() {
        // Up 30%: two ATRs back is above cost, so the stop follows the price.
        assert_eq!(trailed_stop(Some(90.0), 100.0, 130.0, 5.0), Some(120.0));
        // Up only 3%: two ATRs back is below cost, so the stop goes to break-even.
        assert_eq!(trailed_stop(Some(90.0), 100.0, 103.0, 5.0), Some(100.0));
    }

    #[test]
    fn a_loser_keeps_the_stop_the_owner_set() {
        assert_eq!(trailed_stop(Some(90.0), 100.0, 96.0, 5.0), Some(90.0));
        assert_eq!(trailed_stop(None, 100.0, 96.0, 5.0), None);
    }

    #[test]
    fn a_stop_is_never_pulled_back_down() {
        // The owner placed it at 125; the trail would compute 118.
        assert_eq!(trailed_stop(Some(125.0), 100.0, 130.0, 6.0), Some(125.0));
    }

    #[test]
    fn nonsense_inputs_change_nothing() {
        assert_eq!(trailed_stop(Some(90.0), 100.0, 130.0, 0.0), Some(90.0));
        assert_eq!(trailed_stop(Some(90.0), 0.0, 130.0, 5.0), Some(90.0));
    }
}
