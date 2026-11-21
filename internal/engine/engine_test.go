package engine

import (
	"database/sql"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"stock-predictor/internal/db"
	"stock-predictor/internal/llm"
	"stock-predictor/internal/models"
	"stock-predictor/internal/rules"
	"stock-predictor/internal/tstamp"
)

func pool(t *testing.T) *sql.DB {
	t.Helper()
	p, err := db.ConnectAndMigrate(t.Context(), filepath.Join(t.TempDir(), "t.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { p.Close() })
	return p
}

func TestTheReadWindowIsAWeekAndNoMore(t *testing.T) {
	p := pool(t)
	now := time.Date(2026, 11, 13, 5, 0, 0, 0, time.UTC)
	mustExec(t, p,
		`INSERT INTO instruments (id, symbol, class, name, currency, active)
         VALUES ('01a1f000-0000-7000-8000-000000000010', 'X.NS', 'stock', 'X', 'INR', 1)`)
	mustExec(t, p,
		`INSERT INTO news (id, instrument_id, published_at, title, publisher, url)
         VALUES ('01a1f000-0000-7000-8000-000000000001', '01a1f000-0000-7000-8000-000000000010', '2026-11-08T00:00:00Z', 'five days ago', 'Reuters', 'https://example.test/old')`)
	kept, err := storedHeadlines(p, "01a1f000-0000-7000-8000-000000000010", now)
	if err != nil {
		t.Fatal(err)
	}
	if len(kept) != 1 || kept[0].Title != "five days ago" {
		t.Fatalf("%+v", kept)
	}
	mustExec(t, p,
		`INSERT INTO news (id, instrument_id, published_at, title, publisher, url)
         VALUES ('01a1f000-0000-7000-8000-000000000002', '01a1f000-0000-7000-8000-000000000010', '2026-11-04T00:00:00Z', 'nine days ago', 'Reuters', 'https://example.test/older')`)
	kept, _ = storedHeadlines(p, "01a1f000-0000-7000-8000-000000000010", now)
	if len(kept) != 1 {
		t.Fatalf("nine days old is beyond the window: %+v", kept)
	}
}

func mustExec(t *testing.T, p *sql.DB, q string, args ...any) {
	t.Helper()
	if _, err := p.Exec(q, args...); err != nil {
		t.Fatal(err)
	}
}

// The poll reads the last signal twice before it can decide anything, and both reads have to agree
// that an id is the UUID it was written as.
func TestASignalIDStaysTextOnTheWayBack(t *testing.T) {
	p := pool(t)
	inst := "01a1f000-0000-7000-8000-000000000031"
	sig := "01a1f000-0000-7000-8000-000000000032"
	now := time.Date(2026, 11, 13, 5, 0, 0, 0, time.UTC)
	mustExec(t, p, `INSERT INTO instruments (id, symbol, class, name, currency, active)
		VALUES (?, 'X.NS', 'stock', 'X', 'INR', 1)`, inst)
	mustExec(t, p, `INSERT INTO signals (id, instrument_id, action, entry, valid_until, reason_json,
		created_at, confidence, mode, kind, status)
		VALUES (?, ?, 'BUY', 100, ?, '{"rationale":"momentum"}', ?, 'medium', 'long', 'model', 'pending')`,
		sig, inst, tstamp.From(now.Add(18*time.Hour)), tstamp.From(now))

	prev, err := latestSignal(p, inst)
	if err != nil {
		t.Fatalf("latestSignal could not read a text id: %v", err)
	}
	if prev.ID != sig {
		t.Fatalf("id = %q, want %q", prev.ID, sig)
	}

	reused, err := reuseModelSignal(p, inst, 100.5, 2, now)
	if err != nil {
		t.Fatalf("reuseModelSignal could not read a text id: %v", err)
	}
	if !reused {
		t.Fatal("a live model signal inside the band was not reused")
	}
	var entry float64
	if err := p.QueryRow("SELECT entry FROM signals WHERE id = ?", sig).Scan(&entry); err != nil {
		t.Fatal(err)
	}
	if entry != 100.5 {
		t.Fatalf("entry = %v, want the refreshed price", entry)
	}
}

func TestTheBandTriggerFiresOnceADayPerReason(t *testing.T) {
	p := pool(t)
	inst := models.Instrument{ID: "01a1f000-0000-7000-8000-000000000007", Symbol: "X.NS", Class: models.ClassStock}
	now := time.Date(2026, 11, 13, 6, 0, 0, 0, time.UTC)
	if ok, err := bandTriggerFires(p, inst, "ceiling", now); err != nil || !ok {
		t.Fatalf("%v %v", ok, err)
	}
	// Same reason, same day: no second call.
	if ok, _ := bandTriggerFires(p, inst, "ceiling", now); ok {
		t.Fatal("a second call for the same edge")
	}
	// A different reason is a different event.
	if ok, _ := bandTriggerFires(p, inst, "broken", now); !ok {
		t.Fatal("a different reason must fire")
	}
}

func f64(v float64) *float64 { return &v }

func TestACandidateIsClampedToThePotLimits(t *testing.T) {
	limits := rulesLimits()
	c := models.Candidate{Action: models.ActionBuy, Entry: 100, Qty: 1000}
	clamped, ok := clampToLimits(c, limits, 50_000, 0)
	if !ok {
		t.Fatal("dropped")
	}
	// 25% of 50k = 12,500 max fill = 125 units; the rules had 1000.
	if clamped.Qty != 125 {
		t.Fatalf("qty %v", clamped.Qty)
	}
	if _, ok := clampToLimits(c, limits, 50_000, 12_500); ok {
		t.Fatal("no room left must drop it")
	}
	small := models.Candidate{Action: models.ActionBuy, Entry: 100, Qty: 1}
	if _, ok := clampToLimits(small, limits, 50_000, 0); ok {
		t.Fatal("a buy below the minimum is not worth its fees")
	}
	// A sell of a tiny remainder still goes through.
	sell := models.Candidate{Action: models.ActionSell, Entry: 100, Qty: 1}
	if _, ok := clampToLimits(sell, limits, 50_000, 0); !ok {
		t.Fatal("a tiny sell is still a sale")
	}
}

func rulesLimits() rules.Limits {
	return rules.Limits{
		MinProfitPct: 5, MaxProfitPct: 50, MinTradeAmount: 500,
		MaxTradeAmount: 50_000, MaxInstrumentPct: 25,
	}
}

func TestResolveKeepsRulesAsTheSpineForAPass(t *testing.T) {
	now := time.Date(2026, 11, 13, 6, 0, 0, 0, time.UTC)
	c := models.Candidate{Action: models.ActionBuy, Entry: 100, Qty: 10}
	if _, ok := resolve(c, llm.Outcome{Kind: llm.OutcomePass}, now); ok {
		t.Fatal("a pass records nothing")
	}
	res, ok := resolve(c, llm.Outcome{Kind: llm.OutcomeLlm, Proposal: &llm.Proposal{
		Action: models.ActionBuy, Entry: 101, ValidUntil: "2026-11-14T00:00:00Z", Rationale: "x",
	}}, now)
	if !ok || res.entry != 101 || res.qty != 10 {
		t.Fatalf("%+v", res)
	}
	// An urgent rules sell overrides what the model said.
	c.Urgent = true
	res, _ = resolve(c, llm.Outcome{Kind: llm.OutcomeLlm, Proposal: &llm.Proposal{
		Action: models.ActionHold, Entry: 90, Qty: f64(3), ValidUntil: "2026-11-14T00:00:00Z",
	}}, now)
	if res.action != models.ActionBuy || res.qty != 10 {
		t.Fatalf("urgent rules must win: %+v", res)
	}
}

func TestFormatBookSortsByValueAndMarksTheOne(t *testing.T) {
	rows := []llm.HoldingLine{
		{Symbol: "A", Name: "A", Qty: 1, AvgCost: 10, LTP: f64(10)},
		{Symbol: "B", Name: "B", Qty: 1, AvgCost: 100, LTP: f64(100)},
	}
	out := FormatBook(rows, "A")
	lines := strings.Split(strings.TrimSpace(out), "\n")
	if !strings.HasPrefix(lines[0], "B ") {
		t.Fatalf("richest first: %q", out)
	}
	if !strings.Contains(out, "← this one") {
		t.Fatalf("the instrument under review is marked: %q", out)
	}
	if empty := FormatBook(nil, "A"); !strings.Contains(empty, "nothing is held") {
		t.Fatalf("%q", empty)
	}
}

func TestTheWindowIsClampedToTheRequestedHours(t *testing.T) {
	now := time.Date(2026, 11, 13, 10, 0, 0, 0, time.UTC)
	bars := []models.Bar{
		{Ts: "2026-11-13T04:00:00Z", Close: 1},
		{Ts: "2026-11-13T08:00:00Z", Close: 2},
	}
	out := formatWindow(bars, 4, now)
	if strings.Contains(out, "1.00") {
		t.Fatalf("an old bar leaked in: %q", out)
	}
	if !strings.Contains(out, "2.00") {
		t.Fatalf("the recent bar is missing: %q", out)
	}
	if none := formatWindow(nil, 4, now); none != "(no bars in the last window)" {
		t.Fatalf("%q", none)
	}
}

func TestMergeTapeAppendsTodaysCandleOnce(t *testing.T) {
	now := time.Date(2026, 11, 17, 6, 0, 0, 0, time.UTC)
	daily := []models.Bar{
		{Ts: "2026-11-15T00:00:00.000000000+00:00", Close: 100},
		{Ts: "2026-11-16T00:00:00.000000000+00:00", Close: 101},
	}
	tape := []models.Bar{
		{Ts: "2026-11-17T03:45:00.000000000+00:00", Open: 101, High: 103, Low: 100, Close: 102, Volume: 10},
		{Ts: "2026-11-17T04:00:00.000000000+00:00", Open: 102, High: 104, Low: 101, Close: 103, Volume: 20},
	}
	merged := mergeTape(daily, tape, now)
	if len(merged) != 3 {
		t.Fatalf("%d bars", len(merged))
	}
	today := merged[2]
	if today.Close != 103 || today.High != 104 || today.Low != 100 || today.Volume != 20 {
		t.Fatalf("%+v", today)
	}
	if again := mergeTape(merged, tape, now); len(again) != 3 || again[2].Close != 103 {
		t.Fatalf("%+v", again)
	}
	if len(mergeTape(daily, nil, now)) != 2 {
		t.Fatal("no tape must leave the series alone")
	}
}
