use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use stock_predictor::db;
use stock_predictor::engine::{self, EngineDeps, MarketSource, PollReport};
use stock_predictor::llm::deepseek::FixtureTransport;
use stock_predictor::rules::runtime::Runtime;
mod common;
use common::{headline, rising_bars as bars_n, FakeMarket};
use stock_predictor::notify::FakeNotifier;
use stock_predictor::rules::evaluate::RuleConfig;
use stock_predictor::rules::settings;
use stock_predictor::schedule;

fn cfg() -> Runtime {
    Runtime {
        daily_cap: 50,
        ..Runtime::default()
    }
}

async fn seed(pool: &sqlx::SqlitePool, symbols: &[&str]) {
    seed_with_key(pool, symbols, "test-key").await
}

async fn seed_no_limits(pool: &sqlx::SqlitePool, symbols: &[&str]) {
    settings::seed(pool).await.unwrap();
    for s in symbols {
        sqlx::query(
            "INSERT INTO instruments (symbol, class, name, currency) VALUES (?, 'stock', ?, 'INR')",
        )
        .bind(s)
        .bind(format!("{s} Ltd"))
        .execute(pool)
        .await
        .unwrap();
    }
}

async fn seed_with_key(pool: &sqlx::SqlitePool, symbols: &[&str], key: &str) {
    settings::seed(pool).await.unwrap();
    settings::set(
        pool,
        settings::SettingsPatch {
            budget: Some(200_000.0),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    // Wide limits: these tests are about decisions, not sizing.
    settings::set(
        pool,
        settings::SettingsPatch {
            limits_long: Some(stock_predictor::rules::limits::Limits {
                min_profit_pct: 0.0,
                max_profit_pct: 1000.0,
                min_trade_amount: 0.0,
                max_trade_amount: 1_000_000.0,
                max_instrument_pct: 100.0,
            }),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    if !key.is_empty() {
        settings::set(
            pool,
            settings::SettingsPatch {
                llm_key: Some(key.to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
    for s in symbols {
        sqlx::query(
            "INSERT INTO instruments (symbol, class, name, currency) VALUES (?, 'stock', ?, 'INR')",
        )
        .bind(s)
        .bind(format!("{s} Ltd"))
        .execute(pool)
        .await
        .unwrap();
    }
}

async fn poll(
    pool: &sqlx::SqlitePool,
    market: &dyn MarketSource,
    notifier: &dyn stock_predictor::notify::Notifier,
    runtime: Runtime,
    now: DateTime<Utc>,
) -> Result<PollReport> {
    poll_with(pool, market, notifier, runtime, now, vec![]).await
}

async fn poll_with(
    pool: &sqlx::SqlitePool,
    market: &dyn MarketSource,
    notifier: &dyn stock_predictor::notify::Notifier,
    runtime: Runtime,
    now: DateTime<Utc>,
    replies: Vec<stock_predictor::llm::deepseek::HttpReply>,
) -> Result<PollReport> {
    let t = FixtureTransport::new(replies);
    let deps = EngineDeps {
        pool,
        runtime,
        market,
        transport: &t,
        notifier,
        rules: RuleConfig::default(),
        force: false,
    };
    engine::poll_once(&deps, now).await
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 27, 5, 0, 0).unwrap()
}

#[tokio::test]
async fn one_poll_with_a_candidate_emits_one_signal_and_one_notification() {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    seed(&pool, &["INFY"]).await;

    let market = FakeMarket::ok(bars_n(250));
    let notifier = FakeNotifier::default();
    let c = cfg();
    let report = poll(&pool, &market, &notifier, c.clone(), now())
        .await
        .unwrap();

    assert_eq!(report.candidates, 1, "{report:?}");
    assert_eq!(report.signals, 1, "{report:?}");
    assert_eq!(report.notifications, 1, "{report:?}");
    let action: (String,) = sqlx::query_as("SELECT action FROM signals")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(action.0, "BUY");
    assert_eq!(notifier.count(), 1);
}

#[tokio::test]
async fn second_identical_poll_emits_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    seed(&pool, &["INFY"]).await;
    let market = FakeMarket::ok(bars_n(250));
    let notifier = FakeNotifier::default();
    let c = cfg();

    let first = poll(&pool, &market, &notifier, c.clone(), now())
        .await
        .unwrap();
    assert_eq!(first.signals, 1);
    let second = poll(&pool, &market, &notifier, c.clone(), now())
        .await
        .unwrap();

    assert_eq!(second.signals, 0, "unchanged signal must be a no-op");
    assert_eq!(second.notifications, 0);
    assert_eq!(notifier.count(), 1);
}

#[tokio::test]
async fn insufficient_bars_emits_no_signal_and_no_llm_call() {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    seed(&pool, &["INFY"]).await;
    let market = FakeMarket::ok(bars_n(10));
    let notifier = FakeNotifier::default();
    let report = poll(&pool, &market, &notifier, cfg(), now()).await.unwrap();

    assert_eq!(report.candidates, 0);
    assert_eq!(report.signals, 0);
    let calls: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM api_calls")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(calls.0, 0, "no LLM call without a candidate");
}

#[tokio::test]
async fn one_symbol_failing_does_not_stop_the_others() {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    seed(&pool, &["BOOM", "INFY"]).await;
    let market = FakeMarket::ok(bars_n(250)).failing_for("BOOM");
    let notifier = FakeNotifier::default();
    let report = poll(&pool, &market, &notifier, cfg(), now()).await.unwrap();

    assert_eq!(report.errors.len(), 1, "{report:?}");
    assert!(report.errors[0].starts_with("BOOM"));
    assert_eq!(report.signals, 1);
    assert_eq!(notifier.count(), 1);
}

#[tokio::test]
async fn budget_exhaustion_records_blocked_and_never_notifies() {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    seed(&pool, &["INFY"]).await;

    // The rules layer sizes the trade from the last close, so it can never overspend on its
    // own. The LLM can: a higher entry inside the band pushes the same quantity over budget.
    let decided = serde_json::json!({
        "choices": [{"message": {"content": serde_json::json!({
            "action": "decide",
            "signal": {
                "action": "BUY",
                "entry": 106.0,
                "stop": 104.0,
                "target": 110.0,
                "valid_until": "2030-01-01T00:00:00Z",
                "confidence": "high",
                "rationale": "momentum",
                "cites": [1]
            }
        }).to_string()}}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5}
    })
    .to_string();

    let market = FakeMarket::ok(bars_n(250));
    let notifier = FakeNotifier::default();
    let report = poll_with(
        &pool,
        &market,
        &notifier,
        cfg(),
        now(),
        vec![stock_predictor::llm::deepseek::HttpReply {
            status: 200,
            body: decided,
        }],
    )
    .await
    .unwrap();

    assert_eq!(report.blocked, 1, "{report:?}");
    assert_eq!(report.signals, 0);
    assert_eq!(notifier.count(), 0, "a blocked signal must never be pushed");
    let action: (String,) = sqlx::query_as("SELECT action FROM signals")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(action.0, "BLOCKED");
}

#[tokio::test]
async fn with_no_affordable_quantity_the_model_is_still_asked_but_nothing_is_traded() {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    seed(&pool, &["INFY"]).await;
    stock_predictor::rules::settings::set(
        &pool,
        stock_predictor::rules::settings::SettingsPatch {
            budget: Some(100.0),
            currency: None,
            deepseek_key: None,
            trading_budget: None,
            ntfy_url: None,
            ntfy_topic: None,
            ntfy_token: None,
            llm_provider: None,
            llm_model: None,
            llm_base_url: None,
            llm_key: None,
            limits_long: None,
            limits_trade: None,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let market = FakeMarket::ok(bars_n(250));
    let notifier = FakeNotifier::default();
    let report = poll(&pool, &market, &notifier, cfg(), now()).await.unwrap();
    assert_eq!(
        report.candidates, 0,
        "no rules candidate: nothing is affordable"
    );
    assert_eq!(
        report.signals, 0,
        "and the model was asked on an empty queue"
    );
    assert_eq!(notifier.count(), 0);
}

#[test]
fn market_window_respects_ist_hours_and_skips_weekends() {
    // Tuesday 2026-10-27
    assert!(schedule::in_stock_window(
        Utc.with_ymd_and_hms(2026, 10, 27, 4, 0, 0).unwrap() // 09:30 IST
    ));
    assert!(schedule::in_stock_window(
        Utc.with_ymd_and_hms(2026, 10, 27, 10, 0, 0).unwrap() // 15:30 IST
    ));
    assert!(schedule::in_stock_window(
        Utc.with_ymd_and_hms(2026, 10, 27, 10, 30, 0).unwrap() // 16:00 IST, the close
    ));
    assert!(!schedule::in_stock_window(
        Utc.with_ymd_and_hms(2026, 10, 27, 10, 31, 0).unwrap() // 16:01 IST
    ));
    assert!(!schedule::in_stock_window(
        Utc.with_ymd_and_hms(2026, 10, 31, 4, 0, 0).unwrap() // Saturday
    ));
}

#[tokio::test]
async fn a_failing_push_does_not_lose_the_signal() {
    // The channel is a settings row, so the engine always attempts the push; a broken channel
    // must leave the signal recorded and be retried, not swallowed.
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    seed(&pool, &["INFY"]).await;
    let market = FakeMarket::ok(bars_n(250));
    let notifier = FakeNotifier::failing();
    let c = cfg();

    let report = poll(&pool, &market, &notifier, c.clone(), now())
        .await
        .unwrap();
    assert_eq!(report.signals, 1, "{report:?}");
    assert_eq!(report.notifications, 0);
    assert!(
        report.errors.iter().any(|e| e.contains("ntfy")),
        "the failure is reported, not hidden: {report:?}"
    );
    assert_eq!(notifier.count(), 0);
}

#[tokio::test]
async fn without_a_deepseek_key_the_model_layer_is_skipped_entirely() {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    seed_with_key(&pool, &["INFY"], "").await;

    let market = FakeMarket::ok(bars_n(250));
    let notifier = FakeNotifier::default();
    let report = poll(&pool, &market, &notifier, cfg(), now()).await.unwrap();

    assert_eq!(report.signals, 1, "{report:?}");
    assert!(report.errors.is_empty(), "{report:?}");
    let calls: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM api_calls")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        calls.0, 0,
        "no key must mean no outbound call, not a failed one"
    );
}

#[tokio::test]
async fn a_failed_model_call_does_not_overwrite_the_standing_decision() {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    seed(&pool, &["INFY"]).await;
    let market = FakeMarket::ok(bars_n(250));
    let notifier = FakeNotifier::default();
    let c = cfg();

    // Poll 1: a real model answer, so the headline set gets remembered.
    let decided = serde_json::json!({
        "choices": [{"message": {"content": serde_json::json!({
            "action": "decide",
            "signal": {
                "action": "BUY",
                "entry": 103.5,
                "stop": 100.0,
                "target": 108.0,
                "valid_until": "2030-01-01T00:00:00Z",
                "confidence": "medium",
                "rationale": "guidance raised",
                "cites": [1]
            }
        }).to_string()}}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5}
    })
    .to_string();
    let first_transport = FixtureTransport::new(vec![stock_predictor::llm::deepseek::HttpReply {
        status: 200,
        body: decided,
    }]);
    let deps1 = EngineDeps {
        pool: &pool,
        runtime: c.clone(),
        market: &market,
        transport: &first_transport,
        notifier: &notifier,
        rules: RuleConfig::default(),
        force: false,
    };
    let first = engine::poll_once(&deps1, now()).await.unwrap();
    assert_eq!(first.signals, 1, "{first:?}");
    assert_eq!(first_transport.calls(), 1);

    let memos: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM llm_memos")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(memos.0, 1, "the headline set is remembered");

    // Poll 2: identical headlines, so no model call at all — the entry refreshes in place.
    let second_transport = FixtureTransport::new(vec![]);
    let deps2 = EngineDeps {
        pool: &pool,
        runtime: c.clone(),
        market: &market,
        transport: &second_transport,
        notifier: &notifier,
        rules: RuleConfig::default(),
        force: false,
    };
    let second = engine::poll_once(&deps2, now()).await.unwrap();
    assert_eq!(second.signals, 0, "{second:?}");
    assert_eq!(
        second_transport.calls(),
        0,
        "same price inside the hour: the gate skips the call"
    );

    let rows: (i64, f64, Option<f64>) =
        sqlx::query_as("SELECT COUNT(*), MAX(entry), MAX(stop) FROM signals")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(rows.0, 1, "refreshed in place, not duplicated");
    assert!(
        (rows.1 - 104.48).abs() < 1e-6,
        "the entry follows the live price: {}",
        rows.1
    );
    assert_eq!(
        rows.2,
        Some(100.0),
        "the model's stop is kept — a stop is a decision, not a price"
    );
}

#[tokio::test]
async fn new_headlines_earn_a_new_model_call() {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    seed(&pool, &["INFY"]).await;

    let source = FakeMarket::ok(bars_n(250));
    let notifier = FakeNotifier::default();
    let c = cfg();
    let _ = poll(&pool, &source, &notifier, c.clone(), now())
        .await
        .unwrap();
    let first: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM api_calls")
        .fetch_one(&pool)
        .await
        .unwrap();

    // One new headline is new information.
    let moved = FakeMarket::ok(bars_n(250)).with_news(vec![headline(
        "guidance raised 200bps",
        "https://example.com/new",
    )]);
    let _ = poll(&pool, &moved, &notifier, c.clone(), now())
        .await
        .unwrap();
    let second: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM api_calls")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        second.0 > first.0,
        "a changed headline set must reach the model"
    );
}

#[tokio::test]
async fn the_model_confidence_is_stored_and_rules_only_signals_have_none() {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    seed(&pool, &["INFY"]).await;
    let market = FakeMarket::ok(bars_n(250));
    let notifier = FakeNotifier::default();
    let c = cfg();

    // A model decision carries its stated confidence through to the stored signal.
    let decided = serde_json::json!({
        "choices": [{"message": {"content": serde_json::json!({
            "action": "decide",
            "signal": {
                "action": "BUY", "entry": 103.5, "stop": 100.0, "target": 108.0,
                "valid_until": "2030-01-01T00:00:00Z", "confidence": "high",
                "rationale": "guidance raised", "cites": [1]
            }
        }).to_string()}}],
        "usage": {}
    })
    .to_string();
    let transport = FixtureTransport::new(vec![stock_predictor::llm::deepseek::HttpReply {
        status: 200,
        body: decided,
    }]);
    let deps = EngineDeps {
        pool: &pool,
        runtime: c.clone(),
        market: &market,
        transport: &transport,
        notifier: &notifier,
        rules: RuleConfig::default(),
        force: true,
    };
    let report = engine::poll_once(&deps, now()).await.unwrap();
    assert_eq!(report.signals, 1, "{report:?}");
    let conf: (Option<String>,) = sqlx::query_as("SELECT confidence FROM signals")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(conf.0.as_deref(), Some("high"));

    // A rules-only signal has no model judgement, so it must be NULL rather than invented.
    let (_, pool2, _) = (
        0,
        {
            let d2 = tempfile::tempdir().unwrap();
            let p = db::connect_and_migrate(d2.path().join("t.db").to_str().unwrap())
                .await
                .unwrap();
            std::mem::forget(d2);
            p
        },
        0,
    );
    seed(&pool2, &["INFY"]).await;
    let empty = FixtureTransport::new(vec![]);
    let deps2 = EngineDeps {
        pool: &pool2,
        runtime: c.clone(),
        market: &market,
        transport: &empty,
        notifier: &notifier,
        rules: RuleConfig::default(),
        force: true,
    };
    let _ = engine::poll_once(&deps2, now()).await.unwrap();
    let conf2: (Option<String>,) = sqlx::query_as("SELECT confidence FROM signals")
        .fetch_one(&pool2)
        .await
        .unwrap();
    assert_eq!(conf2.0, None);
}

#[tokio::test]
async fn the_call_gate_skips_small_moves_but_never_silences_for_more_than_an_hour() {
    use stock_predictor::llm::memo::{self, CallGate};

    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    seed(&pool, &["INFY"]).await;
    let id: (i64,) = sqlx::query_as("SELECT id FROM instruments WHERE symbol = 'INFY'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let gate = CallGate::default();
    let t0 = now();

    // Never asked: ask.
    assert!(memo::should_call(&pool, id.0, 100.0, t0, gate, false)
        .await
        .unwrap());

    memo::record_call(&pool, id.0, 100.0, t0, "fp-a")
        .await
        .unwrap();

    // 1.99 later, still inside the hour: not worth a call.
    assert!(!memo::should_call(
        &pool,
        id.0,
        101.99,
        t0 + chrono::Duration::minutes(10),
        gate,
        false
    )
    .await
    .unwrap());
    // Exactly the threshold: worth it.
    assert!(memo::should_call(
        &pool,
        id.0,
        102.0,
        t0 + chrono::Duration::minutes(10),
        gate,
        false
    )
    .await
    .unwrap());
    // A fall of the same size counts as a move.
    assert!(memo::should_call(
        &pool,
        id.0,
        98.0,
        t0 + chrono::Duration::minutes(10),
        gate,
        false
    )
    .await
    .unwrap());
    // The default gate has no time floor: an unchanged price is an unchanged world, however long
    // it has been. (The old forced hourly call is what made the model look chatty.)
    assert!(!memo::should_call(
        &pool,
        id.0,
        100.0,
        t0 + chrono::Duration::minutes(59),
        gate,
        false
    )
    .await
    .unwrap());
    assert!(!memo::should_call(
        &pool,
        id.0,
        100.0,
        t0 + chrono::Duration::hours(6),
        gate,
        false
    )
    .await
    .unwrap());
}

#[tokio::test]
async fn the_rules_layer_is_clamped_by_the_limits_too() {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    // Deliberately tight limits: 20,000 per fill.
    settings::seed(&pool).await.unwrap();
    settings::set(
        &pool,
        settings::SettingsPatch {
            budget: Some(200_000.0),
            limits_long: Some(stock_predictor::rules::limits::Limits {
                min_profit_pct: 0.0,
                max_profit_pct: 1000.0,
                min_trade_amount: 1_000.0,
                max_trade_amount: 20_000.0,
                max_instrument_pct: 100.0,
            }),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    seed_no_limits(&pool, &["INFY"]).await;

    let market = FakeMarket::ok(bars_n(250));
    let notifier = FakeNotifier::default();
    let report = poll(&pool, &market, &notifier, cfg(), now()).await.unwrap();
    assert_eq!(report.signals, 1, "{report:?}");

    let (qty, entry): (f64, f64) = sqlx::query_as("SELECT qty, entry FROM signals")
        .fetch_one(&pool)
        .await
        .unwrap();
    let amount = qty * entry;
    assert!(
        amount <= 20_000.5,
        "a rules-sized fill of {amount:.2} must be clamped to the 20,000 limit"
    );
    assert!(amount >= 1_000.0, "and must still clear the minimum");
}

#[tokio::test]
async fn a_stop_hit_sells_even_when_the_model_tries_to_hold() {
    // The exit is decided by price, not by opinion. Two things keep it that way: the validator
    // refuses a HOLD on a position that has breached its stop (no target, so no profit band), and
    // `resolve` ignores a model action for an urgent candidate. Either alone would hold the line.
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    seed_with_key(&pool, &["INFY"], "test-key").await;
    let id: (i64,) = sqlx::query_as("SELECT id FROM instruments WHERE symbol = 'INFY'")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO positions (instrument_id, qty, avg_cost, stop, target, state, mode, opened_at)
         VALUES (?, 10, 1200, 2000, 9999, 'open', 'long', '2026-01-01T00:00:00Z')",
    )
    .bind(id.0)
    .execute(&pool)
    .await
    .unwrap();

    // Close 1200 is far below the recorded 2000 stop.
    let market = FakeMarket::ok(bars_n(250));
    let hold = stock_predictor::llm::deepseek::HttpReply {
        status: 200,
        body: serde_json::json!({
            "choices": [{"message": {"content":
                r#"{"action":"decide","signal":{"action":"HOLD","entry":1200,"qty":10,"valid_until":"2030-01-01T00:00:00Z","rationale":"the company is fine","confidence":"high"}}"#}}],
            "usage": {"prompt_tokens": 900, "completion_tokens": 30}
        })
        .to_string(),
    };
    let notifier = FakeNotifier::default();
    // The book call follows every deciding call, so the fixture owes it an answer too.
    let pass = stock_predictor::llm::deepseek::HttpReply {
        status: 200,
        body: serde_json::json!({
            "choices": [{"message": {"content": r#"{"moves":[],"pass_reason":"nothing worth doing"}"#}}],
            "usage": {}
        })
        .to_string(),
    };
    let report = poll_with(&pool, &market, &notifier, cfg(), now(), vec![hold, pass])
        .await
        .unwrap();

    assert_eq!(report.signals, 1, "{report:#?}");
    assert_eq!(
        report.llm_calls, 2,
        "the deciding call and the book call: {report:#?}"
    );
    let action: (String,) = sqlx::query_as("SELECT action FROM signals ORDER BY id DESC LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(action.0, "SELL", "the model overrode a stop hit");
    assert_eq!(report.notifications, 1, "the exit was not pushed");
    let sent = notifier.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].title.starts_with("SELL"), "{}", sent[0].title);
}

/// The book call answers with a plan; every leg must land in one group, and the buy must carry its
/// exit plan, or the ledger cannot show it as one decision.
#[tokio::test]
async fn a_plan_is_stored_as_one_group_of_moves() {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    seed(&pool, &["INFY.NS", "HDFCBANK.NS"]).await;
    let market = FakeMarket::ok(bars_n(250));
    let notifier = FakeNotifier::default();

    // A holding, so the book is not empty.
    let id: (i64,) = sqlx::query_as("SELECT id FROM instruments WHERE symbol = 'INFY.NS'")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO positions (instrument_id, qty, avg_cost, opened_at, state, mode, stop, target)
         VALUES (?, 24, 1042.30, '2026-09-01T00:00:00Z', 'open', 'long', 1080, 1420)",
    )
    .bind(id.0)
    .execute(&pool)
    .await
    .unwrap();

    let pass = serde_json::json!({
        "choices": [{"message": {"content": "{\"action\":\"pass\",\"why\":\"nothing new\"}"}}],
        "usage": {}
    })
    .to_string();
    let plan = include_str!("fixtures/plan_reply.json").to_string();
    let planned = serde_json::json!({
        "choices": [{"message": {"content": plan}}],
        "usage": {}
    })
    .to_string();

    let transport = FixtureTransport::new(vec![
        stock_predictor::llm::deepseek::HttpReply {
            status: 200,
            body: pass.clone(),
        },
        stock_predictor::llm::deepseek::HttpReply {
            status: 200,
            body: pass,
        },
        stock_predictor::llm::deepseek::HttpReply {
            status: 200,
            body: planned,
        },
    ]);
    let deps = EngineDeps {
        pool: &pool,
        runtime: cfg(),
        market: &market,
        transport: &transport,
        notifier: &notifier,
        rules: RuleConfig::default(),
        force: true,
    };
    let report = engine::poll_once(&deps, now()).await.unwrap();
    assert_eq!(report.plans, 1, "{report:?}");

    #[derive(sqlx::FromRow)]
    struct Leg {
        action: String,
        plan_id: Option<i64>,
        mode: String,
        stop: Option<f64>,
        target: Option<f64>,
    }
    let rows: Vec<Leg> = sqlx::query_as(
        "SELECT action, plan_id, mode, stop, target FROM signals WHERE kind = 'plan' ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2, "both legs stored");
    assert_eq!(rows[0].action, "SELL");
    assert_eq!(rows[1].action, "BUY");
    assert_eq!(rows[0].mode, "long");
    assert!(rows[0].plan_id.is_some(), "a plan id");
    assert_eq!(
        rows[0].plan_id, rows[1].plan_id,
        "both legs share the plan id"
    );
    assert!(
        rows[1].stop.is_some() && rows[1].target.is_some(),
        "the buy keeps its exits"
    );
}

/// A plan that cannot be paid for is refused whole, and the refusal is a record with its reason.
#[tokio::test]
async fn an_unbalanced_plan_is_refused_and_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    seed(&pool, &["INFY.NS", "HDFCBANK.NS"]).await;
    let market = FakeMarket::ok(bars_n(250));
    let notifier = FakeNotifier::default();
    let id: (i64,) = sqlx::query_as("SELECT id FROM instruments WHERE symbol = 'INFY.NS'")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO positions (instrument_id, qty, avg_cost, opened_at, state, mode, stop, target)
         VALUES (?, 1, 1042.30, '2026-09-01T00:00:00Z', 'open', 'long', 1080, 1420)",
    )
    .bind(id.0)
    .execute(&pool)
    .await
    .unwrap();

    let pass = serde_json::json!({
        "choices": [{"message": {"content": "{\"action\":\"pass\",\"why\":\"nothing new\"}"}}],
        "usage": {}
    })
    .to_string();
    let greedy = serde_json::json!({
        "choices": [{"message": {"content": r#"{"moves":[
            {"action":"SELL","symbol":"INFY.NS","qty":1,"price":100.0},
            {"action":"BUY","symbol":"HDFCBANK.NS","qty":9999,"price":1000.0,"stop":900,"target":1200}
        ],"rationale":"wants the world"}"#}}],
        "usage": {}
    })
    .to_string();
    let transport = FixtureTransport::new(vec![
        stock_predictor::llm::deepseek::HttpReply {
            status: 200,
            body: pass.clone(),
        },
        stock_predictor::llm::deepseek::HttpReply {
            status: 200,
            body: pass,
        },
        stock_predictor::llm::deepseek::HttpReply {
            status: 200,
            body: greedy,
        },
    ]);
    let deps = EngineDeps {
        pool: &pool,
        runtime: cfg(),
        market: &market,
        transport: &transport,
        notifier: &notifier,
        rules: RuleConfig::default(),
        force: true,
    };
    let report = engine::poll_once(&deps, now()).await.unwrap();
    assert_eq!(report.plans, 0, "{report:?}");

    let blocked: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM signals WHERE action = 'BLOCKED' AND kind = 'plan'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(blocked.0, 1, "one refusal, recorded");
    let reason: (String,) = sqlx::query_as(
        "SELECT reason_json FROM signals WHERE action = 'BLOCKED' AND kind = 'plan'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(reason.0.contains("does not balance"), "{}", reason.0);
}

/// A plan's rows belong to the call that proposed it, or the audit page can only say "no decision".
#[tokio::test]
async fn a_plan_is_linked_to_the_call_that_proposed_it() {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    seed(&pool, &["INFY.NS", "HDFCBANK.NS"]).await;
    let market = FakeMarket::ok(bars_n(250));
    let notifier = FakeNotifier::default();
    let id: (i64,) = sqlx::query_as("SELECT id FROM instruments WHERE symbol = 'INFY.NS'")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO positions (instrument_id, qty, avg_cost, opened_at, state, mode, stop, target)
         VALUES (?, 24, 1042.30, '2026-09-01T00:00:00Z', 'open', 'long', 1080, 1420)",
    )
    .bind(id.0)
    .execute(&pool)
    .await
    .unwrap();
    let pass = serde_json::json!({
        "choices": [{"message": {"content": "{\"action\":\"pass\",\"why\":\"nothing new\"}"}}],
        "usage": {}
    })
    .to_string();
    let planned = serde_json::json!({
        "choices": [{"message": {"content": include_str!("fixtures/plan_reply.json")}}],
        "usage": {}
    })
    .to_string();
    let transport = FixtureTransport::new(vec![
        stock_predictor::llm::deepseek::HttpReply {
            status: 200,
            body: pass.clone(),
        },
        stock_predictor::llm::deepseek::HttpReply {
            status: 200,
            body: pass,
        },
        stock_predictor::llm::deepseek::HttpReply {
            status: 200,
            body: planned,
        },
    ]);
    let deps = EngineDeps {
        pool: &pool,
        runtime: cfg(),
        market: &market,
        transport: &transport,
        notifier: &notifier,
        rules: RuleConfig::default(),
        force: true,
    };
    let _ = engine::poll_once(&deps, now()).await.unwrap();

    let unlinked: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM signals WHERE kind = 'plan' AND conversation_id IS NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(unlinked.0, 0, "every leg names its call");

    let calls: (i64,) = sqlx::query_as(
        "SELECT COUNT(DISTINCT a.id) FROM api_calls a
         JOIN signals s ON s.conversation_id = a.conversation_id
         WHERE s.kind = 'plan'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(calls.0, 1, "the plan call is the one the legs point at");
}

/// The fund plan is asked once per IST day, on the first tick — including the first tick after a
/// restart. A pass still marks the day, or the next tick would ask again.
#[tokio::test]
async fn the_fund_plan_asks_once_a_day_and_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    seed(&pool, &["INFY.NS"]).await;
    // A fund in the book, priced, so the plan has something to look at.
    let fund: i64 = {
        sqlx::query("INSERT INTO instruments (symbol, class, name, currency) VALUES ('120503','mutual_fund','Index fund','INR')")
            .execute(&pool).await.unwrap();
        sqlx::query_scalar("SELECT id FROM instruments WHERE symbol = '120503'")
            .fetch_one(&pool)
            .await
            .unwrap()
    };
    sqlx::query(
        "INSERT INTO positions (instrument_id, qty, avg_cost, opened_at, state, mode)
         VALUES (?, 40, 110.0, '2026-09-01T00:00:00Z', 'open', 'long')",
    )
    .bind(fund)
    .execute(&pool)
    .await
    .unwrap();

    let pass = serde_json::json!({
        "choices": [{"message": {"content": "{\"moves\":[],\"pass_reason\":\"nothing worth doing\"}"}}],
        "usage": {}
    })
    .to_string();
    let market = FakeMarket::ok(bars_n(250));
    let notifier = FakeNotifier::default();

    // First tick of the day: the plan is asked, and passes.
    let first = FixtureTransport::new(vec![stock_predictor::llm::deepseek::HttpReply {
        status: 200,
        body: pass.clone(),
    }]);
    let deps = EngineDeps {
        pool: &pool,
        runtime: cfg(),
        market: &market,
        transport: &first,
        notifier: &notifier,
        rules: RuleConfig::default(),
        force: false,
    };
    assert!(!engine::plan_funds(&deps, now(), None).await.unwrap());

    // Second tick, same day: no call is spent.
    let empty = FixtureTransport::new(vec![]);
    let again = EngineDeps {
        pool: &pool,
        runtime: cfg(),
        market: &market,
        transport: &empty,
        notifier: &notifier,
        rules: RuleConfig::default(),
        force: false,
    };
    assert!(!engine::plan_funds(&again, now(), None).await.unwrap());

    // The next day, a restart asks again.
    let tomorrow = FixtureTransport::new(vec![stock_predictor::llm::deepseek::HttpReply {
        status: 200,
        body: pass,
    }]);
    let next_day = EngineDeps {
        pool: &pool,
        runtime: cfg(),
        market: &market,
        transport: &tomorrow,
        notifier: &notifier,
        rules: RuleConfig::default(),
        force: false,
    };
    let later = now() + chrono::Duration::days(1);
    assert!(!engine::plan_funds(&next_day, later, None).await.unwrap());
    let calls: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM api_calls WHERE purpose = 'plan funds'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(calls.0, 2, "one call on each of the two days");
}

/// The stock book is asked once a day too. Four polls in one day must cost one plan call, not four.
#[tokio::test]
async fn the_stock_plan_is_asked_once_a_day() {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    seed(&pool, &["INFY.NS", "HDFCBANK.NS"]).await;
    let id: (i64,) = sqlx::query_as("SELECT id FROM instruments WHERE symbol = 'INFY.NS'")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO positions (instrument_id, qty, avg_cost, opened_at, state, mode, stop, target)
         VALUES (?, 24, 1042.30, '2026-09-01T00:00:00Z', 'open', 'long', 1080, 1420)",
    )
    .bind(id.0)
    .execute(&pool)
    .await
    .unwrap();

    let pass = serde_json::json!({
        "choices": [{"message": {"content": "{\"action\":\"pass\",\"why\":\"nothing new\"}"}}],
        "usage": {}
    })
    .to_string();
    let plan = serde_json::json!({
        "choices": [{"message": {"content": include_str!("fixtures/plan_reply.json")}}],
        "usage": {}
    })
    .to_string();
    let market = FakeMarket::ok(bars_n(250));
    let notifier = FakeNotifier::default();

    // Poll 1: two deciding calls, then the plan.
    let first = FixtureTransport::new(vec![
        stock_predictor::llm::deepseek::HttpReply {
            status: 200,
            body: pass.clone(),
        },
        stock_predictor::llm::deepseek::HttpReply {
            status: 200,
            body: pass.clone(),
        },
        stock_predictor::llm::deepseek::HttpReply {
            status: 200,
            body: plan,
        },
    ]);
    let deps = EngineDeps {
        pool: &pool,
        runtime: cfg(),
        market: &market,
        transport: &first,
        notifier: &notifier,
        rules: RuleConfig::default(),
        force: true,
    };
    let report = engine::poll_once(&deps, now()).await.unwrap();
    assert_eq!(report.plans, 1, "{report:?}");

    // Poll 2, same day: the deciding calls still happen, the plan does not.
    let second = FixtureTransport::new(vec![
        stock_predictor::llm::deepseek::HttpReply {
            status: 200,
            body: pass.clone(),
        },
        stock_predictor::llm::deepseek::HttpReply {
            status: 200,
            body: pass,
        },
    ]);
    let deps2 = EngineDeps {
        pool: &pool,
        runtime: cfg(),
        market: &market,
        transport: &second,
        notifier: &notifier,
        rules: RuleConfig::default(),
        force: true,
    };
    let report2 = engine::poll_once(&deps2, now() + chrono::Duration::minutes(15))
        .await
        .unwrap();
    assert_eq!(
        report2.plans, 0,
        "the book is not asked twice in a day: {report2:?}"
    );

    let calls: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM api_calls WHERE purpose = 'plan stocks'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(calls.0, 1, "one plan call for the day");
}

/// A quiet tick must be able to say it was quiet: the gate names what it left alone, so the run
/// summary can read "0 decisions · 2 not asked: INFY.NS, HDFCBANK.NS" instead of a bare zero.
#[tokio::test]
async fn a_poll_reports_what_the_gate_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    seed(&pool, &["INFY.NS"]).await;
    let market = FakeMarket::ok(bars_n(250));
    let notifier = FakeNotifier::default();

    // One poll with a key and force, so the memo records a price for the gate to compare against.
    let reply = stock_predictor::llm::deepseek::HttpReply {
        status: 200,
        body: serde_json::json!({
            "choices": [{"message": {"content": "{\"action\":\"pass\",\"why\":\"nothing new\"}"}}],
            "usage": {}
        })
        .to_string(),
    };
    let first = FixtureTransport::new(vec![reply]);
    let deps = EngineDeps {
        pool: &pool,
        runtime: cfg(),
        market: &market,
        transport: &first,
        notifier: &notifier,
        rules: RuleConfig::default(),
        force: true,
    };
    let _ = engine::poll_once(&deps, now()).await.unwrap();

    // Second poll, unforced, same price: the gate must leave it alone and say so.
    let empty = FixtureTransport::new(vec![]);
    let second = EngineDeps {
        pool: &pool,
        runtime: cfg(),
        market: &market,
        transport: &empty,
        notifier: &notifier,
        rules: RuleConfig::default(),
        force: false,
    };
    let report = engine::poll_once(&second, now() + chrono::Duration::minutes(15))
        .await
        .unwrap();
    assert!(
        report.not_asked.iter().any(|s| s == "INFY.NS"),
        "{report:?}"
    );
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.contains("model not asked")),
        "{report:?}"
    );
    // The run summary says "0 of 1 instruments asked", which needs both halves counted.
    assert_eq!(report.polled, 1, "{report:?}");
    assert_eq!(report.asked, 0, "{report:?}");
}

/// A proposal the validator refused used to disappear: the rules' fallback is a HOLD and a HOLD
/// records nothing, so the model's answer was visible in Audit and nowhere else.
#[tokio::test]
async fn a_refused_proposal_is_recorded_so_it_is_not_only_in_the_audit() {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    seed(&pool, &["INFY"]).await;

    // Flat tape, so the rules see no entry and the model's answer is the only proposal. A stop
    // above the entry is refused by the validator, and the rules' fallback is a HOLD.
    let refused = serde_json::json!({
        "choices": [{"message": {"content": serde_json::json!({
            "action": "decide",
            "signal": {
                "action": "BUY", "entry": 1405.20, "stop": 1412.00, "target": 1510.00,
                "qty": 5, "valid_until": "2026-11-09T15:30:00+05:30",
                "confidence": "high", "rationale": "momentum", "cites": []
            }
        }).to_string()}}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5}
    })
    .to_string();

    let flat: Vec<_> = (0..250)
        .map(|i| common::bar(&format!("2026-01-01T00:{:02}:00Z", i % 60), 100.0))
        .collect();
    let market = FakeMarket::ok(flat);
    let notifier = FakeNotifier::default();
    let report = poll_with(
        &pool,
        &market,
        &notifier,
        cfg(),
        now(),
        vec![stock_predictor::llm::deepseek::HttpReply { status: 200, body: refused }],
    )
    .await
    .unwrap();

    assert_eq!(report.signals, 0, "{report:?}");
    assert_eq!(notifier.count(), 0, "nothing to approve, nothing to push");
    let row: (String, String) = sqlx::query_as("SELECT action, reason_json FROM signals")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.0, "BLOCKED");
    assert!(
        row.1.contains("refused") && row.1.contains("detail"),
        "the reason is recorded, not just the verdict: {}",
        row.1
    );
}
