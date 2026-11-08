use stock_predictor::db;
use stock_predictor::store;

mod common;
use common::FakeMarket;

async fn setup() -> (tempfile::TempDir, sqlx::SqlitePool, i64) {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    let id: (i64,) = sqlx::query_as(
        "INSERT INTO instruments (symbol, class, name, currency)
         VALUES ('INFY.NS', 'stock', 'Infosys', 'INR') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    (dir, pool, id.0)
}

#[tokio::test]
async fn refreshing_a_new_symbol_stores_prices_and_headlines_immediately() {
    let (_d, pool, id) = setup().await;
    let market = FakeMarket::ok(vec![
        common::bar("2026-10-24T03:45:00Z", 1150.2),
        common::bar("2026-10-27T03:45:00Z", 1163.0),
    ])
    .with_news(vec![common::headline(
        "Guidance raised",
        "https://example.com/1",
    )]);

    let (bars, news) = store::refresh(&pool, id, "INFY.NS", &market).await.unwrap();
    assert_eq!((bars, news), (2, 1));

    let stored: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM bars WHERE instrument_id = ?")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored.0, 2);
    let stored_news: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM news WHERE instrument_id = ?")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored_news.0, 1);
}

#[tokio::test]
async fn the_same_headline_url_is_never_stored_twice() {
    let (_d, pool, id) = setup().await;
    let market = FakeMarket::ok(vec![]).with_news(vec![common::headline(
        "Guidance raised",
        "https://example.com/same",
    )]);
    store::refresh(&pool, id, "INFY.NS", &market).await.unwrap();
    store::refresh(&pool, id, "INFY.NS", &market).await.unwrap();
    let n: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM news")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n.0, 1);
}

#[tokio::test]
async fn a_symbol_with_no_series_fails_the_refresh_but_leaves_nothing_half_written() {
    let (_d, pool, id) = setup().await;
    let mut market = FakeMarket::empty();
    market.fail_bars = true;
    assert!(store::refresh(&pool, id, "NOSUCH.NS", &market)
        .await
        .is_err());
    let n: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM news")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        n.0, 0,
        "headlines are only stored after the series succeeds"
    );
}
