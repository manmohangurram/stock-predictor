package rules

import (
	"database/sql"
	"fmt"
	"math"
	"strconv"
	"strings"
	"time"

	"stock-predictor/internal/models"
	"stock-predictor/internal/tstamp"
)

// The month boundary of a pot.
//
// Money arrives once a month and does not roll over: on the first tick of a new month the
// previous balance expires as its own ledger kind, then the allowance is granted. Both are
// inserts — the ledger stays append-only — and the expiry is never counted as a trading
// result.

type MonthReport struct {
	Month   string
	Expired float64
	Granted float64
}

// YYYY-MM in IST — the month the market is in, not the server's month.
func ISTMonth(now time.Time) string {
	ist := now.Add(330 * time.Minute) // 15:30 IST == 10:00 UTC; NSE has no DST
	y, m, _ := ist.Date()
	return fmt.Sprintf("%04d-%02d", y, int(m))
}

func monthKey(mode models.Mode, bucket string) string {
	if bucket != "" {
		return fmt.Sprintf("pot_month_%s_%s", mode, bucket)
	}
	return fmt.Sprintf("pot_month_%s", mode)
}

func grantedKey(mode models.Mode, bucket string) string {
	if bucket != "" {
		return fmt.Sprintf("pot_granted_%s_%s", mode, bucket)
	}
	return fmt.Sprintf("pot_granted_%s", mode)
}

// Idempotent for a given allowance: it grants on the first call of a month, and again only
// if the configured allowance changed — raising it mid-month tops the pot up, lowering it
// takes the difference back as an expiry. Never twice for the same numbers.
func EnsureMonth(pool *sql.DB, mode models.Mode, bucket string, allowance float64, now time.Time) (MonthReport, error) {
	month := ISTMonth(now)
	gKey := grantedKey(mode, bucket)

	var seen string
	err := pool.QueryRow("SELECT value FROM settings WHERE key = ?", monthKey(mode, bucket)).Scan(&seen)
	found := err == nil
	if err != nil && err != sql.ErrNoRows {
		return MonthReport{}, err
	}
	var granted float64
	var gv string
	if err := pool.QueryRow("SELECT value FROM settings WHERE key = ?", gKey).Scan(&gv); err == nil {
		if f, perr := strconv.ParseFloat(strings.TrimSpace(gv), 64); perr == nil {
			granted = f
		}
	}

	sameMonth := found && seen == month
	if sameMonth && math.Abs(granted-allowance) < 1e-9 {
		return MonthReport{Month: month, Expired: 0, Granted: 0}, nil
	}

	existing, err := CashInBucket(pool, mode, bucket)
	if err != nil {
		return MonthReport{}, err
	}
	expired := 0.0
	if !sameMonth && existing > 0.0 {
		expired = existing
	}
	delta := allowance
	if sameMonth {
		delta = allowance - granted
	}

	tx, err := pool.Begin()
	if err != nil {
		return MonthReport{}, fmt.Errorf("begin the month reset: %w", err)
	}
	defer tx.Rollback()
	stamp := tstamp.From(now)
	if expired > 0.0 {
		if _, err := tx.Exec(
			"INSERT INTO cash_ledger (ts, kind, amount, ref, mode, bucket) VALUES (?, ?, ?, ?, ?, ?)",
			stamp, string(CashExpiry), -expired, fmt.Sprintf("unspent allowance expired %s", month),
			string(mode), bucket,
		); err != nil {
			return MonthReport{}, err
		}
	}
	if math.Abs(delta) > 1e-9 {
		ref := fmt.Sprintf("allowance %s", month)
		if sameMonth {
			ref = fmt.Sprintf("allowance changed to %v", allowance)
		}
		if _, err := tx.Exec(
			"INSERT INTO cash_ledger (ts, kind, amount, ref, mode, bucket) VALUES (?, ?, ?, ?, ?, ?)",
			stamp, string(CashBudget), delta, ref, string(mode), bucket,
		); err != nil {
			return MonthReport{}, err
		}
	}
	for _, kv := range [][2]string{
		{monthKey(mode, bucket), month},
		{gKey, TrimNumber(allowance)},
	} {
		if _, err := tx.Exec(
			"INSERT INTO settings (key, value, updated_at) VALUES (?, ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
			kv[0], kv[1], stamp,
		); err != nil {
			return MonthReport{}, err
		}
	}
	if err := tx.Commit(); err != nil {
		return MonthReport{}, err
	}

	return MonthReport{Month: month, Expired: expired, Granted: delta}, nil
}
