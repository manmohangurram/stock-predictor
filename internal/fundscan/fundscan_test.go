package fundscan

import (
	"database/sql"
	"encoding/json"
	"path/filepath"
	"testing"
	"time"

	"stock-predictor/internal/db"
	"stock-predictor/internal/ids"
	"stock-predictor/internal/market"
	"stock-predictor/internal/schedule"
)

// A fund move is sized in rupees and has no unit count until the NAV is known, so the proposal the
// scan stores must carry the money, not a zero.
func TestAFundProposalCarriesTheMoneyItWouldSpend(t *testing.T) {
	pool, err := db.ConnectAndMigrate(t.Context(), filepath.Join(t.TempDir(), "t.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { pool.Close() })
	id := ids.New()
	if _, err := pool.Exec(
		`INSERT INTO instruments (id, symbol, class, name, currency, active)
         VALUES (?, 'AMFI:118955', 'mutual_fund', 'HDFC Flexi Cap Fund', 'INR', 1)`, id); err != nil {
		t.Fatal(err)
	}
	if err := storeSignal(pool, id, "HDFC Flexi Cap Fund", "118955", 2425.742, 63.8, 10_000,
		time.Now().UTC()); err != nil {
		t.Fatal(err)
	}
	var qty sql.NullFloat64
	var reason string
	if err := pool.QueryRow("SELECT qty, reason_json FROM signals").Scan(&qty, &reason); err != nil {
		t.Fatal(err)
	}
	if qty.Valid {
		t.Fatalf("a fund move has no unit count before the NAV: %v", qty.Float64)
	}
	stored := map[string]any{}
	if err := json.Unmarshal([]byte(reason), &stored); err != nil {
		t.Fatal(err)
	}
	if stored["amount"] != 10_000.0 {
		t.Fatalf("the proposal names no money: %s", reason)
	}
}

func TestANAVSeriesBecomesDailyBars(t *testing.T) {
	h := market.FundHistory{
		Code: "120503", Name: "Axis ELSS", FundHouse: "Axis", Category: "ELSS",
		Navs: [][2]any{{"2026-11-12", 74.05}, {"2026-11-13", 74.62}},
	}
	bars := NAVBars(h)
	if len(bars) != 2 {
		t.Fatalf("%d", len(bars))
	}
	if bars[1].Ts != "2026-11-13T00:00:00Z" || bars[1].Close != 74.62 {
		t.Fatalf("%+v", bars[1])
	}
	if bars[1].Open != bars[1].High || bars[1].Volume != 0 {
		t.Fatalf("%+v", bars[1])
	}
}

func TestTheDayIsIST(t *testing.T) {
	// 20:00 UTC is 01:30 IST the next day.
	now := time.Date(2026, 11, 9, 20, 0, 0, 0, time.UTC)
	if got := schedule.ISTDay(now); got != "2026-11-10" {
		t.Fatalf("%q", got)
	}
}

func TestSimilarFundsClash(t *testing.T) {
	held := []string{"Parag Parikh Flexi Cap Fund"}
	if !clashes("Equity Scheme - Flexi Cap Fund", held) {
		t.Fatal("a second flexi cap is the same bet")
	}
	if clashes("Equity Scheme - ELSS", held) {
		t.Fatal("an ELSS is a different bet")
	}
	if clashes("", held) {
		t.Fatal("an empty category cannot clash")
	}
}
