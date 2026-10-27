use stock_predicter::market::amfi;

const FIXTURE_NAVALL: &str = include_str!("fixtures/amfi_navall.txt");

#[test]
fn parses_semicolon_separated_nav_row() {
    let row = amfi::parse_nav(FIXTURE_NAVALL, "120503").unwrap();
    assert_eq!(row.scheme_code, "120503");
    assert!((row.nav - 45.6721).abs() < 1e-9);
    assert_eq!(row.date, "2026-10-24");
    assert!(row.name.contains("Axis Bluechip"));
}

#[test]
fn unknown_scheme_is_an_error_not_a_zero_nav() {
    assert!(amfi::parse_nav(FIXTURE_NAVALL, "000000").is_err());
}

#[test]
fn header_and_blank_lines_are_skipped() {
    let row = amfi::parse_nav(FIXTURE_NAVALL, "119551").unwrap();
    assert!((row.nav - 78.9540).abs() < 1e-9);
}
