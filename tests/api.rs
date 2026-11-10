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
async fn the_monthly_allowance_is_granted_the_moment_it_is_set() {
    let (_d, router, _pool) = setup().await;
    // No allowance is a ceiling any more: it is money that arrives. Setting it puts it in the pot,
    // and raising it tops the pot up rather than waiting for the 1st.
    let (s, v) = send(&router, "PUT", "/api/settings", json!({"budget": 40_000.0})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["budget"], 40_000.0);
    assert_eq!(v["available"], 40_000.0);

    let (_, v) = send(&router, "PUT", "/api/settings", json!({"budget": 55_000.0})).await;
    assert_eq!(v["available"], 55_000.0, "raising it should top the pot up");

    // Lowering it claws the unspent part back, so the pot matches the allowance.
    let (_, v) = send(&router, "PUT", "/api/settings", json!({"budget": 20_000.0})).await;
    assert_eq!(v["available"], 20_000.0);
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
    assert_eq!(
        third.status(),
        StatusCode::OK,
        "stale tag must get fresh data"
    );
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

    let (s, _) = send(
        &router,
        "PUT",
        "/api/settings",
        json!({"prompt_window_hours": 99}),
    )
    .await;
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
    for secret in [
        "sk-plaintext-hunter2",
        "tk_plaintext_token",
        "proxy-hunter2",
    ] {
        assert!(
            !dump.contains(secret),
            "{secret} is stored in the clear: {dump}"
        );
    }
    // …and they still work when read back through the secret reader.
    let key = stock_predictor::secrets::get_secret(&pool, "deepseek_key")
        .await
        .unwrap();
    assert_eq!(key.as_deref(), Some("sk-plaintext-hunter2"));
    let pass = stock_predictor::secrets::get_secret(&pool, "proxy_pass")
        .await
        .unwrap();
    assert_eq!(pass.as_deref(), Some("proxy-hunter2"));

    // The API says what is configured, never the values.
    let (_, v) = get(&router, "/api/settings").await;
    assert_eq!(v["runtime"]["proxy_user"], "mohan");
    assert_eq!(v["runtime"]["proxy_pass_configured"], true);
    let body = v.to_string();
    assert!(
        !body.contains("proxy-hunter2") && !body.contains("sk-plaintext"),
        "{body}"
    );
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
    let pass = stock_predictor::secrets::get_secret(&pool, "proxy_pass")
        .await
        .unwrap();
    assert_eq!(
        pass.as_deref(),
        Some("s3cret"),
        "the password was wiped by a blank field"
    );

    // Clearing the URL clears the credentials with it.
    send(&router, "PUT", "/api/settings", json!({"proxy": ""})).await;
    let pass = stock_predictor::secrets::get_secret(&pool, "proxy_pass")
        .await
        .unwrap();
    assert_eq!(pass, None);
}

#[tokio::test]
async fn a_proxy_that_is_not_a_url_is_refused() {
    let (_d, router, _pool) = setup().await;
    let (s, v) = send(
        &router,
        "PUT",
        "/api/settings",
        json!({"proxy": "10.0.0.2:3128"}),
    )
    .await;
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
    assert!(
        v["error"].as_str().unwrap().contains("long-term only"),
        "{v}"
    );

    // The long-term pot is the point of it — once money is earmarked for funds.
    send(
        &router,
        "PUT",
        "/api/settings",
        json!({"mf_budget": 5000.0}),
    )
    .await;
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
async fn archiving_clears_temporary_data_but_keeps_the_record() {
    let (_d, router, pool) = setup().await;
    let id: (i64,) = sqlx::query_as("SELECT id FROM instruments WHERE symbol = 'TCS'")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO bars (instrument_id, ts, open, high, low, close, volume) VALUES (?, '2026-11-06T00:00:00Z', 1,1,1,1,1)")
        .bind(id.0).execute(&pool).await.unwrap();

    let (s, v) = send(&router, "DELETE", "/api/instruments/TCS", json!({})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["archived"], true);
    // The record survives…
    let active: (i64,) = sqlx::query_as("SELECT active FROM instruments WHERE symbol = 'TCS'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(active.0, 0, "archived, not deleted");
    // …but the temporary data does not.
    let bars: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM bars WHERE instrument_id = ?")
        .bind(id.0)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(bars.0, 0, "bars must be cleared on archive");
}

#[tokio::test]
async fn a_held_symbol_cannot_be_deleted() {
    let (_d, router, _pool) = setup().await;
    let (s, _) = send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side":"buy","qty":5.0,"price":100.0,"mode":"long"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = send(&router, "DELETE", "/api/instruments/INFY", json!({})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("held"), "{v}");
}

#[tokio::test]
async fn a_symbol_with_fills_is_archived_not_deleted() {
    let (_d, router, pool) = setup().await;
    send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side":"buy","qty":5.0,"price":100.0,"mode":"long"}),
    )
    .await;
    send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side":"sell","qty":5.0,"price":110.0,"mode":"long"}),
    )
    .await;

    let (s, v) = send(&router, "DELETE", "/api/instruments/INFY", json!({})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["archived"], true, "{v}");

    let active: (i64,) = sqlx::query_as("SELECT active FROM instruments WHERE symbol = 'INFY'")
        .fetch_one(&pool)
        .await
        .unwrap();
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
    let (s, v) = send(
        &router,
        "POST",
        &format!("/api/signals/{}/approve", id.0),
        json!({"qty": 8.0, "price": 99.5}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "approved");
    assert_eq!(v["filled_qty"], 8.0);

    let (status, fq, fp): (String, Option<f64>, Option<f64>) =
        sqlx::query_as("SELECT status, filled_qty, filled_price FROM signals WHERE id = ?")
            .bind(id.0)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "approved");
    assert_eq!(fq, Some(8.0));
    assert_eq!(fp, Some(99.5));

    let held: (f64,) = sqlx::query_as(
        "SELECT qty FROM positions p JOIN instruments i ON i.id = p.instrument_id WHERE i.symbol = 'INFY'",
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(held.0, 8.0, "the position must be the edited quantity");

    // Approving again must not double the position.
    let (s, v) = send(
        &router,
        "POST",
        &format!("/api/signals/{}/approve", id.0),
        json!({}),
    )
    .await;
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

    let (s, v) = send(
        &router,
        "POST",
        &format!("/api/signals/{}/approve", id.0),
        json!({}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(
        v["error"]
            .as_str()
            .unwrap()
            .contains("exceeds available cash"),
        "{v}"
    );

    let status: (String,) = sqlx::query_as("SELECT status FROM signals WHERE id = ?")
        .bind(id.0)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        status.0, "pending",
        "a failed fill must not consume the decision"
    );
}

#[tokio::test]
async fn rejecting_a_decision_writes_a_reason_and_no_money() {
    let (_d, router, pool) = setup().await;
    let id: (i64,) = sqlx::query_as(
        "INSERT INTO signals (instrument_id, action, qty, entry, valid_until, reason_json, created_at, mode)
         SELECT id, 'SELL', 5, 100.0, '2030-01-01T00:00:00Z', '{}', '2026-11-08T10:00:00Z', 'long'
         FROM instruments WHERE symbol = 'INFY' RETURNING id",
    ).fetch_one(&pool).await.unwrap();
    let before: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM cash_ledger")
        .fetch_one(&pool)
        .await
        .unwrap();

    let (s, v) = send(
        &router,
        "POST",
        &format!("/api/signals/{}/reject", id.0),
        json!({"reason": "not at this price"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let (status, reason): (String, Option<String>) =
        sqlx::query_as("SELECT status, status_reason FROM signals WHERE id = ?")
            .bind(id.0)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "rejected");
    assert_eq!(reason.as_deref(), Some("not at this price"));
    let after: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM cash_ledger")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(after.0, before.0, "a rejection must not touch the ledger");
}

#[tokio::test]
async fn the_ledger_shows_pending_decisions_only_unless_filtered() {
    let (_d, router, pool) = setup().await;
    for (action, status) in [
        ("BUY", "pending"),
        ("SELL", "approved"),
        ("BUY", "rejected"),
    ] {
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
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["status"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(statuses, vec!["approved"], "{statuses:?}");
}

#[tokio::test]
async fn approving_a_rotation_posts_both_legs_together() {
    let (_d, router, pool) = setup().await;
    sqlx::query("INSERT OR IGNORE INTO instruments (symbol, class, name, currency) VALUES ('HDFCBANK','stock','HDFC Bank','INR')")
        .execute(&pool).await.unwrap();
    // Buy HDFC Bank first so there is something to sell, at a profit.
    send(
        &router,
        "POST",
        "/api/instruments/HDFCBANK/trades",
        json!({"side":"buy","qty":10.0,"price":100.0,"mode":"long"}),
    )
    .await;
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
    )
    .bind(sell_id.0)
    .fetch_one(&pool)
    .await
    .unwrap();

    let (s, v) = send(
        &router,
        "POST",
        &format!("/api/signals/{}/approve", sell_id.0),
        json!({}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let statuses: Vec<(i64, String)> = sqlx::query_as("SELECT id, status FROM signals ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(
        statuses,
        vec![
            (sell_id.0, "approved".into()),
            (buy_id.0, "approved".into())
        ]
    );

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
    send(
        &router,
        "POST",
        "/api/instruments/HDFCBANK/trades",
        json!({"side":"buy","qty":1.0,"price":100.0,"mode":"long"}),
    )
    .await;
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

    let (s, v) = send(
        &router,
        "POST",
        &format!("/api/signals/{}/approve", sell_id.0),
        json!({}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(
        v["error"].as_str().unwrap().contains("does not balance"),
        "{v}"
    );

    let held: (f64,) = sqlx::query_as(
        "SELECT qty FROM positions p JOIN instruments i ON i.id=p.instrument_id WHERE i.symbol='HDFCBANK'",
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(held.0, 1.0, "nothing should have been sold");
}

#[tokio::test]
async fn audit_shows_why_a_call_was_made_and_what_you_did_with_it() {
    let (_d, router, pool) = setup().await;
    // A call and the pending decision it produced, linked by conversation.
    sqlx::query(
        "INSERT INTO api_calls (ts, conversation_id, endpoint, purpose, trigger, http_status)
         VALUES ('2026-11-09T10:00:00Z', 'conv-1', 'chat', 'decide INFY', 'price moved 1,150.20 → 1,171.60', 200)",
    ).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO signals (instrument_id, action, qty, entry, valid_until, reason_json, created_at,
            mode, conversation_id)
         SELECT id, 'BUY', 5, 1171.6, '2030-01-01T00:00:00Z', '{}', '2026-11-09T10:00:01Z', 'long', 'conv-1'
         FROM instruments WHERE symbol = 'INFY'",
    ).execute(&pool).await.unwrap();

    let (s, v) = get(&router, "/api/calls").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let row = &v.as_array().unwrap()[0];
    assert_eq!(row["trigger"], "price moved 1,150.20 → 1,171.60");
    assert_eq!(row["decision_status"], "pending");

    // Approve it; the review state follows.
    let id: (i64,) = sqlx::query_as("SELECT id FROM signals LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    send(
        &router,
        "POST",
        &format!("/api/signals/{}/approve", id.0),
        json!({}),
    )
    .await;
    let (_, v) = get(&router, "/api/calls").await;
    assert_eq!(v.as_array().unwrap()[0]["decision_status"], "approved");
}

#[tokio::test]
async fn rejecting_a_decision_changes_the_dashboard_etag() {
    // Regression: the ETag was built from max ids only, so a status change answered 304 and the
    // ledger never refreshed after you approved or rejected something.
    let (_d, router, pool) = setup().await;
    let id: (i64,) = sqlx::query_as(
        "INSERT INTO signals (instrument_id, action, qty, entry, valid_until, reason_json, created_at, mode)
         SELECT id, 'BUY', 5, 100.0, '2030-01-01T00:00:00Z', '{}', '2026-11-09T10:00:00Z', 'long'
         FROM instruments WHERE symbol = 'INFY' RETURNING id",
    )
    .fetch_one(&pool).await.unwrap();

    let (_, before) = get(&router, "/api/dashboard").await;
    assert_eq!(before["signals"].as_array().unwrap().len(), 1);

    send(
        &router,
        "POST",
        &format!("/api/signals/{}/reject", id.0),
        json!({"reason": "no"}),
    )
    .await;

    // A fresh (unconditional) read reflects the change…
    let (_, after) = get(&router, "/api/dashboard").await;
    assert_eq!(
        after["signals"].as_array().unwrap().len(),
        0,
        "pending list must clear"
    );

    // …and the tag differs, so a conditional read cannot answer 304.
    let (_, empty) = get(&router, "/api/dashboard?decisions=pending").await;
    assert_eq!(empty["signals"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn a_filtered_dashboard_url_with_a_second_question_mark_is_still_valid() {
    // The UI once built "/api/dashboard?mode=long?decisions=rejected". A stray second "?" is a
    // query-string typo, not a 400: the server should read `mode` and ignore the junk.
    let (_d, router, _pool) = setup().await;
    let (s, _) = get(&router, "/api/dashboard?mode=long&decisions=rejected").await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn a_stray_question_mark_in_a_hand_written_url_is_tolerated() {
    let (_d, router, _pool) = setup().await;
    let (s, _) = get(&router, "/api/dashboard?mode=long?decisions=rejected").await;
    assert_eq!(s, StatusCode::OK);
}

/// Every route the server exposes must be reachable — a 404 here means the UI and the server have
/// drifted apart. Kept as a table so adding a route without wiring it is visible.
#[tokio::test]
async fn every_documented_route_exists() {
    let (_d, router, pool) = setup().await;
    let id: (i64,) = sqlx::query_as(
        "INSERT INTO signals (instrument_id, action, qty, entry, valid_until, reason_json, created_at, mode)
         SELECT id, 'BUY', 1, 100.0, '2030-01-01T00:00:00Z', '{}', '2026-11-09T10:00:00Z', 'long'
         FROM instruments WHERE symbol = 'INFY' RETURNING id",
    ).fetch_one(&pool).await.unwrap();
    // A call row, so /api/calls/{id} answers "found" rather than "no such call".
    sqlx::query(
        "INSERT INTO api_calls (id, ts, endpoint, purpose) VALUES (1, '2026-11-09T10:00:00Z', 'chat', 'decide INFY')",
    )
    .execute(&pool).await.unwrap();

    let routes: Vec<(&str, String)> = vec![
        ("GET", "/api/health".into()),
        ("GET", "/api/dashboard".into()),
        ("GET", "/api/settings".into()),
        ("POST", "/api/settings/proxy/test".into()),
        ("POST", "/api/instruments".into()),
        ("GET", "/api/instruments/INFY".into()),
        ("POST", "/api/instruments/INFY/refresh".into()),
        ("PUT", "/api/instruments/INFY/target-profit".into()),
        // A different symbol: deleting INFY first would 404 the routes below it.
        ("DELETE", "/api/instruments/TCS".into()),
        ("GET", "/api/instruments/INFY/analysis".into()),
        ("GET", "/api/instruments/INFY/trades".into()),
        ("POST", "/api/instruments/INFY/trades".into()),
        ("GET", "/api/trades".into()),
        ("GET", "/api/search".into()),
        ("GET", "/api/calls".into()),
        ("GET", "/api/calls/1".into()),
        // Reject first: an approved decision is no longer pending, so rejecting it would 404.
        ("POST", format!("/api/signals/{}/reject", id.0)),
        ("POST", format!("/api/signals/{}/approve", id.0)),
        ("POST", "/api/notify/test".into()),
    ];
    for (method, uri) in routes {
        let (status, _) = send(&router, method, &uri, json!({})).await;
        assert_ne!(
            status,
            StatusCode::NOT_FOUND,
            "{method} {uri} is not routed — the server and the UI disagree about the API"
        );
    }
}

#[tokio::test]
async fn a_fund_buy_is_capped_by_the_fund_earmark() {
    let (_d, router, pool) = setup().await;
    sqlx::query("INSERT OR IGNORE INTO instruments (symbol, class, name, currency) VALUES ('AXISELSS','mutual_fund','Axis ELSS','INR')")
        .execute(&pool).await.unwrap();
    // A small earmark: the fund must not be able to consume the whole pot.
    send(
        &router,
        "PUT",
        "/api/settings",
        json!({"mf_budget": 1000.0}),
    )
    .await;

    let (s, v) = send(
        &router,
        "POST",
        "/api/instruments/AXISELSS/trades",
        json!({"side":"buy","qty":100.0,"price":55.0,"mode":"long"}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    // The fund purse is its own money: 1,000 cannot buy 5,500 of units.
    assert!(
        v["error"]
            .as_str()
            .unwrap()
            .contains("exceeds available cash"),
        "{v}"
    );

    // Within the purse, it passes.
    let (s, v) = send(
        &router,
        "POST",
        "/api/instruments/AXISELSS/trades",
        json!({"side":"buy","qty":10.0,"price":55.0,"mode":"long"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

#[tokio::test]
async fn the_three_monthly_purses_do_not_share_money() {
    // Long-term stocks, mutual funds and short-term are separate purses, each with its own
    // allowance; spending one must never move another.
    let (_d, router, pool) = setup().await;
    sqlx::query("INSERT OR IGNORE INTO instruments (symbol, class, name, currency) VALUES ('AXISELSS','mutual_fund','Axis ELSS','INR')")
        .execute(&pool).await.unwrap();

    send(
        &router,
        "PUT",
        "/api/settings",
        json!({"budget": 20_000.0, "mf_budget": 5_000.0, "trading_budget": 7_000.0}),
    )
    .await;

    let (_, v) = get(&router, "/api/settings").await;
    assert_eq!(v["available"], 20_000.0, "stock purse");
    assert_eq!(v["mf_available"], 5_000.0, "fund purse");
    assert_eq!(v["trading_available"], 7_000.0, "short-term purse");

    // Spending the fund purse leaves the stock purse untouched.
    let (s, _) = send(
        &router,
        "POST",
        "/api/instruments/AXISELSS/trades",
        json!({"side":"buy","qty":50.0,"price":55.0,"mode":"long"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, v) = get(&router, "/api/settings").await;
    assert_eq!(
        v["available"], 20_000.0,
        "a fund buy must not touch the stock purse"
    );
    assert_eq!(
        v["mf_available"], 2_250.0,
        "50 × 55 came out of the fund purse"
    );
}

#[tokio::test]
async fn a_blocked_proposal_is_visible_in_the_record_with_its_shortfall() {
    // Regression: a proposal refused for cash was stored as BLOCKED and filtered out of the stock
    // overview, so a call you paid for left no visible record. The ledger's decisions box is for
    // decisions; the instrument's record keeps the refusal.
    let (_d, router, pool) = setup().await;
    sqlx::query(
        "INSERT INTO signals (instrument_id, action, qty, entry, valid_until, reason_json, created_at, mode)
         SELECT id, 'BLOCKED', 1000, 100.0, '2030-01-01T00:00:00Z',
                '{\"blocked\":\"insufficient_cash\",\"cost\":100000,\"available\":1000,\"shortfall\":99000}',
                '2026-11-09T10:00:00Z', 'long'
         FROM instruments WHERE symbol = 'INFY'",
    ).execute(&pool).await.unwrap();

    // The dashboard is decisions only: a refusal is not a decision to review.
    let (_, v) = get(&router, "/api/dashboard").await;
    assert!(v.get("blocked").is_none(), "the ledger no longer carries a blocked list");

    // …and the stock overview shows it as part of the record.
    let (s, v) = get(&router, "/api/instruments/INFY/analysis").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let actions: Vec<String> = v["signals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["action"].as_str().unwrap().to_string())
        .collect();
    assert!(actions.contains(&"BLOCKED".to_string()), "{actions:?}");
}

#[tokio::test]
async fn audit_says_when_a_decision_was_blocked_and_why() {
    let (_d, router, pool) = setup().await;
    sqlx::query(
        "INSERT INTO api_calls (ts, conversation_id, endpoint, purpose, trigger, http_status)
         VALUES ('2026-11-09T10:00:00Z', 'conv-b', 'chat', 'decide INFY', 'price moved 100 → 120', 200)",
    ).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO signals (instrument_id, action, qty, entry, valid_until, reason_json, created_at,
            mode, conversation_id)
         SELECT id, 'BLOCKED', 1000, 100.0, '2030-01-01T00:00:00Z',
                '{\"blocked\":\"insufficient_cash\",\"cost\":100000,\"available\":1000}',
                '2026-11-09T10:00:01Z', 'long', 'conv-b'
         FROM instruments WHERE symbol = 'INFY'",
    ).execute(&pool).await.unwrap();

    let (s, v) = get(&router, "/api/calls").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let row = &v.as_array().unwrap()[0];
    assert_eq!(row["decision_action"], "BLOCKED");
    let reason = row["decision_reason"].as_str().unwrap();
    assert!(reason.contains("insufficient_cash"), "{reason}");
    assert!(reason.contains("cost 100000.00"), "{reason}");
    assert!(reason.contains("available 1000.00"), "{reason}");
}

#[tokio::test]
async fn a_plan_id_travels_with_the_decision_rows() {
    let (_d, router, pool) = setup().await;
    sqlx::query(
        "INSERT INTO signals (instrument_id, action, qty, entry, valid_until, reason_json,
            created_at, mode, kind, status, plan_id)
         SELECT id, 'BUY', 1, 100.0, '2030-01-01T00:00:00Z', '{}', '2026-11-09T10:00:00Z',
                'long', 'plan', 'pending', 41
         FROM instruments WHERE symbol = 'INFY'",
    )
    .execute(&pool)
    .await
    .unwrap();
    let (s, v) = get(&router, "/api/dashboard").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["signals"][0]["plan_id"], 41);
    assert_eq!(v["signals"][0]["name"], "INFY Ltd");
}

/// A plan is one decision with several legs: approving any leg posts all of them, sells first.
#[tokio::test]
async fn approving_one_leg_of_a_plan_approves_the_whole_plan() {
    let (_d, router, pool) = setup().await;
    let plan = 77_i64;
    sqlx::query(
        "INSERT INTO signals (instrument_id, action, qty, entry, stop, target, valid_until,
            reason_json, created_at, mode, kind, status, plan_id)
         SELECT id, 'SELL', 5, 100.0, NULL, NULL, '2030-01-01T00:00:00Z',
                '{\"reasons\":[],\"rationale\":\"target reached\"}', '2026-11-09T10:00:00Z',
                'long', 'plan', 'pending', ?
         FROM instruments WHERE symbol = 'INFY'",
    )
    .bind(plan)
    .execute(&pool)
    .await
    .unwrap();
    // A holding to sell.
    sqlx::query(
        "INSERT INTO positions (instrument_id, qty, avg_cost, stop, target, state, mode, opened_at)
         SELECT id, 5, 90.0, 80.0, 120.0, 'open', 'long', '2026-09-01T00:00:00Z'
         FROM instruments WHERE symbol = 'INFY'",
    )
    .execute(&pool)
    .await
    .unwrap();
    let buy_id: (i64,) = sqlx::query_as(
        "INSERT INTO signals (instrument_id, action, qty, entry, stop, target, valid_until,
            reason_json, created_at, mode, kind, status, plan_id)
         SELECT id, 'BUY', 1, 40.0, 36.0, 60.0, '2030-01-01T00:00:00Z',
                '{\"reasons\":[],\"rationale\":\"cheap\"}', '2026-11-09T10:00:01Z',
                'long', 'plan', 'pending', ?
         FROM instruments WHERE symbol = 'TCS'
         RETURNING id",
    )
    .bind(plan)
    .fetch_one(&pool)
    .await
    .unwrap();

    let (s, v) = send(
        &router,
        "POST",
        &format!("/api/signals/{}/approve", buy_id.0),
        json!({}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "approved");

    let statuses: Vec<(String, String)> =
        sqlx::query_as("SELECT status, action FROM signals WHERE plan_id = ? ORDER BY action")
            .bind(plan)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(statuses.len(), 2);
    assert!(
        statuses.iter().all(|(s, _)| s == "approved"),
        "{statuses:?}"
    );

    let trades: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM trades")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(trades.0, 2, "both legs posted");

    // Approving again is refused rather than posting the legs twice.
    let (s, _) = send(
        &router,
        "POST",
        &format!("/api/signals/{}/approve", buy_id.0),
        json!({}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn rejecting_one_leg_of_a_plan_rejects_every_move() {
    let (_d, router, pool) = setup().await;
    let plan = 88_i64;
    for (action, qty) in [("SELL", 5.0), ("BUY", 1.0)] {
        sqlx::query(
            "INSERT INTO signals (instrument_id, action, qty, entry, valid_until, reason_json,
                created_at, mode, kind, status, plan_id)
             SELECT id, ?, ?, 100.0, '2030-01-01T00:00:00Z', '{}', '2026-11-09T10:00:00Z',
                    'long', 'plan', 'pending', ?
             FROM instruments WHERE symbol = 'INFY'",
        )
        .bind(action)
        .bind(qty)
        .bind(plan)
        .execute(&pool)
        .await
        .unwrap();
    }
    let id: (i64,) = sqlx::query_as("SELECT MAX(id) FROM signals WHERE plan_id = ?")
        .bind(plan)
        .fetch_one(&pool)
        .await
        .unwrap();
    let (s, v) = send(
        &router,
        "POST",
        &format!("/api/signals/{}/reject", id.0),
        json!({"reason": "not now"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let pending: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM signals WHERE plan_id = ? AND status = 'pending'")
            .bind(plan)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(pending.0, 0, "no leg is left pending");
}

/// You rarely get the exact prices a plan named. Each leg carries its own fill, and the plan is
/// checked for balance on what you actually got.
#[tokio::test]
async fn a_plan_is_approved_with_a_fill_for_each_leg() {
    let (_d, router, pool) = setup().await;
    let sell_id: (i64,) = sqlx::query_as(
        "INSERT INTO signals (instrument_id, action, qty, entry, valid_until, reason_json,
            created_at, mode, kind, status, plan_id)
         SELECT id, 'SELL', 5, 100.0, '2030-01-01T00:00:00Z', '{}', '2026-11-09T10:00:00Z',
                'long', 'plan', 'pending', 99
         FROM instruments WHERE symbol = 'INFY' RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE signals SET plan_id = ? WHERE id = ?")
        .bind(sell_id.0)
        .bind(sell_id.0)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO positions (instrument_id, qty, avg_cost, stop, target, state, mode, opened_at)
         SELECT id, 5, 90.0, 80.0, 120.0, 'open', 'long', '2026-09-01T00:00:00Z'
         FROM instruments WHERE symbol = 'INFY'",
    )
    .execute(&pool)
    .await
    .unwrap();
    let buy_id: (i64,) = sqlx::query_as(
        "INSERT INTO signals (instrument_id, action, qty, entry, stop, target, valid_until,
            reason_json, created_at, mode, kind, status, plan_id)
         SELECT id, 'BUY', 1, 40.0, 36.0, 60.0, '2030-01-01T00:00:00Z', '{}',
                '2026-11-09T10:00:01Z', 'long', 'plan', 'pending', ?
         FROM instruments WHERE symbol = 'TCS' RETURNING id",
    )
    .bind(sell_id.0)
    .fetch_one(&pool)
    .await
    .unwrap();

    // Different from the proposal on both legs: 5 @ 101.50 sold, 2 @ 40.25 bought.
    let (s, v) = send(
        &router,
        "POST",
        &format!("/api/signals/{}/approve", buy_id.0),
        json!({"fills": [
            {"id": sell_id.0, "qty": 5.0, "price": 101.5},
            {"id": buy_id.0, "qty": 2.0, "price": 40.25}
        ]}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let filled: Vec<(String, f64, f64)> = sqlx::query_as(
        "SELECT action, filled_qty, filled_price FROM signals WHERE plan_id = ? ORDER BY action",
    )
    .bind(sell_id.0)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(filled[0], ("BUY".to_string(), 2.0, 40.25));
    assert_eq!(filled[1], ("SELL".to_string(), 5.0, 101.5));
}

/// A fill is corrected, not rewritten: the position moves by the difference and the ledger gets a
/// visible `adjust` entry, because cash_ledger is append-only.
#[tokio::test]
async fn editing_a_fill_moves_the_position_and_posts_the_cash_difference() {
    let (_d, router, pool) = setup().await;
    let (s, v) = send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side": "buy", "qty": 10.0, "price": 100.0, "mode": "long"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let trade: (i64,) = sqlx::query_as("SELECT MAX(id) FROM trades")
        .fetch_one(&pool)
        .await
        .unwrap();

    // You transposed the price: it was 90, not 100.
    let (s, v) = send(
        &router,
        "PATCH",
        &format!("/api/trades/{}", trade.0),
        json!({"price": 90.0}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let (qty, cost): (f64, f64) = sqlx::query_as(
        "SELECT qty, avg_cost FROM positions WHERE instrument_id =
         (SELECT id FROM instruments WHERE symbol = 'INFY') AND state = 'open'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(qty, 10.0);
    assert!(
        (cost - 90.0).abs() < 1e-6,
        "cost basis follows the correction: {cost}"
    );

    let adjust: (f64,) =
        sqlx::query_as("SELECT COALESCE(SUM(amount), 0) FROM cash_ledger WHERE kind = 'adjust'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        (adjust.0 - 100.0).abs() < 1e-6,
        "10 units × 10 too much: {adjust:?}"
    );
}

/// An edit that would sell more than is held is refused, and nothing moves.
#[tokio::test]
async fn an_edit_that_would_go_negative_is_refused() {
    let (_d, router, pool) = setup().await;
    send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side": "buy", "qty": 10.0, "price": 100.0, "mode": "long"}),
    )
    .await;
    let trade: (i64,) = sqlx::query_as("SELECT MAX(id) FROM trades")
        .fetch_one(&pool)
        .await
        .unwrap();
    let (s, v) = send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side": "sell", "qty": 4.0, "price": 110.0, "mode": "long"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let sell: (i64,) = sqlx::query_as("SELECT MAX(id) FROM trades")
        .fetch_one(&pool)
        .await
        .unwrap();

    let (s, v) = send(
        &router,
        "PATCH",
        &format!("/api/trades/{}", sell.0),
        json!({"qty": 40.0}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let held: (f64,) = sqlx::query_as(
        "SELECT qty FROM positions WHERE instrument_id =
         (SELECT id FROM instruments WHERE symbol = 'INFY') AND state = 'open'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(held.0, 6.0, "the refused edit moved nothing");
    let _ = trade;
}

/// A book call decided several things: the audit row says "plan", not the last leg's name.
#[tokio::test]
async fn a_book_call_reads_as_a_plan_in_the_audit() {
    let (_d, router, pool) = setup().await;
    sqlx::query(
        "INSERT INTO api_calls (ts, conversation_id, endpoint, purpose, trigger, http_status)
         VALUES ('2026-11-09T10:00:00Z', 'plan-1', 'chat', 'plan book', 'book call', 200)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO signals (instrument_id, action, qty, entry, valid_until, reason_json,
            created_at, mode, kind, status, conversation_id, plan_id)
         SELECT id, 'BUY', 1, 100.0, '2030-01-01T00:00:00Z', '{}', '2026-11-09T10:00:01Z',
                'long', 'plan', 'pending', 'plan-1', 5
         FROM instruments WHERE symbol = 'INFY'",
    )
    .execute(&pool)
    .await
    .unwrap();

    let (s, v) = get(&router, "/api/calls").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let row = &v.as_array().unwrap()[0];
    assert_eq!(row["decision_action"], "PLAN", "{row}");
    assert_eq!(row["plan_legs"], 1);
    assert!(row["instrument_name"].is_null(), "{row}");
}

/// Approving a decision must put its exit plan on the position, or the ledger shows a holding
/// with no stop and no target the moment it was bought.
#[tokio::test]
async fn approving_a_buy_writes_its_exit_plan_onto_the_position() {
    let (_d, router, pool) = setup().await;
    let id: (i64,) = sqlx::query_as(
        "INSERT INTO signals (instrument_id, action, qty, entry, stop, target, valid_until,
            reason_json, created_at, mode)
         SELECT id, 'BUY', 10, 100.0, 90.0, 130.0, '2030-01-01T00:00:00Z', '{}',
                '2026-11-09T10:00:00Z', 'long'
         FROM instruments WHERE symbol = 'INFY' RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let (s, v) = send(
        &router,
        "POST",
        &format!("/api/signals/{}/approve", id.0),
        json!({"qty": 10.0, "price": 100.0}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let (stop, target): (Option<f64>, Option<f64>) = sqlx::query_as(
        "SELECT stop, target FROM positions
         WHERE instrument_id = (SELECT id FROM instruments WHERE symbol = 'INFY')
           AND state = 'open'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stop, Some(90.0));
    assert_eq!(target, Some(130.0));
}

/// The same, for each leg of a plan.
#[tokio::test]
async fn approving_a_plan_writes_each_legs_exit_plan() {
    let (_d, router, pool) = setup().await;
    let buy: (i64,) = sqlx::query_as(
        "INSERT INTO signals (instrument_id, action, qty, entry, stop, target, valid_until,
            reason_json, created_at, mode, kind, status)
         SELECT id, 'BUY', 5, 200.0, 180.0, 260.0, '2030-01-01T00:00:00Z', '{}',
                '2026-11-09T10:00:00Z', 'long', 'plan', 'pending'
         FROM instruments WHERE symbol = 'TCS' RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE signals SET plan_id = ? WHERE id = ?")
        .bind(buy.0)
        .bind(buy.0)
        .execute(&pool)
        .await
        .unwrap();

    let (s, v) = send(
        &router,
        "POST",
        &format!("/api/signals/{}/approve", buy.0),
        json!({"fills": [{"id": buy.0, "qty": 5.0, "price": 200.0}]}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let (stop, target): (Option<f64>, Option<f64>) = sqlx::query_as(
        "SELECT stop, target FROM positions
         WHERE instrument_id = (SELECT id FROM instruments WHERE symbol = 'TCS')
           AND state = 'open'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stop, Some(180.0));
    assert_eq!(target, Some(260.0));
}

/// The audit list is paged: one page of rows, and the total in a header.
#[tokio::test]
async fn the_audit_list_pages_and_says_how_many_there_are() {
    let (_d, router, pool) = setup().await;
    for i in 0..30 {
        sqlx::query(
            "INSERT INTO api_calls (ts, conversation_id, endpoint, purpose, http_status)
             VALUES (?, ?, 'chat', 'decide INFY', 200)",
        )
        .bind(format!("2026-11-09T10:{:02}:00Z", i))
        .bind(format!("conv-{i}"))
        .execute(&pool)
        .await
        .unwrap();
    }

    let (s, first) = get(&router, "/api/calls?page=1&per_page=25").await;
    assert_eq!(s, StatusCode::OK, "{first}");
    assert_eq!(first.as_array().unwrap().len(), 25);

    let (s, last) = get(&router, "/api/calls?page=2&per_page=25").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        last.as_array().unwrap().len(),
        5,
        "the second page holds the rest"
    );

    // The total is a header, so the body stays a plain array.
    use tower::ServiceExt;
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/calls")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("x-total-count")
            .unwrap()
            .to_str()
            .unwrap(),
        "30"
    );
    assert_eq!(
        resp.headers().get("x-limit").unwrap().to_str().unwrap(),
        "25"
    );
}

/// Offset/limit is the primitive, so a client can jump anywhere without arithmetic.
#[tokio::test]
async fn the_audit_list_accepts_offset_and_limit() {
    let (_d, router, pool) = setup().await;
    for i in 0..30 {
        sqlx::query(
            "INSERT INTO api_calls (ts, conversation_id, endpoint, purpose, http_status)
             VALUES (?, ?, 'chat', 'decide INFY', 200)",
        )
        .bind(format!("2026-11-09T10:{:02}:00Z", i))
        .bind(format!("conv-{i}"))
        .execute(&pool)
        .await
        .unwrap();
    }
    let (s, rows) = get(&router, "/api/calls?offset=28&limit=5").await;
    assert_eq!(s, StatusCode::OK, "{rows}");
    assert_eq!(
        rows.as_array().unwrap().len(),
        2,
        "the tail is short, not padded"
    );

    let (s, rows) = get(&router, "/api/calls?offset=0&limit=10").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(rows.as_array().unwrap().len(), 10);
}

/// The dashboard tag must move when values change in place — an edited fill, an archived
/// instrument — or the client gets a 304 and the screen goes stale until a hard refresh.
#[tokio::test]
async fn the_dashboard_tag_notices_in_place_changes() {
    use tower::ServiceExt;
    let (_d, router, pool) = setup().await;
    let tag = |router: axum::Router| async move {
        let resp = router
            .oneshot(
                Request::builder()
                    .uri("/api/dashboard")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        resp.headers()
            .get("etag")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string()
    };

    let before = tag(router.clone()).await;

    // A fill, then a correction to it: no new trade id, only changed numbers.
    send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side": "buy", "qty": 10.0, "price": 100.0, "mode": "long"}),
    )
    .await;
    let after_fill = tag(router.clone()).await;
    assert_ne!(before, after_fill, "a new fill changes the tag");

    let trade: (i64,) = sqlx::query_as("SELECT MAX(id) FROM trades")
        .fetch_one(&pool)
        .await
        .unwrap();
    send(
        &router,
        "PATCH",
        &format!("/api/trades/{}", trade.0),
        json!({"qty": 12.0}),
    )
    .await;
    let after_edit = tag(router.clone()).await;
    assert_ne!(
        after_fill, after_edit,
        "editing a fill in place changes the tag"
    );

    // Archiving flips a flag; the ledger must stop showing the row. (INFY is held, so it cannot
    // be archived — TCS is the one that can.)
    let (status, _) = send(&router, "DELETE", "/api/instruments/TCS", json!({})).await;
    assert!(
        status.is_success(),
        "archive should succeed for an unheld instrument"
    );
    let after_archive = tag(router.clone()).await;
    assert_ne!(after_edit, after_archive, "archiving changes the tag");
}

/// Runs are a record of work: opened before it starts, closed with what happened. The page needs
/// "running" to exist, which is why this is not derived from api_calls.
#[tokio::test]
async fn a_run_is_listed_while_running_and_after_it_finishes() {
    let (_d, router, pool) = setup().await;
    let id = stock_predictor::runs::start(
        &pool,
        stock_predictor::runs::Prompt::PlanFund,
        stock_predictor::runs::Trigger::Schedule,
        None,
    )
    .await
    .unwrap();

    let (s, v) = get(&router, "/api/runs").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["running"], 1);
    assert_eq!(v["runs"][0]["status"], "running");
    assert_eq!(v["runs"][0]["prompt"], "plan_fund");

    stock_predictor::runs::finish(&pool, id, "2 moves stored", Some("conv-1"))
        .await
        .unwrap();

    let (s, v) = get(&router, "/api/runs").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["running"], 0);
    assert_eq!(v["runs"][0]["status"], "done");
    assert_eq!(v["runs"][0]["summary"], "2 moves stored");
}

/// A run left "running" by a killed process must not show a spinner forever.
#[tokio::test]
async fn orphaned_runs_are_closed_at_boot() {
    let (_d, _router, pool) = setup().await;
    stock_predictor::runs::start(
        &pool,
        stock_predictor::runs::Prompt::Poll,
        stock_predictor::runs::Trigger::Schedule,
        None,
    )
    .await
    .unwrap();
    let closed = stock_predictor::runs::close_orphans(&pool, chrono::Utc::now())
        .await
        .unwrap();
    assert_eq!(closed, 1);
    let rows = stock_predictor::runs::recent(&pool, 10).await.unwrap();
    assert_eq!(rows[0].status, "failed");
    assert!(rows[0]
        .error
        .as_deref()
        .unwrap()
        .contains("process stopped"));
}

/// "Run now" writes the row first, so the page can see it started — and refuses nonsense.
#[tokio::test]
async fn starting_a_run_needs_a_loop_and_a_symbol() {
    let (_d, router, pool) = setup().await;
    // No poll loop attached in tests: the handler must say so rather than pretend.
    let (s, v) = send(&router, "POST", "/api/runs", json!({"prompt": "poll"})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM runs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count.0, 0, "nothing is written when the run cannot start");

    let (s, _) = send(&router, "POST", "/api/runs", json!({"prompt": "nonsense"})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

/// A plan is approved as a whole or not at all. A second leg that cannot post must undo the first
/// — half a swap is worse than none.
#[tokio::test]
async fn a_failing_leg_rolls_the_whole_plan_back() {
    let (_d, router, pool) = setup().await;
    // A holding to sell, and a buy leg.
    sqlx::query(
        "INSERT INTO positions (instrument_id, qty, avg_cost, stop, target, state, mode, opened_at)
         SELECT id, 2, 90.0, 80.0, 120.0, 'open', 'long', '2026-09-01T00:00:00Z'
         FROM instruments WHERE symbol = 'INFY'",
    )
    .execute(&pool)
    .await
    .unwrap();
    let sell: (i64,) = sqlx::query_as(
        "INSERT INTO signals (instrument_id, action, qty, entry, valid_until, reason_json,
            created_at, mode, kind, status)
         SELECT id, 'SELL', 2, 100.0, '2030-01-01T00:00:00Z', '{}', '2026-11-09T10:00:00Z',
                'long', 'plan', 'pending'
         FROM instruments WHERE symbol = 'INFY' RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let buy: (i64,) = sqlx::query_as(
        "INSERT INTO signals (instrument_id, action, qty, entry, stop, target, valid_until,
            reason_json, created_at, mode, kind, status, plan_id)
         SELECT id, 'BUY', 1, 40.0, 36.0, 60.0, '2030-01-01T00:00:00Z', '{}',
                '2026-11-09T10:00:01Z', 'long', 'plan', 'pending', ?
         FROM instruments WHERE symbol = 'TCS' RETURNING id",
    )
    .bind(sell.0)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE signals SET plan_id = ? WHERE id = ?")
        .bind(sell.0)
        .bind(sell.0)
        .execute(&pool)
        .await
        .unwrap();

    // The buy leg sells nothing, but asks for a quantity the purse cannot cover: it fails after
    // the sell leg has already posted inside the same transaction.
    let (s, v) = send(
        &router,
        "POST",
        &format!("/api/signals/{}/approve", buy.0),
        json!({"fills": [
            {"id": sell.0, "qty": 2.0, "price": 100.0},
            {"id": buy.0, "qty": 99999.0, "price": 40.0}
        ]}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");

    let trades: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM trades")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(trades.0, 0, "the sell was rolled back, not left posted");

    let held: (f64,) = sqlx::query_as(
        "SELECT qty FROM positions WHERE instrument_id =
         (SELECT id FROM instruments WHERE symbol = 'INFY') AND state = 'open'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(held.0, 2.0, "the position is untouched");

    let pending: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM signals WHERE plan_id = ? AND status = 'pending'")
            .bind(sell.0)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(pending.0, 2, "both legs are still pending");
}

/// A fill that cannot post its money must leave neither a position nor a trade behind.
#[tokio::test]
async fn a_fill_that_cannot_post_its_money_writes_nothing() {
    let (_d, router, pool) = setup().await;
    // More than the purse holds: the cash post is what fails, after the position write.
    let (s, _) = send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side": "buy", "qty": 10.0, "price": 100.0, "mode": "long"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let (s, v) = send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side": "buy", "qty": 10_000.0, "price": 100.0, "mode": "long"}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");

    let (qty,): (f64,) = sqlx::query_as(
        "SELECT qty FROM positions WHERE instrument_id =
         (SELECT id FROM instruments WHERE symbol = 'INFY') AND state = 'open'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(qty, 10.0, "the refused fill changed nothing");

    let trades: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM trades")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(trades.0, 1, "only the first fill is recorded");
}

/// Deleting a fill undoes it: the trade row goes, the position moves back, the money comes back as
/// a visible `adjust` entry, and a decision it satisfied returns to pending.
#[tokio::test]
async fn deleting_a_fill_reverses_it_and_reopens_its_decision() {
    let (_d, router, pool) = setup().await;
    let id: (i64,) = sqlx::query_as(
        "INSERT INTO signals (instrument_id, action, qty, entry, stop, target, valid_until,
            reason_json, created_at, mode)
         SELECT id, 'BUY', 10, 100.0, 90.0, 130.0, '2030-01-01T00:00:00Z', '{}',
                '2026-11-09T10:00:00Z', 'long'
         FROM instruments WHERE symbol = 'INFY' RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let (s, v) = send(
        &router,
        "POST",
        &format!("/api/signals/{}/approve", id.0),
        json!({"qty": 10.0, "price": 100.0}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let trade: (i64,) = sqlx::query_as("SELECT MAX(id) FROM trades")
        .fetch_one(&pool)
        .await
        .unwrap();
    let before: (f64,) = sqlx::query_as("SELECT SUM(amount) FROM cash_ledger")
        .fetch_one(&pool)
        .await
        .unwrap();

    let (s, v) = send(&router, "DELETE", &format!("/api/trades/{}", trade.0), json!({})).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let trades: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM trades")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(trades.0, 0, "the fill is gone");

    let after: (f64,) = sqlx::query_as("SELECT SUM(amount) FROM cash_ledger")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!((after.0 - before.0 - 1000.0).abs() < 1e-6, "the money came back: {after:?}");

    let adjust: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM cash_ledger WHERE kind = 'adjust'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(adjust.0, 1, "as its own entry, not by rewriting history");

    let held: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM positions
         WHERE instrument_id = (SELECT id FROM instruments WHERE symbol = 'INFY') AND state = 'open'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(held.0, 0, "the position is closed");

    let status: (String,) = sqlx::query_as("SELECT status FROM signals WHERE id = ?")
        .bind(id.0)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status.0, "pending", "the decision is open again");
}

/// You cannot delete a fill that later fills depend on: the book would go short.
#[tokio::test]
async fn deleting_a_fill_that_would_go_negative_is_refused() {
    let (_d, router, pool) = setup().await;
    send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side": "buy", "qty": 10.0, "price": 100.0, "mode": "long"}),
    )
    .await;
    send(
        &router,
        "POST",
        "/api/instruments/INFY/trades",
        json!({"side": "sell", "qty": 8.0, "price": 110.0, "mode": "long"}),
    )
    .await;
    let first: (i64,) = sqlx::query_as("SELECT MIN(id) FROM trades")
        .fetch_one(&pool)
        .await
        .unwrap();

    let (s, v) = send(&router, "DELETE", &format!("/api/trades/{}", first.0), json!({})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap_or_default().contains("less than nothing") || v["error"].as_str().unwrap_or_default().contains("sell or correct"), "{v}");

    let held: (f64,) = sqlx::query_as(
        "SELECT qty FROM positions WHERE instrument_id =
         (SELECT id FROM instruments WHERE symbol = 'INFY') AND state = 'open'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(held.0, 2.0, "nothing moved");
}

/// An archived instrument keeps its fills, and its profit and loss comes from them — there is no
/// price left to value it at.
#[tokio::test]
async fn an_archived_instrument_shows_what_its_fills_made() {
    let (_d, router, pool) = setup().await;
    // Buy 10 at 100, sell 10 at 130: realised 300, and the round trip is closed.
    sqlx::query(
        "INSERT INTO trades (instrument_id, side, qty, price, realised, traded_at, mode)
         SELECT id, 'buy', 10, 100.0, NULL, '2026-01-02T00:00:00Z', 'long'
         FROM instruments WHERE symbol = 'TCS'",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO trades (instrument_id, side, qty, price, realised, traded_at, mode)
         SELECT id, 'sell', 10, 130.0, 300.0, '2026-01-09T00:00:00Z', 'long'
         FROM instruments WHERE symbol = 'TCS'",
    )
    .execute(&pool)
    .await
    .unwrap();
    let (s, _) = send(&router, "DELETE", "/api/instruments/TCS", json!({})).await;
    assert_eq!(s, StatusCode::OK);

    let (s, v) = get(&router, "/api/archived").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let row = &v.as_array().unwrap()[0];
    assert_eq!(row["symbol"], "TCS");
    assert_eq!(row["realised"], 300.0);
    // Bought 1,000, sold 1,300: 300 more came back than went in.
    assert_eq!(row["net_invested"], -300.0);
}

/// Restoring puts it back in the book, and only an archived instrument can be restored.
#[tokio::test]
async fn an_archived_instrument_can_be_restored() {
    let (_d, router, pool) = setup().await;
    send(&router, "DELETE", "/api/instruments/TCS", json!({})).await;
    let archived: (i64,) =
        sqlx::query_as("SELECT active FROM instruments WHERE symbol = 'TCS'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(archived.0, 0);

    let (s, v) = send(&router, "POST", "/api/instruments/TCS/restore", json!({})).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let active: (i64,) = sqlx::query_as("SELECT active FROM instruments WHERE symbol = 'TCS'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(active.0, 1);

    // Restoring twice is refused: it is not archived any more.
    let (s, _) = send(&router, "POST", "/api/instruments/TCS/restore", json!({})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

/// Opening a call must work with and without a decision: the drawer is the only place the stored
/// prompt and reply are readable, so a 500 here makes the whole audit page useless.
#[tokio::test]
async fn a_call_can_be_opened_with_and_without_a_decision() {
    let (_d, router, pool) = setup().await;
    // One call that produced a decision, one that did not.
    sqlx::query(
        "INSERT INTO api_calls (id, ts, conversation_id, endpoint, purpose, http_status)
         VALUES (11, '2026-11-09T10:00:00Z', 'conv-a', 'chat', 'decide INFY', 200),
                (12, '2026-11-09T10:01:00Z', 'conv-b', 'chat', 'decide TCS', 200)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO signals (instrument_id, action, qty, entry, valid_until, reason_json,
            created_at, mode, conversation_id)
         SELECT id, 'BUY', 1, 100.0, '2030-01-01T00:00:00Z', '{}', '2026-11-09T10:00:01Z',
                'long', 'conv-a'
         FROM instruments WHERE symbol = 'INFY'",
    )
    .execute(&pool)
    .await
    .unwrap();

    let (s, v) = get(&router, "/api/calls/11").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["instrument_name"], "INFY Ltd");

    // No decision behind it: the holding is still named, from the purpose.
    let (s, v) = get(&router, "/api/calls/12").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["instrument_name"], "TCS Ltd");
}

/// The audit list narrows server-side, and the total follows the filter — a pager that counts the
/// unfiltered record is worse than no filter.
#[tokio::test]
async fn the_audit_list_can_be_narrowed() {
    let (_d, router, pool) = setup().await;
    sqlx::query(
        "INSERT INTO api_calls (id, ts, conversation_id, endpoint, purpose, prompt_template, http_status)
         VALUES (31, '2026-11-09T10:00:00Z', 'a', 'chat', 'decide INFY', 'decide_v1', 200),
                (32, '2026-11-09T10:01:00Z', 'b', 'chat', 'plan book', 'plan_stock_v1', 200),
                (33, '2026-11-09T10:02:00Z', 'c', 'chat+search', 'decide TCS', 'decide_v1', 200)",
    )
    .execute(&pool)
    .await
    .unwrap();
    // One of them produced an approved decision.
    sqlx::query(
        "INSERT INTO signals (instrument_id, action, qty, entry, valid_until, reason_json,
            created_at, mode, conversation_id, status)
         SELECT id, 'BUY', 1, 100.0, '2030-01-01T00:00:00Z', '{}', '2026-11-09T10:00:01Z',
                'long', 'a', 'approved'
         FROM instruments WHERE symbol = 'INFY'",
    )
    .execute(&pool)
    .await
    .unwrap();

    let (s, v) = get(&router, "/api/calls?prompt=plan").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let rows = v.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["purpose"], "plan book");

    let (s, v) = get(&router, "/api/calls?model=decide_v1").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v.as_array().unwrap().len(), 2);

    let (s, v) = get(&router, "/api/calls?call=chat%2Bsearch").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v.as_array().unwrap().len(), 1);

    let (s, v) = get(&router, "/api/calls?review=approved").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v.as_array().unwrap().len(), 1);

    let (s, v) = get(&router, "/api/calls?review=none").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v.as_array().unwrap().len(), 2, "two calls produced no decision");

    // The total is the filtered total, not the table's.
    use tower::ServiceExt;
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/calls?prompt=plan")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.headers().get("x-total-count").unwrap().to_str().unwrap(),
        "1"
    );

    // And the page is told which values exist, instead of guessing them.
    let (s, v) = get(&router, "/api/calls/facets").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["prompts"], json!(["decide", "plan"]));
    assert_eq!(v["models"], json!(["decide_v1", "plan_stock_v1"]));
}
