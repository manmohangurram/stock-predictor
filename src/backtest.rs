use crate::models::AssetClass;
use crate::models::Instrument;
use crate::models::{Action, Bar};
use crate::rules::evaluate::{self, CandidateInput, RuleConfig};
use crate::rules::indicators;

#[derive(Debug, Clone, PartialEq)]
pub struct BacktestReport {
    pub trades: usize,
    pub wins: usize,
    pub hit_rate: f64,
    pub max_drawdown: f64,
    pub blocked_by_budget: usize,
    pub final_equity: f64,
}

struct Open {
    qty: f64,
    entry: f64,
    stop: Option<f64>,
    target: Option<f64>,
}

/// Walks bars forward calling the same `evaluate` the live engine calls — no second copy of
/// the rules to rot. A candidate the budget refused is counted, not silently dropped.
pub fn run(bars: &[Bar], cfg: &RuleConfig, budget: f64) -> BacktestReport {
    let instrument = Instrument {
        id: 0,
        symbol: "BACKTEST".into(),
        class: AssetClass::Stock,
        name: "backtest".into(),
        currency: "INR".into(),
        active: true,
    };

    let mut cash = budget;
    let mut peak = budget;
    let mut max_drawdown = 0.0f64;
    let mut open: Option<Open> = None;
    let mut trades = 0usize;
    let mut wins = 0usize;
    let mut blocked = 0usize;

    for i in indicators::MIN_BARS..bars.len() {
        let window = &bars[..=i];
        let Ok(ind) = indicators::compute(window) else {
            continue;
        };
        let close = bars[i].close;

        if let Some(pos) = &open {
            let exit = match (pos.stop, pos.target) {
                (Some(s), _) if bars[i].low <= s => Some(s),
                (_, Some(t)) if bars[i].high >= t => Some(t),
                _ => None,
            };
            if let Some(exit) = exit {
                let proceeds = pos.qty * exit;
                cash += proceeds;
                trades += 1;
                if exit > pos.entry {
                    wins += 1;
                }
                open = None;
            }
        }

        if open.is_none() {
            let position_model = None;
            if let Some(c) = evaluate::candidate(CandidateInput {
                instrument: &instrument,
                bars: window,
                ind: &ind,
                position: position_model,
                available_cash: cash,
                cfg: *cfg,
            }) {
                if c.action == Action::Buy {
                    let cost = c.qty * c.entry;
                    if cost > cash + 1e-9 {
                        blocked += 1;
                    } else {
                        cash -= cost;
                        open = Some(Open {
                            qty: c.qty,
                            entry: c.entry,
                            stop: c.stop,
                            target: c.target,
                        });
                    }
                }
            }
        }

        let equity = cash + open.as_ref().map(|p| p.qty * close).unwrap_or(0.0);
        peak = peak.max(equity);
        max_drawdown = max_drawdown.max(peak - equity);
    }

    if let Some(pos) = open {
        cash += pos.qty * bars.last().map(|b| b.close).unwrap_or(pos.entry);
        trades += 1;
        if bars.last().map(|b| b.close).unwrap_or(0.0) > pos.entry {
            wins += 1;
        }
    }

    BacktestReport {
        trades,
        wins,
        hit_rate: if trades == 0 {
            0.0
        } else {
            wins as f64 / trades as f64
        },
        max_drawdown,
        blocked_by_budget: blocked,
        final_equity: cash,
    }
}
