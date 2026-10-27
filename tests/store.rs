use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use stock_predicter::db;
use stock_predicter::market::MarketSource;
use stock_predicter::models::{Bar, Headline};
use stock_predicter::store;

struct Fake {
    bars: Vec<Bar>,
    news: Vec<Headline>,
    bars_fail: bool,
}

impl MarketSource for Fake {
    fn bars<'a>(
        &'a self,
        _s: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Bar>>> + Send + 'a>> {
        let out = if self.bars_fail {
            Err(anyhow::anyhow!("no series for this symbol"))
        } else {
            Ok(self.bars.clone())
        };
        Box::pin(async move { out })
    }
    fn intraday<'a>(
        &'a self,
        _s: &'a str,
        _i: &'a str,
        _r: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Bar>>> + Send + 'a>> {
        Box::pin(async move { Ok(vec![]) })
    }
    fn news<'a>(
        &'a self,
        _s: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Headline>>> + Send + 'a>> {
        let n = self.news.clone();
        Box::pin(async move { Ok(n) })
    }
}

fn bar(ts: &str, close: f64) -> Bar {
    Bar {
        ts: ts.into(),
        open: close,
        high: close,
        low: close,
        close,
        volume: 1,
    }
}

fn headline(url: &str) -> Headline {
    Headline {
        id: 0,
        title: "Guidance raised".into(),
        publisher: "Reuters".into(),
        published_at: "2026-10-25T00:00:00Z".into(),
        url: url.into(),
    }
}

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
    let market = Fake {
        bars: vec![bar("2026-10-24T03:45:00Z", 1150.2), bar("2026-10-27T03:45:00Z", 1163.0)],
        news: vec![headline("https://example.com/1")],
        bars_fail: false,
    };

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
    let market = Fake {
        bars: vec![],
        news: vec![headline("https://example.com/same")],
        bars_fail: false,
    };
    store::refresh(&pool, id, "INFY.NS", &market).await.unwrap();
    store::refresh(&pool, id, "INFY.NS", &market).await.unwrap();
    let n: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM news").fetch_one(&pool).await.unwrap();
    assert_eq!(n.0, 1);
}

#[tokio::test]
async fn a_symbol_with_no_series_fails_the_refresh_but_leaves_nothing_half_written() {
    let (_d, pool, id) = setup().await;
    let market = Fake {
        bars: vec![],
        news: vec![headline("https://example.com/1")],
        bars_fail: true,
    };
    assert!(store::refresh(&pool, id, "NOSUCH.NS", &market).await.is_err());
    let n: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM news").fetch_one(&pool).await.unwrap();
    assert_eq!(n.0, 0, "headlines are only stored after the series succeeds");
}
