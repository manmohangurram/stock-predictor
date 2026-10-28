use stock_predicter::db;
use stock_predicter::models::{Action, AssetClass, Candidate};
use stock_predicter::notify::{self, FakeNotifier, Notification, Priority};

fn candidate(action: Action, urgent: bool) -> Candidate {
    Candidate {
        instrument_id: 1,
        action,
        reasons: vec!["rule".into()],
        entry: 1405.20,
        stop: Some(1412.0),
        target: Some(1510.0),
        qty: 5.0,
        timing_note: AssetClass::Stock.timing_note().into(),
        urgent,
    }
}

async fn pool() -> (tempfile::TempDir, sqlx::SqlitePool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    let pool = db::connect_and_migrate(path.to_str().unwrap())
        .await
        .unwrap();
    (dir, pool)
}

#[tokio::test]
async fn same_dedupe_key_sends_once() {
    let (_d, pool) = pool().await;
    let fake = FakeNotifier::default();
    let n = Notification::from_candidate("INFY", "INR", &candidate(Action::Buy, false)).unwrap();
    assert!(notify::dispatch(&pool, &fake, &n).await.unwrap());
    assert!(!notify::dispatch(&pool, &fake, &n).await.unwrap());
    assert_eq!(fake.count(), 1);
}

#[tokio::test]
async fn sell_uses_high_priority() {
    let n = Notification::from_candidate("INFY", "INR", &candidate(Action::Sell, true)).unwrap();
    assert_eq!(n.priority, Priority::High);
    assert!(n.title.starts_with("SELL"));
    let b = Notification::from_candidate("INFY", "INR", &candidate(Action::Buy, false)).unwrap();
    assert_eq!(b.priority, Priority::Default);
    assert!(b.title.contains("limit 1405.20"));
}

#[tokio::test]
async fn hold_and_none_never_produce_a_notification() {
    assert!(Notification::from_candidate("INFY", "INR", &candidate(Action::Hold, false)).is_none());
    assert!(Notification::from_candidate("INFY", "INR", &candidate(Action::None, false)).is_none());
}

#[tokio::test]
async fn notifier_failure_returns_err_and_leaves_no_row() {
    let (_d, pool) = pool().await;
    let fake = FakeNotifier::failing();
    let n = Notification::from_candidate("INFY", "INR", &candidate(Action::Buy, false)).unwrap();
    assert!(notify::dispatch(&pool, &fake, &n).await.is_err());
    let rows: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM notifications")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows.0, 0);
}

#[tokio::test]
async fn failed_send_is_retried_on_the_next_tick() {
    let (_d, pool) = pool().await;
    let n = Notification::from_candidate("INFY", "INR", &candidate(Action::Buy, false)).unwrap();
    assert!(notify::dispatch(&pool, &FakeNotifier::failing(), &n)
        .await
        .is_err());
    let ok = FakeNotifier::default();
    assert!(notify::dispatch(&pool, &ok, &n).await.unwrap());
    assert_eq!(ok.count(), 1);
}

#[tokio::test]
async fn non_ascii_titles_do_not_break_the_header() {
    let n = Notification::from_candidate("INFY", "INR", &candidate(Action::Sell, true)).unwrap();
    assert!(n.title.contains('·'));
    // the header encoder is exercised by Ntfy::send; here we assert the copy is as designed
    assert!(n.body.contains("stop 1412.00 hit"));
}

#[test]
fn a_buy_notification_carries_the_whole_exit_plan() {
    let c = candidate(Action::Buy, false);
    let n = Notification::from_candidate("INFY", "INR", &c).unwrap();
    assert!(n.body.contains("sell if ≤ 1412.00"), "{}", n.body);
    assert!(n.body.contains("take profit ≥ 1510.00"), "{}", n.body);
    assert!(n.title.contains("limit 1405.20"), "{}", n.title);
}

#[test]
fn a_sell_notification_still_names_the_trigger() {
    let c = candidate(Action::Sell, true);
    let n = Notification::from_candidate("INFY", "INR", &c).unwrap();
    assert!(n.body.contains("stop 1412.00 hit"), "{}", n.body);
}

#[test]
fn the_ntfy_request_carries_the_headers_the_service_expects() {
    use stock_predicter::notify::{payload, Notification, Priority};

    let n = Notification {
        title: "SELL · INFY 17 units".into(),
        body: "stop 1200 hit".into(),
        priority: Priority::High,
        dedupe_key: "k".into(),
        signal_id: None,
        symbol: "INFY.NS".into(),
    };

    let (headers, body) = payload(&n, "http://192.168.1.10:8080");
    let get = |name: &str| {
        headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    };

    // ntfy reads these exact names; the non-ASCII middot is folded so the header is legal.
    assert_eq!(get("Title").as_deref(), Some("SELL - INFY 17 units"));
    assert_eq!(get("Priority").as_deref(), Some("high"));
    assert_eq!(get("Tags").as_deref(), Some("rotating_light"));
    assert_eq!(get("Click").as_deref(), Some("http://192.168.1.10:8080/i/INFY.NS"));
    assert_eq!(body, "stop 1200 hit");
}

#[test]
fn without_a_public_url_there_is_no_dead_deep_link() {
    use stock_predicter::notify::{payload, Notification, Priority};
    let n = Notification {
        title: "BUY".into(),
        body: "x".into(),
        priority: Priority::Default,
        dedupe_key: "k".into(),
        signal_id: None,
        symbol: "INFY.NS".into(),
    };
    let (headers, _) = payload(&n, "");
    assert!(headers.iter().all(|(k, _)| k != "Click"), "no link when the host is unknown");
    assert_eq!(
        headers.iter().find(|(k, _)| k == "Priority").map(|(_, v)| v.as_str()),
        Some("default")
    );
    assert_eq!(
        headers.iter().find(|(k, _)| k == "Tags").map(|(_, v)| v.as_str()),
        Some("chart_with_upwards_trend")
    );
}
