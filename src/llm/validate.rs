use chrono::{DateTime, Utc};
use thiserror::Error;

use super::parse::Proposal;
use crate::models::Action;
use crate::rules::indicators::Bands;

/// A stop may sit further from the close than the entry band; anything beyond this many
/// ATRs is a typo or a hallucination, not a stop.
pub const MAX_STOP_ATRS: f64 = 5.0;
pub const MAX_TARGET_ATRS: f64 = 10.0;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum ValidationError {
    #[error("stop on the wrong side of entry")]
    StopOnWrongSide,
    #[error("entry is outside the ATR band around the last close")]
    PriceOutOfBand,
    #[error("stop or target is implausibly far from the entry")]
    DistanceTooFar,
    #[error("the trade costs more than the cash available")]
    InsufficientCash,
    #[error("sell proposed for an instrument that is not held")]
    NotHeld,
    #[error("valid_until is not a future session")]
    BadValidity,
    #[error("cited headline id was never sent to the model")]
    UnknownHeadlineId,
    #[error("a decision must cite at least one headline")]
    MissingCitation,
    #[error("a buy must carry both an exit stop and a take-profit target")]
    MissingExit,
    #[error("quantity must be a positive, finite number")]
    BadQuantity,
    #[error("cannot sell more units than are held")]
    TooManyUnits,
    #[error("take-profit does not reach the required {required_pct}%")]
    TargetBelowRequirement { required_pct: f64 },
    #[error("take-profit exceeds the {max_pct}% limit — do not chase fantasy targets")]
    TargetAboveLimit { max_pct: f64 },
    #[error("trade is below the minimum of {min}")]
    TradeTooSmall { min: f64 },
    #[error("trade is above the maximum of {max}")]
    TradeTooLarge { max: f64 },
    #[error("this would put {already} + the trade over the {cap} cap for one instrument")]
    InstrumentLimit { cap: f64, already: f64 },
}

pub struct ValidateCtx<'a> {
    /// `entry` must sit inside this band; stop and target are deliberately outside it.
    pub bands: Bands,
    pub atr: f64,
    pub available_cash: f64,
    pub qty: f64,
    pub held_qty: f64,
    pub sent_headline_ids: &'a [i64],
    /// True when this instrument can actually be exited at a price (stocks). Funds and IPOs
    /// cannot, so requiring an exit from them would be nonsense.
    pub requires_exit: bool,
    /// The limits for the pot this trade belongs to.
    pub limits: crate::rules::limits::Limits,
    /// What this pot has already committed to this instrument.
    pub instrument_exposure: f64,
    /// The pot's ceiling, which the percentage caps are measured against.
    pub pot: f64,
    pub now: DateTime<Utc>,
}

pub fn check(p: &Proposal, ctx: &ValidateCtx<'_>) -> Result<(), ValidationError> {
    if !p.entry.is_finite() || p.entry <= 0.0 {
        return Err(ValidationError::PriceOutOfBand);
    }

    match p.action {
        Action::Buy => {
            if ctx.requires_exit && (p.stop.is_none() || p.target.is_none()) {
                return Err(ValidationError::MissingExit);
            }
            if let Some(stop) = p.stop {
                // Written as a positive test so a NaN stop fails instead of slipping through.
                if !stop.is_finite() || stop >= p.entry {
                    return Err(ValidationError::StopOnWrongSide);
                }
            }
        }
        Action::Sell => {
            if ctx.held_qty <= 0.0 {
                return Err(ValidationError::NotHeld);
            }
            if let Some(stop) = p.stop {
                if !stop.is_finite() || stop <= p.entry {
                    return Err(ValidationError::StopOnWrongSide);
                }
            }
        }
        Action::Hold | Action::None => {}
    }

    if p.entry < ctx.bands.min || p.entry > ctx.bands.max {
        return Err(ValidationError::PriceOutOfBand);
    }

    let atr = ctx.atr.max(1e-9);
    if let Some(stop) = p.stop {
        if (p.entry - stop).abs() > MAX_STOP_ATRS * atr {
            return Err(ValidationError::DistanceTooFar);
        }
    }
    if let Some(target) = p.target {
        if (target - p.entry).abs() > MAX_TARGET_ATRS * atr {
            return Err(ValidationError::DistanceTooFar);
        }
    }

    // The model may size the trade; if it does, its number is what gets checked.
    if let Some(qty) = p.qty {
        if !qty.is_finite() || qty <= 0.0 {
            return Err(ValidationError::BadQuantity);
        }
        if p.action == Action::Sell && qty > ctx.held_qty + 1e-9 {
            return Err(ValidationError::TooManyUnits);
        }
    }
    let units = p.qty.unwrap_or(ctx.qty);
    if p.action == Action::Buy && units * p.entry > ctx.available_cash + 1e-9 {
        return Err(ValidationError::InsufficientCash);
    }

    // Size limits, applied to both directions: a sale is a transaction too.
    let amount = units * p.entry;
    ctx.limits.check(amount)?;
    if p.action == Action::Buy {
        ctx.limits
            .check_instrument(ctx.pot, ctx.instrument_exposure, amount)?;
        if let Some(target) = p.target {
            let achieved = (target - p.entry) / p.entry * 100.0;
            ctx.limits.check_profit(achieved)?;
        }
    }

    let until = DateTime::parse_from_rfc3339(&p.valid_until)
        .map_err(|_| ValidationError::BadValidity)?
        .with_timezone(&Utc);
    if until <= ctx.now {
        return Err(ValidationError::BadValidity);
    }

    // Cite what you were given. An instrument with no headlines has nothing to cite, so
    // demanding one would make any decision about it impossible — which is not the intent.
    if !ctx.sent_headline_ids.is_empty() && p.cites.is_empty() {
        return Err(ValidationError::MissingCitation);
    }
    if p.cites.iter().any(|id| !ctx.sent_headline_ids.contains(id)) {
        return Err(ValidationError::UnknownHeadlineId);
    }

    Ok(())
}
