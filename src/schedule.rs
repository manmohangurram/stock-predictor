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
    // The session is 09:15–15:30; the wider 09:00–16:00 window picks up the open and the close.
    (9 * 60..=16 * 60).contains(&mins)
}

/// Outside the session the only thing that changes is news, so a pass every two hours is
/// enough to have a decision ready for the next open.
pub fn off_hours_interval(now: DateTime<Utc>) -> u64 {
    if in_stock_window(now) {
        0 // caller uses its own in-session cadence
    } else {
        7_200
    }
}

/// One pass a day for AMFI NAV and IPO status, after the Indian evening.
pub fn is_daily_pass(now: DateTime<Utc>) -> bool {
    let ist = now + chrono::Duration::minutes(330);
    ist.hour() == 19 && ist.minute() < 10
}
