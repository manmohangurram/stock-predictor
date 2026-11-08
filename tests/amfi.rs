use stock_predictor::market::amfi;

/// Captured from the live file on 2026-10-27: eight columns, CRLF, NAV at index 6.
const FIXTURE_NAVALL: &str = include_str!("fixtures/amfi_navall.txt");

/// The older six-column shape, still parsed as a fallback (no header mapping available).
const LEGACY: &str = "Scheme Code;Scheme Name;ISIN Div Payout/ ISIN Growth;ISIN Div Reinvestment;Net Asset Value;Date\n\
120503;Axis Bluechip Fund - Growth;INF846K01EW2;-;45.6721;24-Oct-2026\n";

#[test]
fn reads_nav_from_the_current_eight_column_file() {
    let row = amfi::parse_nav(FIXTURE_NAVALL, "120503").unwrap();
    assert_eq!(row.scheme_code, "120503");
    assert!((row.nav - 105.3605).abs() < 1e-9, "nav was {}", row.nav);
    assert_eq!(row.date, "2026-10-27");
    assert!(row.name.contains("Axis ELSS"), "name was {}", row.name);
    assert!(row.name.contains("Direct Plan"), "plan is part of the name");
}

#[test]
fn falls_back_to_the_legacy_six_column_layout() {
    let row = amfi::parse_nav(LEGACY, "120503").unwrap();
    assert!((row.nav - 45.6721).abs() < 1e-9);
    assert_eq!(row.date, "2026-10-24");
}

#[test]
fn unknown_scheme_is_an_error_not_a_zero_nav() {
    assert!(amfi::parse_nav(FIXTURE_NAVALL, "000000").is_err());
}

#[test]
fn crlf_line_endings_do_not_poison_the_nav() {
    let crlf = FIXTURE_NAVALL.replace('\n', "\r\n");
    let row = amfi::parse_nav(&crlf, "120503").unwrap();
    assert!((row.nav - 105.3605).abs() < 1e-9);
}
