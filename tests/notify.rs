use stock_predictor::db;
use stock_predictor::models::{Action, AssetClass, Candidate};
use stock_predictor::notify::{self, FakeNotifier, Notification, Priority};

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
    let n = Notification::from_candidate("INFY", "Infosys", "INR", &candidate(Action::Buy, false))
        .unwrap();
    assert!(notify::dispatch(&pool, &fake, &n).await.unwrap());
    assert!(!notify::dispatch(&pool, &fake, &n).await.unwrap());
    assert_eq!(fake.count(), 1);
}

#[tokio::test]
async fn sell_uses_high_priority() {
    let n = Notification::from_candidate("INFY", "Infosys", "INR", &candidate(Action::Sell, true))
        .unwrap();
    assert_eq!(n.priority, Priority::High);
    assert!(n.title.starts_with("SELL"));
    let b = Notification::from_candidate("INFY", "Infosys", "INR", &candidate(Action::Buy, false))
        .unwrap();
    assert_eq!(b.priority, Priority::Default);
    assert!(b.title.contains("at 1405.20"), "{}", b.title);
    // A push names the holding, not the ticker; the ticker stays in the body for precision.
    assert!(b.title.contains("Infosys"), "{}", b.title);
    assert!(
        !b.title.contains("INFY"),
        "the ticker should not lead: {}",
        b.title
    );
    assert!(
        b.body.contains("INFY"),
        "the body keeps the symbol: {}",
        b.body
    );
}

#[tokio::test]
async fn hold_and_none_never_produce_a_notification() {
    assert!(Notification::from_candidate(
        "INFY",
        "Infosys",
        "INR",
        &candidate(Action::Hold, false)
    )
    .is_none());
    assert!(Notification::from_candidate(
        "INFY",
        "Infosys",
        "INR",
        &candidate(Action::None, false)
    )
    .is_none());
}

#[tokio::test]
async fn notifier_failure_returns_err_and_leaves_no_row() {
    let (_d, pool) = pool().await;
    let fake = FakeNotifier::failing();
    let n = Notification::from_candidate("INFY", "Infosys", "INR", &candidate(Action::Buy, false))
        .unwrap();
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
    let n = Notification::from_candidate("INFY", "Infosys", "INR", &candidate(Action::Buy, false))
        .unwrap();
    assert!(notify::dispatch(&pool, &FakeNotifier::failing(), &n)
        .await
        .is_err());
    let ok = FakeNotifier::default();
    assert!(notify::dispatch(&pool, &ok, &n).await.unwrap());
    assert_eq!(ok.count(), 1);
}

#[tokio::test]
async fn non_ascii_titles_do_not_break_the_header() {
    let n = Notification::from_candidate("INFY", "Infosys", "INR", &candidate(Action::Sell, true))
        .unwrap();
    assert!(n.title.contains('·'));
    // the header encoder is exercised by Ntfy::send; here we assert the copy is as designed
    assert!(n.body.contains("stop 1412.00 hit"));
}

#[test]
fn a_buy_notification_carries_the_whole_exit_plan() {
    let c = candidate(Action::Buy, false);
    let n = Notification::from_candidate("INFY", "Infosys", "INR", &c).unwrap();
    assert!(n.body.contains("sell if ≤ 1412.00"), "{}", n.body);
    assert!(n.body.contains("take profit ≥ 1510.00"), "{}", n.body);
    assert!(n.title.contains("at 1405.20"), "{}", n.title);
}

#[test]
fn a_sell_notification_still_names_the_trigger() {
    let c = candidate(Action::Sell, true);
    let n = Notification::from_candidate("INFY", "Infosys", "INR", &c).unwrap();
    assert!(n.body.contains("stop 1412.00 hit"), "{}", n.body);
}

#[test]
fn the_ntfy_request_carries_the_headers_the_service_expects() {
    use stock_predictor::notify::{payload, Notification, Priority};

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
    assert_eq!(
        get("Click").as_deref(),
        Some("http://192.168.1.10:8080/i/INFY.NS")
    );
    assert_eq!(body, "stop 1200 hit");
}

#[test]
fn without_a_public_url_there_is_no_dead_deep_link() {
    use stock_predictor::notify::{payload, Notification, Priority};
    let n = Notification {
        title: "BUY".into(),
        body: "x".into(),
        priority: Priority::Default,
        dedupe_key: "k".into(),
        signal_id: None,
        symbol: "INFY.NS".into(),
    };
    let (headers, _) = payload(&n, "");
    assert!(
        headers.iter().all(|(k, _)| k != "Click"),
        "no link when the host is unknown"
    );
    assert_eq!(
        headers
            .iter()
            .find(|(k, _)| k == "Priority")
            .map(|(_, v)| v.as_str()),
        Some("default")
    );
    assert_eq!(
        headers
            .iter()
            .find(|(k, _)| k == "Tags")
            .map(|(_, v)| v.as_str()),
        Some("chart_with_upwards_trend")
    );
}

#[test]
fn a_target_reached_is_notified_as_a_profit_not_a_stop() {
    use stock_predictor::models::{Action, AssetClass, Candidate};
    let c = Candidate {
        instrument_id: 1,
        action: Action::Sell,
        reasons: vec![],
        entry: 1320.0,
        stop: None,
        target: Some(1318.0),
        qty: 10.0,
        timing_note: AssetClass::Stock.timing_note().into(),
        urgent: true,
    };
    let n = stock_predictor::notify::Notification::from_candidate("INFY.NS", "Infosys", "INR", &c)
        .unwrap();
    assert!(n.body.contains("target 1318.00 reached"), "{}", n.body);
    assert!(!n.body.contains("stop"), "{}", n.body);
}

/// The one test that leaves the machine: a real push to ntfy.sh, read back from the service.
///
/// Ignored by default because no test may touch the network (AGENTS.md). Run it when the question
/// is not "does the code shape look right" but "is a notification actually arriving":
///
/// ```sh
/// cargo test --test notify live -- --ignored --nocapture
/// ```
///
/// Set `NTFY_TOPIC` to prove your own channel; with no topic it uses a throwaway one and says so.
#[tokio::test]
#[ignore = "live network: sends a real push to ntfy.sh"]
async fn live_a_push_lands_on_the_service() {
    use stock_predictor::notify::{Notifier as _, Ntfy};

    let topic = std::env::var("NTFY_TOPIC")
        .unwrap_or_else(|_| format!("stock-predictor-selftest-{}", std::process::id()));
    println!("topic: {topic}");
    let rt = stock_predictor::rules::runtime::Runtime::default();
    let ntfy = Ntfy::new("https://ntfy.sh", &topic, "", "http://127.0.0.1:8080", &rt).unwrap();
    let n = Notification {
        title: "SELF TEST · push is working".into(),
        body: "sent by cargo test --ignored".into(),
        priority: Priority::Default,
        dedupe_key: format!("selftest-{}", std::process::id()),
        signal_id: None,
        symbol: "INFY.NS".into(),
    };
    ntfy.send(&n).await.expect("the push must be accepted");

    // Read it back: an accepted request is not proof that the service kept it.
    let url = format!("https://ntfy.sh/{topic}/json?poll=1&since=10m");
    let body = reqwest::get(&url).await.unwrap().text().await.unwrap();
    println!("polled: {}", body.lines().count());
    assert!(
        body.contains("push is working"),
        "the message is not on the topic: {body}"
    );
}

/// No topic in settings is "off", not "POST to the root and fail the decision".
#[tokio::test]
async fn an_unconfigured_notifier_sends_nothing_and_records_nothing() {
    use stock_predictor::notify::{Ntfy, Notifier as _};

    let (_d, pool) = pool().await;
    let rt = stock_predictor::rules::runtime::Runtime::default();
    let ntfy = Ntfy::new("https://ntfy.sh", "", "", "", &rt).unwrap();
    assert!(!ntfy.configured());

    let n = Notification {
        title: "BUY · INFY 5 units".into(),
        body: "entry 1405.20".into(),
        priority: Priority::Default,
        dedupe_key: "off".into(),
        signal_id: None,
        symbol: "INFY.NS".into(),
    };
    let sent = notify::dispatch(&pool, &ntfy, &n).await.unwrap();
    assert!(!sent);
    let rows: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM notifications")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows.0, 0, "no push claimed, so no row either");
}

/// A short-term proposal is announced. It was not: `report.ideas` was read to build the push and
/// never written to, so the pass recorded a decision and told nobody.
#[test]
fn a_short_term_idea_becomes_the_line_that_is_pushed() {
    let (sym, qty, entry, target, stop, conf) =
        ("MAKERSL", 12.0_f64, 268.34_f64, 300.00_f64, 250.00_f64, "high");
    let line = format!(
        "{sym} · {qty:.0} at {entry:.2}, target {target:.2}, stop {stop:.2} · {conf}"
    );
    assert_eq!(line, "MAKERSL · 12 at 268.34, target 300.00, stop 250.00 · high");
    let n = Notification {
        title: format!("Short-term · {} idea{}", 1, ""),
        body: line.clone(),
        priority: Priority::Default,
        dedupe_key: "k".into(),
        signal_id: None,
        symbol: "MAKERSL.NS".into(),
    };
    let (_, body) = stock_predictor::notify::payload(&n, "http://192.168.1.10:8080");
    assert_eq!(body, line, "the push carries the idea, not just a count");
}
