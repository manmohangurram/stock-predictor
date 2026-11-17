package runs

import (
	"database/sql"
	"path/filepath"
	"testing"
	"time"

	"stock-predictor/internal/db"
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

func TestARunExistsWhileTheWorkIsGoing(t *testing.T) {
	p := pool(t)
	symbol := "INFY.NS"
	id, err := Start(p, Decide, TriggerManual, &symbol)
	if err != nil {
		t.Fatal(err)
	}
	if n, err := Running(p); err != nil || n != 1 {
		t.Fatalf("running=%d err=%v", n, err)
	}
	if err := Finish(p, id, "done it", nil); err != nil {
		t.Fatal(err)
	}
	if n, _ := Running(p); n != 0 {
		t.Fatal("finished runs are not running")
	}
	rows, err := Recent(p, 10)
	if err != nil || len(rows) != 1 {
		t.Fatalf("%v %v", rows, err)
	}
	if rows[0].Summary == nil || *rows[0].Summary != "done it" {
		t.Fatalf("%+v", rows[0].Summary)
	}
}

func TestAFailedRunKeepsItsError(t *testing.T) {
	p := pool(t)
	id, _ := Start(p, PlanStock, TriggerSchedule, nil)
	if err := Fail(p, id, "the reply did not parse"); err != nil {
		t.Fatal(err)
	}
	rows, _ := Recent(p, 10)
	if len(rows) != 1 || rows[0].Error == nil || *rows[0].Error != "the reply did not parse" {
		t.Fatalf("%+v", rows)
	}
}

func TestOrphansAreClosedAtBoot(t *testing.T) {
	p := pool(t)
	if _, err := Start(p, Poll, TriggerSchedule, nil); err != nil {
		t.Fatal(err)
	}
	n, err := CloseOrphans(p, time.Now().UTC())
	if err != nil || n != 1 {
		t.Fatalf("%d %v", n, err)
	}
	rows, _ := Recent(p, 10)
	if rows[0].Status != "failed" || rows[0].Error == nil {
		t.Fatalf("%+v", rows[0])
	}
}

func TestLatestByPromptIsOneRowPerKind(t *testing.T) {
	p := pool(t)
	for i := 0; i < 2; i++ {
		id, _ := Start(p, Poll, TriggerSchedule, nil)
		_ = Finish(p, id, "poll", nil)
	}
	id, _ := Start(p, PlanTrade, TriggerSchedule, nil)
	_ = Finish(p, id, "plan", nil)

	rows, err := LatestByPrompt(p)
	if err != nil || len(rows) != 2 {
		t.Fatalf("%v %v", rows, err)
	}
}

func TestParsePromptKnowsTheHistoricalKind(t *testing.T) {
	if p, ok := ParsePrompt("short"); !ok || p != Short {
		t.Fatal("old rows must still read")
	}
	if _, ok := ParsePrompt("nonsense"); ok {
		t.Fatal("unknown prompt accepted")
	}
}
