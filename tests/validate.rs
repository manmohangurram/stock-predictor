use chrono::{Duration, Utc};
use stock_predicter::llm::parse::{Confidence, Proposal};
use stock_predicter::llm::validate::{check, ValidateCtx, ValidationError};
use stock_predicter::models::Action;
use stock_predicter::rules::indicators::atr_bands;

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
        bands: atr_bands(105.0, 2.0),
        atr: 2.0,
        available_cash: 10_000.0,
        qty: 95.0,
        held_qty: 0.0,
        sent_headline_ids: sent,
        requires_exit: true,
        required_profit_pct: None,
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
    use stock_predicter::llm::parse::verdict;
    let ok = r#"{"action":"decide","signal":{"action":"BUY","entry":105.0,"stop":101.0,"target":111.0,"valid_until":"2030-01-01T00:00:00Z","confidence":"high","rationale":"r","cites":[1]}}"#;
    assert!(matches!(
        verdict(ok).unwrap(),
        stock_predicter::llm::parse::LlmVerdict::Decide(_)
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
    c.required_profit_pct = Some(15.0);
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
    c.required_profit_pct = Some(50.0);
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
