use thiserror::Error;

use crate::models::Bar;

/// Half-width of the validator's allowed price band, in ATRs. Defined once and used by both
/// `atr_bands` and the LLM proposal validator so the two can never drift apart.
pub const BAND_MULT: f64 = 1.0;

pub const MIN_BARS: usize = 200;

#[derive(Debug, Clone, PartialEq)]
pub struct Indicators {
    pub sma50: f64,
    pub sma200: f64,
    pub rsi14: f64,
    pub atr14: f64,
    pub high52: f64,
    pub low52: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bands {
    pub min: f64,
    pub max: f64,
}

#[derive(Debug, Error, PartialEq)]
pub enum IndicatorError {
    #[error("insufficient bars: need {MIN_BARS}, got {0}")]
    InsufficientBars(usize),
    #[error("non-finite indicator value — bad input series")]
    NonFinite,
}

/// Every output is finite or the call errors. A NaN signal is worse than no signal.
pub fn compute(bars: &[Bar]) -> Result<Indicators, IndicatorError> {
    if bars.len() < MIN_BARS {
        return Err(IndicatorError::InsufficientBars(bars.len()));
    }
    let closes: Vec<f64> = bars.iter().map(|b| b.close).collect();

    let ind = Indicators {
        sma50: mean(&closes[closes.len() - 50..]),
        sma200: mean(&closes[closes.len() - 200..]),
        rsi14: rsi(&closes, 14),
        atr14: atr(bars, 14),
        high52: bars[bars.len().saturating_sub(252)..]
            .iter()
            .map(|b| b.high)
            .fold(f64::MIN, f64::max),
        low52: bars[bars.len().saturating_sub(252)..]
            .iter()
            .map(|b| b.low)
            .fold(f64::MAX, f64::min),
    };

    let finite = ind.sma50.is_finite()
        && ind.sma200.is_finite()
        && ind.rsi14.is_finite()
        && ind.atr14.is_finite()
        && ind.high52.is_finite()
        && ind.low52.is_finite();
    if !finite {
        return Err(IndicatorError::NonFinite);
    }
    Ok(ind)
}

pub fn atr_bands(last_close: f64, atr14: f64) -> Bands {
    let span = atr14 * BAND_MULT;
    Bands {
        min: last_close - span,
        max: last_close + span,
    }
}

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return f64::NAN;
    }
    xs.iter().sum::<f64>() / xs.len() as f64
}

fn rsi(closes: &[f64], period: usize) -> f64 {
    if closes.len() <= period {
        return f64::NAN;
    }
    let mut gains = 0.0;
    let mut losses = 0.0;
    for i in 1..=period {
        let d = closes[i] - closes[i - 1];
        if d >= 0.0 {
            gains += d
        } else {
            losses -= d
        }
    }
    let (mut ag, mut al) = (gains / period as f64, losses / period as f64);

    for i in period + 1..closes.len() {
        let d = closes[i] - closes[i - 1];
        let (g, l) = if d >= 0.0 { (d, 0.0) } else { (0.0, -d) };
        ag = (ag * (period as f64 - 1.0) + g) / period as f64;
        al = (al * (period as f64 - 1.0) + l) / period as f64;
    }
    if al == 0.0 {
        return 100.0;
    }
    let rs = ag / al;
    100.0 - (100.0 / (1.0 + rs))
}

fn atr(bars: &[Bar], period: usize) -> f64 {
    if bars.len() <= period {
        return f64::NAN;
    }
    let tr_at = |i: usize| -> f64 {
        let b = &bars[i];
        let prev = bars[i - 1].close;
        (b.high - b.low)
            .max((b.high - prev).abs())
            .max((b.low - prev).abs())
    };
    let mut atr = (1..=period).map(tr_at).sum::<f64>() / period as f64;
    for i in period + 1..bars.len() {
        atr = (atr * (period as f64 - 1.0) + tr_at(i)) / period as f64;
    }
    atr
}
