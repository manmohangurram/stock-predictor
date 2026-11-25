package runs

import (
	"path/filepath"
	"testing"

	"stock-predictor/internal/db"
)

// The uuid rewrite gave every older run a random id, so a run's key is not a clock. "Last ran"
// must come from started_at: a successful run after this one has to take the card.
func TestTheLatestRunIsTheNewestByTimeNotByKey(t *testing.T) {
	pool, err := db.ConnectAndMigrate(t.Context(), filepath.Join(t.TempDir(), "t.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { pool.Close() })

	// The id that sorts last belongs to the older run.
	rows := [][3]string{
		{"ffffffff-0000-4000-8000-000000000001", "2026-11-20T09:00:00.000000000+00:00", "the older trade plan"},
		{"00000000-0000-4000-8000-000000000002", "2026-11-24T09:00:00.000000000+00:00", "the newer trade plan"},
	}
	for _, row := range rows {
		if _, err := pool.Exec(
			`INSERT INTO runs (id, prompt, trigger, status, started_at, summary)
             VALUES (?, 'plan_trade', 'schedule', 'ok', ?, ?)`,
			row[0], row[1], row[2]); err != nil {
			t.Fatal(err)
		}
	}

	latest, err := LatestByPrompt(pool)
	if err != nil {
		t.Fatal(err)
	}
	if len(latest) != 1 || latest[0].ID != rows[1][0] {
		t.Fatalf("last ran must be the newest run, whatever its id: %+v", latest)
	}
	if latest[0].Summary == nil || *latest[0].Summary != rows[1][2] {
		t.Fatalf("%+v", latest[0])
	}

	recent, err := Recent(pool, 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(recent) != 2 || recent[0].ID != rows[1][0] {
		t.Fatalf("the runs list is newest first: %+v", recent)
	}
}
