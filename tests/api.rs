use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use stock_predictor::api::{self, AppState};
use stock_predictor::config::Config;
use stock_predictor::db;
use stock_predictor::rules::settings;

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
    settings::seed(&pool).await.unwrap();
    settings::set(
        &pool,
        settings::SettingsPatch {
            budget: Some(100_000.0),
            ..Default::default()
        },
    )
    .await
    .unwrap();
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
    let router = api::router(AppState::new(pool.clone(), cfg()));
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
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).to_string()))
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
    assert_eq!(
        v["instrument"]["timing_note"],
        "limit order, intraday or next open"
    );
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
    assert!(
        failed.contains(&"trend"),
        "flat series must fail the trend rule: {failed:?}"
    );
    assert_eq!(v["indicators"]["sma50"], 100.0);
}

#[tokio::test]
async fn analysis_of_an_instrument_with_no_bars_says_so() {
    let (_d, router, _pool) = setup().await;
    let (s, v) = get(&router, "/api/instruments/TCS/analysis").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["indicators"].is_null());
    assert!(v["checks"][0]["detail"]
        .as_str()
        .unwrap()
        .contains("need 200"));
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
    assert!(
        !v.to_string().contains("sk-live"),
        "GET leaked the key: {v}"
    );
}

#[tokio::test]
async fn settings_report_no_key_when_none_is_configured() {
    let (_d, router, _pool) = setup().await;
    let (_, v) = get(&router, "/api/settings").await;
    assert_eq!(v["deepseek_configured"], false);
}

#[tokio::test]
async fn recording_a_buy_then_a_sell_writes_history_and_realised_pnl() {
    let (_d, router, _pool) = setup().await;
    let (s, v) = send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side": "buy", "qty": 10.0, "price": 100.0}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["realised"], 0.0);

    let (_, d) = get(&router, "/api/dashboard").await;
    assert_eq!(d["committed"], 1000.0);

    // Sell half at a profit: realised = 5 * (120 - 100)
    let (s, v) = send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side": "sell", "qty": 5.0, "price": 120.0}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["realised"], 100.0);

    let (_, d) = get(&router, "/api/dashboard").await;
    assert_eq!(d["committed"], 500.0);
    assert_eq!(d["realised"], 100.0);

    let (s, v) = get(&router, "/api/instruments/INFY/trades").await;
    assert_eq!(s, StatusCode::OK);
    let rows = v.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["side"], "sell");
    assert_eq!(rows[0]["realised"], 100.0);
    assert_eq!(rows[1]["side"], "buy");
}

#[tokio::test]
async fn selling_more_than_is_held_is_refused() {
    let (_d, router, _pool) = setup().await;
    let _ = send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side": "buy", "qty": 2.0, "price": 100.0}),
    )
    .await;
    let (s, v) = send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side": "sell", "qty": 5.0, "price": 100.0}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
}

#[tokio::test]
async fn a_buy_beyond_the_budget_is_refused_and_records_nothing() {
    let (_d, router, _pool) = setup().await;
    let (s, _) = send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side": "buy", "qty": 1000.0, "price": 200.0}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (_, v) = get(&router, "/api/instruments/INFY/trades").await;
    assert_eq!(v.as_array().unwrap().len(), 0, "no half-written trade");
}

#[tokio::test]
async fn adding_an_instrument_makes_it_visible_and_idempotent() {
    let (_d, router, _pool) = setup().await;
    let (s, v) = send(
        &router,
        "POST",
        "/api/instruments",
        json!({"code": "WIPRO.NS", "name": "Wipro"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["symbol"], "WIPRO.NS");
    assert_eq!(v["class"], "stock");

    let (s, _) = send(
        &router,
        "POST",
        "/api/instruments",
        json!({"code": "WIPRO.NS", "name": "Wipro"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "adding twice is not an error");
}

#[tokio::test]
async fn an_empty_search_returns_nothing_without_calling_out() {
    let (_d, router, _pool) = setup().await;
    let (s, v) = get(&router, "/api/search?q=i").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn a_backfilled_trade_keeps_the_date_it_happened() {
    let (_d, router, _pool) = setup().await;
    let (s, v) = send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side": "buy", "qty": 10.0, "price": 90.0, "at": "2026-07-14"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let (_, rows) = get(&router, "/api/instruments/INFY/trades").await;
    let at = rows[0]["traded_at"].as_str().unwrap();
    assert!(at.starts_with("2026-07-14"), "date preserved, got {at}");
    // YYYY-MM-DD means that session's 15:30 IST close.
    assert!(at.contains("10:00:00"), "stamped at the close, got {at}");

    // The position must carry the same date, or history sorts wrong.
    let (_, d) = get(&router, "/api/dashboard").await;
    assert_eq!(d["positions"][0]["symbol"], "INFY");
    assert_eq!(d["committed"], 900.0);
}

#[tokio::test]
async fn an_rfc3339_timestamp_is_accepted_to_the_second() {
    let (_d, router, _pool) = setup().await;
    let (s, _) = send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side": "buy", "qty": 1.0, "price": 100.0, "at": "2026-08-03T09:41:00Z"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, rows) = get(&router, "/api/instruments/INFY/trades").await;
    assert!(rows[0]["traded_at"]
        .as_str()
        .unwrap()
        .starts_with("2026-08-03T09:41:00"));
}

#[tokio::test]
async fn a_future_or_unparseable_date_is_refused() {
    let (_d, router, _pool) = setup().await;
    for bad in ["2099-01-01", "yesterday", "2026-13-45"] {
        let (s, v) = send(
            &router,
            "POST",
            "/api/instruments/INFY/trades",
            json!({"side": "buy", "qty": 1.0, "price": 100.0, "at": bad}),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad} should be refused: {v}");
    }
    let (_, rows) = get(&router, "/api/instruments/INFY/trades").await;
    assert_eq!(rows.as_array().unwrap().len(), 0, "nothing half-written");
}

#[tokio::test]
async fn an_omitted_date_still_means_now() {
    let (_d, router, _pool) = setup().await;
    let (s, _) = send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side": "buy", "qty": 1.0, "price": 100.0}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, rows) = get(&router, "/api/instruments/INFY/trades").await;
    let at = rows[0]["traded_at"].as_str().unwrap();
    assert!(at.starts_with(&chrono::Utc::now().format("%Y-%m-%d").to_string()));
}

#[tokio::test]
async fn a_sell_cannot_be_dated_before_the_position_was_opened() {
    let (_d, router, _pool) = setup().await;
    let (s, _) = send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side": "buy", "qty": 10.0, "price": 100.0, "at": "2026-08-01"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // Selling in July something bought in August is a history that runs backwards.
    let (s, v) = send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side": "sell", "qty": 5.0, "price": 120.0, "at": "2026-07-01"}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"]
        .as_str()
        .unwrap_or_default()
        .contains("before the position"));

    // The same sale, dated after the buy, is fine.
    let (s, _) = send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side": "sell", "qty": 5.0, "price": 120.0, "at": "2026-09-01"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn a_backfilled_buy_moves_the_position_start_date_backwards() {
    let (_d, router, _pool) = setup().await;
    let _ = send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side": "buy", "qty": 10.0, "price": 100.0}),
    )
    .await;
    // Entered later, happened earlier.
    let (s, _) = send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side": "buy", "qty": 5.0, "price": 90.0, "at": "2026-06-10"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // A sale between the backfilled buy and today is now legal.
    let (s, v) = send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side": "sell", "qty": 5.0, "price": 120.0, "at": "2026-07-01"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

#[tokio::test]
async fn the_two_pots_never_draw_on_each_other() {
    let (_d, router, _pool) = setup().await;
    // Give trading its own pot.
    let (s, v) = send(
        &router,
        "PUT",
        "/api/settings",
        json!({"trading_budget": 20000.0}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // Spend the whole trading pot.
    let (s, v) = send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side": "buy", "qty": 200.0, "price": 100.0, "mode": "trade"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // Long-term is untouched, and still has its own full budget.
    let (_, long) = get(&router, "/api/dashboard?mode=long").await;
    assert_eq!(long["committed"], 0.0);
    assert_eq!(long["budget"], 100_000.0);
    assert_eq!(long["positions"].as_array().unwrap().len(), 0);

    let (_, trade) = get(&router, "/api/dashboard?mode=trade").await;
    assert_eq!(trade["mode"], "trade");
    assert_eq!(trade["budget"], 20_000.0);
    assert_eq!(trade["committed"], 20_000.0);
    assert_eq!(trade["available"], 0.0);
    assert_eq!(trade["positions"].as_array().unwrap().len(), 1);

    // A trading buy cannot dip into the long-term pot even though that pot is full.
    let (s, v) = send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side": "buy", "qty": 1.0, "price": 100.0, "mode": "trade"}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
}

#[tokio::test]
async fn the_same_symbol_can_be_held_in_both_pots_separately() {
    let (_d, router, _pool) = setup().await;
    let _ = send(
        &router,
        "PUT",
        "/api/settings",
        json!({"trading_budget": 50_000.0}),
    )
    .await;
    let _ = send(
        &router,
        "POST",
        "/api/instruments/TCS/trades",
        json!({"side": "buy", "qty": 2.0, "price": 1000.0, "mode": "long"}),
    )
    .await;
    let _ = send(
        &router,
        "POST",
        "/api/instruments/TCS/trades",
        json!({"side": "buy", "qty": 5.0, "price": 900.0, "mode": "trade"}),
    )
    .await;

    let (_, long) = get(&router, "/api/dashboard?mode=long").await;
    let (_, trade) = get(&router, "/api/dashboard?mode=trade").await;
    assert_eq!(long["positions"][0]["qty"], 2.0);
    assert_eq!(trade["positions"][0]["qty"], 5.0);

    let (_, all) = get(&router, "/api/trades").await;
    let rows = all.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|r| r["mode"] == "long"));
    assert!(rows.iter().any(|r| r["mode"] == "trade"));
}

#[tokio::test]
async fn the_dashboard_answers_304_when_nothing_changed() {
    let (_d, router, _pool) = setup().await;

    // First GET: a body and an ETag.
    use tower::ServiceExt;
    let first = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/dashboard")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let etag = first
        .headers()
        .get("etag")
        .expect("etag header")
        .to_str()
        .unwrap()
        .to_string();

    // Same tag back: 304, no body.
    let second = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/dashboard")
                .header("if-none-match", &etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::NOT_MODIFIED);
    let bytes = second.into_body().collect().await.unwrap().to_bytes();
    assert!(bytes.is_empty(), "a 304 must not carry a body");

    // Change something, and the tag must move.
    let _ = send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side": "buy", "qty": 1.0, "price": 100.0}),
    )
    .await;
    let third = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/dashboard")
                .header("if-none-match", &etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(third.status(), StatusCode::OK, "stale tag must get fresh data");
}

#[tokio::test]
async fn settings_expose_runtime_defaults_and_no_proxy_secret() {
    let (_d, router, _pool) = setup().await;
    let (s, v) = get(&router, "/api/settings").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let r = &v["runtime"];
    assert_eq!(r["poll_secs"], 900);
    assert_eq!(r["off_hours_secs"], 7200);
    assert_eq!(r["prompt_window_hours"], 4);
    assert_eq!(r["min_price_move"], 2.0);
    // No time floor by default: a call is for a change, not for the clock.
    assert_eq!(r["max_silence_secs"], 0);
    assert_eq!(r["daily_cap"], 200);
    assert_eq!(r["proxy_configured"], false);
    assert_eq!(r["proxy_host"], "");
}

#[tokio::test]
async fn a_runtime_patch_persists_and_echoes() {
    let (_d, router, _pool) = setup().await;
    let (s, v) = send(
        &router,
        "PUT",
        "/api/settings",
        json!({"poll_secs": 600, "daily_cap": 40, "prompt_window_hours": 2}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["runtime"]["poll_secs"], 600);
    assert_eq!(v["runtime"]["daily_cap"], 40);
    assert_eq!(v["runtime"]["prompt_window_hours"], 2);
    // Untouched knobs keep their values: a per-tab stamp sends only its own fields.
    assert_eq!(v["runtime"]["off_hours_secs"], 7200);
}

#[tokio::test]
async fn out_of_range_runtime_values_are_rejected_with_the_range() {
    let (_d, router, _pool) = setup().await;
    let (s, v) = send(&router, "PUT", "/api/settings", json!({"poll_secs": 5})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().unwrap().contains("60"), "{v}");

    let (s, _) = send(&router, "PUT", "/api/settings", json!({"prompt_window_hours": 99})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn proxy_credentials_are_never_echoed() {
    let (_d, router, _pool) = setup().await;
    let (s, v) = send(
        &router,
        "PUT",
        "/api/settings",
        json!({"proxy": "http://mohan:hunter2@10.0.0.2:3128", "proxy_bypass": "localhost"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["runtime"]["proxy_configured"], true);
    assert_eq!(v["runtime"]["proxy_host"], "10.0.0.2:3128");
    assert!(!v.to_string().contains("hunter2"), "the secret leaked: {v}");

    // An empty string clears it.
    let (s, v) = send(&router, "PUT", "/api/settings", json!({"proxy": ""})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["runtime"]["proxy_configured"], false);
}

#[tokio::test]
async fn every_secret_is_unreadable_in_the_database() {
    let (_d, router, pool) = setup().await;
    let (s, _) = send(
        &router,
        "PUT",
        "/api/settings",
        json!({
            "llm_key": "sk-plaintext-hunter2",
            "ntfy_token": "tk_plaintext_token",
            "proxy": "http://10.0.0.2:3128",
            "proxy_user": "mohan",
            "proxy_pass": "proxy-hunter2",
        }),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let rows: Vec<(String, String)> = sqlx::query_as("SELECT key, value FROM settings")
        .fetch_all(&pool)
        .await
        .unwrap();
    let dump = serde_json::to_string(&rows).unwrap();
    for secret in ["sk-plaintext-hunter2", "tk_plaintext_token", "proxy-hunter2"] {
        assert!(!dump.contains(secret), "{secret} is stored in the clear: {dump}");
    }
    // …and they still work when read back through the secret reader.
    let key = stock_predictor::secrets::get_secret(&pool, "deepseek_key").await.unwrap();
    assert_eq!(key.as_deref(), Some("sk-plaintext-hunter2"));
    let pass = stock_predictor::secrets::get_secret(&pool, "proxy_pass").await.unwrap();
    assert_eq!(pass.as_deref(), Some("proxy-hunter2"));

    // The API says what is configured, never the values.
    let (_, v) = get(&router, "/api/settings").await;
    assert_eq!(v["runtime"]["proxy_user"], "mohan");
    assert_eq!(v["runtime"]["proxy_pass_configured"], true);
    let body = v.to_string();
    assert!(!body.contains("proxy-hunter2") && !body.contains("sk-plaintext"), "{body}");
}

#[tokio::test]
async fn a_proxy_password_survives_a_patch_that_leaves_it_blank() {
    let (_d, router, pool) = setup().await;
    send(
        &router,
        "PUT",
        "/api/settings",
        json!({"proxy": "http://10.0.0.2:3128", "proxy_user": "mohan", "proxy_pass": "s3cret"}),
    )
    .await;
    // Stamping the tab again without retyping the password must keep it.
    let (s, v) = send(
        &router,
        "PUT",
        "/api/settings",
        json!({"proxy": "http://10.0.0.3:3128", "proxy_user": "mohan", "proxy_pass": ""}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let pass = stock_predictor::secrets::get_secret(&pool, "proxy_pass").await.unwrap();
    assert_eq!(pass.as_deref(), Some("s3cret"), "the password was wiped by a blank field");

    // Clearing the URL clears the credentials with it.
    send(&router, "PUT", "/api/settings", json!({"proxy": ""})).await;
    let pass = stock_predictor::secrets::get_secret(&pool, "proxy_pass").await.unwrap();
    assert_eq!(pass, None);
}

#[tokio::test]
async fn a_proxy_that_is_not_a_url_is_refused() {
    let (_d, router, _pool) = setup().await;
    let (s, v) = send(&router, "PUT", "/api/settings", json!({"proxy": "10.0.0.2:3128"})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().unwrap().contains("http://"), "{v}");
}

#[tokio::test]
async fn a_mutual_fund_cannot_be_filled_into_the_trading_pot() {
    let (_d, router, _pool) = setup().await;
    sqlx::query(
        "INSERT INTO instruments (symbol, class, name, currency) VALUES ('AXISELSS', 'mutual_fund', 'Axis ELSS', 'INR')",
    )
    .execute(&_pool)
    .await
    .unwrap();

    let (s, v) = send(
        &router,
        "POST",
        "/api/instruments/AXISELSS/trades",
        json!({"side": "buy", "qty": 10.0, "price": 55.0, "mode": "trade"}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("long-term only"), "{v}");

    // The long-term pot is the point of it.
    let (s, v) = send(
        &router,
        "POST",
        "/api/instruments/AXISELSS/trades",
        json!({"side": "buy", "qty": 10.0, "price": 55.0, "mode": "long"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

#[tokio::test]
async fn a_symbol_without_history_is_deleted_with_its_bars() {
    let (_d, router, pool) = setup().await;
    let id: (i64,) = sqlx::query_as("SELECT id FROM instruments WHERE symbol = 'TCS'")
        .fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO bars (instrument_id, ts, open, high, low, close, volume) VALUES (?, '2026-11-06T00:00:00Z', 1,1,1,1,1)")
        .bind(id.0).execute(&pool).await.unwrap();

    let (s, v) = send(&router, "DELETE", "/api/instruments/TCS", json!({})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["deleted"], true);
    let left: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM instruments WHERE symbol = 'TCS'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(left.0, 0);
    let bars: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM bars WHERE instrument_id = ?")
        .bind(id.0).fetch_one(&pool).await.unwrap();
    assert_eq!(bars.0, 0, "orphaned bars");
}

#[tokio::test]
async fn a_held_symbol_cannot_be_deleted() {
    let (_d, router, _pool) = setup().await;
    let (s, _) = send(&router, "POST", "/api/instruments/INFY/trades",
        json!({"side":"buy","qty":5.0,"price":100.0,"mode":"long"})).await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = send(&router, "DELETE", "/api/instruments/INFY", json!({})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("held"), "{v}");
}

#[tokio::test]
async fn a_symbol_with_fills_is_archived_not_deleted() {
    let (_d, router, pool) = setup().await;
    send(&router, "POST", "/api/instruments/INFY/trades",
        json!({"side":"buy","qty":5.0,"price":100.0,"mode":"long"})).await;
    send(&router, "POST", "/api/instruments/INFY/trades",
        json!({"side":"sell","qty":5.0,"price":110.0,"mode":"long"})).await;

    let (s, v) = send(&router, "DELETE", "/api/instruments/INFY", json!({})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["archived"], true, "{v}");
    assert_eq!(v["deleted"], false);

    let active: (i64,) = sqlx::query_as("SELECT active FROM instruments WHERE symbol = 'INFY'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(active.0, 0, "it should be hidden from polling");
    let fills: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM trades t JOIN instruments i ON i.id = t.instrument_id WHERE i.symbol = 'INFY'",
    )
    .fetch_one(&pool).await.unwrap();
    assert_eq!(fills.0, 2, "history must survive");
}

#[tokio::test]
async fn approving_a_decision_records_the_fill_you_actually_got() {
    let (_d, router, pool) = setup().await;
    // A pending BUY decision, as the engine would have written it.
    let id: (i64,) = sqlx::query_as(
        "INSERT INTO signals (instrument_id, action, qty, entry, stop, target, valid_until,
            reason_json, created_at, mode)
         SELECT id, 'BUY', 10, 100.0, 95.0, 120.0, '2030-01-01T00:00:00Z', '{}', '2026-11-08T10:00:00Z', 'long'
         FROM instruments WHERE symbol = 'INFY' RETURNING id",
    )
    .fetch_one(&pool).await.unwrap();

    // Edited numbers: you got 8 units at 99.50, not 10 at 100.
    let (s, v) = send(&router, "POST", &format!("/api/signals/{}/approve", id.0),
        json!({"qty": 8.0, "price": 99.5})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "approved");
    assert_eq!(v["filled_qty"], 8.0);

    let (status, fq, fp): (String, Option<f64>, Option<f64>) = sqlx::query_as(
        "SELECT status, filled_qty, filled_price FROM signals WHERE id = ?",
    ).bind(id.0).fetch_one(&pool).await.unwrap();
    assert_eq!(status, "approved");
    assert_eq!(fq, Some(8.0));
    assert_eq!(fp, Some(99.5));

    let held: (f64,) = sqlx::query_as(
        "SELECT qty FROM positions p JOIN instruments i ON i.id = p.instrument_id WHERE i.symbol = 'INFY'",
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(held.0, 8.0, "the position must be the edited quantity");

    // Approving again must not double the position.
    let (s, v) = send(&router, "POST", &format!("/api/signals/{}/approve", id.0), json!({})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let held: (f64,) = sqlx::query_as(
        "SELECT qty FROM positions p JOIN instruments i ON i.id = p.instrument_id WHERE i.symbol = 'INFY'",
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(held.0, 8.0);
}

#[tokio::test]
async fn a_fill_that_cannot_be_recorded_leaves_the_decision_pending() {
    let (_d, router, pool) = setup().await;
    let id: (i64,) = sqlx::query_as(
        "INSERT INTO signals (instrument_id, action, qty, entry, valid_until, reason_json, created_at, mode)
         SELECT id, 'BUY', 10_000, 1000.0, '2030-01-01T00:00:00Z', '{}', '2026-11-08T10:00:00Z', 'long'
         FROM instruments WHERE symbol = 'INFY' RETURNING id",
    ).fetch_one(&pool).await.unwrap();

    let (s, v) = send(&router, "POST", &format!("/api/signals/{}/approve", id.0), json!({})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("exceeds available cash"), "{v}");

    let status: (String,) = sqlx::query_as("SELECT status FROM signals WHERE id = ?")
        .bind(id.0).fetch_one(&pool).await.unwrap();
    assert_eq!(status.0, "pending", "a failed fill must not consume the decision");
}

#[tokio::test]
async fn rejecting_a_decision_writes_a_reason_and_no_money() {
    let (_d, router, pool) = setup().await;
    let id: (i64,) = sqlx::query_as(
        "INSERT INTO signals (instrument_id, action, qty, entry, valid_until, reason_json, created_at, mode)
         SELECT id, 'SELL', 5, 100.0, '2030-01-01T00:00:00Z', '{}', '2026-11-08T10:00:00Z', 'long'
         FROM instruments WHERE symbol = 'INFY' RETURNING id",
    ).fetch_one(&pool).await.unwrap();
    let before: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM cash_ledger").fetch_one(&pool).await.unwrap();

    let (s, v) = send(&router, "POST", &format!("/api/signals/{}/reject", id.0),
        json!({"reason": "not at this price"})).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let (status, reason): (String, Option<String>) =
        sqlx::query_as("SELECT status, status_reason FROM signals WHERE id = ?")
            .bind(id.0).fetch_one(&pool).await.unwrap();
    assert_eq!(status, "rejected");
    assert_eq!(reason.as_deref(), Some("not at this price"));
    let after: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM cash_ledger").fetch_one(&pool).await.unwrap();
    assert_eq!(after.0, before.0, "a rejection must not touch the ledger");
}

#[tokio::test]
async fn the_ledger_shows_pending_decisions_only_unless_filtered() {
    let (_d, router, pool) = setup().await;
    for (action, status) in [("BUY", "pending"), ("SELL", "approved"), ("BUY", "rejected")] {
        sqlx::query(
            "INSERT INTO signals (instrument_id, action, qty, entry, valid_until, reason_json,
                created_at, mode, status)
             SELECT id, ?, 1, 100.0, '2030-01-01T00:00:00Z', '{}', '2026-11-08T10:00:00Z', 'long', ?
             FROM instruments WHERE symbol = 'INFY'",
        )
        .bind(action)
        .bind(status)
        .execute(&pool)
        .await
        .unwrap();
    }

    let (_, v) = get(&router, "/api/dashboard").await;
    let statuses: Vec<String> = v["signals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["status"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(statuses, vec!["pending"], "{statuses:?}");

    let (_, v) = get(&router, "/api/dashboard?decisions=approved").await;
    let statuses: Vec<String> = v["signals"]
        .as_array().unwrap().iter()
        .map(|s| s["status"].as_str().unwrap().to_string()).collect();
    assert_eq!(statuses, vec!["approved"], "{statuses:?}");
}

#[tokio::test]
async fn approving_a_rotation_posts_both_legs_together() {
    let (_d, router, pool) = setup().await;
    sqlx::query("INSERT OR IGNORE INTO instruments (symbol, class, name, currency) VALUES ('HDFCBANK','stock','HDFC Bank','INR')")
        .execute(&pool).await.unwrap();
    // Buy HDFC Bank first so there is something to sell, at a profit.
    send(&router, "POST", "/api/instruments/HDFCBANK/trades",
        json!({"side":"buy","qty":10.0,"price":100.0,"mode":"long"})).await;
    sqlx::query("INSERT INTO bars (instrument_id, ts, open, high, low, close, volume)
                 SELECT id, '2026-11-08T00:00:00Z', 150,151,149,150,1 FROM instruments WHERE symbol='HDFCBANK'")
        .execute(&pool).await.unwrap();

    let sell_id: (i64,) = sqlx::query_as(
        "INSERT INTO signals (instrument_id, action, qty, entry, valid_until, reason_json, created_at,
            mode, kind, status)
         SELECT id, 'SELL', 10, 150.0, '2030-01-01T00:00:00Z', '{}', '2026-11-08T10:00:00Z',
                'long', 'rotation', 'pending'
         FROM instruments WHERE symbol='HDFCBANK' RETURNING id",
    ).fetch_one(&pool).await.unwrap();
    let buy_id: (i64,) = sqlx::query_as(
        "INSERT INTO signals (instrument_id, action, qty, entry, stop, target, valid_until,
            reason_json, created_at, mode, kind, status, pair_id)
         SELECT id, 'BUY', 2, 500.0, 460.0, 600.0, '2030-01-01T00:00:00Z', '{}',
                '2026-11-08T10:00:00Z', 'long', 'rotation', 'pending', ?
         FROM instruments WHERE symbol='INFY' RETURNING id",
    ).bind(sell_id.0).fetch_one(&pool).await.unwrap();

    let (s, v) = send(&router, "POST", &format!("/api/signals/{}/approve", sell_id.0), json!({})).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let statuses: Vec<(i64, String)> = sqlx::query_as("SELECT id, status FROM signals ORDER BY id")
        .fetch_all(&pool).await.unwrap();
    assert_eq!(statuses, vec![(sell_id.0, "approved".into()), (buy_id.0, "approved".into())]);

    let sold: (f64,) = sqlx::query_as(
        "SELECT qty FROM positions p JOIN instruments i ON i.id=p.instrument_id WHERE i.symbol='HDFCBANK'",
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(sold.0, 0.0, "the sell leg should have posted");
    let bought: (f64,) = sqlx::query_as(
        "SELECT qty FROM positions p JOIN instruments i ON i.id=p.instrument_id WHERE i.symbol='INFY'",
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(bought.0, 2.0, "the buy leg should have posted");
}

#[tokio::test]
async fn a_rotation_that_does_not_balance_is_refused_before_any_money_moves() {
    let (_d, router, pool) = setup().await;
    sqlx::query("INSERT OR IGNORE INTO instruments (symbol, class, name, currency) VALUES ('HDFCBANK','stock','HDFC Bank','INR')")
        .execute(&pool).await.unwrap();
    send(&router, "POST", "/api/instruments/HDFCBANK/trades",
        json!({"side":"buy","qty":1.0,"price":100.0,"mode":"long"})).await;
    let sell_id: (i64,) = sqlx::query_as(
        "INSERT INTO signals (instrument_id, action, qty, entry, valid_until, reason_json, created_at,
            mode, kind, status)
         SELECT id, 'SELL', 1, 100.0, '2030-01-01T00:00:00Z', '{}', '2026-11-08T10:00:00Z',
                'long', 'rotation', 'pending' FROM instruments WHERE symbol='HDFCBANK' RETURNING id",
    ).fetch_one(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO signals (instrument_id, action, qty, entry, valid_until, reason_json, created_at,
            mode, kind, status, pair_id)
         SELECT id, 'BUY', 1000, 500.0, '2030-01-01T00:00:00Z', '{}', '2026-11-08T10:00:00Z',
                'long', 'rotation', 'pending', ? FROM instruments WHERE symbol='INFY'",
    ).bind(sell_id.0).execute(&pool).await.unwrap();

    let (s, v) = send(&router, "POST", &format!("/api/signals/{}/approve", sell_id.0), json!({})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("does not balance"), "{v}");

    let held: (f64,) = sqlx::query_as(
        "SELECT qty FROM positions p JOIN instruments i ON i.id=p.instrument_id WHERE i.symbol='HDFCBANK'",
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(held.0, 1.0, "nothing should have been sold");
}
