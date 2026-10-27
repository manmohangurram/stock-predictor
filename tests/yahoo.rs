const FIXTURE_CHART: &str = include_str!("fixtures/yahoo_chart.json");
const FIXTURE_NEWS: &str = include_str!("fixtures/yahoo_news.json");

use stock_predicter::market::yahoo;

#[test]
fn parses_chart_fixture_into_bars() {
    let bars = yahoo::parse_chart(FIXTURE_CHART).unwrap();
    assert_eq!(bars.len(), 5);
    assert!((bars[0].close - 141.2).abs() < 1e-9);
    assert_eq!(bars[0].ts, "2024-10-22T01:30:00Z");
    assert_eq!(bars[0].volume, 1_200_000);
}

#[test]
fn empty_chart_yields_empty_vec_not_error() {
    assert_eq!(
        yahoo::parse_chart(r#"{"chart":{"result":[]}}"#).unwrap().len(),
        0
    );
}

#[test]
fn missing_result_is_an_error_so_drift_is_loud() {
    assert!(yahoo::parse_chart(r#"{"chart":{}}"#).is_err());
}

#[test]
fn null_candle_is_skipped_not_zeroed() {
    let body = r#"{"chart":{"result":[{"timestamp":[1,2],"indicators":{"quote":[{"open":[1,2],"high":[1,2],"low":[1,2],"close":[10.0,null],"volume":[5,6]}]}}]}}"#;
    let bars = yahoo::parse_chart(body).unwrap();
    assert_eq!(bars.len(), 1);
    assert!((bars[0].close - 10.0).abs() < 1e-9);
}

#[test]
fn news_parser_keeps_headlines_and_never_a_body() {
    let h = yahoo::parse_news(FIXTURE_NEWS).unwrap();
    assert_eq!(h.len(), 2, "duplicate url must collapse");
    assert_eq!(h[0].title, "Infosys guidance raised");
    assert_eq!(h[0].publisher, "Reuters");
    assert!(h[0].url.starts_with("https://"));
    assert_eq!(h[0].published_at, "2024-10-25T23:46:40Z");
}

#[test]
fn news_shape_change_is_an_error() {
    assert!(yahoo::parse_news(r#"{"items":[]}"#).is_err());
}

#[test]
fn news_query_strips_the_exchange_suffix() {
    assert_eq!(yahoo::news_query("INFY.NS"), "INFY");
    assert_eq!(yahoo::news_query("HDFCBANK.BO"), "HDFCBANK");
    assert_eq!(yahoo::news_query("INFY"), "INFY");
    assert_eq!(yahoo::news_query("BRK.B"), "BRK.B");
}

#[test]
fn chart_url_carries_the_symbol_as_a_path_segment() {
    // Regression: the client once sent /v8/finance/chart?range=1y with no symbol at all,
    // which Yahoo answers with 404 for every request.
    assert_eq!(yahoo::chart_url("INFY.NS"), "https://query1.finance.yahoo.com/v8/finance/chart/INFY.NS");
}
