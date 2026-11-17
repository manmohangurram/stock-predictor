package db

import (
	"testing"
)

func TestMigrationsApplyTwiceAndWalIsOn(t *testing.T) {
	pool, err := Connect(t.TempDir() + "/test.db")
	if err != nil {
		t.Fatal(err)
	}
	defer pool.Close()
	if err := Migrate(pool); err != nil {
		t.Fatal(err)
	}
	if err := Migrate(pool); err != nil {
		t.Fatalf("second run must be a no-op: %v", err)
	}
	var count int
	if err := pool.QueryRow("SELECT COUNT(*) FROM schema_migrations").Scan(&count); err != nil {
		t.Fatal(err)
	}
	if count != len(Migrations) {
		t.Fatalf("applied %d, expected %d", count, len(Migrations))
	}
	var mode string
	if err := pool.QueryRow("PRAGMA journal_mode").Scan(&mode); err != nil {
		t.Fatal(err)
	}
	if mode != "wal" {
		t.Fatalf("journal mode is %q, want wal", mode)
	}
	var fk int
	if err := pool.QueryRow("PRAGMA foreign_keys").Scan(&fk); err != nil {
		t.Fatal(err)
	}
	if fk != 1 {
		t.Fatalf("foreign_keys is %d, want 1", fk)
	}
	var tables int
	if err := pool.QueryRow(
		"SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
	).Scan(&tables); err != nil {
		t.Fatal(err)
	}
	if tables == 0 {
		t.Fatal("no tables created")
	}
}

func TestLegacySibling(t *testing.T) {
	if got := LegacySibling("data/stock-predictor.db"); got == "" {
		t.Fatal("expected the old name")
	}
	if got := LegacySibling("data/stock-predicter.db"); got != "" {
		t.Fatalf("unexpected sibling %q", got)
	}
}
