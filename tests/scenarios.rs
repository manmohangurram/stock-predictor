//! Table-driven check of the prompt contract: what DeepSeek returns, and what the system does
//! with it. Every case is dummy data — no network, no key, no cost.

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use serde_json::json;
use stock_predicter::config::Config;
use stock_predicter::db;
use stock_predicter::llm::deepseek::{FixtureTransport, HttpReply};
use stock_predicter::llm::orchestrate::{self, LlmDeps, Outcome, RunInput};
use stock_predicter::llm::validate::ValidationError;
use stock_predicter::market::MarketSource;
use stock_predicter::models::{Action, AssetClass, Bar, Candidate, Headline, Instrument};
use stock_predicter::rules::indicators::Indicators;

struct FakeMarket {
    bars: Vec<Bar>,
}

impl MarketSource for FakeMarket {
    fn bars<'a>(&'a self, _s: &'a str) -> Pin<Box<dyn Future<Output = Result<Vec<Bar>>> + Send + 'a>> {
        let b = self.bars.clone();
        Box::pin(async move { Ok(b) })
    }
    fn intraday<'a>(
        &'a self,
        _s: &'a str,
        _i: &'a str,
        _r: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Bar>>> + Send + 'a>> {
        let b = self.bars.clone();
        Box::pin(async move { Ok(b) })
    }
    fn news<'a>(
        &'a self,
        _s: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Headline>>> + Send + 'a>> {
        Box::pin(async move { Ok(vec![]) })
    }
}

fn instrument(class: AssetClass) -> Instrument {
    Instrument {
        id: 1,
        symbol: "INFY.NS".into(),
        class,
        name: "Infosys".into(),
        currency: "INR".into(),
        active: true,
    }
}

fn indicators() -> Indicators {
    Indicators {
        sma50: 1084.66,
        sma200: 1222.06,
        rsi14: 64.7,
        atr14: 10.0,
        high52: 1728.0,
        low52: 980.4,
    }
}

fn candidate(action: Action) -> Candidate {
    Candidate {
        instrument_id: 1,
        action,
        reasons: vec!["close 1163.00 below recorded stop 1200.00".into()],
        entry: 1163.0,
        stop: Some(1104.85),
        target: Some(1279.3),
        qty: 10.0,
        timing_note: AssetClass::Stock.timing_note().into(),
        urgent: action == Action::Sell,
    }
}

fn headlines() -> Vec<Headline> {
    vec![Headline {
        id: 0,
        title: "Infosys guidance raised".into(),
        publisher: "Reuters".into(),
        published_at: "2026-10-25T00:00:00Z".into(),
        url: "https://example.com/1".into(),
    }]
}

fn cfg() -> Config {
    Config::from_map(
        &[("DEEPSEEK_API_KEY", "k"), ("LLM_DAILY_CAP", "50")]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    )
    .unwrap()
}

fn chat(content: serde_json::Value) -> HttpReply {
    HttpReply {
        status: 200,
        body: json!({
            "choices": [{"message": {"content": content.to_string()}}],
            "usage": {"prompt_tokens": 900, "completion_tokens": 60}
        })
        .to_string(),
    }
}

fn chat_raw(content: &str) -> HttpReply {
    HttpReply {
        status: 200,
        body: json!({"choices": [{"message": {"content": content}}], "usage": {}}).to_string(),
    }
}

fn decide_json(action: &str, entry: f64, stop: serde_json::Value, target: serde_json::Value) -> serde_json::Value {
    json!({
        "action": "decide",
        "signal": {
            "action": action,
            "entry": entry,
            "stop": stop,
            "target": target,
            "valid_until": "2030-01-01T00:00:00Z",
            "confidence": "medium",
            "rationale": "guidance raised and the stop is intact",
            "cites": [1]
        }
    })
}

async fn run_case(
    reply: HttpReply,
    held: f64,
    class: AssetClass,
) -> (Outcome, FixtureTransport) {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    let market = FakeMarket { bars: vec![] };
    let t = FixtureTransport::new(vec![reply]);
    let cfg = cfg();
    let deps = LlmDeps {
        pool: &pool,
        cfg: &cfg,
        transport: &t,
        market: &market,
    };
    let i = instrument(class);
    let x = indicators();
    let c = candidate(if held > 0.0 { Action::Sell } else { Action::Buy });
    let hs = headlines();
    let d = orchestrate::run(
        &deps,
        RunInput {
            instrument: &i,
            ind: &x,
            candidate: &c,
            available_cash: 20_000.0,
            held_qty: held,
            headlines: &hs,
            price_window: "27 09:30 1163.00 1164.10 1162.50 1163.40 12500",
            daily_summary: "2026-10-24 1150.20, 2026-10-27 1163.00",
            positions: "INFY.NS qty 10.000 at cost 1163.00, stop 1200.00",
            recent_trades: "none in the last 7 days",
            last_decision: "none",
            now: chrono::Utc::now(),
        },
    )
    .await
    .unwrap();
    (d.outcome, t)
}

/// The whole matrix of responses the model can return, and what must happen to each.
#[tokio::test]
async fn every_response_shape_lands_where_it_should() {
    struct Case {
        name: &'static str,
        reply: HttpReply,
        held: f64,
        class: AssetClass,
        expect: fn(&Outcome) -> bool,
    }

    let cases = vec![
        Case {
            name: "valid BUY with both exits is accepted",
            reply: chat(decide_json("BUY", 1163.0, json!(1150.0), json!(1200.0))),
            held: 0.0,
            class: AssetClass::Stock,
            expect: |o| matches!(o, Outcome::Llm(_)),
        },
        Case {
            name: "BUY without a take-profit is rejected (MissingExit)",
            reply: chat(decide_json("BUY", 1163.0, json!(1150.0), json!(null))),
            held: 0.0,
            class: AssetClass::Stock,
            expect: |o| matches!(o, Outcome::Rejected { error: ValidationError::MissingExit, .. }),
        },
        Case {
            name: "BUY without a stop is rejected (MissingExit)",
            reply: chat(decide_json("BUY", 1163.0, json!(null), json!(1200.0))),
            held: 0.0,
            class: AssetClass::Stock,
            expect: |o| matches!(o, Outcome::Rejected { error: ValidationError::MissingExit, .. }),
        },
        Case {
            name: "BUY with stop above entry is rejected",
            reply: chat(decide_json("BUY", 1163.0, json!(1170.0), json!(1200.0))),
            held: 0.0,
            class: AssetClass::Stock,
            expect: |o| matches!(o, Outcome::Rejected { error: ValidationError::StopOnWrongSide, .. }),
        },
        Case {
            name: "entry far outside the ATR band is rejected",
            reply: chat(decide_json("BUY", 1400.0, json!(1300.0), json!(1500.0))),
            held: 0.0,
            class: AssetClass::Stock,
            expect: |o| matches!(o, Outcome::Rejected { error: ValidationError::PriceOutOfBand, .. }),
        },
        Case {
            name: "a fund is allowed to omit exits",
            reply: chat(decide_json("BUY", 1163.0, json!(null), json!(null))),
            held: 0.0,
            class: AssetClass::MutualFund,
            expect: |o| matches!(o, Outcome::Llm(_)),
        },
        Case {
            name: "SELL of something not held is rejected",
            reply: chat(decide_json("SELL", 1163.0, json!(1200.0), json!(null))),
            held: 0.0,
            class: AssetClass::Stock,
            expect: |o| matches!(o, Outcome::Rejected { error: ValidationError::NotHeld, .. }),
        },
        Case {
            name: "a pass is a pass",
            reply: chat(json!({"action": "pass", "why": "no new information"})),
            held: 0.0,
            class: AssetClass::Stock,
            expect: |o| matches!(o, Outcome::Pass { .. }),
        },
        Case {
            name: "prose around the JSON is a parse failure, not a crash",
            reply: chat_raw("Sure! Here is my answer: {\"action\":\"pass\"}"),
            held: 0.0,
            class: AssetClass::Stock,
            expect: |o| matches!(o, Outcome::Rules(_)),
        },
        Case {
            name: "truncated JSON falls back to the rules",
            reply: chat_raw("{\"action\":\"decide\",\"signal\":{\"act"),
            held: 0.0,
            class: AssetClass::Stock,
            expect: |o| matches!(o, Outcome::Rules(_)),
        },
        Case {
            name: "a string where a number belongs falls back to the rules",
            reply: chat_raw(
                "{\"action\":\"decide\",\"signal\":{\"action\":\"BUY\",\"entry\":\"1163\",\"stop\":1150,\"target\":1200,\"valid_until\":\"2030-01-01T00:00:00Z\",\"confidence\":\"high\",\"rationale\":\"x\",\"cites\":[1]}}",
            ),
            held: 0.0,
            class: AssetClass::Stock,
            expect: |o| matches!(o, Outcome::Rules(_)),
        },
        Case {
            name: "an unknown action falls back to the rules",
            reply: chat(json!({"action": "buy_it_all"})),
            held: 0.0,
            class: AssetClass::Stock,
            expect: |o| matches!(o, Outcome::Rules(_)),
        },
        Case {
            name: "a citation to a headline that was never sent is rejected",
            reply: chat(json!({"action":"decide","signal":{
                "action":"BUY","entry":1163.0,"stop":1150.0,"target":1200.0,
                "valid_until":"2030-01-01T00:00:00Z","confidence":"high",
                "rationale":"x","cites":[99]}})),
            held: 0.0,
            class: AssetClass::Stock,
            expect: |o| matches!(o, Outcome::Rejected { error: ValidationError::UnknownHeadlineId, .. }),
        },
        Case {
            name: "no citation at all is rejected",
            reply: chat(json!({"action":"decide","signal":{
                "action":"BUY","entry":1163.0,"stop":1150.0,"target":1200.0,
                "valid_until":"2030-01-01T00:00:00Z","confidence":"high",
                "rationale":"x","cites":[]}})),
            held: 0.0,
            class: AssetClass::Stock,
            expect: |o| matches!(o, Outcome::Rejected { error: ValidationError::MissingCitation, .. }),
        },
        Case {
            name: "an expired valid_until is rejected",
            reply: chat(json!({"action":"decide","signal":{
                "action":"BUY","entry":1163.0,"stop":1150.0,"target":1200.0,
                "valid_until":"2020-01-01T00:00:00Z","confidence":"high",
                "rationale":"x","cites":[1]}})),
            held: 0.0,
            class: AssetClass::Stock,
            expect: |o| matches!(o, Outcome::Rejected { error: ValidationError::BadValidity, .. }),
        },
        Case {
            name: "extra fields the prompt never asked for are ignored",
            reply: chat(json!({"action":"decide","signal":{
                "action":"BUY","entry":1163.0,"stop":1150.0,"target":1200.0,
                "valid_until":"2030-01-01T00:00:00Z","confidence":"high",
                "rationale":"x","cites":[1],"qty":500,"leverage":10}})),
            held: 0.0,
            class: AssetClass::Stock,
            expect: |o| matches!(o, Outcome::Llm(_)),
        },
    ];

    for case in cases {
        let (outcome, t) = run_case(case.reply, case.held, case.class).await;
        assert!(
            (case.expect)(&outcome),
            "scenario failed: {}\n  got: {outcome:?}",
            case.name
        );
        assert_eq!(t.calls(), 1, "{}: one round trip expected", case.name);
    }
}

/// Escalation shapes: the model asks for article bodies, for price history, for both, or twice.
#[tokio::test]
async fn escalation_shapes_are_bounded() {
    // Articles only.
    let (outcome, t) = escalate(vec![
        chat(json!({"action":"need_more","headlines":[{"id":1,"why":"guidance detail"}]})),
        HttpReply { status: 200, body: json!({"title":"t","content":"Guidance raised"}).to_string() },
        chat(decide_json("BUY", 1163.0, json!(1150.0), json!(1200.0))),
    ])
    .await;
    assert!(matches!(outcome, Outcome::Llm(_)), "{outcome:?}");
    assert_eq!(t.calls(), 3);

    // Price history only: no article fetch, still one escalation.
    let (outcome, t) = escalate(vec![
        chat(json!({"action":"need_more","price_history":true})),
        chat(decide_json("BUY", 1163.0, json!(1150.0), json!(1200.0))),
    ])
    .await;
    assert!(matches!(outcome, Outcome::Llm(_)), "{outcome:?}");
    assert_eq!(t.calls(), 2, "history comes from the market, not a fetch call");

    // Both at once.
    let (outcome, t) = escalate(vec![
        chat(json!({"action":"need_more","headlines":[{"id":1,"why":"x"}],"price_history":true})),
        HttpReply { status: 200, body: json!({"title":"t","content":"body"}).to_string() },
        chat(decide_json("BUY", 1163.0, json!(1150.0), json!(1200.0))),
    ])
    .await;
    assert!(matches!(outcome, Outcome::Llm(_)), "{outcome:?}");
    assert_eq!(t.calls(), 3);

    // Asking twice is a pass — one escalation only.
    let (outcome, _t) = escalate(vec![
        chat(json!({"action":"need_more","price_history":true})),
        chat(json!({"action":"need_more","price_history":true})),
    ])
    .await;
    assert!(matches!(outcome, Outcome::Pass { .. }), "{outcome:?}");

    // Asking for something that does not exist is a pass, not a crash.
    let (outcome, _t) = escalate(vec![chat(json!({"action":"need_more"}))]).await;
    assert!(matches!(outcome, Outcome::Rules(_)), "{outcome:?}");
}

async fn escalate(replies: Vec<HttpReply>) -> (Outcome, FixtureTransport) {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    let market = FakeMarket {
        bars: vec![Bar {
            ts: "2026-10-21T05:00:00Z".into(),
            open: 1140.0,
            high: 1145.0,
            low: 1138.0,
            close: 1142.0,
            volume: 9000,
        }],
    };
    let t = FixtureTransport::new(replies);
    let cfg = cfg();
    let deps = LlmDeps { pool: &pool, cfg: &cfg, transport: &t, market: &market };
    let i = instrument(AssetClass::Stock);
    let x = indicators();
    let c = candidate(Action::Buy);
    let hs = headlines();
    let d = orchestrate::run(
        &deps,
        RunInput {
            instrument: &i,
            ind: &x,
            candidate: &c,
            available_cash: 20_000.0,
            held_qty: 0.0,
            headlines: &hs,
            price_window: "27 09:30 1163.00 1164.10 1162.50 1163.40 12500",
            daily_summary: "2026-10-27 1163.00",
            positions: "not held",
            recent_trades: "none",
            last_decision: "none",
            now: chrono::Utc::now(),
        },
    )
    .await
    .unwrap();
    (d.outcome, t)
}
