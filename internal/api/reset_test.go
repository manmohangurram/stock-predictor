package api

import (
	"net/http"
	"testing"
)

func TestDeletingEverythingEmptiesTheBookAndKeepsTheAdmin(t *testing.T) {
	pool, r, _ := setup(t)

	if code, body := do(t, r, http.MethodPost, "/api/reset", map[string]any{"symbol": "NOT-IN-THE-BOOK"}, nil); code != http.StatusBadRequest {
		t.Fatalf("an unknown symbol answered %d %s", code, body)
	}
	if n := countRows(t, pool, "SELECT COUNT(*) FROM instruments"); n != 1 {
		t.Fatalf("a refused reset still deleted: %d instruments", n)
	}

	code, body := do(t, r, http.MethodPost, "/api/reset", map[string]any{"symbol": "infy.ns"}, nil)
	if code != http.StatusOK {
		t.Fatalf("%d %s", code, body)
	}

	for _, table := range []string{"instruments", "signals", "trades", "positions", "cash_ledger", "settings", "api_keys", "mcp_calls"} {
		if n := countRows(t, pool, "SELECT COUNT(*) FROM "+table); n != 0 {
			t.Fatalf("%s still holds %d rows", table, n)
		}
	}
	if n := countRows(t, pool, "SELECT COUNT(*) FROM admin"); n != 1 {
		t.Fatalf("the admin went with the book: %d", n)
	}

	// still signed in: the token setup() holds must keep working on the empty book
	if code, body := do(t, r, http.MethodGet, "/api/dashboard?mode=long", nil, nil); code != http.StatusOK {
		t.Fatalf("the session did not survive the reset: %d %s", code, body)
	}
}
