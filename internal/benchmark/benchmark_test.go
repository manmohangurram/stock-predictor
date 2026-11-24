package benchmark

import (
	"testing"

	"stock-predictor/internal/market"
)

func TestTheCSVGivesBareTickersAndNames(t *testing.T) {
	csv := "Company Name,Industry,Symbol,Series,ISIN Code\n" +
		"ABB India Ltd.,Capital Goods,ABB,EQ,INE117A01022\n" +
		"Adani Energy Solutions Ltd.,Power,ADANIENSOL,EQ,INE931S01010\n"
	rows := ParseConstituents(csv)
	if len(rows) != 2 {
		t.Fatalf("%+v", rows)
	}
	if rows[0] != (market.Constituent{Symbol: "ABB.NS", Name: "ABB India Ltd."}) {
		t.Fatalf("%+v", rows[0])
	}
}

func TestAHeaderRowIsNotAConstituent(t *testing.T) {
	if got := ParseConstituents("Company Name,Industry,Symbol,Series\n"); len(got) != 0 {
		t.Fatalf("%+v", got)
	}
}

func TestTheReturnIsMeasuredOverFiveSessions(t *testing.T) {
	closes := []float64{100, 101, 102, 103, 104, 110}
	r, ok := ReturnOver(closes, 5)
	if !ok || r-0.10 > 1e-9 || r-0.10 < -1e-9 {
		t.Fatalf("%v %v", r, ok)
	}
	if _, ok := ReturnOver(closes[:5], 5); ok {
		t.Fatal("five closes is not enough")
	}
	if _, ok := ReturnOver([]float64{100, 0, 1, 2, 3, 4}, 5); ok {
		t.Fatal("a zero close is not a price")
	}
}

func TestOnlyPositiveReturnsAreOfferedBestFirst(t *testing.T) {
	pick := func(s string, r float64) Pick {
		return Pick{Symbol: s + ".NS", Name: s, Close: 100, Ret5: r}
	}
	out := Rank([]Pick{pick("A", 0.02), pick("B", -0.03), pick("C", 0.08)}, 10)
	if len(out) != 2 || out[0].Symbol != "C.NS" || out[1].Symbol != "A.NS" {
		t.Fatalf("%+v", out)
	}
	if len(Rank([]Pick{pick("A", -0.01), pick("B", -0.02)}, 10)) != 0 {
		t.Fatal("a market that fell must offer nothing")
	}
}

func TestTheCapIsTen(t *testing.T) {
	many := []Pick{}
	for i := 0; i < 25; i++ {
		many = append(many, Pick{Symbol: "S.NS", Name: "S", Close: 100, Ret5: 0.001 * float64(i)})
	}
	if got := len(Rank(many, Top)); got != Top {
		t.Fatalf("%d", got)
	}
}

func TestTheLineTheModelReadsNamesTheWindow(t *testing.T) {
	line := Pick{Symbol: "ABB.NS", Name: "ABB India", Close: 7400.5, Ret5: 0.0612}.Line("INR")
	if !contains(line, "7400.50 INR") || !contains(line, "+6.12% over 5 sessions") {
		t.Fatalf("%q", line)
	}
}

func TestTheLineCarriesBothAveragesThePromptAsksFor(t *testing.T) {
	d50, d200 := 100.0, 80.0
	line := Pick{Symbol: "ABB.NS", Name: "ABB India", Close: 110, Ret5: 0.02, DMA50: &d50, DMA200: &d200}.Line("INR")
	if !contains(line, "+10.0% against its 50-day") || !contains(line, "+37.5% against its 200-day") {
		t.Fatalf("%q", line)
	}
	if got := (Pick{Symbol: "S.NS", Name: "S", Close: 110, Ret5: 0.02}).Line("INR"); contains(got, "day") {
		t.Fatalf("a short series must not invent an average: %q", got)
	}
}

func TestAnAverageNeedsAFullWindow(t *testing.T) {
	full := make([]float64, 200)
	for i := range full {
		full[i] = 100
	}
	if got, ok := average(full, 200); !ok || got != 100 {
		t.Fatalf("%v %v", got, ok)
	}
	if _, ok := average(full[:199], 200); ok {
		t.Fatal("199 closes is not a 200-day average")
	}
}

func TestTheSteadySliceIsNamesThatAreNotMoving(t *testing.T) {
	d50, d200 := 100.0, 100.0
	mk := func(sym string, close float64) Pick {
		fifty := d50
		return Pick{Symbol: sym, Name: sym, Close: close, Ret5: 0.01, DMA50: &fifty, DMA200: &d200}
	}
	measured := []Pick{
		mk("CALM.NS", 101),   // 1% above the 200-day: steady
		mk("EDGE.NS", 104),   // 4% above: already extended
		mk("CHASE.NS", 160),  // 60% above: the vertical run the prompt refuses
		mk("SAGGING.NS", 95), // below its 50-day
	}
	out := SteadyList(measured, map[string]bool{}, 5)
	if len(out) != 1 || out[0].Symbol != "CALM.NS" {
		t.Fatalf("%+v", out)
	}
	if got := SteadyList(measured, map[string]bool{"CALM.NS": true}, 5); len(got) != 0 {
		t.Fatalf("a name already offered as a mover is not offered twice: %+v", got)
	}
}

func contains(s, sub string) bool {
	for i := 0; i+len(sub) <= len(s); i++ {
		if s[i:i+len(sub)] == sub {
			return true
		}
	}
	return false
}
