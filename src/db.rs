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
    (
        "0009_runtime_settings",
        include_str!("../migrations/0009_runtime_settings.sql"),
    ),
];

pub fn path_of(path: &str) -> &str {
    path.strip_prefix("sqlite://")
        .or_else(|| path.strip_prefix("sqlite:"))
        .unwrap_or(path)
}

/// Move a database written under an older name to the current one.
///
/// Rule: **the file with the old name is the real one**, so it becomes the file with the new name.
/// If something already sits at the new path it is moved aside first, never deleted — an empty
/// database created by an earlier version must not hide the ledger that is still under the old name.
///
/// The WAL is checkpointed before the move: renaming only the `.db` would silently drop the commits
/// still in `-wal`.
pub async fn migrate_legacy_path(old: &str, new: &str) -> Result<Option<String>> {
    let (old, new) = (path_of(old), path_of(new));
    if !std::path::Path::new(old).exists() {
        return Ok(None);
    }

    if std::path::Path::new(new).exists() {
        let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
        let aside = format!("{new}.bak-{stamp}");
        std::fs::rename(new, &aside).with_context(|| format!("move {new} aside"))?;
        tracing::info!(at = %aside, "the database at the new name is kept as a backup");
    }

    // Fold the WAL back into the file before moving it.
    let pool = connect(old).await?;
    let _ = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)").execute(&pool).await;
    pool.close().await;

    if let Some(dir) = std::path::Path::new(new).parent() {
        if !dir.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(dir);
        }
    }
    if std::fs::rename(old, new).is_err() {
        std::fs::copy(old, new).context("copy the old database")?;
        tracing::warn!(left_in_place = old, "copied instead of moved");
    }
    for suffix in ["-wal", "-shm"] {
        let stale = format!("{old}{suffix}");
        if std::path::Path::new(&stale).exists() {
            let _ = std::fs::remove_file(stale);
        }
    }
    Ok(Some(new.to_string()))
}

pub async fn connect(path: &str) -> Result<SqlitePool> {
    // Create the parent directory: a missing folder should not look like a corrupt database.
    if let Some(dir) = std::path::Path::new(path_of(path)).parent() {
        if !dir.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(dir);
        }
    }
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
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql.to_string()))
            .execute(pool)
            .await
            .with_context(|| format!("apply migration {id}"))?;
        sqlx::query("INSERT INTO schema_migrations (id, applied_at) VALUES (?, ?)")
            .bind(id)
            .bind(chrono::Utc::now().to_rfc3339())
            .execute(pool)
            .await?;
    }

    // Cheap planner statistics for the indexes above; SQLite recommends this after schema or
    // data changes and it is a no-op when there is nothing to do.
    let _ = sqlx::raw_sql("PRAGMA optimize").execute(pool).await;
    Ok(())
}

pub async fn connect_and_migrate(path: &str) -> Result<SqlitePool> {
    let pool = connect(path).await?;
    migrate(&pool).await?;
    Ok(pool)
}
