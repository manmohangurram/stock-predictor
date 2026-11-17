package rules

import (
	"database/sql"
	"math"
	"path/filepath"
	"testing"
	"time"

	"stock-predictor/internal/db"
	"stock-predictor/internal/models"
)

func TestAnEmptyTableIsTheDefaultRuntime(t *testing.T) {
	if r := RuntimeFromMap(map[string]string{}); r != DefaultRuntime() {
		t.Fatalf("got %+v", r)
	}
}

func TestValuesAreClampedIntoTheirRange(t *testing.T) {
	r := RuntimeFromMap(map[string]string{
		"poll_secs":           "5",
		"prompt_window_hours": "99",
		"daily_cap":           "-3",
	})
	if r.PollSecs != 60 {
		t.Fatalf("poll_secs %d", r.PollSecs)
	}
	if r.PromptWindowHours != 24 {
		t.Fatalf("prompt_window_hours %d", r.PromptWindowHours)
	}
	if r.DailyCap != 0 {
		t.Fatalf("daily_cap %d", r.DailyCap)
	}
}

func TestACorruptRowFallsBackToTheDefault(t *testing.T) {
	r := RuntimeFromMap(map[string]string{"poll_secs": "often", "unknown": "x"})
	if r.PollSecs != 900 {
		t.Fatalf("poll_secs %d", r.PollSecs)
	}
	r = RuntimeFromMap(map[string]string{"min_price_move": "NaN"})
	if r.MinPriceMove != 2.0 {
		t.Fatalf("min_price_move %v", r.MinPriceMove)
	}
}

func TestBlankTextMeansNoProxy(t *testing.T) {
	r := RuntimeFromMap(map[string]string{"proxy": "  "})
	if r.Proxy != nil {
		t.Fatalf("proxy %v", *r.Proxy)
	}
}

func pool(t *testing.T) *sql.DB {
	t.Helper()
	pool, err := db.ConnectAndMigrate(t.Context(), filepath.Join(t.TempDir(), "t.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { pool.Close() })
	return pool
}

func at(y int, m time.Month, d int) time.Time {
	return time.Date(y, m, d, 6, 0, 0, 0, time.UTC)
}

func TestANewMonthGrantsTheAllowance(t *testing.T) {
	pool := pool(t)
	r, err := EnsureMonth(pool, models.ModeLong, "", 50_000.0, at(2026, 11, 2))
	if err != nil {
		t.Fatal(err)
	}
	if r.Granted != 50_000.0 || r.Expired != 0.0 {
		t.Fatalf("%+v", r)
	}
	cash, err := CashIn(pool, models.ModeLong)
	if err != nil {
		t.Fatal(err)
	}
	if cash != 50_000.0 {
		t.Fatalf("cash %v", cash)
	}
}

func TestTheSameMonthIsNeverGrantedTwice(t *testing.T) {
	pool := pool(t)
	if _, err := EnsureMonth(pool, models.ModeLong, "", 50_000.0, at(2026, 11, 2)); err != nil {
		t.Fatal(err)
	}
	second, err := EnsureMonth(pool, models.ModeLong, "", 50_000.0, at(2026, 11, 20))
	if err != nil {
		t.Fatal(err)
	}
	if second.Granted != 0.0 {
		t.Fatalf("granted %v", second.Granted)
	}
	cash, _ := CashIn(pool, models.ModeLong)
	if cash != 50_000.0 {
		t.Fatalf("cash %v", cash)
	}
}

func TestUnspentCashExpiresAndIsNotATradingLoss(t *testing.T) {
	pool := pool(t)
	if _, err := EnsureMonth(pool, models.ModeLong, "", 50_000.0, at(2026, 11, 2)); err != nil {
		t.Fatal(err)
	}
	december, err := EnsureMonth(pool, models.ModeLong, "", 50_000.0, at(2026, 12, 2))
	if err != nil {
		t.Fatal(err)
	}
	if december.Expired != 50_000.0 {
		t.Fatalf("expired %v, want November's remainder gone", december.Expired)
	}
	cash, _ := CashIn(pool, models.ModeLong)
	if cash != 50_000.0 {
		t.Fatalf("cash %v", cash)
	}
	var realised sql.NullFloat64
	if err := pool.QueryRow("SELECT SUM(realised) FROM trades WHERE mode = 'long'").Scan(&realised); err != nil {
		t.Fatal(err)
	}
	if realised.Valid && realised.Float64 != 0 {
		t.Fatalf("an expiry must not touch realised P&L, got %v", realised.Float64)
	}
}

func TestAMonthWithNoCashWritesNoExpiry(t *testing.T) {
	pool := pool(t)
	r, err := EnsureMonth(pool, models.ModeTrade, "", 0.0, at(2026, 11, 2))
	if err != nil {
		t.Fatal(err)
	}
	if r.Expired != 0.0 {
		t.Fatalf("expired %v", r.Expired)
	}
	rows, err := pool.Query("SELECT kind FROM cash_ledger")
	if err != nil {
		t.Fatal(err)
	}
	defer rows.Close()
	for rows.Next() {
		var kind string
		if err := rows.Scan(&kind); err != nil {
			t.Fatal(err)
		}
		if kind == "expiry" {
			t.Fatal("no expiry row may exist")
		}
	}
}

func TestUnitsRoundDownToThreeDecimals(t *testing.T) {
	if got := UnitsFloor(12.3456789); math.Abs(got-12.345) > 1e-12 {
		t.Fatalf("units %v", got)
	}
}

func TestISTMonthUsesTheMarketMonth(t *testing.T) {
	// 2026-11-30 19:00 UTC is 2026-12-01 00:30 IST.
	if got := ISTMonth(time.Date(2026, 11, 30, 19, 0, 0, 0, time.UTC)); got != "2026-12" {
		t.Fatalf("month %q", got)
	}
	if got := ISTMonth(time.Date(2026, 11, 16, 9, 0, 0, 0, time.UTC)); got != "2026-11" {
		t.Fatalf("month %q", got)
	}
}
