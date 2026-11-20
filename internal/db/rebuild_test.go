package db

import (
	"database/sql"
	"testing"
)

func migrationIndex(t *testing.T, id string) int {
	t.Helper()
	for i, m := range Migrations {
		if m.ID == id {
			return i
		}
	}
	t.Fatalf("migration %s is missing", id)
	return -1
}

func mustExec(t *testing.T, pool *sql.DB, query string, args ...any) {
	t.Helper()
	if _, err := pool.Exec(query, args...); err != nil {
		t.Fatalf("%s: %v", query, err)
	}
}

// A notification points at a signal, and SQLite refuses to drop a table something points at. This
// is the state a real database arrives in, and the rewrite must survive it.
func TestTheSignalsRewriteSurvivesANotification(t *testing.T) {
	pool, err := Connect(t.TempDir() + "/t.db")
	if err != nil {
		t.Fatal(err)
	}
	defer pool.Close()

	idx := migrationIndex(t, "0027_signals_uuid_keys")
	applyPrefix(t, pool, idx)

	mustExec(t, pool, `INSERT INTO instruments (symbol, class, name, currency, active, uuid)
        VALUES ('X.NS', 'stock', 'X', 'INR', 1, '01a1f000-0000-7000-8000-0000000000a1')`)
	signalUUID := "01a1f000-0000-7000-8000-0000000000a2"
	notificationUUID := "01a1f000-0000-7000-8000-0000000000a3"
	mustExec(t, pool, `INSERT INTO signals (instrument_id, action, created_at, uuid)
        VALUES (1, 'BUY', '2026-11-20T00:00:00Z', ?)`, signalUUID)
	mustExec(t, pool, `INSERT INTO notifications (signal_id, sent_at, channel, dedupe_key, uuid)
        VALUES (1, '2026-11-20T00:00:00Z', 'ntfy', 'k1', ?)`, notificationUUID)

	if _, err := pool.Exec(Migrations[idx].SQL); err != nil {
		t.Fatalf("the rewrite must run with a notification present: %v", err)
	}

	var idType, signalID string
	if err := pool.QueryRow("SELECT typeof(id), id FROM signals").Scan(&idType, &signalID); err != nil {
		t.Fatal(err)
	}
	if idType != "text" || signalID != signalUUID {
		t.Fatalf("signals.id is %s %q, want text %q", idType, signalID, signalUUID)
	}
	var noteSignal string
	if err := pool.QueryRow("SELECT signal_id FROM notifications WHERE dedupe_key = 'k1'").Scan(&noteSignal); err != nil {
		t.Fatal(err)
	}
	if noteSignal != signalUUID {
		t.Fatalf("the notification points at %q, want %q", noteSignal, signalUUID)
	}
}

// A database left half-rewritten by the first version of this migration has a stray signals_new
// and no record of having applied it. The rewrite has to clean that up rather than refuse to run.
func TestTheSignalsRewriteClearsAHalfBuiltTable(t *testing.T) {
	pool, err := Connect(t.TempDir() + "/t.db")
	if err != nil {
		t.Fatal(err)
	}
	defer pool.Close()

	idx := migrationIndex(t, "0027_signals_uuid_keys")
	applyPrefix(t, pool, idx)
	mustExec(t, pool, `CREATE TABLE signals_new (id TEXT PRIMARY KEY)`)
	mustExec(t, pool, `INSERT INTO instruments (symbol, class, name, currency, active, uuid)
        VALUES ('X.NS', 'stock', 'X', 'INR', 1, '01a1f000-0000-7000-8000-0000000000b1')`)
	mustExec(t, pool, `INSERT INTO signals (instrument_id, action, created_at, uuid)
        VALUES (1, 'BUY', '2026-11-20T00:00:00Z', '01a1f000-0000-7000-8000-0000000000b2')`)

	if _, err := pool.Exec(Migrations[idx].SQL); err != nil {
		t.Fatalf("a half-built table must not block the rewrite: %v", err)
	}
	var n int
	if err := pool.QueryRow("SELECT COUNT(*) FROM signals WHERE action = 'BUY'").Scan(&n); err != nil {
		t.Fatal(err)
	}
	if n != 1 {
		t.Fatalf("%d signals survived, want 1", n)
	}
}

// The hub migration rewrites instruments and every child in one pass, with a notification still
// pointing at signals. It runs straight after the signals rewrite, so the pair is tested together.
func TestTheInstrumentRewriteRunsAfterTheSignalsRewrite(t *testing.T) {
	pool, err := Connect(t.TempDir() + "/t.db")
	if err != nil {
		t.Fatal(err)
	}
	defer pool.Close()

	signals := migrationIndex(t, "0027_signals_uuid_keys")
	hub := migrationIndex(t, "0028_instruments_uuid_key")
	applyPrefix(t, pool, signals)
	mustExec(t, pool, `INSERT INTO instruments (symbol, class, name, currency, active, uuid)
        VALUES ('X.NS', 'stock', 'X', 'INR', 1, '01a1f000-0000-7000-8000-0000000000c1')`)
	mustExec(t, pool, `INSERT INTO signals (instrument_id, action, created_at, uuid)
        VALUES (1, 'BUY', '2026-11-20T00:00:00Z', '01a1f000-0000-7000-8000-0000000000c2')`)
	mustExec(t, pool, `INSERT INTO notifications (signal_id, sent_at, channel, dedupe_key, uuid)
        VALUES (1, '2026-11-20T00:00:00Z', 'ntfy', 'k2', '01a1f000-0000-7000-8000-0000000000c3')`)

	if _, err := pool.Exec(Migrations[signals].SQL); err != nil {
		t.Fatalf("0027: %v", err)
	}
	if _, err := pool.Exec(Migrations[hub].SQL); err != nil {
		t.Fatalf("0028: %v", err)
	}

	var instrumentType, signalInstrument, noteSignal string
	if err := pool.QueryRow("SELECT typeof(id) FROM instruments").Scan(&instrumentType); err != nil {
		t.Fatal(err)
	}
	if err := pool.QueryRow("SELECT instrument_id FROM signals").Scan(&signalInstrument); err != nil {
		t.Fatal(err)
	}
	if err := pool.QueryRow("SELECT signal_id FROM notifications").Scan(&noteSignal); err != nil {
		t.Fatal(err)
	}
	if instrumentType != "text" || signalInstrument != "01a1f000-0000-7000-8000-0000000000c1" {
		t.Fatalf("instrument id %s, signal points at %q", instrumentType, signalInstrument)
	}
	if noteSignal != "01a1f000-0000-7000-8000-0000000000c2" {
		t.Fatalf("the notification lost its decision: %q", noteSignal)
	}
}
