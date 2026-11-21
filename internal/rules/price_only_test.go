package rules

import (
	"testing"

	"stock-predictor/internal/models"
)

// A refused daily series leaves the rules with the close and the recorded levels. That has to be
// enough to take a target or a stop, and never enough to open a position.
func TestPriceOnlySellsOnATargetAndNeverBuys(t *testing.T) {
	instr := &models.Instrument{ID: "01a1f000-0000-7000-8000-000000000041", Symbol: "T.NS", Class: models.ClassStock}
	bars := []models.Bar{{Close: 140}}

	held := Candidate(CandidateInput{
		Instrument: instr, Bars: bars, Position: &models.Position{Qty: 10, Target: fptr(140)},
		AvailableCash: 10_000, Cfg: DefaultRuleConfig(), PriceOnly: true,
	})
	if held == nil || held.Action != models.ActionSell {
		t.Fatalf("a target reached should sell, got %+v", held)
	}
	if held.Qty != 10 {
		t.Fatalf("qty %v", held.Qty)
	}

	if got := Candidate(CandidateInput{
		Instrument: instr, Bars: bars, AvailableCash: 10_000, Cfg: DefaultRuleConfig(),
		PriceOnly: true,
	}); got != nil {
		t.Fatalf("price-only must never open a position, got %+v", got)
	}
}
