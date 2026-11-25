package market

import (
	"strconv"
	"testing"
	"time"
)

// The tape asks for one day, so the request carries one day — not the five it used to.
func TestTheTapeRangeIsOneDay(t *testing.T) {
	now := time.Date(2026, 11, 25, 4, 0, 0, 0, time.UTC)
	q := chartQuery("15m", "1d", now)
	if got := q.Get("interval"); got != "15m" {
		t.Fatalf("interval %q", got)
	}
	from, err := strconv.ParseInt(q.Get("period1"), 10, 64)
	if err != nil {
		t.Fatal(err)
	}
	if now.Sub(time.Unix(from, 0)) != 24*time.Hour {
		t.Fatalf("period1 is %v before now, wanted 24h", now.Sub(time.Unix(from, 0)))
	}
	if to, _ := strconv.ParseInt(q.Get("period2"), 10, 64); to != now.Unix() {
		t.Fatalf("period2 %d", to)
	}
}
