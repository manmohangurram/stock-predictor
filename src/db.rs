use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

pub const MIGRATIONS: &[(&str, &str)] = &[
    ("0001_init", include_str!("../migrations/0001_init.sql")),
    (
        "0002_llm_memos",
        include_str!("../migrations/0002_llm_memos.sql"),
    ),
    ("0003_trades", include_str!("../migrations/0003_trades.sql")),
    ("0004_modes", include_str!("../migrations/0004_modes.sql")),
    (
        "0005_signal_confidence",
        include_str!("../migrations/0005_signal_confidence.sql"),
    ),
    (
        "0006_target_profit",
        include_str!("../migrations/0006_target_profit.sql"),
    ),
    (
        "0007_call_gate",
        include_str!("../migrations/0007_call_gate.sql"),
    ),
    ("0008_indexes", include_str!("../migrations/0008_indexes.sql")),
];

pub fn path_of(path: &str) -> &str {
    path.strip_prefix("sqlite://")
        .or_else(|| path.strip_prefix("sqlite:"))
        .unwrap_or(path)
}

pub async fn connect(path: &str) -> Result<SqlitePool> {
    let opts = SqliteConnectOptions::new()
        .filename(path_of(path))
        .create_if_missing(true)
        .foreign_keys(true)
        // One writer, reads on every page load: WAL plus relaxed sync is the right trade and
        // takes a 250-row batch from 18 ms to a few.
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal);

    SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await
        .context("connect sqlite")
}

/// Ordered, applied once each, tracked in `schema_migrations`. A single idempotent file was
/// enough until a migration needed \`ALTER TABLE\`, which cannot be re-run.
pub async fn migrate(pool: &SqlitePool) -> Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE IF NOT EXISTS schema_migrations (id TEXT PRIMARY KEY, applied_at TEXT NOT NULL)",
    )
    .execute(pool)
    .await
    .context("create schema_migrations")?;

    for (id, sql) in MIGRATIONS {
        let done: Option<(String,)> =
            sqlx::query_as("SELECT id FROM schema_migrations WHERE id = ?")
                .bind(id)
                .fetch_optional(pool)
                .await?;
        if done.is_some() {
            continue;
        }
        sqlx::raw_sql(sql)
            .execute(pool)
            .await
            .with_context(|| format!("apply migration {id}"))?;
        sqlx::query("INSERT INTO schema_migrations (id, applied_at) VALUES (?, ?)")
            .bind(id)
            .bind(chrono::Utc::now().to_rfc3339())
            .execute(pool)
            .await?;
    }
    Ok(())
}

pub async fn connect_and_migrate(path: &str) -> Result<SqlitePool> {
    let pool = connect(path).await?;
    migrate(&pool).await?;
    Ok(pool)
}
