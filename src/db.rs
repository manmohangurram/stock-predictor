use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

pub const SCHEMA: &str = include_str!("../migrations/0001_init.sql");

pub fn path_of(path: &str) -> &str {
    path.strip_prefix("sqlite://")
        .or_else(|| path.strip_prefix("sqlite:"))
        .unwrap_or(path)
}

pub async fn connect(path: &str) -> Result<SqlitePool> {
    let opts = SqliteConnectOptions::new()
        .filename(path_of(path))
        .create_if_missing(true)
        .foreign_keys(true);

    SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await
        .context("connect sqlite")
}

pub async fn migrate(pool: &SqlitePool) -> Result<()> {
    sqlx::raw_sql(SCHEMA)
        .execute(pool)
        .await
        .context("apply schema")?;
    Ok(())
}

pub async fn connect_and_migrate(path: &str) -> Result<SqlitePool> {
    let pool = connect(path).await?;
    migrate(&pool).await?;
    Ok(pool)
}
