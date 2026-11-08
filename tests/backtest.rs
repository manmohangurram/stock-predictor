use stock_predictor::backtest;
use stock_predictor::models::Bar;
use stock_predictor::rules::evaluate::RuleConfig;

fn bars() -> Vec<Bar> {
    (0..400)
        .map(|i| {
            let c = 95.0 + (i % 10) as f64 * 0.5 + i as f64 * 0.02;
            Bar {
                ts: format!("2026-01-01T00:00:{}Z", i % 60),
                open: c,
                high: c + 0.8,
                low: c - 0.8,
                close: c,
                volume: 10_000,
            }
        })
        .collect()
}

#[test]
fn synthetic_uptrend_produces_trades_and_bounded_drawdown() {
    let r = backtest::run(&bars(), &RuleConfig::default(), 100_000.0);
    assert!(r.trades > 0, "{r:?}");
    assert!(r.max_drawdown <= 100_000.0);
    assert!((0.0..=1.0).contains(&r.hit_rate));
}

#[test]
fn a_budget_too_small_to_trade_counts_blocks_instead_of_dropping_them() {
    let r = backtest::run(&bars(), &RuleConfig::default(), 1.0);
    assert_eq!(r.trades, 0);
    assert_eq!(r.final_equity, 1.0);
}

#[test]
fn empty_or_short_series_is_a_no_op() {
    let r = backtest::run(&bars()[..10], &RuleConfig::default(), 1000.0);
    assert_eq!(r.trades, 0);
    assert_eq!(r.final_equity, 1000.0);
    assert_eq!(r.hit_rate, 0.0);
}
