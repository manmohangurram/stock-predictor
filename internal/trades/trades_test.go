package trades

import (
	"database/sql"
	"path/filepath"
	"testing"
	"time"

	"stock-predictor/internal/db"
	"stock-predictor/internal/ids"
	"stock-predictor/internal/ledger"
	"stock-predictor/internal/models"
	"stock-predictor/internal/rules"
)

func setup(t *testing.T) (*sql.DB, string) {
	t.Helper()
	pool, err := db.ConnectAndMigrate(t.Context(), filepath.Join(t.TempDir(), "t.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { pool.Close() })
	id := ids.New()
	if _, err := pool.Exec(
		"INSERT INTO instruments (id, symbol, class, name, currency, active) VALUES (?, 'INFY.NS', 'stock', 'Infosys', 'INR', 1)",
		id,
	); err != nil {
		t.Fatal(err)
	}
	if _, err := rules.EnsureMonth(pool, models.ModeLong, "", 100_000, time.Now().UTC()); err != nil {
		t.Fatal(err)
	}
	return pool, id
}

func TestASellDownToZeroKeepsTheRowVisible(t *testing.T) {
	pool, id := setup(t)
	if err := Record(pool, id, SideBuy, 10, 100, nil, nil, models.ModeLong); err != nil {
		t.Fatal(err)
	}
	if err := Record(pool, id, SideSell, 10, 120, nil, nil, models.ModeLong); err != nil {
		t.Fatal(err)
	}

	rows, err := ledger.Positions(pool, models.ModeLong)
	if err != nil {
		t.Fatal(err)
	}
	if len(rows) != 1 || rows[0].Qty != 0 {
		t.Fatalf("the sold-out holding must stay, with 0 units: %+v", rows)
	}
	if rows[0].State != "closed" {
		t.Fatalf("state %q", rows[0].State)
	}

	hist, err := History(pool, nil, 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(hist) != 2 {
		t.Fatalf("both fills stay in the history: %+v", hist)
	}

	// Archiving is what removes it from the ledger.
	if _, err := pool.Exec("UPDATE instruments SET active = 0, archived_at = '2026-11-16T00:00:00Z' WHERE id = ?", id); err != nil {
		t.Fatal(err)
	}
	rows, err = ledger.Positions(pool, models.ModeLong)
	if err != nil {
		t.Fatal(err)
	}
	if len(rows) != 0 {
		t.Fatalf("an archived instrument leaves the ledger: %+v", rows)
	}
}

func TestEditingAFillUnarchivesItsInstrument(t *testing.T) {
	pool, id := setup(t)
	if err := Record(pool, id, SideBuy, 10, 100, nil, nil, models.ModeLong); err != nil {
		t.Fatal(err)
	}
	if _, err := pool.Exec("UPDATE instruments SET active = 0, archived_at = '2026-11-16T00:00:00Z' WHERE id = ?", id); err != nil {
		t.Fatal(err)
	}

	var tradeID string
	if err := pool.QueryRow("SELECT id FROM trades LIMIT 1").Scan(&tradeID); err != nil {
		t.Fatal(err)
	}
	newQty := 12.0
	if err := Edit(pool, tradeID, &newQty, nil, nil); err != nil {
		t.Fatal(err)
	}

	var active int
	var archived sql.NullString
	if err := pool.QueryRow("SELECT active, archived_at FROM instruments WHERE id = ?", id).Scan(&active, &archived); err != nil {
		t.Fatal(err)
	}
	if active != 1 || archived.Valid {
		t.Fatalf("editing an archived fill must restore it: active=%d archived=%v", active, archived)
	}
	rows, err := ledger.Positions(pool, models.ModeLong)
	if err != nil {
		t.Fatal(err)
	}
	if len(rows) != 1 || rows[0].Qty != 12 {
		t.Fatalf("the corrected holding is back: %+v", rows)
	}
}

func TestABuyMustFitThePurse(t *testing.T) {
	pool, id := setup(t)
	if err := Record(pool, id, SideBuy, 2000, 100, nil, nil, models.ModeLong); err == nil {
		t.Fatal("buy beyond the allowance accepted")
	}
}

func TestAMutualFundCannotEnterTheTradingPot(t *testing.T) {
	pool, _ := setup(t)
	fundID := "01a1f000-0000-7000-8000-0000000000f1"
	if _, err := pool.Exec(
		"INSERT INTO instruments (id, symbol, class, name, currency, active) VALUES (?, 'F', 'mutual_fund', 'Fund', 'INR', 1)", fundID,
	); err != nil {
		t.Fatal(err)
	}
	if err := Record(pool, fundID, SideBuy, 1, 100, nil, nil, models.ModeTrade); err == nil {
		t.Fatal("a fund entered the trading pot")
	}
}

func TestParseWhenStampsABareDateAtTheClose(t *testing.T) {
	now := time.Date(2026, 11, 16, 12, 0, 0, 0, time.UTC)
	got, err := ParseWhen("2026-11-13", now)
	if err != nil {
		t.Fatal(err)
	}
	// 15:30 IST == 10:00 UTC.
	if got.Hour() != 10 || got.Minute() != 0 {
		t.Fatalf("got %s", got)
	}
	if _, err := ParseWhen("2099-01-01", now); err == nil {
		t.Fatal("a future fill accepted")
	}
}
