package api

import (
	"net/http"
	"testing"
	"time"

	"stock-predictor/internal/ids"
	"stock-predictor/internal/tstamp"
)

func TestTheHistoryKeepsOnlyTheNewestFiveHundred(t *testing.T) {
	pool, r, _ := setup(t)
	if _, err := pool.Exec("INSERT INTO api_keys (id, name, scope, key_hash, prefix, created_at) VALUES (?, 'test', 'write', 'h', 'spk_x', ?)",
		ids.New(), tstamp.Now()); err != nil {
		t.Fatal(err)
	}
	var keyID string
	if err := pool.QueryRow("SELECT id FROM api_keys LIMIT 1").Scan(&keyID); err != nil {
		t.Fatal(err)
	}

	now := time.Now().UTC()
	for i := 0; i < 600; i++ {
		if _, err := pool.Exec(
			`INSERT INTO mcp_calls (id, ts, key_id, tool, args_json, status, ms) VALUES (?, ?, ?, 'get_dashboard', '{}', 'ok', 1)`,
			ids.New(), tstamp.From(now.Add(time.Duration(i)*time.Millisecond)), keyID,
		); err != nil {
			t.Fatal(err)
		}
	}

	code, body := do(t, r, http.MethodGet, "/api/mcp/calls?limit=5000", nil, nil)
	if code != http.StatusOK {
		t.Fatalf("%d %s", code, body)
	}

	var n int
	if err := pool.QueryRow("SELECT COUNT(*) FROM mcp_calls").Scan(&n); err != nil {
		t.Fatal(err)
	}
	if n != 500 {
		t.Fatalf("%d rows kept, want 500", n)
	}

	var newest string
	if err := pool.QueryRow("SELECT id FROM mcp_calls ORDER BY id DESC LIMIT 1").Scan(&newest); err != nil {
		t.Fatal(err)
	}
	if again := ids.New(); again == newest {
		t.Fatal("the id generator repeated itself")
	}

	var oldest string
	if err := pool.QueryRow("SELECT id FROM mcp_calls ORDER BY id ASC LIMIT 1").Scan(&oldest); err != nil {
		t.Fatal(err)
	}
	if oldest >= newest {
		t.Fatalf("the surviving window is not the newest: %s..%s", oldest, newest)
	}
}
