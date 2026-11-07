use serde::{Deserialize, Serialize};

use crate::llm::validate::ValidationError;
use crate::models::Mode;

/// Risk limits for one pot. Percentages are of that pot; amounts are in the account currency.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Limits {
    pub min_profit_pct: f64,
    pub max_profit_pct: f64,
    pub min_trade_amount: f64,
    pub max_trade_amount: f64,
    /// Cap on total exposure to a single instrument, as a percentage of the pot. A percentage
    /// rather than a fixed amount so the cap keeps its meaning when the budget changes.
    pub max_instrument_pct: f64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            min_profit_pct: 5.0,
            max_profit_pct: 50.0,
            min_trade_amount: 500.0,
            max_trade_amount: 50_000.0,
            max_instrument_pct: 25.0,
        }
    }
}

impl Limits {
    pub fn key_prefix(mode: Mode) -> String {
        format!("limits_{}", mode.as_str())
    }

    pub fn field(&self, name: &str) -> Option<f64> {
        match name {
            "min_profit_pct" => Some(self.min_profit_pct),
            "max_profit_pct" => Some(self.max_profit_pct),
            "min_trade_amount" => Some(self.min_trade_amount),
            "max_trade_amount" => Some(self.max_trade_amount),
            "max_instrument_pct" => Some(self.max_instrument_pct),
            _ => None,
        }
    }

    pub fn with(mut self, name: &str, value: f64) -> Option<Self> {
        match name {
            "min_profit_pct" => self.min_profit_pct = value,
            "max_profit_pct" => self.max_profit_pct = value,
            "min_trade_amount" => self.min_trade_amount = value,
            "max_trade_amount" => self.max_trade_amount = value,
            "max_instrument_pct" => self.max_instrument_pct = value,
            _ => return None,
        }
        Some(self)
    }

    /// The largest a single fill may be, also bounded by the room left on that instrument.
    pub fn max_fill(&self, pot: f64, spent_on_instrument: f64) -> f64 {
        let room = (pot * self.max_instrument_pct / 100.0 - spent_on_instrument).max(0.0);
        self.max_trade_amount.min(room)
    }

    pub fn check(&self, amount: f64) -> Result<(), ValidationError> {
        if amount < self.min_trade_amount {
            return Err(ValidationError::TradeTooSmall {
                min: self.min_trade_amount,
            });
        }
        if amount > self.max_trade_amount {
            return Err(ValidationError::TradeTooLarge {
                max: self.max_trade_amount,
            });
        }
        Ok(())
    }

    pub fn check_instrument(
        &self,
        pot: f64,
        already_spent: f64,
        amount: f64,
    ) -> Result<(), ValidationError> {
        let cap = pot * self.max_instrument_pct / 100.0;
        if already_spent + amount > cap + 1e-9 {
            return Err(ValidationError::InstrumentLimit {
                cap,
                already: already_spent,
            });
        }
        Ok(())
    }

    /// `floor` is this stock's own expectation (which defaults to `min_profit_pct`); the
    /// ceiling is always the global one, so a stock cannot buy itself a bigger allowance.
    pub fn check_profit(&self, pct: f64, floor: f64) -> Result<(), ValidationError> {
        let floor = floor.max(self.min_profit_pct);
        if pct + 1e-9 < floor {
            return Err(ValidationError::TargetBelowRequirement { required_pct: floor });
        }
        if pct > self.max_profit_pct + 1e-9 {
            return Err(ValidationError::TargetAboveLimit {
                max_pct: self.max_profit_pct,
            });
        }
        Ok(())
    }

    /// A stock's expectation must sit inside the global band, or it can never be satisfied.
    pub fn allows_target(&self, target: f64) -> bool {
        target >= self.min_profit_pct - 1e-9 && target <= self.max_profit_pct + 1e-9
    }
}
