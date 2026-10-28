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
fn config_only_carries_deployment_settings() {
    // Budget, currency, keys and ntfy are edited in the UI and live in the database, so none
    // of them appear here: one source of truth, and it is not the environment.
    let cfg = Config::from_map(&map(&[
        ("BUDGET", "200000"),
        ("DEEPSEEK_API_KEY", "k"),
        ("NTFY_TOPIC", "t"),
        ("NTFY_BASE_URL", "https://ntfy.sh"),
    ]))
    .unwrap();
    assert_eq!(cfg.poll_secs, 900);
    // The call gate: a 2-unit move, an hourly floor, and a two-hour off-hours cadence.
    assert_eq!(cfg.llm_min_price_move, 2.0);
    assert_eq!(cfg.llm_max_silence_secs, 3600);
    assert_eq!(cfg.off_hours_secs, 7200);
    assert_eq!(cfg.llm_daily_cap, 200);
}

#[test]
fn a_stray_budget_in_the_environment_is_ignored() {
    // It used to be a seed. Now the UI owns it, so a typo there must not break the boot.
    assert!(Config::from_map(&map(&[("BUDGET", "lots")])).is_ok());
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
