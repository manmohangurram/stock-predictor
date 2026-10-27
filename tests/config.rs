use std::collections::BTreeMap;

use stock_predicter::config::Config;
use stock_predicter::db;

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn config_defaults_poll_and_cap() {
    let cfg = Config::from_map(&map(&[
        ("BUDGET", "200000"),
        ("DEEPSEEK_API_KEY", "k"),
        ("NTFY_TOPIC", "t"),
        ("NTFY_BASE_URL", "https://ntfy.sh"),
    ]))
    .unwrap();
    assert_eq!(cfg.seed_budget, 200_000.0);
    assert_eq!(cfg.seed_currency, "INR");
    assert_eq!(cfg.poll_secs, 900);
    // The call gate: a 2-unit move, an hourly floor, and a two-hour off-hours cadence.
    assert_eq!(cfg.llm_min_price_move, 2.0);
    assert_eq!(cfg.llm_max_silence_secs, 3600);
    assert_eq!(cfg.off_hours_secs, 7200);
    assert_eq!(cfg.llm_daily_cap, 200);
}

#[test]
fn non_numeric_budget_is_an_error() {
    assert!(Config::from_map(&map(&[("BUDGET", "lots")])).is_err());
}

#[tokio::test]
async fn migrations_create_the_settings_and_ledger_tables() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    let pool = db::connect_and_migrate(path.to_str().unwrap())
        .await
        .unwrap();

    let tables: Vec<(String,)> =
        sqlx::query_as("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .fetch_all(&pool)
            .await
            .unwrap();
    let names: Vec<String> = tables.into_iter().map(|t| t.0).collect();
    for expected in [
        "api_calls",
        "bars",
        "cash_ledger",
        "instruments",
        "ipo_applications",
        "news",
        "notifications",
        "positions",
        "search_results",
        "settings",
        "signals",
    ] {
        assert!(names.contains(&expected.to_string()), "missing {expected}");
    }
}
