package schedule

import (
	"testing"
	"time"
)

func ist(y int, m time.Month, d, h, min int) time.Time {
	return time.Date(y, m, d, h, min, 0, 0, time.FixedZone("IST", 330*60)).UTC()
}

func TestTheSessionHasThreeSlotsAndNothingOutsideIt(t *testing.T) {
	cases := []struct {
		at   time.Time
		want string
		ok   bool
	}{
		{ist(2026, 11, 11, 9, 5), "2026-11-11T09", true},
		{ist(2026, 11, 11, 9, 4), "", false},
		{ist(2026, 11, 11, 9, 15), "2026-11-11T09", true},
		{ist(2026, 11, 11, 10, 59), "2026-11-11T09", true},
		{ist(2026, 11, 11, 11, 15), "2026-11-11T11", true},
		{ist(2026, 11, 11, 13, 20), "2026-11-11T13", true},
		{ist(2026, 11, 11, 15, 30), "", false},
		{ist(2026, 11, 11, 9, 0), "", false},
		{ist(2026, 11, 7, 11, 0), "", false},
	}
	for _, c := range cases {
		got, ok := TradeSlot(c.at)
		if ok != c.ok || got != c.want {
			t.Fatalf("%s: got %q,%v want %q,%v", c.at, got, ok, c.want, c.ok)
		}
	}
}

func TestASlotIsUniquePerDayAndSlotNotPerTick(t *testing.T) {
	a, _ := TradeSlot(ist(2026, 11, 11, 11, 16))
	b, _ := TradeSlot(ist(2026, 11, 11, 12, 55))
	if a != b {
		t.Fatalf("the same slot, however many ticks: %q %q", a, b)
	}
	c, _ := TradeSlot(ist(2026, 11, 11, 13, 16))
	if a == c {
		t.Fatal("a new slot must be a new question")
	}
}

func TestInStockWindowIsISTAndWeekdaysOnly(t *testing.T) {
	if InStockWindow(ist(2026, 11, 14, 10, 0)) {
		t.Fatal("Saturday is not a session")
	}
	if !InStockWindow(ist(2026, 11, 16, 9, 30)) {
		t.Fatal("Monday morning is a session")
	}
	if InStockWindow(ist(2026, 11, 16, 17, 0)) {
		t.Fatal("after the close")
	}
}

func TestTheDailyPlansWaitForNineOhFive(t *testing.T) {
	if _, ok := DailySlot(ist(2026, 11, 17, 0, 15)); ok {
		t.Fatal("a book question at 00:15 is asked about yesterday with the exchange shut")
	}
	if _, ok := DailySlot(ist(2026, 11, 17, 8, 55)); ok {
		t.Fatal("before 09:05 there is no slot")
	}
	got, ok := DailySlot(ist(2026, 11, 17, 9, 5))
	if !ok || got != "2026-11-17" {
		t.Fatalf("%q %v", got, ok)
	}
	if _, ok := DailySlot(ist(2026, 11, 17, 15, 30)); !ok {
		t.Fatal("the slot stays open after the open")
	}
	if next := NextDailySlot(ist(2026, 11, 17, 7, 0)); !next.Equal(ist(2026, 11, 17, 9, 5)) {
		t.Fatalf("before the slot, next is today: %s", next)
	}
	if next := NextDailySlot(ist(2026, 11, 17, 10, 0)); !next.Equal(ist(2026, 11, 18, 9, 5)) {
		t.Fatalf("after the slot, next is tomorrow: %s", next)
	}
}

func TestISTDayUsesTheMarketDay(t *testing.T) {
	// 19:30 UTC is 01:00 IST the next day.
	if got := ISTDay(time.Date(2026, 11, 30, 19, 30, 0, 0, time.UTC)); got != "2026-12-01" {
		t.Fatalf("%q", got)
	}
}
