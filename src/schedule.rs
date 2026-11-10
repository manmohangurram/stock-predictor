use chrono::{DateTime, Datelike, Timelike, Utc};

/// NSE session. Polling starts five minutes before the open and stops five after the close.
pub fn in_stock_window(now: DateTime<Utc>) -> bool {
    let ist = now + chrono::Duration::minutes(330);
    if matches!(ist.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun) {
        return false;
    }
    let mins = ist.hour() * 60 + ist.minute();
    // The session is 09:15–15:30; the wider 09:00–16:00 window picks up the open and the close.
    (9 * 60..=16 * 60).contains(&mins)
}

