use stock_predicter::db;
use stock_predicter::rules::runtime::{self, Runtime};

async fn fresh() -> (tempfile::TempDir, sqlx::SqlitePool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    let pool = db::connect_and_migrate(path.to_str().unwrap())
        .await
        .unwrap();
    (dir, pool)
}

#[tokio::test]
async fn an_empty_table_yields_the_documented_defaults() {
    let (_d, pool) = fresh().await;
    sqlx::query("DELETE FROM settings").execute(&pool).await.unwrap();
    assert_eq!(runtime::load(&pool).await.unwrap(), Runtime::default());
}

#[tokio::test]
async fn the_migration_rows_match_the_defaults() {
    let (_d, pool) = fresh().await;
    assert_eq!(runtime::load(&pool).await.unwrap(), Runtime::default());
}

#[tokio::test]
async fn a_stored_value_is_read_back() {
    let (_d, pool) = fresh().await;
    sqlx::query("UPDATE settings SET value = '600' WHERE key = 'poll_secs'")
        .execute(&pool)
        .await
        .unwrap();
    // The proxy has no default row: absent is the direct connection, so it must be inserted.
    sqlx::query(
        "INSERT OR REPLACE INTO settings (key, value, updated_at)          VALUES ('proxy', 'http://127.0.0.1:3128', datetime('now'))",
    )
    .execute(&pool)
    .await
    .unwrap();
    let r = runtime::load(&pool).await.unwrap();
    assert_eq!(r.poll_secs, 600);
    assert_eq!(r.proxy.as_deref(), Some("http://127.0.0.1:3128"));
}

#[tokio::test]
async fn a_corrupt_row_does_not_fail_the_read() {
    let (_d, pool) = fresh().await;
    sqlx::query("UPDATE settings SET value = 'whenever' WHERE key = 'off_hours_secs'")
        .execute(&pool)
        .await
        .unwrap();
    let r = runtime::load(&pool).await.unwrap();
    assert_eq!(r.off_hours_secs, 7_200);
}
