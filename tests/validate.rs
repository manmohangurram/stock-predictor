use chrono::{Duration, Utc};
use stock_predictor::llm::parse::{Confidence, Proposal};
use stock_predictor::llm::validate::{check, ValidateCtx, ValidationError};
use stock_predictor::models::Action;
use stock_predictor::rules::indicators::atr_bands;
use stock_predictor::rules::limits::Limits;

fn proposal(action: Action, entry: f64, stop: Option<f64>, target: Option<f64>) -> Proposal {
    Proposal {
        action,
        qty: None,
        entry,
        stop,
        target,
        valid_until: (Utc::now() + Duration::hours(6)).to_rfc3339(),
        confidence: Confidence::Medium,
        rationale: "because".into(),
        cites: vec![1],
    }
}

fn ctx<'a>(sent: &'a [i64]) -> ValidateCtx<'a> {
    ValidateCtx {
        requires_high_confidence: false,
        bands: atr_bands(105.0, 2.0),
        atr: 2.0,
        available_cash: 10_000.0,
        qty: 95.0,
        held_qty: 0.0,
        sent_headline_ids: sent,
        requires_exit: true,
        limits: Limits {
            min_profit_pct: 0.0,
            max_profit_pct: 1000.0,
            min_trade_amount: 0.0,
            max_trade_amount: f64::MAX,
            max_instrument_pct: 100.0,
        },
        required_profit_pct: 5.0,
        instrument_exposure: 0.0,
        pot: 100_000.0,
        now: Utc::now(),
    }
}

#[test]
fn valid_proposal_passes() {
    let p = proposal(Action::Buy, 105.0, Some(101.0), Some(111.0));
    assert_eq!(check(&p, &ctx(&[1, 2])), Ok(()));
}

#[test]
fn stop_above_entry_on_buy_is_rejected() {
    let p = proposal(Action::Buy, 105.0, Some(108.0), Some(111.0));
    assert_eq!(check(&p, &ctx(&[1])), Err(ValidationError::StopOnWrongSide));
}

#[test]
fn entry_outside_atr_band_is_rejected() {
    let p = proposal(Action::Buy, 130.0, Some(120.0), Some(140.0));
    assert_eq!(check(&p, &ctx(&[1])), Err(ValidationError::PriceOutOfBand));
}

#[test]
fn implausibly_far_stop_is_rejected() {
    let p = proposal(Action::Buy, 105.0, Some(60.0), Some(111.0));
    assert_eq!(check(&p, &ctx(&[1])), Err(ValidationError::DistanceTooFar));
}

#[test]
fn qty_cost_above_available_cash_is_rejected() {
    let p = proposal(Action::Buy, 105.0, Some(101.0), Some(111.0));
    let mut c = ctx(&[1]);
    c.available_cash = 100.0; // 95 units * 105 = 9975
    assert_eq!(check(&p, &c), Err(ValidationError::InsufficientCash));
}

#[test]
fn sell_of_unheld_instrument_is_rejected() {
    let p = proposal(Action::Sell, 105.0, Some(110.0), None);
    assert_eq!(check(&p, &ctx(&[1])), Err(ValidationError::NotHeld));
}

#[test]
fn past_valid_until_is_rejected() {
    let mut p = proposal(Action::Buy, 105.0, Some(101.0), Some(111.0));
    p.valid_until = (Utc::now() - Duration::hours(1)).to_rfc3339();
    assert_eq!(check(&p, &ctx(&[1])), Err(ValidationError::BadValidity));
}

#[test]
fn cited_headline_id_never_sent_is_rejected() {
    let mut p = proposal(Action::Buy, 105.0, Some(101.0), Some(111.0));
    p.cites = vec![7];
    assert_eq!(
        check(&p, &ctx(&[1, 2])),
        Err(ValidationError::UnknownHeadlineId)
    );
}

#[test]
fn decision_without_any_citation_is_rejected() {
    let mut p = proposal(Action::Buy, 105.0, Some(101.0), Some(111.0));
    p.cites = vec![];
    assert_eq!(check(&p, &ctx(&[1])), Err(ValidationError::MissingCitation));
}

#[test]
fn parsed_shapes_round_trip_and_truncated_json_is_an_error() {
    use stock_predictor::llm::parse::verdict;
    let ok = r#"{"action":"decide","signal":{"action":"BUY","entry":105.0,"stop":101.0,"target":111.0,"valid_until":"2030-01-01T00:00:00Z","confidence":"high","rationale":"r","cites":[1]}}"#;
    assert!(matches!(
        verdict(ok).unwrap(),
        stock_predictor::llm::parse::LlmVerdict::Decide(_)
    ));
    assert!(verdict(r#"{"action":"decide","signal":{"action":"BU"#).is_err());
    assert!(
        verdict(r#"{"action":"need_more","requests":[{"headline_id":2,"why":"margin"}]}"#).is_ok()
    );
    assert!(verdict(r#"{"action":"pass","why":"nothing"}"#).is_ok());
}

#[test]
fn a_buy_without_a_take_profit_target_is_rejected() {
    let p = proposal(Action::Buy, 105.0, Some(101.0), None);
    assert_eq!(check(&p, &ctx(&[1])), Err(ValidationError::MissingExit));
}

#[test]
fn a_buy_without_a_stop_is_rejected() {
    let p = proposal(Action::Buy, 105.0, None, Some(111.0));
    assert_eq!(check(&p, &ctx(&[1])), Err(ValidationError::MissingExit));
}

#[test]
fn a_fund_is_not_required_to_carry_an_exit_price() {
    let p = proposal(Action::Buy, 105.0, None, None);
    let mut c = ctx(&[1]);
    c.requires_exit = false;
    assert_eq!(check(&p, &c), Ok(()));
}

#[test]
fn a_decision_about_an_instrument_with_no_headlines_needs_no_citation() {
    let mut p = proposal(Action::Buy, 105.0, Some(101.0), Some(111.0));
    p.cites = vec![];
    let mut c = ctx(&[]); // nothing was sent to the model
    c.requires_exit = true;
    assert_eq!(check(&p, &c), Ok(()));
}

#[test]
fn with_headlines_sent_a_citation_is_still_required() {
    let mut p = proposal(Action::Buy, 105.0, Some(101.0), Some(111.0));
    p.cites = vec![];
    assert_eq!(
        check(&p, &ctx(&[1, 2])),
        Err(ValidationError::MissingCitation)
    );
}

#[test]
fn the_model_may_size_a_buy_and_it_is_checked_against_cash() {
    let mut p = proposal(Action::Buy, 105.0, Some(101.0), Some(111.0));
    p.qty = Some(50.0); // 50 * 105 = 5250, inside 10,000
    assert_eq!(check(&p, &ctx(&[1])), Ok(()));

    p.qty = Some(500.0); // 52,500, beyond the pot
    assert_eq!(
        check(&p, &ctx(&[1])),
        Err(ValidationError::InsufficientCash)
    );
}

#[test]
fn the_model_may_size_a_sell_but_not_beyond_what_is_held() {
    let mut p = proposal(Action::Sell, 105.0, Some(110.0), None);
    let mut c = ctx(&[1]);
    c.held_qty = 17.0;

    p.qty = Some(17.0);
    assert_eq!(check(&p, &c), Ok(()));
    p.qty = Some(18.0);
    assert_eq!(check(&p, &c), Err(ValidationError::TooManyUnits));
    p.qty = Some(0.0);
    assert_eq!(check(&p, &c), Err(ValidationError::BadQuantity));
}

#[test]
fn no_quantity_from_the_model_leaves_the_rules_sizing_in_charge() {
    let mut p = proposal(Action::Buy, 105.0, Some(101.0), Some(111.0));
    p.qty = None;
    let mut c = ctx(&[1]);
    c.qty = 20.0; // rules sizing: 2100, fits
    assert_eq!(check(&p, &c), Ok(()));
    c.available_cash = 100.0; // rules sizing no longer fits
    assert_eq!(check(&p, &c), Err(ValidationError::InsufficientCash));
}

#[test]
fn a_take_profit_below_the_owner_expectation_is_rejected() {
    // The band is 105 ± 2 ATR, so the entry must sit near 105.
    let mut p = proposal(Action::Buy, 105.0, Some(101.0), Some(110.0)); // +4.8%
    let mut c = ctx(&[1]);
    c.limits.min_profit_pct = 15.0;
    assert_eq!(
        check(&p, &c),
        Err(ValidationError::TargetBelowRequirement { required_pct: 15.0 })
    );

    p.target = Some(121.0); // +15.2%, clears the bar
    assert_eq!(check(&p, &c), Ok(()));
}

#[test]
fn a_profit_expectation_only_governs_buys() {
    let p = proposal(Action::Sell, 105.0, Some(110.0), None);
    let mut c = ctx(&[1]);
    c.limits.min_profit_pct = 50.0;
    c.held_qty = 10.0;
    assert_eq!(
        check(&p, &c),
        Ok(()),
        "a sale is not judged on profit expectation"
    );
}

#[test]
fn a_nan_stop_is_rejected_rather_than_slipping_through() {
    // Clippy's neg_cmp_op_on_partial_ord warning pointed at a real hole: `!(stop < entry)`
    // is false for NaN, so an unparseable stop used to pass.
    let p = proposal(Action::Buy, 105.0, Some(f64::NAN), Some(111.0));
    assert_eq!(check(&p, &ctx(&[1])), Err(ValidationError::StopOnWrongSide));
}

#[test]
fn a_target_above_the_profit_ceiling_is_rejected() {
    // +15% on a 105 entry: inside the 10-ATR distance check (20 points), over a 10% ceiling.
    let p = proposal(Action::Buy, 105.0, Some(101.0), Some(120.75));
    let mut c = ctx(&[1]);
    c.limits.max_profit_pct = 10.0;
    c.limits.max_trade_amount = f64::MAX;
    assert_eq!(
        check(&p, &c),
        Err(ValidationError::TargetAboveLimit { max_pct: 10.0 })
    );
}

#[test]
fn trade_size_limits_apply_in_both_directions() {
    let mut c = ctx(&[1]);
    c.limits.min_trade_amount = 500.0;
    c.limits.max_trade_amount = 5_000.0;

    // 95 units at 105 = 9,975: too large.
    let too_big = proposal(Action::Buy, 105.0, Some(101.0), Some(111.0));
    assert_eq!(
        check(&too_big, &c),
        Err(ValidationError::TradeTooLarge { max: 5_000.0 })
    );

    // 1 unit at 105 = 105: too small.
    let mut too_small = proposal(Action::Buy, 105.0, Some(101.0), Some(111.0));
    too_small.qty = Some(1.0);
    assert_eq!(
        check(&too_small, &c),
        Err(ValidationError::TradeTooSmall { min: 500.0 })
    );

    // A sale is a transaction too.
    c.held_qty = 100.0;
    let mut sell = proposal(Action::Sell, 105.0, Some(110.0), None);
    sell.qty = Some(1.0);
    assert_eq!(
        check(&sell, &c),
        Err(ValidationError::TradeTooSmall { min: 500.0 })
    );
}

#[test]
fn the_per_instrument_cap_counts_what_is_already_held() {
    let mut c = ctx(&[1]);
    c.pot = 100_000.0;
    c.limits.max_instrument_pct = 10.0; // cap = 10,000
    c.instrument_exposure = 9_000.0; // 9,000 already in it
    c.limits.max_trade_amount = f64::MAX;

    let mut p = proposal(Action::Buy, 105.0, Some(101.0), Some(111.0));
    p.qty = Some(20.0); // 2,100 more: over the cap
    assert_eq!(
        check(&p, &c),
        Err(ValidationError::InstrumentLimit {
            cap: 10_000.0,
            already: 9_000.0
        })
    );

    p.qty = Some(9.0); // 945 more: inside
    assert_eq!(check(&p, &c), Ok(()));
}

#[test]
fn a_stock_expectation_raises_the_floor_but_cannot_beat_the_ceiling() {
    // Pot band: 5% minimum, 50% maximum.
    let p = proposal(Action::Buy, 105.0, Some(101.0), Some(115.5)); // +10%
    let mut c = ctx(&[1]);
    c.limits.min_profit_pct = 5.0;
    c.limits.max_profit_pct = 50.0;
    c.limits.max_trade_amount = f64::MAX;

    // Default expectation (the band minimum) is satisfied by +10%.
    c.required_profit_pct = 5.0;
    assert_eq!(check(&p, &c), Ok(()));

    // This stock wants 15%: the same +10% target now fails.
    c.required_profit_pct = 15.0;
    assert_eq!(
        check(&p, &c),
        Err(ValidationError::TargetBelowRequirement { required_pct: 15.0 })
    );

    // A stock asking for 80% would sit outside the pot's band, so the ceiling still wins.
    c.required_profit_pct = 80.0;
    let mut high = proposal(Action::Buy, 105.0, Some(101.0), Some(120.0)); // +14%
    high.target = Some(120.0);
    assert!(matches!(
        check(&high, &c),
        Err(ValidationError::TargetBelowRequirement { .. })
            | Err(ValidationError::TargetAboveLimit { .. })
    ));
}

#[test]
fn the_band_bounds_what_a_stock_may_ask_for() {
    let l = Limits {
        min_profit_pct: 5.0,
        max_profit_pct: 50.0,
        ..Limits::default()
    };
    assert!(!l.allows_target(4.9));
    assert!(l.allows_target(5.0));
    assert!(l.allows_target(50.0));
    assert!(!l.allows_target(50.1));
}

#[test]
fn a_mutual_fund_belongs_to_the_long_term_pot_only() {
    use stock_predictor::models::{AssetClass, Mode};
    assert!(AssetClass::Stock.pot_allowed(Mode::Trade));
    assert!(AssetClass::MutualFund.pot_allowed(Mode::Long));
    assert!(!AssetClass::MutualFund.pot_allowed(Mode::Trade));
}

#[test]
fn a_short_term_buy_must_be_high_confidence() {
    // The trade book's horizon forces an exit, so half a case is refused outright.
    let p = proposal(Action::Buy, 105.0, Some(101.0), Some(115.5));
    let mut c = ctx(&[1]);
    c.limits.min_profit_pct = 5.0;
    c.limits.max_profit_pct = 50.0;
    c.limits.max_trade_amount = f64::MAX;
    c.requires_high_confidence = true;

    let mut medium = p.clone();
    medium.confidence = Confidence::Medium;
    assert_eq!(check(&medium, &c), Err(ValidationError::ConfidenceTooLow));

    let mut high = p;
    high.confidence = Confidence::High;
    assert_eq!(check(&high, &c), Ok(()));

    // The long-term book is untouched by the rule.
    c.requires_high_confidence = false;
    let mut low = high.clone();
    low.confidence = Confidence::Low;
    assert_eq!(check(&low, &c), Ok(()));
}
