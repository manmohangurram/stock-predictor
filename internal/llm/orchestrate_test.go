package llm

import (
	"database/sql"
	"encoding/json"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"stock-predictor/internal/db"
	"stock-predictor/internal/market"
	"stock-predictor/internal/models"
	"stock-predictor/internal/rules"
	"stock-predictor/internal/validate"
)

func llmPool(t *testing.T) *sql.DB {
	t.Helper()
	pool, err := db.ConnectAndMigrate(t.Context(), filepath.Join(t.TempDir(), "t.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { pool.Close() })
	return pool
}

func chatEnvelope(content string) string {
	b, _ := json.Marshal(map[string]any{
		"choices": []map[string]any{{"message": map[string]any{"content": content}}},
		"usage":   map[string]any{"prompt_tokens": 10, "completion_tokens": 5},
	})
	return string(b)
}

func baseInput() RunInput {
	ind := rules.Indicators{SMA50: 100, SMA200: 90, RSI14: 55, ATR14: 3, High52: 120, Low52: 80}
	return RunInput{
		Instrument: &models.Instrument{
			ID: "01a1f000-0000-7000-8000-000000000001", Symbol: "INFY.NS", Class: models.ClassStock, Name: "Infosys", Currency: "INR", Active: true,
		},
		Ind: &ind,
		Candidate: models.Candidate{
			InstrumentID: "01a1f000-0000-7000-8000-000000000001", Action: models.ActionBuy, Entry: 100, Qty: 100,
		},
		AvailableCash: 50_000, HeldQty: 0, Limits: rules.DefaultLimits(),
		RequiredProfitPct: 5, Pot: 50_000, WindowHours: 4,
		Mode: models.ModeLong, Now: time.Date(2026, 11, 16, 6, 0, 0, 0, time.UTC),
	}
}

func TestPhaseOneFailureFallsBackToRulesNumbers(t *testing.T) {
	pool := llmPool(t)
	deps := LlmDeps{Pool: pool, Runtime: rules.DefaultRuntime(), Transport: NewFixtureTransport(nil)}
	d, err := Run(&deps, baseInput())
	if err != nil {
		t.Fatal(err)
	}
	if d.Outcome.Kind != OutcomeRules || d.Outcome.Candidate == nil {
		t.Fatalf("%+v", d.Outcome)
	}
}

func TestAPassStoresNoDecision(t *testing.T) {
	pool := llmPool(t)
	transport := NewFixtureTransport([]HttpReply{{Status: 200, Body: chatEnvelope(`{"action":"pass","why":"no edge"}`)}})
	deps := LlmDeps{Pool: pool, Runtime: rules.DefaultRuntime(), Transport: transport}
	d, err := Run(&deps, baseInput())
	if err != nil {
		t.Fatal(err)
	}
	if d.Outcome.Kind != OutcomePass || d.Outcome.Why != "no edge" {
		t.Fatalf("%+v", d.Outcome)
	}
}

func TestAValidProposalBecomesTheDecision(t *testing.T) {
	pool := llmPool(t)
	proposal := `{"action":"decide","signal":{"action":"buy","qty":100,"entry":100,
        "stop":95,"target":110,"valid_until":"2026-11-20T10:00:00Z","confidence":"high",
        "rationale":"trend","cites":[]}}`
	transport := NewFixtureTransport([]HttpReply{{Status: 200, Body: chatEnvelope(proposal)}})
	deps := LlmDeps{Pool: pool, Runtime: rules.DefaultRuntime(), Transport: transport}
	d, err := Run(&deps, baseInput())
	if err != nil {
		t.Fatal(err)
	}
	if d.Outcome.Kind != OutcomeLlm || d.Outcome.Proposal == nil {
		t.Fatalf("%+v", d.Outcome)
	}
}

func TestABadProposalIsRejectedWithItsReason(t *testing.T) {
	pool := llmPool(t)
	proposal := `{"action":"decide","signal":{"action":"buy","qty":100,"entry":100,
        "stop":105,"target":110,"valid_until":"2026-11-20T10:00:00Z","confidence":"high",
        "rationale":"trend","cites":[]}}`
	transport := NewFixtureTransport([]HttpReply{{Status: 200, Body: chatEnvelope(proposal)}})
	deps := LlmDeps{Pool: pool, Runtime: rules.DefaultRuntime(), Transport: transport}
	d, err := Run(&deps, baseInput())
	if err != nil {
		t.Fatal(err)
	}
	if d.Outcome.Kind != OutcomeRejected || d.Outcome.Error == nil {
		t.Fatalf("%+v", d.Outcome)
	}
	if d.Outcome.Error.Kind != validate.KindStopOnWrongSide {
		t.Fatalf("kind %q", d.Outcome.Error.Kind)
	}
}

type fakeMarket struct{ bars []models.Bar }

func (f fakeMarket) Bars(string) ([]models.Bar, error)             { return f.bars, nil }
func (f fakeMarket) News(string) ([]models.Headline, error)        { return nil, nil }
func (f fakeMarket) Constituents() ([]market.Constituent, error)   { return nil, nil }
func (f fakeMarket) Intraday(_, _, _ string) ([]models.Bar, error) { return f.bars, nil }

func TestOneEscalationThenADecision(t *testing.T) {
	pool := llmPool(t)
	in := baseInput()
	in.Headlines = []models.Headline{{Title: "Something happened", URL: "https://example.in/a", PublishedAt: "2026-11-16T00:00:00Z"}}
	needMore := `{"action":"need_more","headlines":[{"id":1}],"price_history":true}`
	proposal := `{"action":"decide","signal":{"action":"buy","qty":100,"entry":100,
        "stop":95,"target":110,"valid_until":"2026-11-20T10:00:00Z","confidence":"high",
        "rationale":"trend","cites":[1]}}`
	transport := NewFixtureTransport([]HttpReply{
		{Status: 200, Body: chatEnvelope(needMore)},
		{Status: 200, Body: `{"content":"an article body"}`},
		{Status: 200, Body: chatEnvelope(proposal)},
	})
	deps := LlmDeps{
		Pool: pool, Runtime: rules.DefaultRuntime(), Transport: transport,
		Market: fakeMarket{bars: []models.Bar{{Ts: "2026-11-16T05:00:00Z", Close: 100}}},
	}
	d, err := Run(&deps, in)
	if err != nil {
		t.Fatal(err)
	}
	if d.Outcome.Kind != OutcomeLlm {
		t.Fatalf("%+v", d.Outcome)
	}
	if transport.Calls() != 3 {
		t.Fatalf("expected decide, fetch, escalate; got %d calls", transport.Calls())
	}
}

func TestDailyCapSpendsNoCall(t *testing.T) {
	pool := llmPool(t)
	for i := 0; i < 3; i++ {
		if _, err := pool.Exec(
			"INSERT INTO api_calls (ts, endpoint) VALUES (?, 'chat')", time.Now().UTC().Format(time.RFC3339),
		); err != nil {
			t.Fatal(err)
		}
	}
	rt := rules.DefaultRuntime()
	rt.DailyCap = 3
	transport := NewFixtureTransport([]HttpReply{{Status: 200, Body: chatEnvelope(`{"action":"pass"}`)}})
	deps := LlmDeps{Pool: pool, Runtime: rt, Transport: transport}
	d, err := Run(&deps, baseInput())
	if err != nil {
		t.Fatal(err)
	}
	if d.Outcome.Kind != OutcomeRules {
		t.Fatalf("%+v", d.Outcome)
	}
	if transport.Calls() != 0 {
		t.Fatal("the cap must spend no call")
	}
}

func TestTheLimitsBlockShowsTheRoomLeft(t *testing.T) {
	block := LimitsBlock(rules.DefaultLimits(), 50_000, 5_000)
	if !strings.Contains(block, "leaving") || !strings.Contains(block, "5000.00") {
		t.Fatalf("%q", block)
	}
}

func TestFormatHistoryIsCompactAndOldestFirst(t *testing.T) {
	out := FormatHistory([]models.Bar{
		{Ts: "2026-11-16T05:00:00Z", Open: 1, High: 2, Low: 0.5, Close: 1.5, Volume: 100},
		{Ts: "2026-11-16T06:00:00Z", Open: 2, High: 3, Low: 1.5, Close: 2.5, Volume: 200},
	})
	lines := strings.Split(out, "\n")
	if len(lines) != 2 {
		t.Fatalf("%q", out)
	}
	if !strings.Contains(lines[0], "11-16 10:30") {
		t.Fatalf("IST rendering: %q", lines[0])
	}
	if !strings.HasPrefix(lines[0], "11-16") {
		t.Fatalf("oldest first: %q", out)
	}
}
