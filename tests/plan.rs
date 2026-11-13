use chrono::{TimeZone, Utc};
use stock_predictor::llm::plan::{self, Answer, Cash, Move, Plan, Scope};
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
    let cash = Cash {
        stock: 500.0,
        fund: 0.0,
        trade: 0.0,
    };
    plan::validate(
        &parsed(),
        &book(),
        cash,
        &at(1_000_000.0),
        "INR",
        now(),
        &[],
        Scope::Stock,
    )
    .unwrap();
}

#[test]
fn a_plan_that_buys_more_than_it_frees_is_refused() {
    let p = Plan {
        moves: vec![
            Move {
                action: Action::Sell,
                symbol: "INFY.NS".into(),
                qty: Some(1.0),
                price: Some(100.0),
                amount: None,
                stop: None,
                target: None,
                why: "token sale".into(),
            valid_until: None,
            },
            Move {
                action: Action::Buy,
                symbol: "HDFCBANK.NS".into(),
                qty: Some(100.0),
                price: Some(100.0),
                amount: None,
                stop: Some(90.0),
                target: Some(120.0),
                why: "big".into(),
            valid_until: None,
            },
        ],
        rationale: "unbalanced".into(),
    };
    let cash = Cash {
        stock: 500.0,
        fund: 0.0,
        trade: 0.0,
    };
    let e = plan::validate(
        &p,
        &book(),
        cash,
        &at(1_000_000.0),
        "INR",
        now(),
        &[],
        Scope::Stock,
    )
    .unwrap_err();
    assert!(e.to_string().contains("does not balance"), "{e}");
}

#[test]
fn a_plan_selling_what_is_not_held_is_refused() {
    let p = Plan {
        moves: vec![Move {
            action: Action::Sell,
            symbol: "TCS.NS".into(),
            qty: Some(1.0),
            price: Some(100.0),
            amount: None,
            stop: None,
            target: None,
            why: "not mine".into(),
            valid_until: None,
        }],
        rationale: String::new(),
    };
    let cash = Cash {
        stock: 10_000.0,
        fund: 0.0,
        trade: 0.0,
    };
    let e = plan::validate(
        &p,
        &book(),
        cash,
        &at(1_000_000.0),
        "INR",
        now(),
        &[],
        Scope::Stock,
    )
    .unwrap_err();
    assert!(e.to_string().contains("not held"), "{e}");
}

#[test]
fn selling_more_than_is_held_is_refused() {
    let p = Plan {
        moves: vec![Move {
            action: Action::Sell,
            symbol: "INFY.NS".into(),
            qty: Some(25.0),
            price: Some(1146.30),
            amount: None,
            stop: None,
            target: None,
            why: "too much".into(),
            valid_until: None,
        }],
        rationale: String::new(),
    };
    let e = plan::validate(
        &p,
        &book(),
        Cash::default(),
        &at(1_000_000.0),
        "INR",
        now(),
        &[],
        Scope::Stock,
    )
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
                qty: Some(10.0),
                price: Some(1146.30),
                amount: None,
                stop: None,
                target: None,
                why: "a".into(),
            valid_until: None,
            },
            Move {
                action: Action::Buy,
                symbol: "INFY.NS".into(),
                qty: Some(10.0),
                price: Some(1146.30),
                amount: None,
                stop: Some(1000.0),
                target: Some(1300.0),
                why: "b".into(),
            valid_until: None,
            },
        ],
        rationale: String::new(),
    };
    let e = plan::validate(
        &p,
        &book(),
        Cash::default(),
        &at(1_000_000.0),
        "INR",
        now(),
        &[],
        Scope::Stock,
    )
    .unwrap_err();
    assert!(e.to_string().contains("twice"), "{e}");
}

/// Order is a convention, not a legality: a plan that lists its buy first still executes sells
/// first, and a buy that the purse can cover needs no sale beside it.
#[test]
fn a_plan_may_list_its_moves_in_any_order() {
    let body = r#"{"moves":[
      {"action":"BUY","symbol":"HDFCBANK.NS","qty":1,"price":793.65,"stop":700,"target":980},
      {"action":"SELL","symbol":"INFY.NS","qty":1,"price":1146.30}
    ]}"#;
    let plan = match plan::parse(body).unwrap() {
        Answer::Plan(p) => p,
        Answer::Pass { .. } => panic!("expected a plan"),
    };
    assert_eq!(plan.moves[0].action, Action::Buy);

    // Affordable on cash alone: 793.65 bought, no sale needed.
    plan::validate(
        &Plan {
            moves: vec![plan.moves[0].clone()],
            rationale: String::new(),
        },
        &book(),
        Cash {
            stock: 5_000.0,
            fund: 0.0,
        trade: 0.0,
        },
        &at(1_000_000.0),
        "INR",
        now(),
        &[],
        Scope::Stock,
    )
    .unwrap();
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
    assert!(
        block.contains("INFY.NS | Infosys | 24.000 | 1042.30"),
        "{block}"
    );
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
                qty: Some(5.0),
                price: Some(387.35),
                amount: None,
                stop: None,
                target: None,
                why: "dead money".into(),
            valid_until: None,
            },
            Move {
                action: Action::Buy,
                symbol: "0P0000XUY3.BO".into(),
                qty: Some(37.0),
                price: Some(265.04),
                amount: None,
                stop: Some(240.0),
                target: Some(320.0),
                why: "park the proceeds in an index fund".into(),
            valid_until: None,
            },
        ],
        rationale: "consolidate".into(),
    };
    let cash = Cash {
        stock: 4_000.0,
        fund: 0.0,
        trade: 0.0,
    };
    let funds = vec!["0P0000XUY3.BO".to_string()];
    let e = plan::validate(
        &plan,
        &held,
        cash,
        &at(1_000_000.0),
        "INR",
        now(),
        &funds,
        Scope::Stock,
    )
    .unwrap_err();
    assert!(e.to_string().contains("is a mutual fund"), "{e}");
    assert!(e.to_string().contains("fund plan"), "{e}");
}

/// The fund plan: rupees, no stops, and only funds. A stock in a fund plan is refused, and so is
/// a fund plan that spends the stock purse.
#[test]
fn a_fund_plan_is_sized_in_rupees_and_only_touches_funds() {
    let mut held = book();
    held.push(HoldingLine {
        symbol: "120503".into(),
        name: "Nifty 50 index fund".into(),
        qty: 40.0,
        avg_cost: 110.0,
        ltp: Some(125.0),
        stop: None,
        target: None,
    });
    let funds = vec!["120503".to_string()];
    let plan = Plan {
        moves: vec![
            Move {
                action: Action::Sell,
                symbol: "120503".into(),
                qty: None,
                price: None,
                amount: Some(5_000.0),
                stop: None,
                target: None,
                why: "duplicates the large-cap holding".into(),
            valid_until: None,
            },
            Move {
                action: Action::Buy,
                symbol: "120503".into(),
                qty: None,
                price: None,
                amount: Some(5_000.0),
                stop: None,
                target: None,
                why: "re-cut the same money".into(),
            valid_until: None,
            },
        ],
        rationale: String::new(),
    };
    // Same symbol twice is refused, so keep it honest: a separate fund for the buy.
    let e = plan::validate(
        &plan,
        &held,
        Cash {
            stock: 0.0,
            fund: 5_000.0,
        trade: 0.0,
        },
        &at(1_000_000.0),
        "INR",
        now(),
        &funds,
        Scope::Fund,
    )
    .unwrap_err();
    assert!(e.to_string().contains("twice"), "{e}");

    let buy_only = Plan {
        moves: vec![Move {
            action: Action::Buy,
            symbol: "120503".into(),
            qty: None,
            price: None,
            amount: Some(5_000.0),
            stop: None,
            target: None,
            why: "fills the large-cap gap".into(),
            valid_until: None,
        }],
        rationale: String::new(),
    };
    plan::validate(
        &buy_only,
        &held,
        Cash {
            stock: 100_000.0,
            fund: 5_000.0,
        trade: 0.0,
        },
        &at(1_000_000.0),
        "INR",
        now(),
        &funds,
        Scope::Fund,
    )
    .unwrap();

    // Stock money does not count for a fund.
    let e = plan::validate(
        &buy_only,
        &held,
        Cash {
            stock: 100_000.0,
            fund: 0.0,
        trade: 0.0,
        },
        &at(1_000_000.0),
        "INR",
        now(),
        &funds,
        Scope::Fund,
    )
    .unwrap_err();
    assert!(e.to_string().contains("fund purse does not balance"), "{e}");

    // And a stock has no business in a fund plan.
    let stock_plan = Plan {
        moves: vec![Move {
            action: Action::Buy,
            symbol: "INFY.NS".into(),
            qty: Some(1.0),
            price: Some(100.0),
            amount: None,
            stop: Some(90.0),
            target: Some(120.0),
            why: "wrong book".into(),
            valid_until: None,
        }],
        rationale: String::new(),
    };
    let e = plan::validate(
        &stock_plan,
        &held,
        Cash {
            stock: 0.0,
            fund: 9_999.0,
        trade: 0.0,
        },
        &at(1_000_000.0),
        "INR",
        now(),
        &funds,
        Scope::Fund,
    )
    .unwrap_err();
    assert!(e.to_string().contains("may only move mutual funds"), "{e}");
}

/// A trade move must say when it goes stale, within three sessions, and may not be a fund.
#[test]
fn a_trade_move_needs_an_expiry_inside_three_sessions() {
    use stock_predictor::llm::plan::{Move, Plan, Scope, TRADE_MAX_SESSIONS};
    let now = chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 11, 11, 6, 0, 0).unwrap();
    let cash = Cash {
        stock: 0.0,
        fund: 0.0,
        trade: 50_000.0,
    };
    let move_with = |valid_until: Option<String>, symbol: &str| Move {
        action: Action::Buy,
        symbol: symbol.into(),
        qty: Some(100.0),
        price: Some(100.0),
        amount: None,
        stop: Some(95.0),
        target: Some(110.0),
        why: "momentum".into(),
        valid_until,
    };
    let plan = |m: Move| Plan {
        moves: vec![m],
        rationale: "one trade".into(),
    };
    let ok_until = (now + chrono::Duration::days(1)).to_rfc3339();

    let book: Vec<HoldingLine> = vec![];
    let check = |m: Move| {
        plan::validate(
            &plan(m),
            &book,
            cash,
            &at(100_000.0),
            "INR",
            now,
            &[],
            Scope::Trade,
        )
    };

    assert!(check(move_with(Some(ok_until.clone()), "IDEA.NS")).is_ok());
    // No expiry: an idea with no shelf life is not this book's trade.
    let err = check(move_with(None, "IDEA.NS")).unwrap_err().to_string();
    assert!(err.contains("expiry"), "{err}");
    // Too far out: the deadline is the close of the third *session*, not three calendar days.
    let far = (plan::session_deadline(now, TRADE_MAX_SESSIONS) + chrono::Duration::hours(1))
        .to_rfc3339();
    let err = check(move_with(Some(far), "IDEA.NS")).unwrap_err().to_string();
    assert!(err.contains("sessions"), "{err}");
    // Already past.
    let past = (now - chrono::Duration::hours(1)).to_rfc3339();
    let err = check(move_with(Some(past), "IDEA.NS")).unwrap_err().to_string();
    assert!(err.contains("already past"), "{err}");
}

/// A SELL is an exit, not an idea: demanding an expiry on it refused a good "take the profit now".
#[test]
fn a_friday_expiry_may_name_tuesday() {
    // Reported from a live trade plan: Friday 13 Nov 2026, after the close, the model named
    // Tuesday 17 Nov. That is the *second* session out, but three calendar days from Friday is
    // Monday, so the weekend was counted as two sessions that do not exist and the plan was
    // refused as "more than 3 sessions out".
    use stock_predictor::llm::plan::{Move, Plan, Scope};
    // Friday 13 Nov 2026, 19:00 IST — after the close.
    let now = chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 11, 13, 13, 30, 0).unwrap();
    let tue_close = chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 11, 17, 10, 0, 0)
        .unwrap()
        .to_rfc3339();
    let plan = Plan {
        moves: vec![Move {
            action: Action::Buy,
            symbol: "BONFIG.NS".into(),
            qty: Some(12.0),
            price: Some(794.55),
            amount: None,
            stop: Some(755.0),
            target: Some(880.0),
            why: "Q2FY27 results".into(),
            valid_until: Some(tue_close),
        }],
        rationale: "one clean entry".into(),
    };
    let r = plan::validate(
        &plan,
        &[],
        Cash {
            stock: 0.0,
            fund: 0.0,
            trade: 10_000.0,
        },
        &at(50_000.0),
        "INR",
        now,
        &[],
        Scope::Trade,
    );
    assert!(r.is_ok(), "Tuesday is the second session out: {r:?}");
}

#[test]
fn a_trade_sell_needs_no_expiry_but_a_buy_does() {
    let now = chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 11, 11, 6, 0, 0).unwrap();
    let cash = Cash {
        stock: 0.0,
        fund: 0.0,
        trade: 50_000.0,
    };
    let held: Vec<HoldingLine> = vec![HoldingLine {
        symbol: "RELIANCE.NS".into(),
        name: "Reliance".into(),
        qty: 5.0,
        avg_cost: 1200.0,
        ltp: Some(1339.70),
        stop: None,
        target: None,
    }];
    let sell = Move {
        action: Action::Sell,
        symbol: "RELIANCE.NS".into(),
        qty: Some(5.0),
        price: Some(1339.70),
        amount: None,
        stop: None,
        target: None,
        why: "unprotected winner".into(),
        valid_until: None,
    };
    let plan = Plan {
        moves: vec![sell],
        rationale: "take the profit".into(),
    };
    let ctx_ok = plan::validate(&plan, &held, cash, &at(100_000.0), "INR", now, &[], Scope::Trade);
    assert!(ctx_ok.is_ok(), "a sell needs no expiry: {ctx_ok:?}");
}
