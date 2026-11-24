// Package schedule holds the market clock: when the exchange is open and when a trade plan
// has a slot.
package schedule

import (
	"fmt"
	"time"
)

// NSE session. Polling starts five minutes before the open and stops five after the close.
func InStockWindow(now time.Time) bool {
	ist := now.Add(330 * time.Minute) // 15:30 IST == 10:00 UTC; NSE has no DST
	if ist.Weekday() == time.Saturday || ist.Weekday() == time.Sunday {
		return false
	}
	mins := ist.Hour()*60 + ist.Minute()
	// The session is 09:15–15:30; the wider 09:00–16:00 window picks up the open and the close.
	return mins >= 9*60 && mins <= 16*60
}

// The trade plan's slots: inside the session, at 09:05, 11:15 and 13:15 IST. The first one is
// five minutes before the open, which is as early as a plan can be read and acted on.
//
// The slot id is what the marker stores — 2026-11-11T11 — so a retry inside the same slot is
// free, and a new slot is a new question. Outside the session there is no slot: a trade that
// cannot be entered is not worth a call.
func TradeSlot(now time.Time) (string, bool) {
	ist := now.Add(330 * time.Minute)
	if ist.Weekday() == time.Saturday || ist.Weekday() == time.Sunday {
		return "", false
	}
	mins := ist.Hour()*60 + ist.Minute()
	var slotHour int
	switch {
	case mins >= 545 && mins <= 674: // 09:05–11:14
		slotHour = 9
	case mins >= 675 && mins <= 794: // 11:15–13:14
		slotHour = 11
	case mins >= 795 && mins <= 929: // 13:15–15:29
		slotHour = 13
	default:
		return "", false // before the open, or too late to enter a T+1 trade
	}
	return fmt.Sprintf("%04d-%02d-%02dT%02d", ist.Year(), int(ist.Month()), ist.Day(), slotHour), true
}

// ISTDay is YYYY-MM-DD of the current IST day — the unit the daily gates are written in.
func ISTDay(now time.Time) string {
	ist := now.Add(330 * time.Minute)
	return fmt.Sprintf("%04d-%02d-%02d", ist.Year(), int(ist.Month()), ist.Day())
}

// The daily plans ask once per IST day, and not before 09:05: a book question answered at 00:15
// is answered about yesterday's closes with the exchange shut, and the run then owns the day.
func DailySlot(now time.Time) (string, bool) {
	ist := now.Add(330 * time.Minute)
	if ist.Hour()*60+ist.Minute() < 9*60+5 {
		return "", false
	}
	return ISTDay(now), true
}

// When the daily slot next opens: 09:05 IST today if it has not passed, else tomorrow's.
func NextDailySlot(now time.Time) time.Time {
	loc := time.FixedZone("IST", 330*60)
	ist := now.In(loc)
	at := time.Date(ist.Year(), ist.Month(), ist.Day(), 9, 5, 0, 0, loc)
	if !at.After(now) {
		return at.AddDate(0, 0, 1)
	}
	return at
}

// UntilNextSlot is the time to the next boundary — 09:05, 11:15 or 13:15 IST — so the loop can
// wake on the slot instead of wherever a fixed interval happens to land. Zero when one is due now.
func UntilNextSlot(now time.Time) time.Duration {
	ist := now.In(time.FixedZone("IST", 330*60))
	for day := 0; day < 5; day++ {
		d := ist.AddDate(0, 0, day)
		if d.Weekday() == time.Saturday || d.Weekday() == time.Sunday {
			continue
		}
		for _, hm := range [][2]int{{9, 5}, {11, 15}, {13, 15}} {
			at := time.Date(d.Year(), d.Month(), d.Day(), hm[0], hm[1], 0, 0, d.Location())
			if at.Equal(ist) {
				return 0
			}
			if at.After(ist) {
				return at.Sub(ist)
			}
		}
	}
	return 0
}
