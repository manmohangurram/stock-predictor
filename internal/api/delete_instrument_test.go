package api

import (
	"database/sql"
	"encoding/json"
	"net/http"
	"testing"

	"stock-predictor/internal/models"
	"stock-predictor/internal/rules"
	"stock-predictor/internal/trades"
)

func TestDeletingAStockRemovesItAndReturnsItsCash(t *testing.T) {
	pool, r, id := setup(t)
	if err := trades.Record(pool, id, trades.SideBuy, 10, 100, nil, nil, models.ModeLong); err != nil {
		t.Fatal(err)
	}
	if err := trades.Record(pool, id, trades.SideSell, 10, 120, nil, nil, models.ModeLong); err != nil {
		t.Fatal(err)
	}
	if _, err := pool.Exec(
		`INSERT INTO bars (instrument_id, ts, close) VALUES (?, '2026-11-20T00:00:00Z', 120)`, id); err != nil {
		t.Fatal(err)
	}

	code, body := do(t, r, http.MethodDelete, "/api/instruments/INFY.NS/records", nil, nil)
	if code != http.StatusOK {
		t.Fatalf("%d %s", code, body)
	}
	var res struct {
		Cash float64 `json:"cash"`
	}
	_ = json.Unmarshal(body, &res)
	if res.Cash != -200 {
		t.Fatalf("the round trip cost 1000 and returned 1200, so deleting it takes that 200 profit back: %+v", res)
	}

	cash, err := rules.CashInBucket(pool, models.ModeLong, "")
	if err != nil {
		t.Fatal(err)
	}
	if cash != 100_000 {
		t.Fatalf("the pot must read as if the stock was never traded: %v", cash)
	}

	for _, table := range []string{"instruments", "bars", "trades", "positions", "signals"} {
		var n int
		if err := pool.QueryRow("SELECT COUNT(*) FROM " + table).Scan(&n); err != nil {
			t.Fatal(err)
		}
		if n != 0 {
			t.Fatalf("%s still holds %d rows", table, n)
		}
	}
}

func TestDeletingAnArchivedStockRemovesItFromTheArchivedPage(t *testing.T) {
	pool, r, id := setup(t)
	if err := trades.Record(pool, id, trades.SideBuy, 10, 100, nil, nil, models.ModeLong); err != nil {
		t.Fatal(err)
	}
	if err := trades.Record(pool, id, trades.SideSell, 10, 110, nil, nil, models.ModeLong); err != nil {
		t.Fatal(err)
	}
	if code, body := do(t, r, http.MethodDelete, "/api/instruments/INFY.NS", nil, nil); code != http.StatusOK {
		t.Fatalf("archiving answered %d %s", code, body)
	}

	_, body := do(t, r, http.MethodGet, "/api/archived", nil, nil)
	var archived []ArchivedRow
	_ = json.Unmarshal(body, &archived)
	if len(archived) != 1 {
		t.Fatalf("the archived page should list it before the delete: %s", body)
	}

	if code, body := do(t, r, http.MethodDelete, "/api/instruments/INFY.NS/records", nil, nil); code != http.StatusOK {
		t.Fatalf("deleting an archived stock answered %d %s", code, body)
	}

	_, body = do(t, r, http.MethodGet, "/api/archived", nil, nil)
	archived = nil
	_ = json.Unmarshal(body, &archived)
	if len(archived) != 0 {
		t.Fatalf("a deleted stock must leave the archived page: %s", body)
	}
}

func TestDeletingAHeldStockIsRefused(t *testing.T) {
	pool, r, id := setup(t)
	if err := trades.Record(pool, id, trades.SideBuy, 10, 100, nil, nil, models.ModeLong); err != nil {
		t.Fatal(err)
	}

	code, body := do(t, r, http.MethodDelete, "/api/instruments/INFY.NS/records", nil, nil)
	if code != http.StatusBadRequest {
		t.Fatalf("a held stock must not be deletable: %d %s", code, body)
	}

	var held float64
	if err := pool.QueryRow(
		"SELECT CAST(COALESCE(SUM(qty), 0) AS REAL) FROM positions WHERE state <> 'closed'",
	).Scan(&held); err != nil && err != sql.ErrNoRows {
		t.Fatal(err)
	}
	if held != 10 {
		t.Fatalf("a refused delete still moved the position: %v held", held)
	}
}
