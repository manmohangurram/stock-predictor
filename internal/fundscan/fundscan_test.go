package fundscan

import (
	"testing"
	"time"

	"stock-predictor/internal/market"
	"stock-predictor/internal/schedule"
)

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
