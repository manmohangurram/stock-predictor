package trend

import (
	"database/sql"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"stock-predictor/internal/db"
	"stock-predictor/internal/ids"
	"stock-predictor/internal/llm"
	"stock-predictor/internal/market"
	"stock-predictor/internal/models"
)

func f64(v float64) *float64 { return &v }

func cand(symbol, source string, pct *float64) candidate {
	c := candidate{symbol: symbol, source: source}
	if pct != nil {
		c.ltp = f64(100)
		c.changePct = pct
		c.previousClose = f64(99)
	}
	return c
}

func TestASymbolNamedTwiceIsOneRowAndTheExchangeKeepsIt(t *testing.T) {
	pct := 5.1
	out := dedupe([]candidate{
		cand("TATAMOTORS.NS", "gainers", &pct),
		cand("TATAMOTORS.NS", "search", nil),
		cand("IDEA.NS", "search", nil),
	})
	if len(out) != 2 {
		t.Fatalf("%+v", out)
	}
	var tata *candidate
	for i := range out {
		if out[i].symbol == "TATAMOTORS.NS" {
			tata = &out[i]
		}
	}
	if tata == nil || tata.source != "gainers" {
		t.Fatalf("the row with a price wins: %+v", tata)
	}
	if out[0].symbol != "TATAMOTORS.NS" {
		t.Fatal("ranked by the money moved, unpriced last")
	}
}

// One real session: three names up 10-11% on small money, and the two biggest turnovers up 3-4%.
// The list and its evidence budget follow the rupees, not the percentage.
func TestTheListFollowsTheMoneyNotThePercentage(t *testing.T) {
	moved := func(symbol string, pct, turnover float64) candidate {
		return candidate{
			symbol:        symbol,
			source:        "gainers",
			ltp:           f64(100),
			changePct:     f64(pct),
			previousClose: f64(99),
			turnover:      f64(turnover),
		}
	}
	out := dedupe([]candidate{
		moved("SCPL.NS", 11.25, 125_800_000),
		moved("ITI.NS", 11.12, 13_480_300_000),
		moved("SAMBHV.NS", 10.69, 4_000_600_000),
		moved("ABINFRA.NS", 10.32, 35_600_000),
		moved("TCS.NS", 3.53, 18_189_000_000),
	})
	got := []string{}
	for _, c := range out {
		got = append(got, c.symbol)
	}
	want := "ITI.NS TCS.NS SAMBHV.NS SCPL.NS ABINFRA.NS"
	if strings.Join(got, " ") != want {
		t.Fatalf("%v, wanted %s", got, want)
	}
}

func TestAnExchangeRowSaysItsPriceAndADiscoveredOneDoesNotPretendTo(t *testing.T) {
	priced := fromGainer(market.Gainer{Symbol: "TATAMOTORS", LTP: 780.5, PreviousClose: 742.1, PerChange: 5.17}, "gainers")
	if !strings.Contains(priced.describe(), "780.50 on the exchange feed") {
		t.Fatalf("%q", priced.describe())
	}
	if priced.shortName() != "TATAMOTORS" {
		t.Fatalf("%q", priced.shortName())
	}
	if !strings.Contains(cand("IDEA.NS", "search", nil).describe(), "tape") {
		t.Fatal("an unpriced name must point at the tape")
	}
}

func TestAFeedPriceFarFromOurCloseIsCalledOut(t *testing.T) {
	c := cand("CHOLAFIN.NS", "search", f64(3.82))
	c.ltp = f64(1604.2)
	c.previousClose = f64(1545.0)
	warn := c.disagreesWith(f64(1790.80))
	if !strings.Contains(warn, "1790.80") || !strings.Contains(warn, "stale") {
		t.Fatalf("%q", warn)
	}
	near := c
	near.ltp = f64(1795.0)
	if near.disagreesWith(f64(1790.80)) != "" {
		t.Fatal("a near price is not a disagreement")
	}
	if c.disagreesWith(nil) != "" {
		t.Fatal("no price of our own is not a disagreement")
	}
}

func TestTheScansNoteSaysThatItIsUnverified(t *testing.T) {
	note := scanNote("Up 3.82% to ₹1604.2")
	if !strings.Contains(note, "unverified") || !strings.Contains(note, "do not take a price") {
		t.Fatalf("%q", note)
	}
	if scanNote("  ") != "no note from the scan" {
		t.Fatalf("%q", scanNote("  "))
	}
}

func hit(title, snippet, date string) llm.SearchHit {
	d := date
	return llm.SearchHit{Title: title, Snippet: snippet, Date: &d}
}

func TestOnlyTheLastThreeDaysSurvive(t *testing.T) {
	now := time.Date(2026, 11, 11, 6, 0, 0, 0, time.UTC)
	undated := llm.SearchHit{Title: "undated quote page"}
	kept := recentHits([]llm.SearchHit{
		hit("today", "s", "2026-11-11T04:00:00+00:00"),
		hit("yesterday", "s", "2026-11-10T16:00:00+00:00"),
		hit("three days", "s", "2026-11-08T16:00:00+00:00"),
		hit("a week old", "s", "2026-11-04T16:00:00+00:00"),
		hit("a March transcript", "s", "2026-03-10T16:00:00+00:00"),
		undated,
	}, now)
	if len(kept) != 3 {
		t.Fatalf("%+v", kept)
	}
}

func TestADateIsReadAsADayNotATimestamp(t *testing.T) {
	if dayOf("2026-11-08T16:00:00+00:00") != "2026-11-08" {
		t.Fatal("day parse")
	}
	if dayOf("yesterday") != "" || dayOf("") != "" {
		t.Fatal("garbage became a day")
	}
}

func TestTheQueryAsksForTheWindowAndEnglish(t *testing.T) {
	now := time.Date(2026, 11, 11, 6, 0, 0, 0, time.UTC)
	if got := datedQuery("CHOLAFIN news today", now); got != "CHOLAFIN news today after:2026-11-08 lang:en" {
		t.Fatalf("%q", got)
	}
}

func TestAHitAboutADifferentCompanyIsDropped(t *testing.T) {
	name := "Antarctica"
	kept := relevant([]llm.SearchHit{
		hit("Anthropic retracts its $70–80bn IPO range", "$ANT listing expected next week", "2026-11-11"),
		hit("Antarctica Ltd jumps 19.67% on the BSE", "Antarctica Ltd closed at Rs 0.73", "2026-11-11"),
	}, "ANTGRAPHIC", &name)
	if len(kept) != 1 || !strings.Contains(kept[0].Title, "Antarctica") {
		t.Fatalf("%+v", kept)
	}
}

func TestTheQueryAsksForTheNameWhenThereIsOne(t *testing.T) {
	name := "Antarctica"
	if got := newsQuery("ANTGRAPHIC", &name); got != `"Antarctica" OR ANTGRAPHIC share news` {
		t.Fatalf("%q", got)
	}
	if got := newsQuery("IDEA", nil); got != "IDEA share news" {
		t.Fatalf("%q", got)
	}
}

func TestTheMicroCapsAtTheBandAreNotOffered(t *testing.T) {
	price, turnover, locked := 10.0, 10_000_000.0, 19.5
	mk := func(ltp, turnoverV, pct float64) candidate {
		return candidate{
			symbol: "X.NS", source: "gainers", ltp: f64(ltp), turnover: f64(turnoverV), changePct: f64(pct),
			previousClose: f64(ltp / (1 + pct/100)),
		}
	}
	if worthOffering(mk(1.38, 5_195_000, 20.0), price, turnover, locked) {
		t.Fatal("ORCHASP offered")
	}
	if worthOffering(mk(0.73, 2_773_000, 19.67), price, turnover, locked) {
		t.Fatal("ANTGRAPHIC offered")
	}
	if worthOffering(mk(63.76, 135_500_000, 19.98), price, turnover, locked) {
		t.Fatal("a locked stock offered")
	}
	if !worthOffering(mk(1619, 12_897_300_000, 14.95), price, turnover, locked) {
		t.Fatal("a liquid mover was refused")
	}
	if !worthOffering(cand("IDEA.NS", "search", nil), price, turnover, locked) {
		t.Fatal("a name without numbers is left to the model")
	}
}

func TestTheTapeIsNewestFirstAndShort(t *testing.T) {
	bars := []models.Bar{
		{Ts: "2026-11-07", Close: 100},
		{Ts: "2026-11-08", Close: 101},
		{Ts: "2026-11-09", Close: 102},
	}
	out := tape(bars)
	if !strings.HasPrefix(out, "2026-11-09") {
		t.Fatalf("%q", out)
	}
	if len(strings.Split(out, "\n")) != 3 {
		t.Fatalf("%q", out)
	}
}

func pool(t *testing.T) *sql.DB {
	t.Helper()
	p, err := db.ConnectAndMigrate(t.Context(), filepath.Join(t.TempDir(), "t.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { p.Close() })
	return p
}

func TestACreatedRowWithNoDecisionIsDropped(t *testing.T) {
	p := pool(t)
	id, ok, err := ensureInstrument(p, "MAKERSL.NS", "MAKERSL")
	if err != nil || !ok {
		t.Fatalf("%v %v", ok, err)
	}
	if _, err := p.Exec("INSERT INTO bars (instrument_id, ts, close) VALUES (?, '2026-11-10', 100.0)", id); err != nil {
		t.Fatal(err)
	}
	dropped, err := SweepGlances(p)
	if err != nil || dropped != 1 {
		t.Fatalf("%d %v", dropped, err)
	}
	var left int
	if err := p.QueryRow("SELECT COUNT(*) FROM instruments WHERE symbol = 'MAKERSL.NS'").Scan(&left); err != nil {
		t.Fatal(err)
	}
	if left != 0 {
		t.Fatal("the row stayed")
	}
	var bars int
	if err := p.QueryRow("SELECT COUNT(*) FROM bars").Scan(&bars); err != nil {
		t.Fatal(err)
	}
	if bars != 0 {
		t.Fatal("orphan bars stayed")
	}
}

func TestACreatedRowThatProposedSomethingStays(t *testing.T) {
	p := pool(t)
	id, _, err := ensureInstrument(p, "MAKERSL.NS", "MAKERSL")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := p.Exec(
		`INSERT INTO signals (id, instrument_id, action, qty, entry, valid_until, reason_json,
            created_at, mode, status, kind)
         VALUES (?, ?, 'BUY', 10, 268.34, '2026-11-12T15:30:00+05:30', '{}',
                 '2026-11-11T06:00:00Z', 'trade', 'pending', 'trend')`, ids.New(), id,
	); err != nil {
		t.Fatal(err)
	}
	dropped, err := SweepGlances(p)
	if err != nil || dropped != 0 {
		t.Fatalf("%d %v", dropped, err)
	}
}
