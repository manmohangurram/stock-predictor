use chrono::{DateTime, Datelike, Timelike, Utc};

/// NSE session. Polling starts five minutes before the open and stops five after the close.
pub fn in_stock_window(now: DateTime<Utc>) -> bool {
    let ist = now + chrono::Duration::minutes(330);
    if matches!(
        ist.weekday(),
        chrono::Weekday::Sat | chrono::Weekday::Sun
    ) {
        return false;
    }
    let mins = ist.hour() * 60 + ist.minute();
    (9 * 60 + 10..=15 * 60 + 35).contains(&mins)
}

/// One pass a day for AMFI NAV and IPO status, after the Indian evening.
pub fn is_daily_pass(now: DateTime<Utc>) -> bool {
    let ist = now + chrono::Duration::minutes(330);
    ist.hour() == 19 && ist.minute() < 10
}
