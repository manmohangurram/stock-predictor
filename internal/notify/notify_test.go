package notify

import (
	"database/sql"
	"path/filepath"
	"strings"
	"testing"

	"stock-predictor/internal/db"
	"stock-predictor/internal/models"
)

func pool(t *testing.T) *sql.DB {
	t.Helper()
	p, err := db.ConnectAndMigrate(t.Context(), filepath.Join(t.TempDir(), "t.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { p.Close() })
	return p
}

func f64(v float64) *float64 { return &v }

func TestASellSaysWhichExitFired(t *testing.T) {
	stop := 90.0
	n := FromCandidate("INFY.NS", "Infosys", "INR", models.Candidate{
		Action: models.ActionSell, Qty: 10, Entry: 89, Stop: &stop,
		TimingNote: "limit order, intraday or next open",
	})
	if n == nil {
		t.Fatal("no notification")
	}
	if n.Priority != PriorityHigh || !strings.Contains(n.Title, "SELL") {
		t.Fatalf("%+v", n)
	}
	if !strings.Contains(n.Body, "stop 90.00 hit") {
		t.Fatalf("%q", n.Body)
	}
}

func TestABuyShowsBothExitsAndTheCost(t *testing.T) {
	stop, target := 90.0, 120.0
	n := FromCandidate("INFY.NS", "Infosys", "INR", models.Candidate{
		Action: models.ActionBuy, Qty: 10, Entry: 100, Stop: &stop, Target: &target,
		TimingNote: "limit order, intraday or next open",
	})
	if n == nil || n.Priority != PriorityDefault {
		t.Fatalf("%+v", n)
	}
	if !strings.Contains(n.Body, "cost 1000.00 INR") ||
		!strings.Contains(n.Body, "sell if ≤ 90.00") ||
		!strings.Contains(n.Body, "take profit ≥ 120.00") {
		t.Fatalf("%q", n.Body)
	}
}

func TestAnUnactionableActionMakesNoNotification(t *testing.T) {
	if n := FromCandidate("X", "X", "INR", models.Candidate{Action: models.ActionHold}); n != nil {
		t.Fatalf("%+v", n)
	}
}

func TestTheDedupeKeyIgnoresQuantity(t *testing.T) {
	stop := 90.0
	a := DedupeKey("INFY.NS", models.Candidate{Action: models.ActionBuy, Entry: 100, Stop: &stop})
	b := DedupeKey("INFY.NS", models.Candidate{Action: models.ActionBuy, Entry: 100, Stop: &stop})
	if a != b {
		t.Fatal("same idea, same key")
	}
	c := DedupeKey("INFY.NS", models.Candidate{Action: models.ActionBuy, Entry: 101, Stop: &stop})
	if a == c {
		t.Fatal("a different price is a different alert")
	}
}

func TestPayloadFoldsNonASCIIAndAddsTheLink(t *testing.T) {
	n := Notification{
		Title: "SELL · Infosys · 10 units", Body: "now 89.00", Priority: PriorityHigh,
		DedupeKey: "k", Symbol: "INFY.NS",
	}
	headers, body := Payload(n, "https://example.in")
	if body != "now 89.00" {
		t.Fatalf("%q", body)
	}
	if strings.ContainsAny(headers["Title"], "·") {
		t.Fatalf("a non-latin-1 header: %q", headers["Title"])
	}
	if headers["Click"] != "https://example.in/i/INFY.NS" {
		t.Fatalf("%q", headers["Click"])
	}
	if headers["Tags"] != "rotating_light" {
		t.Fatalf("%q", headers["Tags"])
	}
}

func TestDispatchSendsOnceAndRecordsOnlyOnSuccess(t *testing.T) {
	p := pool(t)
	n := Notification{Title: "t", Body: "b", Priority: PriorityDefault, DedupeKey: "once"}
	fake := &FakeNotifier{}

	sent, err := Dispatch(p, fake, n)
	if err != nil || !sent {
		t.Fatalf("%v %v", sent, err)
	}
	sent, err = Dispatch(p, fake, n)
	if err != nil || sent {
		t.Fatal("the same key must not send twice")
	}
	if fake.Count() != 1 {
		t.Fatalf("sent %d", fake.Count())
	}

	failing := FailingNotifier()
	n.DedupeKey = "fails"
	if _, err := Dispatch(p, failing, n); err == nil {
		t.Fatal("a failed send must surface")
	}
	var rows int
	if err := p.QueryRow("SELECT COUNT(*) FROM notifications WHERE dedupe_key = 'fails'").Scan(&rows); err != nil {
		t.Fatal(err)
	}
	if rows != 0 {
		t.Fatal("a failed push must not be recorded, or it is never retried")
	}
}
