package engine

import (
	"strings"
	"testing"

	"stock-predictor/internal/llm"
	"stock-predictor/internal/models"
	"stock-predictor/internal/notify"
)

// A plan is the one decision that arrives without a per-stock call, so it is the one that would
// reach the queue silently. One push for the plan, not one per leg, and never twice.
func TestAStoredPlanIsPushedOnce(t *testing.T) {
	p := pool(t)
	mustExec(t, p, `INSERT INTO instruments (id, symbol, class, name, currency, active) VALUES
		('01a1f000-0000-7000-8000-0000000000e1', 'INFY.NS', 'stock', 'Infosys', 'INR', 1)`)

	fake := &notify.FakeNotifier{}
	deps := &EngineDeps{Pool: p, Notifier: fake}

	qty, price, stop, target := 5.0, 1500.0, 1400.0, 1700.0
	plan := llm.Plan{Moves: []llm.Move{{
		Action: models.ActionBuy, Symbol: "INFY.NS",
		Qty: &qty, Price: &price, Stop: &stop, Target: &target,
		Why: "above both averages",
	}}}
	planID := "01a1f000-0000-7000-8000-0000000000f1"

	notifyPlan(deps, llm.ScopeStock, plan, &planID, "INR")
	notifyPlan(deps, llm.ScopeStock, plan, &planID, "INR")

	if fake.Count() != 1 {
		t.Fatalf("one push per plan, got %d: %+v", fake.Count(), fake.Sent)
	}
	n := fake.Sent[0]
	if n.Title != "stock plan · 1 move waiting" {
		t.Fatalf("%q", n.Title)
	}
	if !strings.Contains(n.Body, "BUY Infosys · 5 units at 1500.00") {
		t.Fatalf("%q", n.Body)
	}
	if n.Priority != notify.PriorityDefault {
		t.Fatalf("a buy alone is not a high-priority push: %q", n.Priority)
	}
}

func TestASellInThePlanRaisesThePush(t *testing.T) {
	p := pool(t)
	mustExec(t, p, `INSERT INTO instruments (id, symbol, class, name, currency, active) VALUES
		('01a1f000-0000-7000-8000-0000000000e2', 'TCS.NS', 'stock', 'Tata Consultancy', 'INR', 1)`)
	fake := &notify.FakeNotifier{}
	deps := &EngineDeps{Pool: p, Notifier: fake}

	qty, price := 4.0, 3200.0
	plan := llm.Plan{Moves: []llm.Move{{
		Action: models.ActionSell, Symbol: "TCS.NS", Qty: &qty, Price: &price,
	}}}
	planID := "01a1f000-0000-7000-8000-0000000000f2"
	notifyPlan(deps, llm.ScopeTrade, plan, &planID, "INR")

	if fake.Count() != 1 {
		t.Fatalf("%d", fake.Count())
	}
	if fake.Sent[0].Priority != notify.PriorityHigh {
		t.Fatalf("a sell is high priority: %q", fake.Sent[0].Priority)
	}
	if fake.Sent[0].Title != "trade plan · 1 move waiting" {
		t.Fatalf("%q", fake.Sent[0].Title)
	}
}
