use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use stock_predicter::api::{self, AppState};
use stock_predicter::config::Config;
use stock_predicter::db;
use stock_predicter::rules::settings;

fn cfg() -> Config {
    Config::from_map(
        &[("DEEPSEEK_API_KEY", "k"), ("BUDGET", "100000")]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    )
    .unwrap()
}

async fn setup() -> (tempfile::TempDir, axum::Router, sqlx::SqlitePool) {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect_and_migrate(dir.path().join("t.db").to_str().unwrap())
        .await
        .unwrap();
    settings::seed(&pool, 100_000.0, "INR", "").await.unwrap();
    for sym in ["INFY", "TCS", "XYZIPO"] {
        sqlx::query(
            "INSERT INTO instruments (symbol, class, name, currency) VALUES (?, 'stock', ?, 'INR')",
        )
        .bind(sym)
        .bind(format!("{sym} Ltd"))
        .execute(&pool)
        .await
        .unwrap();
    }
    let router = api::router(AppState { pool: pool.clone(), cfg: cfg() });
    (dir, router, pool)
}

async fn call(router: &axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    use tower::ServiceExt;
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|_| {
            Value::String(String::from_utf8_lossy(&bytes).to_string())
        })
    };
    (status, value)
}

async fn get(router: &axum::Router, uri: &str) -> (StatusCode, Value) {
    call(
        router,
        Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
            .unwrap(),
    )
    .await
}

async fn send(router: &axum::Router, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    call(
        router,
        Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await
}

#[tokio::test]
async fn health_is_ok() {
    let (_d, router, _pool) = setup().await;
    let (s, v) = get(&router, "/api/health").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["ok"], true);
}

#[tokio::test]
async fn dashboard_reports_available_committed_and_blocked() {
    let (_d, router, _pool) = setup().await;
    let (s, v) = send(
        &router,
        "POST",
        "/api/positions",
        json!({"symbol": "INFY", "qty": 10.0, "avg_cost": 142.0}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let (s, v) = get(&router, "/api/dashboard").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["currency"], "INR");
    assert_eq!(v["budget"], 100_000.0);
    assert_eq!(v["committed"], 1420.0);
    assert_eq!(v["available"], 98_580.0);
    assert_eq!(v["positions"].as_array().unwrap().len(), 1);
    assert!(v["blocked"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn post_position_updates_committed_cash() {
    let (_d, router, _pool) = setup().await;
    let (s, v) = send(
        &router,
        "POST",
        "/api/positions",
        json!({"symbol": "TCS", "qty": 4.0, "avg_cost": 500.0, "stop": 480.0}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["symbol"], "TCS");
    assert_eq!(v["stop"], 480.0);

    let (_, d) = get(&router, "/api/dashboard").await;
    assert_eq!(d["committed"], 2000.0);
}

#[tokio::test]
async fn post_position_beyond_budget_is_refused() {
    let (_d, router, _pool) = setup().await;
    let (s, v) = send(
        &router,
        "POST",
        "/api/positions",
        json!({"symbol": "INFY", "qty": 1000.0, "avg_cost": 200.0}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
}

#[tokio::test]
async fn patch_position_records_a_sell_in_the_ledger() {
    let (_d, router, _pool) = setup().await;
    let (_, p) = send(
        &router,
        "POST",
        "/api/positions",
        json!({"symbol": "INFY", "qty": 10.0, "avg_cost": 100.0}),
    )
    .await;
    let id = p["id"].as_i64().unwrap();
    let (s, v) = send(
        &router,
        "PATCH",
        &format!("/api/positions/{id}"),
        json!({"qty": 4.0}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let (_, d) = get(&router, "/api/dashboard").await;
    assert_eq!(d["committed"], 400.0);
}

#[tokio::test]
async fn post_ipo_reserves_without_creating_a_position() {
    let (_d, router, _pool) = setup().await;
    let (s, v) = send(
        &router,
        "POST",
        "/api/ipo",
        json!({"symbol": "XYZIPO", "qty": 150.0, "price": 100.0}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["reserved"], 15_000.0);

    let (_, d) = get(&router, "/api/dashboard").await;
    assert_eq!(d["committed"], 15_000.0);
    assert!(
        d["positions"].as_array().unwrap().is_empty(),
        "an IPO application is a reservation, not a holding"
    );
}

#[tokio::test]
async fn instrument_route_returns_headlines_without_bodies() {
    let (_d, router, pool) = setup().await;
    sqlx::query(
        "INSERT INTO news (instrument_id, published_at, title, publisher, url, body)
         VALUES (1, '2026-10-25T00:00:00Z', 'Guidance raised', 'Reuters', 'https://x/1', 'SECRET BODY')",
    )
    .execute(&pool)
    .await
    .unwrap();

    let (s, v) = get(&router, "/api/instruments/INFY").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["headlines"][0]["title"], "Guidance raised");
    assert!(
        !v.to_string().contains("SECRET BODY"),
        "article bodies must never leave the server"
    );
    assert_eq!(v["instrument"]["timing_note"], "limit order, intraday or next open");
}

#[tokio::test]
async fn calls_route_returns_audit_rows_newest_first() {
    let (_d, router, _pool) = setup().await;
    let (s, v) = get(&router, "/api/calls").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn put_settings_below_committed_returns_409_with_committed() {
    let (_d, router, _pool) = setup().await;
    let _ = send(
        &router,
        "POST",
        "/api/positions",
        json!({"symbol": "INFY", "qty": 10.0, "avg_cost": 142.0}),
    )
    .await;
    let (s, v) = send(&router, "PUT", "/api/settings", json!({"budget": 1000.0})).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"], "below_committed");
    assert_eq!(v["committed"], 1420.0);
}

#[tokio::test]
async fn put_settings_changes_currency_only_when_unlocked() {
    let (_d, router, _pool) = setup().await;
    let (s, v) = send(&router, "PUT", "/api/settings", json!({"currency": "USD"})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["currency"], "USD");

    let _ = send(
        &router,
        "POST",
        "/api/positions",
        json!({"symbol": "INFY", "qty": 1.0, "avg_cost": 10.0}),
    )
    .await;
    let (s, v) = send(&router, "PUT", "/api/settings", json!({"currency": "EUR"})).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"], "currency_locked");
}

#[tokio::test]
async fn put_settings_rejects_negative_budget_and_raises_it() {
    let (_d, router, _pool) = setup().await;
    let (s, _) = send(&router, "PUT", "/api/settings", json!({"budget": -5.0})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    let (s, v) = send(&router, "PUT", "/api/settings", json!({"budget": 250000.0})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["budget"], 250_000.0);
    assert_eq!(v["available"], 250_000.0);
}

#[tokio::test]
async fn malformed_json_is_a_400_not_a_panic() {
    let (_d, router, _pool) = setup().await;
    let (s, _) = call(
        &router,
        Request::builder()
            .method("POST")
            .uri("/api/positions")
            .header("content-type", "application/json")
            .body(Body::from("{not json"))
            .unwrap(),
    )
    .await;
    assert!(s.is_client_error(), "got {s}");
}

#[tokio::test]
async fn analysis_explains_why_there_is_no_signal() {
    let (_d, router, pool) = setup().await;
    // 200 flat bars: enough to compute indicators, no trend, so nothing fires.
    for i in 0..200 {
        sqlx::query(
            "INSERT INTO bars (instrument_id, ts, open, high, low, close, volume)
             VALUES (1, ?, 100.0, 101.0, 99.0, 100.0, 1000)",
        )
        .bind(format!("2026-01-{:03}T00:00:00Z", i))
        .execute(&pool)
        .await
        .unwrap();
    }

    let (s, v) = get(&router, "/api/instruments/INFY/analysis").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["bars"].as_array().unwrap().len(), 120);
    let checks = v["checks"].as_array().unwrap();
    assert!(checks.len() >= 4);
    let failed: Vec<&str> = checks
        .iter()
        .filter(|c| c["passed"] == false)
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert!(failed.contains(&"trend"), "flat series must fail the trend rule: {failed:?}");
    assert_eq!(v["indicators"]["sma50"], 100.0);
}

#[tokio::test]
async fn analysis_of_an_instrument_with_no_bars_says_so() {
    let (_d, router, _pool) = setup().await;
    let (s, v) = get(&router, "/api/instruments/TCS/analysis").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["indicators"].is_null());
    assert!(v["checks"][0]["detail"].as_str().unwrap().contains("need 200"));
}

#[tokio::test]
async fn deepseek_key_can_be_set_from_the_ui_and_never_comes_back() {
    let (_d, router, _pool) = setup().await;
    let (s, v) = send(
        &router,
        "PUT",
        "/api/settings",
        json!({"deepseek_key": "sk-live-abcdef123456"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["deepseek_configured"], true);
    assert!(
        !v.to_string().contains("sk-live-abcdef123456"),
        "the PUT response leaked the key: {v}"
    );

    let (s, v) = get(&router, "/api/settings").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["deepseek_configured"], true);
    assert!(!v.to_string().contains("sk-live"), "GET leaked the key: {v}");
}

#[tokio::test]
async fn settings_report_no_key_when_none_is_configured() {
    let (_d, router, _pool) = setup().await;
    let (_, v) = get(&router, "/api/settings").await;
    assert_eq!(v["deepseek_configured"], false);
}
