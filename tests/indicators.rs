use stock_predicter::models::Bar;
use stock_predicter::rules::indicators::{self, BAND_MULT};

/// `n` bars cycling through `pattern`, so the mean of any full cycle is known exactly.
fn series(pattern: &[f64], n: usize) -> Vec<Bar> {
    (0..n)
        .map(|i| {
            let c = pattern[i % pattern.len()];
            Bar {
                ts: format!("2026-01-{:02}T00:00:00Z", (i % 28) + 1),
                open: c,
                high: c + 0.5,
                low: c - 0.5,
                close: c,
                volume: 1000,
            }
        })
        .collect()
}

#[test]
fn computes_sma_on_known_series() {
    let i = indicators::compute(&series(&[1.0, 2.0, 3.0, 4.0, 5.0], 250)).unwrap();
    assert!((i.sma50 - 3.0).abs() < 1e-9);
    assert!((i.sma200 - 3.0).abs() < 1e-9);
}

#[test]
fn too_few_bars_is_an_error_not_nan() {
    let err = indicators::compute(&series(&[10.0, 11.0], 2)).unwrap_err();
    assert!(matches!(
        err,
        indicators::IndicatorError::InsufficientBars(2)
    ));
}

#[test]
fn rising_series_is_maximally_overbought() {
    let bars: Vec<Bar> = (0..250)
        .map(|i| Bar {
            ts: "2026-01-01T00:00:00Z".into(),
            open: i as f64,
            high: i as f64 + 1.0,
            low: i as f64 - 1.0,
            close: i as f64,
            volume: 1,
        })
        .collect();
    let i = indicators::compute(&bars).unwrap();
    assert!(i.rsi14 > 99.0, "rsi was {}", i.rsi14);
    assert!(i.atr14 > 0.0);
}

#[test]
fn bands_bracket_last_close_by_atr_multiple() {
    let b = indicators::atr_bands(100.0, 2.0);
    assert!(b.min < 100.0 && b.max > 100.0);
    assert!((b.max - b.min - 4.0 * BAND_MULT).abs() < 1e-9);
}
