package db

import (
	"database/sql"
	"fmt"

	"stock-predictor/internal/ids"
)

// The tables whose rows the app hands out: a page URL, a decision button, a call overlay.
var uuidTables = []string{
	"instruments", "signals", "cash_ledger", "ipo_applications", "notifications",
	"search_results",
}

// BackfillUUIDs gives every row that has none an id, oldest row first, so ids sort in the order
// the rows were created. Idempotent: a second call finds nothing to do.
func BackfillUUIDs(pool *sql.DB) (int, error) {
	total := 0
	for _, table := range uuidTables {
		missing, err := rowsWithoutUUID(pool, table)
		if err != nil {
			return total, err
		}
		if len(missing) == 0 {
			continue
		}
		tx, err := pool.Begin()
		if err != nil {
			return total, err
		}
		for _, id := range missing {
			if _, err := tx.Exec("UPDATE "+table+" SET uuid = ? WHERE id = ?", ids.New(), id); err != nil {
				_ = tx.Rollback()
				return total, fmt.Errorf("backfill %s id %d: %w", table, id, err)
			}
		}
		if err := tx.Commit(); err != nil {
			return total, err
		}
		total += len(missing)
	}
	return total, nil
}

func rowsWithoutUUID(pool *sql.DB, table string) ([]int64, error) {
	rows, err := pool.Query("SELECT id FROM " + table + " WHERE uuid IS NULL ORDER BY id")
	if err != nil {
		return nil, fmt.Errorf("find rows without a uuid in %s: %w", table, err)
	}
	defer rows.Close()
	missing := []int64{}
	for rows.Next() {
		var id int64
		if err := rows.Scan(&id); err != nil {
			return nil, err
		}
		missing = append(missing, id)
	}
	return missing, rows.Err()
}
