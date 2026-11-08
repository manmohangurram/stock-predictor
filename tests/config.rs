use std::collections::BTreeMap;

use stock_predictor::config::Config;
use stock_predictor::db;

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn config_only_carries_deployment_facts() {
    // Everything else — budget, keys, ntfy, cadence, the proxy — is a settings row, so a stale
    // variable cannot silently override what the page says.
    let cfg = Config::from_map(&map(&[
        ("BUDGET", "200000"),
        ("DEEPSEEK_API_KEY", "k"),
        ("NTFY_TOPIC", "t"),
    ]))
    .unwrap();
    assert_eq!(cfg.bind, "127.0.0.1:8080");
    assert_eq!(cfg.db_path, "data/stock-predictor.db");
    assert_eq!(cfg.public_url, "");
}

#[test]
fn the_three_deployment_variables_are_read() {
    let cfg = Config::from_map(&map(&[
        ("BIND", "0.0.0.0:9000"),
        ("DB_PATH", "/srv/app.db"),
        ("PUBLIC_URL", "https://ledger.example.in/"),
    ]))
    .unwrap();
    assert_eq!(cfg.bind, "0.0.0.0:9000");
    assert_eq!(cfg.db_path, "/srv/app.db");
    // Trailing slash trimmed: the deep link appends its own.
    assert_eq!(cfg.public_url, "https://ledger.example.in");
}

#[test]
fn a_secret_key_is_read_from_the_environment() {
    let hex = "19".repeat(32);
    let cfg = Config::from_map(&map(&[("SECRET_KEY", &hex)])).unwrap();
    assert_eq!(cfg.secret_key.as_ref().unwrap().len(), 32);

    // base64 of 32 bytes is also accepted
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode([7u8; 32]);
    let cfg = Config::from_map(&map(&[("SECRET_KEY", &b64)])).unwrap();
    assert_eq!(cfg.secret_key.unwrap(), vec![7u8; 32]);
}

#[test]
fn a_short_secret_key_is_refused_with_a_way_to_generate_one() {
    let err = Config::from_map(&map(&[("SECRET_KEY", "hunter2")])).unwrap_err();
    assert!(err.to_string().contains("32 bytes"), "{err}");
    assert!(err.to_string().contains("openssl rand -hex 32"), "{err}");
}

#[test]
fn a_malformed_bind_is_refused_at_boot() {
    assert!(Config::from_map(&map(&[("BIND", "8080")])).is_err());
}

#[test]
fn retired_variables_are_reported_so_they_are_never_silently_ignored() {
    // The map is what boot warns about; the config itself ignores them either way.
    let cfg = Config::from_map(&map(&[
        ("POLL_SECS", "60"),
        ("LLM_DAILY_CAP", "7"),
        ("PROMPT_WINDOW_HOURS", "99"),
    ]))
    .unwrap();
    assert_eq!(cfg.bind, "127.0.0.1:8080");
    for name in ["POLL_SECS", "LLM_DAILY_CAP", "PROMPT_WINDOW_HOURS"] {
        assert!(stock_predictor::config::RETIRED.contains(&name));
    }
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
