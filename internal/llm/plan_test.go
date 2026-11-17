package llm

import (
	"strings"
	"testing"
	"time"

	"stock-predictor/internal/models"
	"stock-predictor/internal/rules"
)

func held() []HoldingLine {
	ltp, stop, target := 1171.60, 1080.0, 1420.0
	return []HoldingLine{{
		Symbol: "INFY.NS", Name: "Infosys", Qty: 24, AvgCost: 1042.30,
		LTP: &ltp, Stop: &stop, Target: &target,
	}}
}

func swap(sellQty, buyQty, buyPrice, target float64) Rotation {
	return Rotation{Swap: &Swap{
		Sell: Leg{Symbol: "INFY.NS", Qty: sellQty, Price: 1171.60},
		Buy: Leg{
			Symbol: "HDFCBANK.NS", Qty: buyQty, Price: buyPrice,
			Stop: f64(buyPrice * 0.92), Target: f64(target),
		},
	}}
}

func f64(v float64) *float64 { return &v }

func TestASensibleSwapIsAllowed(t *testing.T) {
	limits := rules.DefaultLimits()
	limits.MinTradeAmount = 1_000
	limits.MaxTradeAmount = 1_000_000
	if err := ValidateRotation(swap(24, 8, 784.10, 980.0), held(), 1000, limits, models.ModeLong); err != nil {
		t.Fatal(err)
	}
}

func TestTheBuyCannotCostMoreThanTheSalePlusCash(t *testing.T) {
	limits := rules.DefaultLimits()
	limits.MinTradeAmount = 1_000
	limits.MaxTradeAmount = 1_000_000
	err := ValidateRotation(swap(24, 40, 784.10, 980.0), held(), 100, limits, models.ModeLong)
	if err == nil || !strings.Contains(err.Error(), "exceeds proceeds") {
		t.Fatalf("%v", err)
	}
}

func TestSellingALossMakerIsRefused(t *testing.T) {
	rows := held()
	rows[0].LTP = f64(1000.0)
	limits := rules.DefaultLimits()
	limits.MinTradeAmount = 1_000
	limits.MaxTradeAmount = 1_000_000
	err := ValidateRotation(swap(24, 8, 784.10, 980.0), rows, 0, limits, models.ModeLong)
	if err == nil || !strings.Contains(err.Error(), "below the") {
		t.Fatalf("%v", err)
	}
}

func TestABuyWithoutBothExitsIsRefused(t *testing.T) {
	r := swap(24, 8, 784.10, 980.0)
	r.Swap.Buy.Target = nil
	limits := rules.DefaultLimits()
	err := ValidateRotation(r, held(), 0, limits, models.ModeLong)
	if err == nil || !strings.Contains(err.Error(), "stop and a target") {
		t.Fatalf("%v", err)
	}
}

func TestARotationReplyIsParsedFromProseOrAFence(t *testing.T) {
	raw := "Here you go:\n```json\n" + `{"action":"rotate",
 "sell":{"symbol":"INFY.NS","qty":24,"price":1171.60,"why":"target reached"},
 "buy":{"symbol":"HDFCBANK.NS","qty":8,"price":784.10,"stop":700,"target":980,
        "rationale":"stronger trend","confidence":"medium"}}` + "\n```"
	r, err := ParseRotation(raw)
	if err != nil {
		t.Fatal(err)
	}
	if r.Swap == nil || r.Swap.Sell.Symbol != "INFY.NS" || *r.Swap.Buy.Stop != 700 {
		t.Fatalf("%+v", r)
	}
}

func TestAPassIsAValidAnswer(t *testing.T) {
	r, err := ParseRotation(`{"action":"pass","why":"nothing worth selling"}`)
	if err != nil || !r.Pass || !strings.Contains(r.Why, "nothing") {
		t.Fatalf("%+v %v", r, err)
	}
}

func planMove(action models.Action, sym string, qty, price float64) Move {
	return Move{Action: action, Symbol: sym, Qty: f64(qty), Price: f64(price)}
}

func TestPlanBuyNeedsAStopAndTarget(t *testing.T) {
	p := Plan{Moves: []Move{planMove(models.ActionBuy, "TCS.NS", 1, 100)}}
	if err := ValidatePlan(p, nil, Cash{Stock: 10_000}, rules.DefaultLimits(), "INR", time.Now(), nil, ScopeStock); err == nil {
		t.Fatal("a buy without exits passed")
	}
}

func TestPlanCannotSpendMoreThanCashPlusItsOwnSales(t *testing.T) {
	buy := planMove(models.ActionBuy, "TCS.NS", 100, 100)
	buy.Stop, buy.Target = f64(95), f64(110)
	p := Plan{Moves: []Move{buy}}
	cash := Cash{Stock: 5_000}
	if err := ValidatePlan(p, nil, cash, rules.DefaultLimits(), "INR", time.Now(), nil, ScopeStock); err == nil {
		t.Fatal("overspending plan passed")
	}
}

func TestPlanCannotNameASymbolTwice(t *testing.T) {
	sell := planMove(models.ActionSell, "INFY.NS", 1, 100)
	buy := planMove(models.ActionBuy, "INFY.NS", 1, 100)
	buy.Stop, buy.Target = f64(95), f64(110)
	p := Plan{Moves: []Move{sell, buy}}
	book := []HoldingLine{{Symbol: "INFY.NS", Qty: 10, AvgCost: 90, LTP: f64(100)}}
	err := ValidatePlan(p, book, Cash{Stock: 1e6}, rules.DefaultLimits(), "INR", time.Now(), nil, ScopeStock)
	if err == nil || !strings.Contains(err.Error(), "twice") {
		t.Fatalf("%v", err)
	}
}

func TestFundPlanMayOnlyMoveFunds(t *testing.T) {
	m := planMove(models.ActionBuy, "INFY.NS", 0, 0)
	m.Amount = f64(1_000)
	p := Plan{Moves: []Move{m}}
	err := ValidatePlan(p, nil, Cash{Fund: 10_000}, rules.DefaultLimits(), "INR", time.Now(), nil, ScopeFund)
	if err == nil || !strings.Contains(err.Error(), "is a stock") {
		t.Fatalf("%v", err)
	}
}

func TestTradePlanBuyNeedsAnExpiryWithinThreeSessions(t *testing.T) {
	now := time.Date(2026, 11, 9, 6, 0, 0, 0, time.UTC) // Monday
	buy := planMove(models.ActionBuy, "TCS.NS", 1, 100)
	buy.Stop, buy.Target = f64(95), f64(110)
	if err := ValidatePlan(Plan{Moves: []Move{buy}}, nil, Cash{Trade: 10_000}, rules.DefaultLimits(), "INR", now, nil, ScopeTrade); err == nil {
		t.Fatal("a trade buy without expiry passed")
	}
	far := SessionDeadline(now, 5).Format(time.RFC3339)
	buy.ValidUntil = &far
	if err := ValidatePlan(Plan{Moves: []Move{buy}}, nil, Cash{Trade: 10_000}, rules.DefaultLimits(), "INR", now, nil, ScopeTrade); err == nil {
		t.Fatal("an expiry beyond three sessions passed")
	}
	soon := SessionDeadline(now, 2).Format(time.RFC3339)
	buy.ValidUntil = &soon
	if err := ValidatePlan(Plan{Moves: []Move{buy}}, nil, Cash{Trade: 10_000}, rules.DefaultLimits(), "INR", now, nil, ScopeTrade); err != nil {
		t.Fatal(err)
	}
}

func TestATradePlanCannotAddToANameItAlreadyHolds(t *testing.T) {
	now := time.Date(2026, 11, 9, 6, 0, 0, 0, time.UTC) // Monday
	buy := planMove(models.ActionBuy, "TCS.NS", 1, 100)
	buy.Stop, buy.Target = f64(95), f64(110)
	until := SessionDeadline(now, 2).Format(time.RFC3339)
	buy.ValidUntil = &until
	book := []HoldingLine{{Symbol: "TCS.NS", Qty: 10, AvgCost: 90, LTP: f64(100)}}

	err := ValidatePlan(Plan{Moves: []Move{buy}}, book, Cash{Trade: 10_000},
		rules.DefaultLimits(), "INR", now, nil, ScopeTrade)
	if err == nil || !strings.Contains(err.Error(), "already held") {
		t.Fatalf("%v", err)
	}
	// A SELL of the same name is still the point of the book.
	sell := planMove(models.ActionSell, "TCS.NS", 10, 100)
	if err := ValidatePlan(Plan{Moves: []Move{sell}}, book, Cash{Trade: 0},
		rules.DefaultLimits(), "INR", now, nil, ScopeTrade); err != nil {
		t.Fatalf("a sell must remain valid: %v", err)
	}
	// The long-term book may still average up deliberately.
	if err := ValidatePlan(Plan{Moves: []Move{buy}}, book, Cash{Stock: 10_000},
		rules.DefaultLimits(), "INR", now, nil, ScopeStock); err != nil {
		t.Fatalf("the long book may add: %v", err)
	}
}

func TestSessionDeadlineSkipsWeekends(t *testing.T) {
	friday := time.Date(2026, 11, 13, 6, 0, 0, 0, time.UTC)
	got := SessionDeadline(friday, 1)
	if got.Weekday() != time.Monday {
		t.Fatalf("one session after Friday is Monday, got %s", got.Weekday())
	}
	if got.Hour() != 10 {
		t.Fatalf("must land on the 15:30 IST close, got %s", got)
	}
}

func TestPlanParsePassAndTooManyMoves(t *testing.T) {
	a, err := ParsePlan(`{"pass_reason":"no edge"}`)
	if err != nil || !a.Pass || a.Why != "no edge" {
		t.Fatalf("%+v %v", a, err)
	}
	moves := `{"moves":[{"action":"BUY","symbol":"A"},{"action":"BUY","symbol":"B"},{"action":"BUY","symbol":"C"},{"action":"BUY","symbol":"D"}]}`
	if _, err := ParsePlan(moves); err == nil {
		t.Fatal("four moves passed")
	}
	if _, err := ParsePlan(`{"moves":[{"action":"HOLD","symbol":"A"}]}`); err == nil {
		t.Fatal("a non-BUY/SELL move passed")
	}
}

func TestPlanParseToleratesProse(t *testing.T) {
	a, err := ParsePlan("Sure:\n" + `{"moves":[{"action":"SELL","symbol":"INFY.NS","qty":2,"price":10}],"rationale":"x"}`)
	if err != nil || a.Plan == nil || len(a.Plan.Moves) != 1 || a.Plan.Moves[0].Symbol != "INFY.NS" {
		t.Fatalf("%+v %v", a, err)
	}
}
