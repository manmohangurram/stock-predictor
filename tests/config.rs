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

#[tokio::test]
async fn a_database_under_the_old_name_is_moved_with_its_wal() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("stock-predicter.db");
    let new = dir.path().join("stock-predictor.db");

    // Write a row, leave the pool open briefly so a -wal exists, then migrate.
    let pool = db::connect_and_migrate(old.to_str().unwrap())
        .await
        .unwrap();
    sqlx::query("INSERT INTO instruments (symbol, class, name, currency) VALUES ('INFY','stock','Infosys','INR')")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let moved = db::migrate_legacy_path(old.to_str().unwrap(), new.to_str().unwrap())
        .await
        .unwrap();
    assert_eq!(moved.as_deref(), Some(new.to_str().unwrap()));
    assert!(!old.exists(), "the old file should be gone, not duplicated");

    // The row survived the move: the WAL was folded in first.
    let pool = db::connect(new.to_str().unwrap()).await.unwrap();
    let n: (i64,) = sqlx::query_as("SELECT count(*) FROM instruments")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n.0, 1, "the WAL was not checkpointed before the move");
}

#[tokio::test]
async fn the_old_file_wins_and_the_new_one_is_kept_as_a_backup() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("stock-predicter.db");
    let new = dir.path().join("stock-predictor.db");

    let pool = db::connect_and_migrate(old.to_str().unwrap())
        .await
        .unwrap();
    sqlx::query("INSERT INTO instruments (symbol, class, name, currency) VALUES ('INFY','stock','Infosys','INR')")
        .execute(&pool).await.unwrap();
    pool.close().await;
    db::connect_and_migrate(new.to_str().unwrap())
        .await
        .unwrap()
        .close()
        .await;

    let moved = db::migrate_legacy_path(old.to_str().unwrap(), new.to_str().unwrap())
        .await
        .unwrap();
    assert_eq!(moved.as_deref(), Some(new.to_str().unwrap()));
    assert!(!old.exists());

    // The new path now holds the real ledger…
    let pool = db::connect(new.to_str().unwrap()).await.unwrap();
    let n: (i64,) = sqlx::query_as("SELECT count(*) FROM instruments")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n.0, 1);
    // …and what was there is still on disk under .bak-.
    let backups: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains(".bak-"))
        .collect();
    assert_eq!(backups.len(), 1, "the previous file must be kept");
}

#[tokio::test]
async fn an_empty_new_database_never_hides_the_one_with_data() {
    // The reported failure: a new file appeared (empty) while the real ledger stayed under the old
    // name, so the app looked like it had lost everything.
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("stock-predicter.db");
    let new = dir.path().join("stock-predictor.db");

    let pool = db::connect_and_migrate(old.to_str().unwrap())
        .await
        .unwrap();
    sqlx::query("INSERT INTO instruments (symbol, class, name, currency) VALUES ('INFY','stock','Infosys','INR')")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    // An empty database under the new name, as an earlier version would have created.
    db::connect_and_migrate(new.to_str().unwrap())
        .await
        .unwrap()
        .close()
        .await;

    let moved = db::migrate_legacy_path(old.to_str().unwrap(), new.to_str().unwrap())
        .await
        .unwrap();
    assert_eq!(moved.as_deref(), Some(new.to_str().unwrap()));

    let pool = db::connect(new.to_str().unwrap()).await.unwrap();
    let n: (i64,) = sqlx::query_as("SELECT count(*) FROM instruments")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n.0, 1, "the real data did not come back");

    // Whatever was at the new name is kept, not deleted.
    let aside: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains(".bak-"))
        .collect();
    assert_eq!(aside.len(), 1, "the previous database should be kept");
}

#[test]
fn the_legacy_file_is_looked_for_beside_the_configured_one() {
    // The container sets DB_PATH=/data/stock-predictor.db, so the migration must not depend on
    // DB_PATH being unset.
    assert_eq!(
        db::legacy_sibling("/data/stock-predictor.db").as_deref(),
        Some("/data/stock-predicter.db")
    );
    assert_eq!(
        db::legacy_sibling("data/stock-predictor.db").as_deref(),
        Some("data/stock-predicter.db")
    );
    // A custom name has no old spelling to look for.
    assert_eq!(db::legacy_sibling("/data/ledger.db"), None);
}

/// A developer's `.env`: comments, quotes, `export`, and an inline comment after an unquoted value.
#[test]
fn dotenv_lines_are_read_the_way_a_person_writes_them() {
    let raw = r#"
# the key I am testing prompts with
export DEEPSEEK_API_KEY=sk-abc123
QUOTED="a value with spaces"
SINGLE='single quoted'
INLINE=plain # this is a comment
BROKEN_LINE_WITHOUT_EQUALS
=novalue
BAD KEY=value
"#;
    let pairs = stock_predictor::config::parse_dotenv(raw);
    let get = |k: &str| {
        pairs
            .iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(get("DEEPSEEK_API_KEY"), Some("sk-abc123"));
    assert_eq!(get("QUOTED"), Some("a value with spaces"));
    assert_eq!(get("SINGLE"), Some("single quoted"));
    assert_eq!(get("INLINE"), Some("plain"));
    assert_eq!(get("BROKEN_LINE_WITHOUT_EQUALS"), None, "no '=' is skipped");
    assert_eq!(pairs.len(), 4, "only the four well-formed lines: {pairs:?}");
}

/// The variable a key may hide in, per provider — nothing here is written to a setting.
#[test]
fn each_provider_has_one_environment_name() {
    use stock_predictor::llm::provider::Provider;
    assert_eq!(
        stock_predictor::config::key_env_name(Provider::Deepseek),
        "DEEPSEEK_API_KEY"
    );
    assert_eq!(
        stock_predictor::config::key_env_name(Provider::Gemini),
        "GEMINI_API_KEY"
    );
}
