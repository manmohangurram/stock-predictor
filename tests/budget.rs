use stock_predicter::db;
use stock_predicter::models::AssetClass;
use stock_predicter::rules::budget::{self, CashKind, NewCashEntry};
use stock_predicter::rules::settings::{self, SettingsError, SettingsPatch};

async fn fresh(budget: f64) -> (tempfile::TempDir, sqlx::SqlitePool, i64) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    let pool = db::connect_and_migrate(path.to_str().unwrap()).await.unwrap();
    settings::seed(&pool, budget, "INR").await.unwrap();
    let id: (i64,) = sqlx::query_as(
        "INSERT INTO instruments (symbol, class, name, currency) VALUES ('INFY', 'stock', 'Infosys', 'INR') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    (dir, pool, id.0)
}

async fn buy(pool: &sqlx::SqlitePool, instrument_id: i64, qty: f64, avg_cost: f64) {
    sqlx::query(
        "INSERT INTO positions (instrument_id, qty, avg_cost, opened_at, state) VALUES (?, ?, ?, ?, 'open')",
    )
    .bind(instrument_id)
    .bind(qty)
    .bind(avg_cost)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn buy_within_budget_is_allowed() {
    let (_d, pool, _) = fresh(1000.0).await;
    assert!(budget::fits(&pool, 600.0).await.unwrap());
}

#[tokio::test]
async fn buy_exceeding_budget_is_refused() {
    let (_d, pool, _) = fresh(1000.0).await;
    assert!(!budget::fits(&pool, 1001.0).await.unwrap());
}

#[tokio::test]
async fn pending_ipo_reservation_blocks_a_stock_buy() {
    let (_d, pool, id) = fresh(1000.0).await;
    budget::reserve_ipo(&pool, id, 50.0, 10.0).await.unwrap();
    assert!((budget::available(&pool).await.unwrap() - 500.0).abs() < 1e-9);
    assert!(!budget::fits(&pool, 600.0).await.unwrap());
}

#[tokio::test]
async fn fractional_mf_rounding_never_exceeds_ceiling() {
    let (_d, pool, _) = fresh(97.98).await;
    let cost = budget::cost(2.1456, 45.6721, AssetClass::MutualFund);
    assert!((cost - 2.145 * 45.6721).abs() < 1e-9, "cost was {cost}");
    assert!(cost <= 97.98, "unrounded cost would have overspent");
    assert!(budget::fits(&pool, cost).await.unwrap());
}

#[tokio::test]
async fn budget_below_committed_is_refused_with_the_figure() {
    let (_d, pool, id) = fresh(2000.0).await;
    buy(&pool, id, 10.0, 142.0).await;
    let err = settings::set(
        &pool,
        SettingsPatch {
            budget: Some(1000.0),
            currency: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err, SettingsError::BelowCommitted { committed: 1420.0 });
}

#[tokio::test]
async fn currency_change_is_refused_once_ledger_has_entries() {
    let (_d, pool, _) = fresh(1000.0).await;
    budget::record(
        &pool,
        NewCashEntry {
            kind: CashKind::Buy,
            amount: -100.0,
            reference: "test buy".into(),
        },
    )
    .await
    .unwrap();
    let err = settings::set(
        &pool,
        SettingsPatch {
            budget: None,
            currency: Some("USD".into()),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err, SettingsError::CurrencyLocked);
}

#[tokio::test]
async fn currency_change_is_allowed_on_a_fresh_ledger() {
    let (_d, pool, _) = fresh(1000.0).await;
    settings::set(
        &pool,
        SettingsPatch {
            budget: None,
            currency: Some("USD".into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(settings::get(&pool).await.unwrap().currency, "USD");
}

#[tokio::test]
async fn raising_the_budget_is_reflected_in_available_immediately() {
    let (_d, pool, id) = fresh(1000.0).await;
    buy(&pool, id, 10.0, 60.0).await;
    assert!((budget::available(&pool).await.unwrap() - 400.0).abs() < 1e-9);
    settings::set(
        &pool,
        SettingsPatch {
            budget: Some(2000.0),
            currency: None,
        },
    )
    .await
    .unwrap();
    assert!((budget::available(&pool).await.unwrap() - 1400.0).abs() < 1e-9);
}

#[tokio::test]
async fn negative_or_infinite_budget_is_rejected() {
    let (_d, pool, _) = fresh(1000.0).await;
    for bad in [-1.0, f64::INFINITY, f64::NAN] {
        let err = settings::set(
            &pool,
            SettingsPatch {
                budget: Some(bad),
                currency: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err, SettingsError::InvalidAmount);
    }
}
