use chrono::{DateTime, Utc};
use thiserror::Error;

use super::parse::Proposal;
use crate::models::Action;
use crate::rules::indicators::Bands;

/// A stop may sit further from the close than the entry band; anything beyond this many
/// ATRs is a typo or a hallucination, not a stop.
pub const MAX_STOP_ATRS: f64 = 5.0;
pub const MAX_TARGET_ATRS: f64 = 10.0;

#[derive(Debug, Error, PartialEq)]
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
}

pub struct ValidateCtx<'a> {
    /// `entry` must sit inside this band; stop and target are deliberately outside it.
    pub bands: Bands,
    pub atr: f64,
    pub available_cash: f64,
    pub qty: f64,
    pub held_qty: f64,
    pub sent_headline_ids: &'a [i64],
    pub now: DateTime<Utc>,
}

pub fn check(p: &Proposal, ctx: &ValidateCtx<'_>) -> Result<(), ValidationError> {
    if !p.entry.is_finite() || p.entry <= 0.0 {
        return Err(ValidationError::PriceOutOfBand);
    }

    match p.action {
        Action::Buy => {
            if let Some(stop) = p.stop {
                if !(stop < p.entry) {
                    return Err(ValidationError::StopOnWrongSide);
                }
            }
        }
        Action::Sell => {
            if ctx.held_qty <= 0.0 {
                return Err(ValidationError::NotHeld);
            }
            if let Some(stop) = p.stop {
                if !(stop > p.entry) {
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

    if p.action == Action::Buy && ctx.qty * p.entry > ctx.available_cash + 1e-9 {
        return Err(ValidationError::InsufficientCash);
    }

    let until = DateTime::parse_from_rfc3339(&p.valid_until)
        .map_err(|_| ValidationError::BadValidity)?
        .with_timezone(&Utc);
    if until <= ctx.now {
        return Err(ValidationError::BadValidity);
    }

    if p.cites.is_empty() {
        return Err(ValidationError::MissingCitation);
    }
    if p.cites.iter().any(|id| !ctx.sent_headline_ids.contains(id)) {
        return Err(ValidationError::UnknownHeadlineId);
    }

    Ok(())
}
