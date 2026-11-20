package rules

import (
	"database/sql"
	"fmt"
	"math"

	"stock-predictor/internal/models"
	"stock-predictor/internal/tstamp"
)

type CashKind string

const (
	CashBudget CashKind = "budget"
	CashBuy    CashKind = "buy"
	CashSell   CashKind = "sell"
	// Unspent allowance removed at a month boundary. Not a trading loss.
	CashExpiry CashKind = "expiry"
	// The cash difference when a recorded fill is corrected. Never a trade of its own.
	CashAdjust CashKind = "adjust"
)

type NewCashEntry struct {
	Kind CashKind
	// Signed. A budget delta, a negative buy, a positive sell, and so on.
	Amount    float64
	Reference string
	// Which purse inside the pot: "mf" for mutual funds, "" for the pot's main one.
	Bucket string
}

// The purse a purchase draws on. A fund and a stock in the same pot are different money.
func BucketFor(class models.AssetClass) string {
	if class == models.ClassMutualFund {
		return "mf"
	}
	return ""
}

// Units round down to 3 decimals before costing, so rounding can only under-spend.
func UnitsFloor(qty float64) float64 {
	return math.Floor(qty*1000.0) / 1000.0
}

func Cost(qty, price float64, class models.AssetClass) float64 {
	q := qty
	if class == models.ClassMutualFund {
		q = UnitsFloor(qty)
	}
	if q <= 0.0 {
		return 0.0
	}
	return q * price
}

func BudgetTotalIn(pool *sql.DB, mode models.Mode) (float64, error) {
	var sum sql.NullFloat64
	err := pool.QueryRow(
		"SELECT SUM(amount) FROM cash_ledger WHERE kind = 'budget' AND mode = ?", string(mode),
	).Scan(&sum)
	if err != nil {
		return 0, fmt.Errorf("sum budget: %w", err)
	}
	return sum.Float64, nil
}

// Open positions at cost, plus cash locked in pending IPO applications.
func Committed(pool *sql.DB) (float64, error) {
	return CommittedIn(pool, models.ModeLong)
}

func CommittedIn(pool *sql.DB, mode models.Mode) (float64, error) {
	var pos sql.NullFloat64
	if err := pool.QueryRow(
		"SELECT SUM(qty * avg_cost) FROM positions WHERE state <> 'closed' AND mode = ?", string(mode),
	).Scan(&pos); err != nil {
		return 0, fmt.Errorf("sum positions: %w", err)
	}
	// IPO applications are a long-term instrument; trading never reserves for them.
	var ipo sql.NullFloat64
	if mode == models.ModeLong {
		if err := pool.QueryRow(
			"SELECT SUM(amount) FROM ipo_applications WHERE status = 'pending'",
		).Scan(&ipo); err != nil {
			return 0, fmt.Errorf("sum ipo reservations: %w", err)
		}
	}
	return pos.Float64 + ipo.Float64, nil
}

// The money that is actually there: every ledger row, not just the allowances. That is
// what makes a sold profit spendable and an expired allowance gone.
func CashIn(pool *sql.DB, mode models.Mode) (float64, error) {
	return CashInBucket(pool, mode, "")
}

// Cash in one purse. "" is the pot's main purse; "mf" is the mutual-fund purse.
func CashInBucket(pool *sql.DB, mode models.Mode, bucket string) (float64, error) {
	var sum sql.NullFloat64
	err := pool.QueryRow(
		"SELECT SUM(amount) FROM cash_ledger WHERE mode = ? AND COALESCE(bucket, '') = ?",
		string(mode), bucket,
	).Scan(&sum)
	if err != nil {
		return 0, fmt.Errorf("sum cash: %w", err)
	}
	return sum.Float64, nil
}

// The same sum on an existing connection, so a fill checks the purse inside its own
// transaction.
func CashInBucketOn(tx *sql.Tx, mode models.Mode, bucket string) (float64, error) {
	var sum sql.NullFloat64
	err := tx.QueryRow(
		"SELECT SUM(amount) FROM cash_ledger WHERE mode = ? AND COALESCE(bucket, '') = ?",
		string(mode), bucket,
	).Scan(&sum)
	if err != nil {
		return 0, fmt.Errorf("sum cash: %w", err)
	}
	return sum.Float64, nil
}

// Free money in the mutual-fund purse of the long-term pot.
func AvailableMF(pool *sql.DB) (float64, error) {
	return CashInBucket(pool, models.ModeLong, "mf")
}

func AvailableIn(pool *sql.DB, mode models.Mode) (float64, error) {
	var ipo sql.NullFloat64
	if mode == models.ModeLong {
		if err := pool.QueryRow(
			"SELECT SUM(amount) FROM ipo_applications WHERE status = 'pending'",
		).Scan(&ipo); err != nil {
			return 0, fmt.Errorf("sum ipo reservations: %w", err)
		}
	}
	cash, err := CashIn(pool, mode)
	if err != nil {
		return 0, err
	}
	return cash - ipo.Float64, nil
}

func Available(pool *sql.DB) (float64, error) {
	return AvailableIn(pool, models.ModeLong)
}

func Fits(pool *sql.DB, cost float64) (bool, error) {
	return FitsIn(pool, cost, models.ModeLong)
}

func FitsIn(pool *sql.DB, cost float64, mode models.Mode) (bool, error) {
	avail, err := AvailableIn(pool, mode)
	if err != nil {
		return false, err
	}
	return cost <= avail+1e-9, nil
}

func Record(pool *sql.DB, entry NewCashEntry) error {
	return RecordAtMode(pool, entry, tstamp.Now(), models.ModeLong)
}

// The same row, stamped with when it actually happened. Backfilled trades must not land
// at the top of the ledger just because they were entered today.
func RecordAt(pool *sql.DB, entry NewCashEntry, ts string) error {
	return RecordAtMode(pool, entry, ts, models.ModeLong)
}

func RecordAtMode(pool *sql.DB, entry NewCashEntry, ts string, mode models.Mode) error {
	return insertEntry(pool, entry, ts, mode)
}

// The same insert on a transaction, so a fill can post its money in the same transaction
// as the position and the trade row. Half a fill is a corrupted book.
func RecordOn(tx *sql.Tx, entry NewCashEntry, ts string, mode models.Mode) error {
	return insertEntry(tx, entry, ts, mode)
}

type execer interface {
	Exec(query string, args ...any) (sql.Result, error)
}

func insertEntry(e execer, entry NewCashEntry, ts string, mode models.Mode) error {
	_, err := e.Exec(
		"INSERT INTO cash_ledger (ts, kind, amount, ref, mode, bucket) VALUES (?, ?, ?, ?, ?, ?)",
		ts, string(entry.Kind), entry.Amount, entry.Reference, string(mode), entry.Bucket,
	)
	if err != nil {
		return fmt.Errorf("insert cash ledger: %w", err)
	}
	return nil
}

func ReserveIPO(pool *sql.DB, instrumentID string, qty, price float64) error {
	amount := Cost(qty, price, models.ClassIpo)
	_, err := pool.Exec(
		"INSERT INTO ipo_applications (instrument_id, qty, amount, status, applied_at) VALUES (?, ?, ?, 'pending', ?)",
		instrumentID, qty, amount, tstamp.Now(),
	)
	if err != nil {
		return fmt.Errorf("insert ipo application: %w", err)
	}
	return nil
}
