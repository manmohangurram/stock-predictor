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

/// The trade plan's slots: every two hours inside the session, at 09:15, 11:15 and 13:15 IST.
///
/// The slot id is what the marker stores — `2026-11-11T11` — so a retry inside the same slot is
/// free, and a new slot is a new question. Outside the session there is no slot: a trade that
/// cannot be entered is not worth a call.
pub fn trade_slot(now: DateTime<Utc>) -> Option<String> {
    let ist = now + chrono::Duration::minutes(330);
    if matches!(ist.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun) {
        return None;
    }
    let mins = ist.hour() * 60 + ist.minute();
    let slot_hour = match mins {
        555..=674 => 9,   // 09:15–11:14
        675..=794 => 11,  // 11:15–13:14
        795..=929 => 13,  // 13:15–15:29
        _ => return None, // before the open, or too late to enter a T+1 trade
    };
    Some(format!(
        "{:04}-{:02}-{:02}T{slot_hour:02}",
        ist.year(),
        ist.month(),
        ist.day()
    ))
}

#[cfg(test)]
mod slot_tests {
    use super::*;
    use chrono::TimeZone;

    fn ist(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap() - chrono::Duration::minutes(330)
    }

    #[test]
    fn the_session_has_three_slots_and_nothing_outside_it() {
        assert_eq!(
            trade_slot(ist(2026, 11, 11, 9, 15)).as_deref(),
            Some("2026-11-11T09")
        );
        assert_eq!(
            trade_slot(ist(2026, 11, 11, 10, 59)).as_deref(),
            Some("2026-11-11T09")
        );
        assert_eq!(
            trade_slot(ist(2026, 11, 11, 11, 15)).as_deref(),
            Some("2026-11-11T11")
        );
        assert_eq!(
            trade_slot(ist(2026, 11, 11, 13, 20)).as_deref(),
            Some("2026-11-11T13")
        );
        assert_eq!(
            trade_slot(ist(2026, 11, 11, 15, 30)),
            None,
            "too late to enter"
        );
        assert_eq!(trade_slot(ist(2026, 11, 11, 9, 0)), None, "before the open");
        assert_eq!(trade_slot(ist(2026, 11, 7, 11, 0)), None, "Saturday");
    }

    #[test]
    fn a_slot_id_is_unique_per_day_and_slot_not_per_tick() {
        let a = trade_slot(ist(2026, 11, 11, 11, 16)).unwrap();
        let b = trade_slot(ist(2026, 11, 11, 12, 55)).unwrap();
        assert_eq!(a, b, "the same slot, however many ticks happen inside it");
        assert_ne!(a, trade_slot(ist(2026, 11, 11, 13, 16)).unwrap());
        assert_ne!(a, trade_slot(ist(2026, 11, 12, 11, 16)).unwrap());
    }
}
