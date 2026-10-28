use chrono::{DateTime, TimeZone, Utc};
use serde_json::json;
use stock_predicter::config::Config;
use stock_predicter::db;
use stock_predicter::engine::{self, EngineDeps};
use stock_predicter::llm::deepseek::{FixtureTransport, HttpReply};
use stock_predicter::notify::FakeNotifier;
use stock_predicter::rules::evaluate::RuleConfig;
use stock_predicter::rules::settings;

mod common;
use common::{rising_bars as bars, FakeMarket};

fn chat(content: serde_json::Value) -> HttpReply {
    HttpReply {
        status: 200,
        body: json!({
            "choices": [{"message": {"content": content.to_string()}}],
            "usage": {"prompt_tokens": 300, "completion_tokens": 40}
        })
        .to_string(),
    }
}

fn fetch_ok() -> HttpReply {
    HttpReply {
        status: 200,
        body: json!({"title": "t", "content": "Guidance raised by 200bps."}).to_string(),
    }
}

fn need_more() -> serde_json::Value {
    json!({"action": "need_more", "requests": [{"headline_id": 1, "why": "guidance detail"}]})
}

fn decide(entry: f64, stop: f64) -> serde_json::Value {
    json!({
        "action": "decide",
        "signal": {
            "action": "BUY", "entry": entry, "stop": stop, "target": entry + 5.0,
            "valid_until": "2030-01-01T00:00:00Z", "confidence": "high",
            "rationale": "guidance raised with margin expansion", "cites": [1]
        }
    })
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

async fn setup() -> (tempfile::TempDir, sqlx::SqlitePool) {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    settings::seed(&pool).await.unwrap();
    // The ceiling and the key are settings now, so a test that relies on them sets them.
    settings::set(
        &pool,
        settings::SettingsPatch {
            budget: Some(200_000.0),
            llm_key: Some("test-key".into()),
            limits_long: Some(stock_predicter::rules::limits::Limits {
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
    sqlx::query(
        "INSERT INTO instruments (symbol, class, name, currency) VALUES ('INFY', 'stock', 'Infosys', 'INR')",
    )
    .execute(&pool)
    .await
    .unwrap();
    (dir, pool)
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 27, 5, 0, 0).unwrap()
}

#[tokio::test]
async fn full_cycle_signals_once_then_stays_quiet() {
    let (_d, pool) = setup().await;
    let cfg = cfg();
    let market = FakeMarket::ok(bars(250));
    let notifier = FakeNotifier::default();

    let poll1 = FixtureTransport::new(vec![
        chat(need_more()),
        fetch_ok(),
        chat(decide(103.5, 102.0)),
    ]);
    let deps1 = EngineDeps {
        pool: &pool,
        cfg: &cfg,
        market: &market,
        transport: &poll1,
        notifier: &notifier,
        rules: RuleConfig::default(),
        force: false,
    };
    let r1 = engine::poll_once(&deps1, now()).await.unwrap();
    assert_eq!(r1.signals, 1, "{r1:?}");
    assert_eq!(r1.notifications, 1, "{r1:?}");

    let endpoints: Vec<(String,)> = sqlx::query_as("SELECT endpoint FROM api_calls ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap();
    let endpoints: Vec<String> = endpoints.into_iter().map(|e| e.0).collect();
    assert_eq!(endpoints, vec!["chat", "fetch", "chat"]);
    assert_eq!(notifier.count(), 1);

    // Second tick: identical market data, so the price gate skips the call entirely and the
    // standing decision is left alone.
    let poll2 = FixtureTransport::new(vec![]);
    let deps2 = EngineDeps {
        pool: &pool,
        cfg: &cfg,
        market: &market,
        transport: &poll2,
        notifier: &notifier,
        rules: RuleConfig::default(),
        force: false,
    };
    let r2 = engine::poll_once(&deps2, now()).await.unwrap();
    assert_eq!(r2.signals, 0, "no new row for a price refresh");
    assert_eq!(r2.notifications, 0);
    assert_eq!(poll2.calls(), 0, "price did not move: no model call");

    let (n, entry): (i64, f64) = sqlx::query_as("SELECT COUNT(*), MAX(entry) FROM signals")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1, "the signal was refreshed in place, not duplicated");
    assert!(
        (entry - 104.48).abs() < 1e-6,
        "entry refreshed to the live price"
    );

    let calls: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM api_calls")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(calls.0, 3, "no new audit rows on a gated tick");
    assert_eq!(notifier.count(), 1);
}

#[tokio::test]
async fn hallucinated_stop_falls_back_to_the_rules_numbers() {
    let (_d, pool) = setup().await;
    let cfg = cfg();
    let market = FakeMarket::ok(bars(250));
    let notifier = FakeNotifier::default();
    // stop above entry for a BUY: the validator must refuse this outright.
    let transport = FixtureTransport::new(vec![chat(decide(106.0, 120.0))]);
    let deps = EngineDeps {
        pool: &pool,
        cfg: &cfg,
        market: &market,
        transport: &transport,
        notifier: &notifier,
        rules: RuleConfig::default(),
        force: false,
    };
    let report = engine::poll_once(&deps, now()).await.unwrap();
    assert_eq!(report.signals, 1);

    let (entry, stop, reason): (f64, f64, String) =
        sqlx::query_as("SELECT entry, stop, reason_json FROM signals WHERE action = 'BUY'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let reason: serde_json::Value = serde_json::from_str(&reason).unwrap();
    assert!(
        (entry - 104.48).abs() < 1e-6,
        "rules entry used, got {entry}"
    );
    assert!(stop < entry, "rules stop is below entry, got {stop}");
    assert!(
        (stop - (entry - 2.0 * 1.6)).abs() < 1.0,
        "stop should be the rules' 2-ATR level, not the model's 120.00: {stop}"
    );
    assert!(
        reason["reasons"].is_array(),
        "rules reasons are still recorded"
    );
    assert_eq!(notifier.count(), 1);
}
