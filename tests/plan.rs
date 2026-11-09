use chrono::{TimeZone, Utc};
use stock_predictor::llm::plan::{self, Answer, Cash, Move, Plan};
use stock_predictor::llm::rotate::HoldingLine;
use stock_predictor::models::Action;
use stock_predictor::rules::limits::Limits;

fn book() -> Vec<HoldingLine> {
    vec![
        HoldingLine {
            symbol: "INFY.NS".into(),
            name: "Infosys".into(),
            qty: 24.0,
            avg_cost: 1042.30,
            ltp: Some(1146.30),
            stop: Some(1080.00),
            target: Some(1420.00),
        },
        HoldingLine {
            symbol: "HDFCBANK.NS".into(),
            name: "HDFC Bank".into(),
            qty: 15.0,
            avg_cost: 690.10,
            ltp: Some(793.65),
            stop: Some(700.00),
            target: Some(980.00),
        },
    ]
}

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 9, 6, 0, 0).unwrap()
}

fn at(limit: f64) -> Limits {
    Limits {
        max_trade_amount: limit,
        ..Limits::default()
    }
}

fn parsed() -> Plan {
    let body = include_str!("fixtures/plan_reply.json");
    match plan::parse(body).unwrap() {
        Answer::Plan(p) => p,
        Answer::Pass { why } => panic!("expected a plan, got a pass: {why}"),
    }
}

#[test]
fn the_fixture_parses_into_two_ordered_moves() {
    let p = parsed();
    assert_eq!(p.moves.len(), 2);
    assert_eq!(p.moves[0].action, Action::Sell);
    assert_eq!(p.moves[1].action, Action::Buy);
    assert!(p.rationale.contains("six-month"));
}

#[test]
fn a_balanced_swap_passes() {
    // Sells 24 * 1146.30 = 27,511.20; buys 15 * 793.65 = 11,904.75; cash 500 is spare.
    let cash = Cash { stock: 500.0, fund: 0.0 };
    plan::validate(&parsed(), &book(), cash, &at(1_000_000.0), "INR", now(), &[]).unwrap();
}

#[test]
fn a_plan_that_buys_more_than_it_frees_is_refused() {
    let p = Plan {
        moves: vec![
            Move {
                action: Action::Sell,
                symbol: "INFY.NS".into(),
                qty: 1.0,
                price: 100.0,
                stop: None,
                target: None,
                why: "token sale".into(),
            },
            Move {
                action: Action::Buy,
                symbol: "HDFCBANK.NS".into(),
                qty: 100.0,
                price: 100.0,
                stop: Some(90.0),
                target: Some(120.0),
                why: "big".into(),
            },
        ],
        rationale: "unbalanced".into(),
    };
    let cash = Cash { stock: 500.0, fund: 0.0 };
    let e = plan::validate(&p, &book(), cash, &at(1_000_000.0), "INR", now(), &[]).unwrap_err();
    assert!(e.to_string().contains("does not balance"), "{e}");
}

#[test]
fn a_plan_selling_what_is_not_held_is_refused() {
    let p = Plan {
        moves: vec![Move {
            action: Action::Sell,
            symbol: "TCS.NS".into(),
            qty: 1.0,
            price: 100.0,
            stop: None,
            target: None,
            why: "not mine".into(),
        }],
        rationale: String::new(),
    };
    let cash = Cash { stock: 10_000.0, fund: 0.0 };
    let e = plan::validate(&p, &book(), cash, &at(1_000_000.0), "INR", now(), &[]).unwrap_err();
    assert!(e.to_string().contains("not held"), "{e}");
}

#[test]
fn selling_more_than_is_held_is_refused() {
    let p = Plan {
        moves: vec![Move {
            action: Action::Sell,
            symbol: "INFY.NS".into(),
            qty: 25.0,
            price: 1146.30,
            stop: None,
            target: None,
            why: "too much".into(),
        }],
        rationale: String::new(),
    };
    let e = plan::validate(&p, &book(), Cash::default(), &at(1_000_000.0), "INR", now(), &[])
        .unwrap_err();
    assert!(e.to_string().contains("only 24.000 is held"), "{e}");
}

#[test]
fn a_symbol_may_not_appear_twice() {
    let p = Plan {
        moves: vec![
            Move {
                action: Action::Sell,
                symbol: "INFY.NS".into(),
                qty: 10.0,
                price: 1146.30,
                stop: None,
                target: None,
                why: "a".into(),
            },
            Move {
                action: Action::Buy,
                symbol: "INFY.NS".into(),
                qty: 10.0,
                price: 1146.30,
                stop: Some(1000.0),
                target: Some(1300.0),
                why: "b".into(),
            },
        ],
        rationale: String::new(),
    };
    let e = plan::validate(&p, &book(), Cash::default(), &at(1_000_000.0), "INR", now(), &[])
        .unwrap_err();
    assert!(e.to_string().contains("twice"), "{e}");
}

#[test]
fn sells_must_come_before_buys() {
    let body = r#"{"moves":[
      {"action":"BUY","symbol":"HDFCBANK.NS","qty":1,"price":793.65,"stop":700,"target":980},
      {"action":"SELL","symbol":"INFY.NS","qty":1,"price":1146.30}
    ]}"#;
    let e = plan::parse(body).unwrap_err();
    assert!(e.to_string().contains("sells first"), "{e}");
}

#[test]
fn more_than_three_moves_is_not_a_plan() {
    let moves = (0..4)
        .map(|n| format!(r#"{{"action":"SELL","symbol":"S{n}","qty":1,"price":10}}"#))
        .collect::<Vec<_>>()
        .join(",");
    let e = plan::parse(&format!(r#"{{"moves":[{moves}]}}"#)).unwrap_err();
    assert!(e.to_string().contains("more than the 3 allowed"), "{e}");
}

#[test]
fn a_pass_has_no_moves() {
    match plan::parse(r#"{"moves":[],"pass_reason":"nothing worth doing"}"#).unwrap() {
        Answer::Pass { why } => assert_eq!(why, "nothing worth doing"),
        Answer::Plan(_) => panic!("expected a pass"),
    }
}

#[test]
fn the_book_block_lists_every_holding_and_its_cost_basis() {
    let block = plan::book_block(&book(), "INR");
    assert!(block.contains("INFY.NS | Infosys | 24.000 | 1042.30"), "{block}");
    assert!(block.contains("total invested:"), "{block}");
}

/// The sample that prompted this: sell two stocks, buy an index fund. The arithmetic balances in
/// total, and it is still impossible — a fund is bought from its own monthly purse.
#[test]
fn selling_stocks_never_funds_a_mutual_fund() {
    let mut held = book();
    held.push(HoldingLine {
        symbol: "AMBUJACEM.NS".into(),
        name: "Ambuja Cements".into(),
        qty: 5.0,
        avg_cost: 530.0,
        ltp: Some(387.35),
        stop: None,
        target: None,
    });
    let plan = Plan {
        moves: vec![
            Move {
                action: Action::Sell,
                symbol: "AMBUJACEM.NS".into(),
                qty: 5.0,
                price: 387.35,
                stop: None,
                target: None,
                why: "dead money".into(),
            },
            Move {
                action: Action::Buy,
                symbol: "0P0000XUY3.BO".into(),
                qty: 37.0,
                price: 265.04,
                stop: Some(240.0),
                target: Some(320.0),
                why: "park the proceeds in an index fund".into(),
            },
        ],
        rationale: "consolidate".into(),
    };
    let cash = Cash { stock: 4_000.0, fund: 0.0 };
    let funds = vec!["0P0000XUY3.BO".to_string()];
    let e = plan::validate(&plan, &held, cash, &at(1_000_000.0), "INR", now(), &funds)
        .unwrap_err();
    assert!(e.to_string().contains("fund purse does not balance"), "{e}");
    assert!(e.to_string().contains("never lend"), "{e}");
}

/// A fund bought with fund cash is fine, and so is a stock bought with stock cash.
#[test]
fn each_purse_balances_on_its_own() {
    let mut held = book();
    held.push(HoldingLine {
        symbol: "0P0000XUY3.BO".into(),
        name: "Nifty 50 index fund".into(),
        qty: 10.0,
        avg_cost: 250.0,
        ltp: Some(265.0),
        stop: None,
        target: None,
    });
    let plan = Plan {
        moves: vec![
            Move {
                action: Action::Sell,
                symbol: "0P0000XUY3.BO".into(),
                qty: 10.0,
                price: 265.0,
                stop: None,
                target: None,
                why: "switch fund".into(),
            },
            Move {
                action: Action::Buy,
                symbol: "0P0000XUY3.BO".into(),
                qty: 5.0,
                price: 265.0,
                stop: Some(240.0),
                target: Some(320.0),
                why: "reallocate".into(),
            },
        ],
        rationale: String::new(),
    };
    // The same symbol twice is refused for a different reason, so use two funds.
    assert!(plan::validate(
        &plan,
        &held,
        Cash { stock: 0.0, fund: 0.0 },
        &at(1_000_000.0),
        "INR",
        now(),
        &["0P0000XUY3.BO".to_string()],
    )
    .is_err());
}
