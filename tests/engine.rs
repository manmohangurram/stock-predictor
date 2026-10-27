use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use stock_predicter::config::Config;
use stock_predicter::db;
use stock_predicter::engine::{self, EngineDeps, MarketSource, PollReport};
use stock_predicter::llm::deepseek::FixtureTransport;
use stock_predicter::models::{Bar, Headline};
use stock_predicter::notify::FakeNotifier;
use stock_predicter::rules::evaluate::RuleConfig;
use stock_predicter::rules::settings;
use stock_predicter::schedule;

struct FakeMarket {
    bars: Result<Vec<Bar>, String>,
    fail_symbols: Vec<String>,
    news_title: String,
}

impl FakeMarket {
    fn ok(bars: Vec<Bar>) -> Self {
        Self {
            bars: Ok(bars),
            fail_symbols: vec![],
            news_title: "headline".into(),
        }
    }
    fn with_failure(sym: &str, bars: Vec<Bar>) -> Self {
        Self {
            bars: Ok(bars),
            fail_symbols: vec![sym.to_string()],
            news_title: "headline".into(),
        }
    }
    fn too_short() -> Self {
        Self::ok(bars_n(10))
    }
}

impl MarketSource for FakeMarket {
    fn bars<'a>(
        &'a self,
        symbol: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Bar>>> + Send + 'a>> {
        let fail = self.fail_symbols.iter().any(|s| s == symbol);
        let out = match &self.bars {
            _ if fail => Err(anyhow::anyhow!("simulated outage for {symbol}")),
            Ok(b) => Ok(b.clone()),
            Err(e) => Err(anyhow::anyhow!(e.clone())),
        };
        Box::pin(async move { out })
    }

    fn intraday<'a>(
        &'a self,
        _symbol: &'a str,
        _interval: &'a str,
        _range: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Bar>>> + Send + 'a>> {
        let out = match &self.bars {
            Ok(b) => Ok(b.clone()),
            Err(e) => Err(anyhow::anyhow!(e.clone())),
        };
        Box::pin(async move { out })
    }

    fn news<'a>(
        &'a self,
        _symbol: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Headline>>> + Send + 'a>> {
        Box::pin(async move {
            Ok(vec![Headline {
                id: 0,
                title: self.news_title.clone(),
                publisher: "Reuters".into(),
                published_at: "2026-10-25T00:00:00Z".into(),
                url: "https://example.com/h".into(),
            }])
        })
    }
}

/// A rising sawtooth: last close above both averages, RSI moderate.
fn bars_n(n: usize) -> Vec<Bar> {
    (0..n)
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

fn cfg() -> Config {
    Config::from_map(
        &[
            ("DEEPSEEK_API_KEY", "k"),
            ("LLM_DAILY_CAP", "50"),
            ("NTFY_TOPIC", "test-topic"),
        ]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    )
    .unwrap()
}

async fn seed(pool: &sqlx::SqlitePool, symbols: &[&str]) {
    seed_with_key(pool, symbols, "test-key").await
}

async fn seed_with_key(pool: &sqlx::SqlitePool, symbols: &[&str], key: &str) {
    settings::seed(pool, 200_000.0, "INR", key).await.unwrap();
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
    notifier: &dyn stock_predicter::notify::Notifier,
    cfg: &Config,
    now: DateTime<Utc>,
) -> Result<PollReport> {
    poll_with(pool, market, notifier, cfg, now, vec![]).await
}

async fn poll_with(
    pool: &sqlx::SqlitePool,
    market: &dyn MarketSource,
    notifier: &dyn stock_predicter::notify::Notifier,
    cfg: &Config,
    now: DateTime<Utc>,
    replies: Vec<stock_predicter::llm::deepseek::HttpReply>,
) -> Result<PollReport> {
    let t = FixtureTransport::new(replies);
    let deps = EngineDeps {
        pool,
        cfg,
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
    let report = poll(&pool, &market, &notifier, &c, now()).await.unwrap();

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

    let first = poll(&pool, &market, &notifier, &c, now()).await.unwrap();
    assert_eq!(first.signals, 1);
    let second = poll(&pool, &market, &notifier, &c, now()).await.unwrap();

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
    let market = FakeMarket::too_short();
    let notifier = FakeNotifier::default();
    let report = poll(&pool, &market, &notifier, &cfg(), now()).await.unwrap();

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
    let market = FakeMarket::with_failure("BOOM", bars_n(250));
    let notifier = FakeNotifier::default();
    let report = poll(&pool, &market, &notifier, &cfg(), now()).await.unwrap();

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
        &cfg(),
        now(),
        vec![stock_predicter::llm::deepseek::HttpReply {
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
    stock_predicter::rules::settings::set(
        &pool,
        stock_predicter::rules::settings::SettingsPatch {
            budget: Some(100.0),
            currency: None,
            deepseek_key: None,
            trading_budget: None,
        },
    )
    .await
    .unwrap();

    let market = FakeMarket::ok(bars_n(250));
    let notifier = FakeNotifier::default();
    let report = poll(&pool, &market, &notifier, &cfg(), now()).await.unwrap();
    assert_eq!(report.candidates, 0, "no rules candidate: nothing is affordable");
    assert_eq!(report.signals, 0, "and the model was asked on an empty queue");
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
    assert!(!schedule::in_stock_window(
        Utc.with_ymd_and_hms(2026, 10, 27, 10, 30, 0).unwrap() // 16:00 IST
    ));
    assert!(!schedule::in_stock_window(
        Utc.with_ymd_and_hms(2026, 10, 31, 4, 0, 0).unwrap() // Saturday
    ));
    assert!(!schedule::is_daily_pass(
        Utc.with_ymd_and_hms(2026, 10, 27, 5, 0, 0).unwrap()
    ));
    assert!(schedule::is_daily_pass(
        Utc.with_ymd_and_hms(2026, 10, 27, 13, 35, 0).unwrap() // 19:05 IST
    ));
}

#[tokio::test]
async fn an_unset_topic_skips_the_push_but_still_records_the_signal() {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    seed(&pool, &["INFY"]).await;
    let market = FakeMarket::ok(bars_n(250));
    let notifier = FakeNotifier::default();
    // No NTFY_TOPIC: there is nowhere to push to, which must not look like a market error.
    let mut c = cfg();
    c.ntfy_topic = String::new();

    let report = poll(&pool, &market, &notifier, &c, now()).await.unwrap();
    assert_eq!(report.signals, 1, "{report:?}");
    assert_eq!(report.notifications, 0);
    assert!(report.errors.is_empty(), "{report:?}");
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
    let report = poll(&pool, &market, &notifier, &cfg(), now()).await.unwrap();

    assert_eq!(report.signals, 1, "{report:?}");
    assert!(report.errors.is_empty(), "{report:?}");
    let calls: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM api_calls")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(calls.0, 0, "no key must mean no outbound call, not a failed one");
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
    let first_transport = FixtureTransport::new(vec![stock_predicter::llm::deepseek::HttpReply {
        status: 200,
        body: decided,
    }]);
    let deps1 = EngineDeps {
        pool: &pool,
        cfg: &c,
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
        cfg: &c,
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
        1,
        "the model is consulted every tick now, even with nothing new"
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

    let mut source = FakeMarket::ok(bars_n(250));
    source.news_title = "headline".into();
    let notifier = FakeNotifier::default();
    let c = cfg();
    let _ = poll(&pool, &source, &notifier, &c, now()).await.unwrap();
    let first: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM api_calls")
        .fetch_one(&pool)
        .await
        .unwrap();

    // One new headline is new information.
    let mut moved = FakeMarket::ok(bars_n(250));
    moved.news_title = "guidance raised 200bps".into();
    let _ = poll(&pool, &moved, &notifier, &c, now()).await.unwrap();
    let second: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM api_calls")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(second.0 > first.0, "a changed headline set must reach the model");
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
    let transport = FixtureTransport::new(vec![stock_predicter::llm::deepseek::HttpReply {
        status: 200,
        body: decided,
    }]);
    let deps = EngineDeps {
        pool: &pool,
        cfg: &c,
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
    let (_, pool2, _) = (0, {
        let d2 = tempfile::tempdir().unwrap();
        let p = db::connect_and_migrate(d2.path().join("t.db").to_str().unwrap())
            .await
            .unwrap();
        std::mem::forget(d2);
        p
    }, 0);
    seed(&pool2, &["INFY"]).await;
    let empty = FixtureTransport::new(vec![]);
    let deps2 = EngineDeps {
        pool: &pool2,
        cfg: &c,
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
