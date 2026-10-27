use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetClass {
    Stock,
    MutualFund,
    Ipo,
}

impl AssetClass {
    pub fn as_str(self) -> &'static str {
        match self {
            AssetClass::Stock => "stock",
            AssetClass::MutualFund => "mutual_fund",
            AssetClass::Ipo => "ipo",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "stock" => Ok(AssetClass::Stock),
            "mutual_fund" => Ok(AssetClass::MutualFund),
            "ipo" => Ok(AssetClass::Ipo),
            other => Err(anyhow!("unknown asset class {other:?}")),
        }
    }

    /// The execution semantics that actually exist for this class. Carried into the UI and
    /// the prompt so an estimate is never presented as a firm price.
    pub fn timing_note(self) -> &'static str {
        match self {
            AssetClass::Stock => "limit order, intraday or next open",
            AssetClass::MutualFund => "units at NAV of the 15:00 cutoff; cash arrives T+1/T+2",
            AssetClass::Ipo => "cash locked until allotment; not sellable until listing",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bar {
    pub ts: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Headline {
    /// Assigned at assembly time (1..=n) so the model can cite a stable id.
    pub id: i64,
    pub title: String,
    pub publisher: String,
    pub published_at: String,
    pub url: String,
}
