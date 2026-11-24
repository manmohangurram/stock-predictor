package engine

import (
	"strings"
	"testing"

	"stock-predictor/internal/llm"
)

// The fund plan was asked to choose between funds the book already owned. A fund you hold is not
// a candidate; a fund the scan proposed and you do not hold is.
func TestTheFundListDoesNotReOfferWhatYouHold(t *testing.T) {
	p := pool(t)
	mustExec(t, p, `INSERT INTO instruments (id, symbol, class, name, currency, active) VALUES
		('01a1f000-0000-7000-8000-0000000000a1', 'HELD.BO', 'mutual_fund', 'Held Fund', 'INR', 1),
		('01a1f000-0000-7000-8000-0000000000a2', 'NEW.BO', 'mutual_fund', 'New Fund', 'INR', 1),
		('01a1f000-0000-7000-8000-0000000000a3', 'NONAV.BO', 'mutual_fund', 'No Nav Yet', 'INR', 1)`)
	mustExec(t, p, `INSERT INTO bars (instrument_id, ts, close) VALUES
		('01a1f000-0000-7000-8000-0000000000a1', '2026-11-20T00:00:00Z', 100),
		('01a1f000-0000-7000-8000-0000000000a1', '2026-11-21T00:00:00Z', 101),
		('01a1f000-0000-7000-8000-0000000000a2', '2026-11-20T00:00:00Z', 50),
		('01a1f000-0000-7000-8000-0000000000a2', '2026-11-21T00:00:00Z', 51)`)
	mustExec(t, p, `INSERT INTO positions (id, instrument_id, qty, avg_cost, opened_at, state, mode) VALUES
		('01a1f000-0000-7000-8000-0000000000b1', '01a1f000-0000-7000-8000-0000000000a1', 10, 90, '2026-11-01T00:00:00Z', 'open', 'long')`)

	out, err := candidatesForPlan(p, "INR", llm.ScopeFund)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(out, "HELD.BO") {
		t.Fatalf("a fund you already hold is not a candidate:\n%s", out)
	}
	if !strings.Contains(out, "NEW.BO New Fund at 51.00 INR, +2.00% today") {
		t.Fatalf("an unheld fund must be offered with its move:\n%s", out)
	}
	if strings.Contains(out, "NONAV.BO") {
		t.Fatalf("a fund with no NAV cannot be judged:\n%s", out)
	}
}

func TestWithNothingNewTheFundListSaysSo(t *testing.T) {
	p := pool(t)
	mustExec(t, p, `INSERT INTO instruments (id, symbol, class, name, currency, active) VALUES
		('01a1f000-0000-7000-8000-0000000000c1', 'ONLY.BO', 'mutual_fund', 'Only Fund', 'INR', 1)`)
	mustExec(t, p, `INSERT INTO bars (instrument_id, ts, close) VALUES
		('01a1f000-0000-7000-8000-0000000000c1', '2026-11-21T00:00:00Z', 40)`)
	mustExec(t, p, `INSERT INTO positions (id, instrument_id, qty, avg_cost, opened_at, state, mode) VALUES
		('01a1f000-0000-7000-8000-0000000000d1', '01a1f000-0000-7000-8000-0000000000c1', 5, 38, '2026-11-01T00:00:00Z', 'open', 'long')`)

	out, err := candidatesForPlan(p, "INR", llm.ScopeFund)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out, "nothing new on offer today") {
		t.Fatalf("%q", out)
	}
}
