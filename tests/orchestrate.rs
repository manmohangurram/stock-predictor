use serde_json::json;
use stock_predictor::db;
use stock_predictor::llm::deepseek::{FixtureTransport, HttpReply};
use stock_predictor::llm::orchestrate::{self, LlmDeps, Outcome, RunInput};
use stock_predictor::models::{Action, AssetClass, Candidate, Headline, Instrument};
use stock_predictor::rules::indicators::Indicators;
use stock_predictor::rules::runtime::Runtime;

/// Bars the escalation can ask for. Timestamps are fixed so the rendered history is stable.
#[derive(Default)]
struct FakeMarket {
    bars: Vec<stock_predictor::models::Bar>,
}

impl stock_predictor::market::MarketSource for FakeMarket {
    fn bars<'a>(
        &'a self,
        _s: &'a str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = anyhow::Result<Vec<stock_predictor::models::Bar>>>
                + Send
                + 'a,
        >,
    > {
        let b = self.bars.clone();
        Box::pin(async move { Ok(b) })
    }
    fn intraday<'a>(
        &'a self,
        _s: &'a str,
        _i: &'a str,
        _r: &'a str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = anyhow::Result<Vec<stock_predictor::models::Bar>>>
                + Send
                + 'a,
        >,
    > {
        let b = self.bars.clone();
        Box::pin(async move { Ok(b) })
    }
    fn news<'a>(
        &'a self,
        _s: &'a str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = anyhow::Result<Vec<stock_predictor::models::Headline>>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move { Ok(vec![]) })
    }
}

fn run_input<'a>(
    i: &'a Instrument,
    x: &'a Indicators,
    c: &'a Candidate,
    hs: &'a [Headline],
) -> RunInput<'a> {
    RunInput {
        short_context: "",
        mode: stock_predictor::models::Mode::Long,
        call_reason: None,
        instrument: i,
        ind: x,
        candidate: c,
        available_cash: 10_000.0,
        held_qty: 0.0,
        limits: Default::default(),
        required_profit_pct: 5.0,

        instrument_exposure: 0.0,

        pot: 100_000.0,
        window_hours: 6,
        headlines: hs,
        price_window: "27 09:30 100.00 101.00 99.00 100.50 1000",
        daily_summary: "2026-10-24 100.00, 2026-10-27 100.50",
        fundamentals: "unavailable",
        positions: "not held",
        recent_trades: "none in the last 7 days",
        last_decision: "none",
        now: chrono::Utc::now(),
    }
}

fn instrument() -> Instrument {
    Instrument {
        id: 1,
        symbol: "INFY".into(),
        class: AssetClass::Stock,
        name: "Infosys".into(),
        currency: "INR".into(),
        active: true,
    }
}

fn ind() -> Indicators {
    Indicators {
        sma50: 100.0,
        sma200: 90.0,
        rsi14: 55.0,
        atr14: 2.0,
        high52: 130.0,
        low52: 80.0,
    }
}

fn candidate() -> Candidate {
    Candidate {
        instrument_id: 1,
        action: Action::Buy,
        reasons: vec!["trend up".into()],
        entry: 105.0,
        stop: Some(101.0),
        target: Some(111.0),
        qty: 9.0,
        timing_note: AssetClass::Stock.timing_note().into(),
        urgent: false,
    }
}

fn headlines(n: usize) -> Vec<Headline> {
    (0..n)
        .map(|i| Headline {
            id: 0,
            title: format!("headline {i}"),
            publisher: "Reuters".into(),
            published_at: "2026-10-25T00:00:00Z".into(),
            url: format!("https://example.com/{i}"),
        })
        .collect()
}

fn chat(body_json: &str) -> HttpReply {
    HttpReply {
        status: 200,
        body: json!({
            "choices": [{"message": {"role": "assistant", "content": body_json}}],
            "usage": {"prompt_tokens": 100, "completion_tokens": 20}
        })
        .to_string(),
    }
}

fn fetch_body(size: usize) -> HttpReply {
    HttpReply {
        status: 200,
        body: json!({"title": "t", "content": "Z".repeat(size)}).to_string(),
    }
}

fn decide_json(entry: f64, stop: f64) -> String {
    json!({
        "action": "decide",
        "signal": {
            "action": "BUY",
            "entry": entry,
            "stop": stop,
            "target": entry + 6.0,
            "valid_until": "2030-01-01T00:00:00Z",
            "confidence": "medium",
            "rationale": "guidance raised",
            "cites": [1]
        }
    })
    .to_string()
}

fn need_more(ids: &[i64]) -> String {
    json!({
        "action": "need_more",
        "requests": ids.iter().map(|id| json!({"headline_id": id, "why": "margin"})).collect::<Vec<_>>()
    })
    .to_string()
}

fn runtime(cap: u32) -> Runtime {
    Runtime {
        daily_cap: cap,
        ..Runtime::default()
    }
}

async fn pool() -> (tempfile::TempDir, sqlx::SqlitePool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    let pool = db::connect_and_migrate(path.to_str().unwrap())
        .await
        .unwrap();
    (dir, pool)
}

async fn endpoints(pool: &sqlx::SqlitePool) -> Vec<String> {
    sqlx::query_as::<_, (String,)>("SELECT endpoint FROM api_calls ORDER BY id")
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.0)
        .collect()
}

async fn prompts(pool: &sqlx::SqlitePool) -> Vec<String> {
    sqlx::query_as::<_, (String,)>(
        "SELECT request_json FROM api_calls WHERE endpoint = 'chat' ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .map(|r| r.0)
    .collect()
}

#[tokio::test]
async fn need_more_triggers_exactly_one_fetch_round() {
    let (_d, pool) = pool().await;
    let hs = headlines(3);
    let t = FixtureTransport::new(vec![
        chat(&need_more(&[2])),
        fetch_body(500),
        chat(&decide_json(105.0, 101.0)),
    ]);
    let i = instrument();
    let x = ind();
    let c = candidate();

    let market = FakeMarket::default();
    let deps = LlmDeps {
        pool: &pool,
        runtime: runtime(10),
        transport: &t,
        market: &market,
    };
    let decision = orchestrate::run(&deps, run_input(&i, &x, &c, &hs))
        .await
        .unwrap();

    assert!(
        matches!(decision.outcome, Outcome::Llm(_)),
        "{:?}",
        decision.outcome
    );
    assert_eq!(endpoints(&pool).await, vec!["chat", "fetch", "chat"]);
}

#[tokio::test]
async fn second_need_more_becomes_pass() {
    let (_d, pool) = pool().await;
    let hs = headlines(3);
    let t = FixtureTransport::new(vec![
        chat(&need_more(&[2])),
        fetch_body(300),
        chat(&need_more(&[3])),
    ]);
    let i = instrument();
    let x = ind();
    let c = candidate();

    let market = FakeMarket::default();
    let deps = LlmDeps {
        pool: &pool,
        runtime: runtime(10),
        transport: &t,
        market: &market,
    };
    let d = orchestrate::run(&deps, run_input(&i, &x, &c, &hs))
        .await
        .unwrap();
    assert!(matches!(d.outcome, Outcome::Pass { .. }), "{:?}", d.outcome);
    assert_eq!(
        endpoints(&pool)
            .await
            .iter()
            .filter(|e| *e == "chat")
            .count(),
        2
    );
}

#[tokio::test]
async fn more_than_three_requests_are_truncated() {
    let (_d, pool) = pool().await;
    let hs = headlines(5);
    let t = FixtureTransport::new(vec![
        chat(&need_more(&[1, 2, 3, 4, 5])),
        fetch_body(100),
        fetch_body(100),
        fetch_body(100),
        chat(&decide_json(105.0, 101.0)),
    ]);
    let i = instrument();
    let x = ind();
    let c = candidate();

    let market = FakeMarket::default();
    let deps = LlmDeps {
        pool: &pool,
        runtime: runtime(10),
        transport: &t,
        market: &market,
    };
    let _ = orchestrate::run(&deps, run_input(&i, &x, &c, &hs))
        .await
        .unwrap();
    assert_eq!(
        endpoints(&pool)
            .await
            .iter()
            .filter(|e| *e == "fetch")
            .count(),
        3
    );
}

#[tokio::test]
async fn article_bodies_are_truncated_and_total_is_capped() {
    let (_d, pool) = pool().await;
    let hs = headlines(3);
    let t = FixtureTransport::new(vec![
        chat(&need_more(&[1, 2, 3])),
        fetch_body(9_000),
        fetch_body(9_000),
        fetch_body(9_000),
        chat(&decide_json(105.0, 101.0)),
    ]);
    let i = instrument();
    let x = ind();
    let c = candidate();

    let market = FakeMarket::default();
    let deps = LlmDeps {
        pool: &pool,
        runtime: runtime(10),
        transport: &t,
        market: &market,
    };
    let _ = orchestrate::run(&deps, run_input(&i, &x, &c, &hs))
        .await
        .unwrap();

    let p = prompts(&pool).await;
    let round2 = &p[1];
    // Count only long runs: short stray letters live in the JSON envelope, the article
    // text is the multi-thousand-character run.
    let mut runs: Vec<usize> = Vec::new();
    let mut cur = 0usize;
    for ch in round2.chars() {
        if ch == 'Z' {
            cur += 1;
        } else if cur > 0 {
            runs.push(cur);
            cur = 0;
        }
    }
    if cur > 0 {
        runs.push(cur);
    }
    let article_runs: Vec<usize> = runs.into_iter().filter(|n| *n >= 100).collect();
    assert!(
        article_runs.iter().all(|n| *n <= 4_000),
        "an article exceeded the per-article cap: {article_runs:?}"
    );
    let body_chars: usize = article_runs.iter().sum();
    assert!(body_chars <= 10_000, "fed {body_chars} chars");
    assert!(body_chars >= 9_000, "should still feed the budget it has");
}

#[tokio::test]
async fn daily_cap_exhausted_degrades_to_rules_without_a_call() {
    let (_d, pool) = pool().await;
    let hs = headlines(2);
    let t = FixtureTransport::new(vec![]);
    let i = instrument();
    let x = ind();
    let c = candidate();

    let market = FakeMarket::default();
    let deps = LlmDeps {
        pool: &pool,
        runtime: runtime(0),
        transport: &t,
        market: &market,
    };
    let d = orchestrate::run(&deps, run_input(&i, &x, &c, &hs))
        .await
        .unwrap();
    assert!(matches!(d.outcome, Outcome::Rules(_)));
    assert_eq!(t.calls(), 0);
    assert!(endpoints(&pool).await.is_empty());
}

#[tokio::test]
async fn invalid_proposal_degrades_to_rules_numbers() {
    let (_d, pool) = pool().await;
    let hs = headlines(2);
    // stop above entry for a BUY: the validator must refuse it.
    let t = FixtureTransport::new(vec![chat(&decide_json(105.0, 108.0))]);
    let i = instrument();
    let x = ind();
    let c = candidate();

    let market = FakeMarket::default();
    let deps = LlmDeps {
        pool: &pool,
        runtime: runtime(10),
        transport: &t,
        market: &market,
    };
    let d = orchestrate::run(&deps, run_input(&i, &x, &c, &hs))
        .await
        .unwrap();
    // The orchestrator reports the rejection; the engine decides the consequence.
    match d.outcome {
        Outcome::Rejected { error, .. } => assert_eq!(
            error,
            stock_predictor::llm::validate::ValidationError::StopOnWrongSide
        ),
        other => panic!("expected a rejection, got {other:?}"),
    }
}

#[tokio::test]
async fn unparseable_phase_one_degrades_to_rules() {
    let (_d, pool) = pool().await;
    let hs = headlines(2);
    let t = FixtureTransport::new(vec![chat("not json")]);
    let i = instrument();
    let x = ind();
    let c = candidate();

    let market = FakeMarket::default();
    let deps = LlmDeps {
        pool: &pool,
        runtime: runtime(10),
        transport: &t,
        market: &market,
    };
    let d = orchestrate::run(&deps, run_input(&i, &x, &c, &hs))
        .await
        .unwrap();
    assert!(matches!(d.outcome, Outcome::Rules(_)));
}
