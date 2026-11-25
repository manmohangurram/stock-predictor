package api

import (
	"encoding/json"
	"net/http"
	"testing"
)

// The uuid rewrite gave every older row a random id, so a list cut on the key is not a list cut
// on the clock. Both lists the pages read must be newest-first by the time they show.
func TestTheAuditIsOrderedByTheTimeItShows(t *testing.T) {
	pool, r, _ := setup(t)
	// The id that sorts last is the older call: ordering on the key would put it first.
	rows := [][2]string{
		{"ffffffff-0000-4000-8000-000000000001", "2026-11-20T09:00:00.000000000+00:00"},
		{"00000000-0000-4000-8000-000000000002", "2026-11-24T09:00:00.000000000+00:00"},
	}
	for _, row := range rows {
		if _, err := pool.Exec(
			`INSERT INTO api_calls (id, ts, endpoint) VALUES (?, ?, 'chat')`, row[0], row[1]); err != nil {
			t.Fatal(err)
		}
	}
	code, body := do(t, r, http.MethodGet, "/api/calls?limit=10", nil, nil)
	if code != http.StatusOK {
		t.Fatalf("%d %s", code, body)
	}
	var calls []struct {
		ID string `json:"id"`
		TS string `json:"ts"`
	}
	if err := json.Unmarshal(body, &calls); err != nil {
		t.Fatalf("%v: %s", err, body)
	}
	if len(calls) != 2 {
		t.Fatalf("%s", body)
	}
	if calls[0].ID != rows[1][0] {
		t.Fatalf("the newest call must come first: %+v", calls)
	}
}

func TestTheDecisionsAreOrderedByTheTimeTheyShow(t *testing.T) {
	pool, r, id := setup(t)
	rows := [][2]string{
		{"ffffffff-0000-4000-8000-000000000003", "2026-11-20T09:00:00.000000000+00:00"},
		{"00000000-0000-4000-8000-000000000004", "2026-11-24T09:00:00.000000000+00:00"},
	}
	for i, row := range rows {
		if _, err := pool.Exec(
			`INSERT INTO signals (id, instrument_id, action, qty, entry, valid_until, reason_json,
                created_at, status, mode)
             VALUES (?, ?, 'BUY', ?, 100, '2030-01-01T00:00:00Z', '{}', ?, 'pending', 'long')`,
			row[0], id, float64(10+i), row[1]); err != nil {
			t.Fatal(err)
		}
	}
	code, body := do(t, r, http.MethodGet, "/api/dashboard?mode=long", nil, nil)
	if code != http.StatusOK {
		t.Fatalf("%d %s", code, body)
	}
	var dash Dashboard
	if err := json.Unmarshal(body, &dash); err != nil {
		t.Fatal(err)
	}
	if len(dash.Signals) != 2 {
		t.Fatalf("%+v", dash.Signals)
	}
	if dash.Signals[0].ID != rows[1][0] {
		t.Fatalf("the newest decision must come first: %+v", dash.Signals)
	}
}
