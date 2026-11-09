const FIXTURE_CHART: &str = include_str!("fixtures/yahoo_chart.json");
const FIXTURE_NEWS: &str = include_str!("fixtures/yahoo_news.json");

use stock_predictor::market::yahoo;

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
        yahoo::parse_chart(r#"{"chart":{"result":[]}}"#)
            .unwrap()
            .len(),
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
    assert_eq!(
        yahoo::chart_url("INFY.NS"),
        "https://query2.finance.yahoo.com/v8/finance/chart/INFY.NS"
    );
}

#[test]
fn search_keeps_both_the_exchange_listing_and_the_fund() {
    // The fund code used to be dropped as unpriceable. It is priceable — the chart endpoint
    // returns a NAV series for it — so it is kept, as a fund rather than as a stock.
    let body = r#"{"quotes":[
      {"symbol":"INFY.NS","shortname":"Infosys","exchDisp":"NSE","quoteType":"EQUITY"},
      {"symbol":"0P0000N0B6.BO","longname":"HDFC Flexi Cap Dir Gr","exchDisp":"BSE","quoteType":"MUTUALFUND"}
    ]}"#;
    let hits = stock_predictor::market::search::parse_yahoo(body).unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].code, "INFY.NS");
    assert_eq!(hits[0].venue, "NSE");
    assert_eq!(hits[1].code, "0P0000N0B6.BO");
    assert_eq!(hits[1].name, "HDFC Flexi Cap Dir Gr");
    assert_eq!(
        hits[1].class,
        stock_predictor::models::AssetClass::MutualFund
    );
}

#[test]
fn amfi_search_finds_a_scheme_by_name_and_by_code() {
    let text = include_str!("fixtures/amfi_navall.txt");
    let by_name = stock_predictor::market::search::search_amfi(text, "axis elss", 5);
    assert_eq!(by_name.len(), 1, "{by_name:?}");
    assert_eq!(by_name[0].code, "120503");
    assert_eq!(
        by_name[0].class,
        stock_predictor::models::AssetClass::MutualFund
    );
    assert!(by_name[0].price.is_some());

    let by_code = stock_predictor::market::search::amfi_by_code(text, "120503").unwrap();
    assert!(by_code.name.contains("Axis ELSS"));
}

/// Yahoo's real answer for an Indian mutual fund: an opaque code that is still priceable.
#[test]
fn a_yahoo_mutual_fund_is_a_search_hit_of_class_fund() {
    let body = r#"{"quotes":[
      {"exchange":"BSE","shortname":"0P00012ALS.BO","quoteType":"MUTUALFUND",
       "symbol":"0P00012ALS.BO","longname":"Motilal Oswal Midcap Dir Gr","exchDisp":"Bombay"},
      {"exchange":"NSI","shortname":"INFY.NS","quoteType":"EQUITY","symbol":"INFY.NS","exchDisp":"NSE"}
    ]}"#;
    let hits = stock_predictor::market::search::parse_yahoo(body).unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].code, "0P00012ALS.BO");
    // The readable name, not the ticker repeated back.
    assert_eq!(hits[0].name, "Motilal Oswal Midcap Dir Gr");
    assert_eq!(
        hits[0].class,
        stock_predictor::models::AssetClass::MutualFund
    );
    assert_eq!(hits[1].class, stock_predictor::models::AssetClass::Stock);
}

#[test]
fn a_quote_type_that_is_not_an_instrument_is_still_ignored() {
    let body = r#"{"quotes":[{"symbol":"^NSEI","quoteType":"CRYPTOCURRENCY","shortname":"x"}]}"#;
    assert!(stock_predictor::market::search::parse_yahoo(body)
        .unwrap()
        .is_empty());
}
