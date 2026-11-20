package db

import (
	"testing"

	"stock-predictor/internal/ids"
)

func TestEveryRowGetsOneUniqueUUIDInCreationOrder(t *testing.T) {
	pool, err := ConnectAndMigrate(t.Context(), t.TempDir()+"/test.db")
	if err != nil {
		t.Fatal(err)
	}
	defer pool.Close()

	for _, s := range []string{"ONE.NS", "TWO.NS", "THREE.NS"} {
		if _, err := pool.Exec(
			"INSERT INTO instruments (symbol, class, name, currency, active) VALUES (?, 'stock', ?, 'INR', 1)",
			s, s); err != nil {
			t.Fatal(err)
		}
	}

	if n, err := BackfillUUIDs(pool); err != nil {
		t.Fatal(err)
	} else if n != 3 {
		t.Fatalf("backfilled %d rows, want 3", n)
	}

	rows, err := pool.Query("SELECT id, uuid FROM instruments ORDER BY id")
	if err != nil {
		t.Fatal(err)
	}
	defer rows.Close()
	seen := map[string]bool{}
	last := ""
	for rows.Next() {
		var id int64
		var uuid string
		if err := rows.Scan(&id, &uuid); err != nil {
			t.Fatal(err)
		}
		if !ids.Valid(uuid) {
			t.Fatalf("%s got %q", "instrument", uuid)
		}
		if seen[uuid] {
			t.Fatalf("duplicate uuid %s", uuid)
		}
		if last != "" && uuid <= last {
			t.Fatalf("%s does not sort after %s: ids are not in creation order", uuid, last)
		}
		seen[uuid] = true
		last = uuid
	}

	if n, err := BackfillUUIDs(pool); err != nil {
		t.Fatal(err)
	} else if n != 0 {
		t.Fatalf("a second run changed %d rows", n)
	}
}

func TestATableWithNoRowsIsNotAnError(t *testing.T) {
	pool, err := ConnectAndMigrate(t.Context(), t.TempDir()+"/test.db")
	if err != nil {
		t.Fatal(err)
	}
	defer pool.Close()
	if n, err := BackfillUUIDs(pool); err != nil || n != 0 {
		t.Fatalf("n=%d err=%v", n, err)
	}
}
