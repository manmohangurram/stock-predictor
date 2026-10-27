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
}

impl FakeMarket {
    fn ok(bars: Vec<Bar>) -> Self {
        Self {
            bars: Ok(bars),
            fail_symbols: vec![],
        }
    }
    fn with_failure(sym: &str, bars: Vec<Bar>) -> Self {
        Self {
            bars: Ok(bars),
            fail_symbols: vec![sym.to_string()],
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

    fn news<'a>(
        &'a self,
        _symbol: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Headline>>> + Send + 'a>> {
        Box::pin(async move {
            Ok(vec![Headline {
                id: 0,
                title: "headline".into(),
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
    settings::seed(pool, 200_000.0, "INR").await.unwrap();
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
async fn no_affordable_quantity_means_no_candidate_at_all() {
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
        },
    )
    .await
    .unwrap();

    let market = FakeMarket::ok(bars_n(250));
    let notifier = FakeNotifier::default();
    let report = poll(&pool, &market, &notifier, &cfg(), now()).await.unwrap();
    assert_eq!(report.candidates, 0);
    assert_eq!(report.signals, 0);
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
