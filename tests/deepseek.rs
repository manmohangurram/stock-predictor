use stock_predicter::config::Config;
use stock_predicter::db;
use stock_predicter::llm::deepseek::{self, ChatRequest, FixtureTransport, HttpReply};

const DECIDE: &str = include_str!("fixtures/deepseek_decide.json");
const SEARCH: &str = include_str!("fixtures/deepseek_search.json");

async fn pool() -> (tempfile::TempDir, sqlx::SqlitePool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    let pool = db::connect_and_migrate(path.to_str().unwrap()).await.unwrap();
    (dir, pool)
}

fn cfg() -> Config {
    Config::from_map(
        &[
            ("DEEPSEEK_API_KEY", "k"),
            ("DEEPSEEK_BASE_URL", "https://api.deepseek.com"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect(),
    )
    .unwrap()
}

fn req() -> ChatRequest {
    ChatRequest {
        conversation_id: "conv-1".into(),
        round: 1,
        prompt: "decide".into(),
        prompt_template: "decide_v1".into(),
        prompt_hash: "h".into(),
        purpose: "test".into(),
    }
}

async fn rows(pool: &sqlx::SqlitePool) -> Vec<(String, Option<i64>, Option<String>, Option<String>)> {
    sqlx::query_as("SELECT endpoint, http_status, error, response_json FROM api_calls ORDER BY id")
        .fetch_all(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn every_chat_call_writes_one_api_calls_row() {
    let (_d, pool) = pool().await;
    let t = FixtureTransport::ok(DECIDE);
    let reply = deepseek::chat(&pool, &cfg(), &t, req()).await.unwrap();
    assert!(reply.content.contains("pass"));
    assert_eq!(reply.prompt_tokens, 812);

    let r = rows(&pool).await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].0, "chat");
    assert_eq!(r[0].1, Some(200));
    assert!(r[0].2.is_none());
}

#[tokio::test]
async fn malformed_json_body_is_an_error_not_a_panic() {
    let (_d, pool) = pool().await;
    let t = FixtureTransport::ok("not json at all");
    assert!(deepseek::chat(&pool, &cfg(), &t, req()).await.is_err());
    assert_eq!(rows(&pool).await.len(), 1, "the failed call is still audited");
}

#[tokio::test]
async fn api_error_row_is_still_written() {
    let (_d, pool) = pool().await;
    let t = FixtureTransport::new(vec![HttpReply {
        status: 402,
        body: "{\"error\":\"insufficient balance\"}".into(),
    }]);
    assert!(deepseek::chat(&pool, &cfg(), &t, req()).await.is_err());
    let r = rows(&pool).await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].1, Some(402));
    assert!(r[0].2.as_deref().unwrap().contains("402"));
}

#[tokio::test]
async fn search_maps_summary_and_date() {
    let (_d, pool) = pool().await;
    let t = FixtureTransport::ok(SEARCH);
    let hits = deepseek::search(&pool, &cfg(), &t, "XYZ IPO review").await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].title, "XYZ IPO review");
    assert!(hits[0].summary.starts_with("Analysts note"));
    assert_eq!(hits[0].date.as_deref(), Some("2026-10-24T00:00:00+00:00"));
    assert_eq!(rows(&pool).await[0].0, "search");
}

#[tokio::test]
async fn fetch_returns_markdown_and_is_audited() {
    let (_d, pool) = pool().await;
    let t = FixtureTransport::ok(r##"{"title":"t","content":"# Heading\nbody"}"##);
    let md = deepseek::fetch_page(&pool, &cfg(), &t, "https://example.com/a").await.unwrap();
    assert!(md.contains("# Heading"));
    assert_eq!(rows(&pool).await[0].0, "fetch");
}
