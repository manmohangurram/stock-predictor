package rules

import (
	"math"
	"testing"
	"time"

	"stock-predictor/internal/models"
)

func TestMomentumAndPositionInTheYearAreComputed(t *testing.T) {
	bars := series(30, 1.0)
	today := time.Date(2026, 11, 9, 6, 0, 0, 0, time.UTC)
	c := ComputeShortContext(bars, nil, today)
	if c.Ret5D == nil || *c.Ret5D <= 4.0 {
		t.Fatalf("ret_5d %v", c.Ret5D)
	}
	if c.From52wHighPct == nil || *c.From52wHighPct >= 1.0 {
		t.Fatalf("from_52w_high_pct %v", c.From52wHighPct)
	}
	if c.AtrPct == nil || *c.AtrPct <= 0.0 {
		t.Fatalf("atr_pct %v", c.AtrPct)
	}
}

func TestAFlatSeriesHasNoRelVolumeWithoutTape(t *testing.T) {
	bars := series(30, 0.0)
	today := time.Date(2026, 11, 9, 6, 0, 0, 0, time.UTC)
	c := ComputeShortContext(bars, nil, today)
	if c.Rvol != nil {
		t.Fatalf("no intraday volume means no RVOL, not a guess: %v", *c.Rvol)
	}
}

func TestTheSessionClockIsIST(t *testing.T) {
	// 10:00 UTC is 15:30 IST — the close.
	now := time.Date(2026, 11, 9, 10, 0, 0, 0, time.UTC)
	fromOpen, toClose := session(now)
	if fromOpen != 6*60+15 {
		t.Fatalf("from_open %d", fromOpen)
	}
	if toClose != 0 {
		t.Fatalf("to_close %d", toClose)
	}
}

func TestTheCardOmitsWhatItCannotCompute(t *testing.T) {
	var c ShortContext
	text := c.Render()
	if !contains(text, "RVOL —") {
		t.Fatalf("missing RVOL placeholder: %q", text)
	}
	if contains(text, "NaN") {
		t.Fatalf("NaN in the card: %q", text)
	}
}

func contains(s, sub string) bool {
	return len(s) >= len(sub) && (func() bool {
		for i := 0; i+len(sub) <= len(s); i++ {
			if s[i:i+len(sub)] == sub {
				return true
			}
		}
		return false
	})()
}

func bar(close, high, low float64, volume int64) models.Bar {
	return models.Bar{
		Ts: "2026-11-06T00:00:00Z", Open: close, High: high, Low: low, Close: close, Volume: volume,
	}
}

func series(n int, step float64) []models.Bar {
	out := make([]models.Bar, n)
	for i := range out {
		c := 100.0 + float64(i)*step
		out[i] = bar(c, c+1.0, c-1.0, 1000)
	}
	return out
}

func TestComputeIndicatorsNeeds200Bars(t *testing.T) {
	if _, err := ComputeIndicators(series(199, 1.0)); err == nil {
		t.Fatal("199 bars accepted")
	}
	ind, err := ComputeIndicators(series(250, 1.0))
	if err != nil {
		t.Fatal(err)
	}
	if ind.SMA50 != 324.5 { // mean of closes 300..349 in series(250, 1.0)
		t.Fatalf("sma50 %v", ind.SMA50)
	}
	if ind.SMA200 != 249.5 {
		t.Fatalf("sma200 %v", ind.SMA200)
	}
}

func TestComputeIndicatorsRejectsNonFinite(t *testing.T) {
	bars := series(250, 1.0)
	bars[249].High = math.NaN()
	if _, err := ComputeIndicators(bars); err == nil {
		t.Fatal("NaN input accepted")
	}
}

func TestATRBandsUseTheSharedMultiplier(t *testing.T) {
	b := ATRBands(100.0, 2.5)
	if b.Min != 97.5 || b.Max != 102.5 {
		t.Fatalf("bands %+v", b)
	}
}

func TestTheBandSpeaksOnlyWhenAnEdgeIsCrossed(t *testing.T) {
	s := 80.0
	// Paid 100, ceiling 20%: 130 is past it, 110 is not.
	if BandReason(&s, 100.0, 130.0, 20.0) != "ceiling" {
		t.Fatal("ceiling missed")
	}
	if BandReason(&s, 100.0, 110.0, 20.0) != "" {
		t.Fatal("spoke inside the band")
	}
	// Through the stop is the other edge.
	if BandReason(&s, 100.0, 75.0, 20.0) != "broken" {
		t.Fatal("break missed")
	}
	// No stop, no ceiling reached: nothing to say.
	if BandReason(nil, 100.0, 90.0, 20.0) != "" {
		t.Fatal("spoke without cause")
	}
}

func TestAWinnerNeverTrailsBelowItsOwnCost(t *testing.T) {
	held := 90.0
	// Up 30%: two ATRs back is above cost, so the stop follows the price.
	if got := TrailedStop(&held, 100.0, 130.0, 5.0); got == nil || *got != 120.0 {
		t.Fatalf("got %v", got)
	}
	// Up only 3%: two ATRs back is below cost, so the stop goes to break-even.
	if got := TrailedStop(&held, 100.0, 103.0, 5.0); got == nil || *got != 100.0 {
		t.Fatalf("got %v", got)
	}
}

func TestALoserKeepsTheStopTheOwnerSet(t *testing.T) {
	held := 90.0
	if got := TrailedStop(&held, 100.0, 96.0, 5.0); got == nil || *got != 90.0 {
		t.Fatalf("got %v", got)
	}
	if got := TrailedStop(nil, 100.0, 96.0, 5.0); got != nil {
		t.Fatalf("got %v", got)
	}
}

func TestAStopIsNeverPulledBackDown(t *testing.T) {
	// The owner placed it at 125; the trail would compute 118.
	held := 125.0
	if got := TrailedStop(&held, 100.0, 130.0, 6.0); got == nil || *got != 125.0 {
		t.Fatalf("got %v", got)
	}
}

func TestNonsenseInputsChangeNothing(t *testing.T) {
	held := 90.0
	if got := TrailedStop(&held, 100.0, 130.0, 0.0); got == nil || *got != 90.0 {
		t.Fatalf("got %v", got)
	}
	if got := TrailedStop(&held, 0.0, 130.0, 5.0); got == nil || *got != 90.0 {
		t.Fatalf("got %v", got)
	}
}

// Alternating +2/-1 closes: a real-looking uptrend whose RSI sits under the buy ceiling.
func seriesAlt(n int) []models.Bar {
	out := make([]models.Bar, n)
	c := 100.0
	for i := range out {
		if i > 0 {
			if i%2 == 1 {
				c += 2.0
			} else {
				c -= 1.0
			}
		}
		out[i] = bar(c, c+1.0, c-1.0, 1000)
	}
	return out
}

func TestCandidateSellOnStop(t *testing.T) {
	bars := seriesAlt(250)
	ind, err := ComputeIndicators(bars)
	if err != nil {
		t.Fatal(err)
	}
	close := bars[len(bars)-1].Close
	pos := &models.Position{Qty: 10, Stop: fptr(close + 1.0)}
	instr := &models.Instrument{ID: 1, Symbol: "T.NS", Class: models.ClassStock}
	c := Candidate(CandidateInput{
		Instrument: instr, Bars: bars, Ind: &ind, Position: pos, AvailableCash: 0, Cfg: DefaultRuleConfig(),
	})
	if c == nil || c.Action != models.ActionSell || !c.Urgent {
		t.Fatalf("expected an urgent sell, got %+v", c)
	}
}

func TestCandidateBuyInUptrend(t *testing.T) {
	bars := seriesAlt(250)
	ind, err := ComputeIndicators(bars)
	if err != nil {
		t.Fatal(err)
	}
	instr := &models.Instrument{ID: 1, Symbol: "T.NS", Class: models.ClassStock}
	c := Candidate(CandidateInput{
		Instrument: instr, Bars: bars, Ind: &ind, AvailableCash: 10_000, Cfg: DefaultRuleConfig(),
	})
	if c == nil || c.Action != models.ActionBuy {
		t.Fatalf("expected a buy, got %+v", c)
	}
	if c.Stop == nil || c.Target == nil {
		t.Fatal("a stock buy carries both stop and target")
	}
}

func TestCandidateMFBuyCarriesNoStop(t *testing.T) {
	bars := seriesAlt(250)
	ind, err := ComputeIndicators(bars)
	if err != nil {
		t.Fatal(err)
	}
	instr := &models.Instrument{ID: 1, Symbol: "F", Class: models.ClassMutualFund}
	c := Candidate(CandidateInput{
		Instrument: instr, Bars: bars, Ind: &ind, AvailableCash: 10_000, Cfg: DefaultRuleConfig(),
	})
	if c == nil || c.Action != models.ActionBuy {
		t.Fatalf("expected a buy, got %+v", c)
	}
	if c.Stop != nil || c.Target != nil {
		t.Fatal("a fund buy has no price to stop at")
	}
}
