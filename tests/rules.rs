use stock_predicter::models::{Action, AssetClass, Bar, Instrument, Position};
use stock_predicter::rules::evaluate::{candidate, CandidateInput, RuleConfig};
use stock_predicter::rules::indicators::Indicators;

fn instrument(class: AssetClass) -> Instrument {
    Instrument {
        id: 1,
        symbol: "INFY".into(),
        class,
        name: "Infosys".into(),
        currency: "INR".into(),
        active: true,
    }
}

fn bar(close: f64) -> Bar {
    Bar {
        ts: "2026-10-27T00:00:00Z".into(),
        open: close,
        high: close + 1.0,
        low: close - 1.0,
        close,
        volume: 1000,
    }
}

fn ind(sma50: f64, sma200: f64, rsi: f64, atr: f64) -> Indicators {
    Indicators {
        sma50,
        sma200,
        rsi14: rsi,
        atr14: atr,
        high52: 130.0,
        low52: 80.0,
    }
}

fn position(qty: f64, stop: Option<f64>) -> Position {
    Position {
        id: 1,
        instrument_id: 1,
        qty,
        avg_cost: 100.0,
        stop,
        target: Some(160.0),
        state: "open".into(),
    }
}

#[test]
fn uptrend_not_overbought_with_cash_emits_buy() {
    let i = instrument(AssetClass::Stock);
    let bars = vec![bar(105.0)];
    let x = ind(100.0, 90.0, 55.0, 2.0);
    let c = candidate(CandidateInput {
        instrument: &i,
        bars: &bars,
        ind: &x,
        position: None,
        available_cash: 10_000.0,
        cfg: RuleConfig::default(),
    })
    .unwrap();
    assert_eq!(c.action, Action::Buy);
    assert_eq!(c.qty, 95.0);
    assert!((c.stop.unwrap() - 101.0).abs() < 1e-9);
    assert!((c.target.unwrap() - 111.0).abs() < 1e-9);
    assert!(c.reasons.len() >= 2);
}

#[test]
fn overbought_rsi_blocks_buy() {
    let i = instrument(AssetClass::Stock);
    let bars = vec![bar(105.0)];
    let x = ind(100.0, 90.0, 78.0, 2.0);
    assert!(candidate(CandidateInput {
        instrument: &i,
        bars: &bars,
        ind: &x,
        position: None,
        available_cash: 10_000.0,
        cfg: RuleConfig::default(),
    })
    .is_none());
}

#[test]
fn held_position_below_stop_emits_urgent_sell() {
    let i = instrument(AssetClass::Stock);
    let bars = vec![bar(88.0)];
    let x = ind(100.0, 90.0, 55.0, 2.0);
    let p = position(5.0, Some(90.0));
    let c = candidate(CandidateInput {
        instrument: &i,
        bars: &bars,
        ind: &x,
        position: Some(&p),
        available_cash: 0.0,
        cfg: RuleConfig::default(),
    })
    .unwrap();
    assert_eq!(c.action, Action::Sell);
    assert!(c.urgent);
}

#[test]
fn buy_without_cash_emits_none() {
    let i = instrument(AssetClass::Stock);
    let bars = vec![bar(105.0)];
    let x = ind(100.0, 90.0, 55.0, 2.0);
    assert!(candidate(CandidateInput {
        instrument: &i,
        bars: &bars,
        ind: &x,
        position: None,
        available_cash: 0.0,
        cfg: RuleConfig::default(),
    })
    .is_none());
}

#[test]
fn sell_never_exceeds_held_quantity() {
    let i = instrument(AssetClass::Stock);
    let bars = vec![bar(88.0)];
    let x = ind(100.0, 90.0, 55.0, 2.0);
    let p = position(5.0, Some(90.0));
    let c = candidate(CandidateInput {
        instrument: &i,
        bars: &bars,
        ind: &x,
        position: Some(&p),
        available_cash: 0.0,
        cfg: RuleConfig::default(),
    })
    .unwrap();
    assert_eq!(c.qty, 5.0);
}

#[test]
fn mf_and_ipo_never_emit_an_intraday_price_target() {
    for (class, needle) in [
        (AssetClass::MutualFund, "NAV"),
        (AssetClass::Ipo, "allotment"),
    ] {
        let i = instrument(class);
        let bars = vec![bar(105.0)];
        let x = ind(100.0, 90.0, 55.0, 2.0);
        let c = candidate(CandidateInput {
            instrument: &i,
            bars: &bars,
            ind: &x,
            position: None,
            available_cash: 10_000.0,
            cfg: RuleConfig::default(),
        })
        .unwrap();
        assert!(c.stop.is_none(), "{class:?} must not carry a stop");
        assert!(c.target.is_none(), "{class:?} must not carry a target");
        assert!(c.timing_note.contains(needle), "note was {}", c.timing_note);
    }
}

#[test]
fn nan_price_produces_no_candidate() {
    let i = instrument(AssetClass::Stock);
    let bars = vec![bar(f64::NAN)];
    let x = ind(100.0, 90.0, 55.0, 2.0);
    assert!(candidate(CandidateInput {
        instrument: &i,
        bars: &bars,
        ind: &x,
        position: None,
        available_cash: 10_000.0,
        cfg: RuleConfig::default(),
    })
    .is_none());
}

#[test]
fn diagnose_agrees_with_the_candidate() {
    use stock_predicter::rules::evaluate::diagnose;

    let i = instrument(AssetClass::Stock);
    let bars = vec![bar(105.0)];
    let x = ind(100.0, 90.0, 55.0, 2.0);
    let input = CandidateInput {
        instrument: &i,
        bars: &bars,
        ind: &x,
        position: None,
        available_cash: 10_000.0,
        cfg: RuleConfig::default(),
    };
    assert!(candidate(CandidateInput { ..input }).is_some());
    assert!(
        diagnose(&input).iter().all(|c| c.passed),
        "all checks should pass"
    );

    // Same input, overbought: the candidate disappears and exactly one check explains it.
    let x2 = ind(100.0, 90.0, 78.0, 2.0);
    let input2 = CandidateInput { ind: &x2, ..input };
    assert!(candidate(CandidateInput { ..input2 }).is_none());
    let failed: Vec<&str> = diagnose(&input2)
        .iter()
        .filter(|c| !c.passed)
        .map(|c| c.name)
        .collect();
    assert_eq!(failed, vec!["not overbought"]);
}
