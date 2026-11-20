package db

import (
	"database/sql"
	"testing"
)

func applyPrefix(t *testing.T, pool *sql.DB, n int) {
	t.Helper()
	for _, m := range Migrations[:n] {
		if _, err := pool.Exec(m.SQL); err != nil {
			t.Fatalf("apply %s: %v", m.ID, err)
		}
	}
}

func TestTheSameSymbolInTwoCasesBecomesOneInstrument(t *testing.T) {
	pool, err := Connect(t.TempDir() + "/test.db")
	if err != nil {
		t.Fatal(err)
	}
	defer pool.Close()

	before := -1
	for i, m := range Migrations {
		if m.ID == "0023_one_instrument_per_symbol" {
			before = i
		}
	}
	if before < 0 {
		t.Fatal("the symbol migration is missing")
	}
	applyPrefix(t, pool, before)
	last := Migrations[before]

	var keep, drop int64
	if err := pool.QueryRow(
		`INSERT INTO instruments (symbol, class, name, currency, active) VALUES ('INFY.NS','stock','Infosys','INR',1) RETURNING id`,
	).Scan(&keep); err != nil {
		t.Fatal(err)
	}
	if err := pool.QueryRow(
		`INSERT INTO instruments (symbol, class, name, currency, active) VALUES (' infy.ns ','stock','Infosys','INR',0) RETURNING id`,
	).Scan(&drop); err != nil {
		t.Fatal(err)
	}
	if _, err := pool.Exec(
		"INSERT INTO bars (instrument_id, ts, close) VALUES (?, '2026-11-19T00:00:00+00:00', 1140), (?, '2026-11-20T00:00:00+00:00', 1148)",
		keep, drop); err != nil {
		t.Fatal(err)
	}
	if _, err := pool.Exec(
		`INSERT INTO signals (instrument_id, action, created_at) VALUES (?, 'BUY', '2026-11-20T00:00:00+00:00')`, drop,
	); err != nil {
		t.Fatal(err)
	}

	if _, err := pool.Exec(last.SQL); err != nil {
		t.Fatalf("apply %s: %v", last.ID, err)
	}

	var instruments, bars, signals int
	if err := pool.QueryRow("SELECT COUNT(*) FROM instruments WHERE upper(symbol) = 'INFY.NS'").Scan(&instruments); err != nil {
		t.Fatal(err)
	}
	if instruments != 1 {
		t.Fatalf("%d instruments left, want 1", instruments)
	}
	if err := pool.QueryRow("SELECT COUNT(*) FROM bars WHERE instrument_id = ?", keep).Scan(&bars); err != nil {
		t.Fatal(err)
	}
	if bars != 2 {
		t.Fatalf("%d bars on the surviving instrument, want both fills", bars)
	}
	if err := pool.QueryRow("SELECT COUNT(*) FROM signals WHERE instrument_id = ?", keep).Scan(&signals); err != nil {
		t.Fatal(err)
	}
	if signals != 1 {
		t.Fatalf("the decision did not move to the surviving instrument")
	}

	var active int
	if err := pool.QueryRow("SELECT active FROM instruments WHERE id = ?", keep).Scan(&active); err != nil {
		t.Fatal(err)
	}
	if active != 1 {
		t.Fatal("the surviving row lost its active flag")
	}
}

func TestCaseVariantsCannotBeInsertedTwice(t *testing.T) {
	pool, err := ConnectAndMigrate(t.Context(), t.TempDir()+"/test.db")
	if err != nil {
		t.Fatal(err)
	}
	defer pool.Close()

	if _, err := pool.Exec(
		"INSERT INTO instruments (symbol, class, name, currency, active) VALUES ('TCS.NS','stock','TCS','INR',1)",
	); err != nil {
		t.Fatal(err)
	}
	if _, err := pool.Exec(
		"INSERT INTO instruments (symbol, class, name, currency, active) VALUES ('tcs.ns','stock','TCS','INR',1)",
	); err == nil {
		t.Fatal("a lower-case twin was accepted")
	}
}
