package llm

import (
	"strings"
	"testing"
	"time"

	"stock-predictor/internal/models"
	"stock-predictor/internal/rules"
	"stock-predictor/internal/validate"
)

func TestVerdictDecide(t *testing.T) {
	v, err := Verdict(`{"action":"decide","signal":{"action":"buy","entry":101.5,"stop":95.0,
		"target":112.0,"valid_until":"2030-01-01T00:00:00Z","confidence":"high",
		"rationale":"trend","cites":[1,2]}}`)
	if err != nil {
		t.Fatal(err)
	}
	if v.Decide == nil || v.Decide.Action != models.ActionBuy {
		t.Fatalf("got %+v", v)
	}
	if v.Decide.Confidence != ConfHigh || len(v.Decide.Cites) != 2 {
		t.Fatalf("got %+v", v.Decide)
	}
}

func TestVerdictNeedMoreTruthyVariants(t *testing.T) {
	for _, body := range []string{
		`{"action":"need_more","headlines":[{"id":1}],"price_history":true}`,
		`{"action":"need_more","requests":[{"headline_id":1}],"price_history":"7d"}`,
		`{"action":"need_more","headlines":[{"headlineId":1}],"price_history":7}`,
	} {
		v, err := Verdict(body)
		if err != nil {
			t.Fatalf("%s: %v", body, err)
		}
		if v.NeedMore == nil || !v.NeedMore.PriceHistory || len(v.NeedMore.Headlines) != 1 {
			t.Fatalf("%s: got %+v", body, v.NeedMore)
		}
	}
}

func TestVerdictPassAndErrors(t *testing.T) {
	v, err := Verdict(`{"action":"pass","why":"no edge"}`)
	if err != nil || !v.Pass || v.Why != "no edge" {
		t.Fatalf("got %+v, %v", v, err)
	}
	if _, err := Verdict(`{"action":"need_more"}`); err == nil {
		t.Fatal("empty need_more accepted")
	}
	if _, err := Verdict(`{"action":"decide"}`); err == nil {
		t.Fatal("decide without signal accepted")
	}
	if _, err := Verdict(`not json`); err == nil {
		t.Fatal("garbage accepted")
	}
}

func TestTemplateHashIsOfTheTemplateNotTheRenderedText(t *testing.T) {
	a, err := TemplateHash("decide_v1")
	if err != nil {
		t.Fatal(err)
	}
	b, _ := TemplateHash("decide_v1")
	if a != b || len(a) != 64 {
		t.Fatalf("%q %q", a, b)
	}
	if _, err := TemplateHash("nope"); err == nil {
		t.Fatal("unknown template accepted")
	}
	// The old name still resolves, so an audit row written before the split still renders.
	if _, err := TemplateHash("plan_v1"); err != nil {
		t.Fatal(err)
	}
}

func TestRenderSubstitutesEveryField(t *testing.T) {
	ind := &rules.Indicators{SMA50: 100, SMA200: 90, RSI14: 55, ATR14: 2, High52: 120, Low52: 80}
	ctx := &PromptContext{
		Instrument: &models.Instrument{Symbol: "INFY.NS", Class: models.ClassStock, Currency: "INR"},
		Ind:        ind, LastClose: 105, CashAvailable: 10_000,
		Limits: "limits", WindowHours: 4, PriceWindow: "tape", DailyDays: 30,
		DailySummary: "daily", Fundamentals: "fund", ShortContext: "short",
		Positions: "pos", RecentTrades: "trades", LastDecision: "last",
		Now: time.Date(2026, 11, 16, 10, 0, 0, 0, time.UTC),
	}
	out, err := Render("decide_v1", ctx)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(out.Text, "{{") {
		t.Fatalf("unsubstituted placeholder: %s", out.Text)
	}
	if !strings.Contains(out.Text, "INFY.NS") || !strings.Contains(out.Text, "10000.00") {
		t.Fatalf("missing values: %s", out.Text)
	}
	if out.Template != "decide_v1" || out.Hash == "" {
		t.Fatalf("%+v", out)
	}
}

func TestCapLineIsTheFactNotAFence(t *testing.T) {
	if got := CapLine(1234.5, "INR"); !strings.Contains(got, "1234.50 INR") ||
		!strings.Contains(got, "refused, not shrunk") {
		t.Fatalf("%q", got)
	}
}

func TestFingerprintIsOrderIndependent(t *testing.T) {
	a := []models.Headline{{Title: "A", URL: "u1"}, {Title: "B", URL: "u2"}}
	b := []models.Headline{{Title: "B", URL: "u2"}, {Title: "A", URL: "u1"}}
	if Fingerprint(a) != Fingerprint(b) {
		t.Fatal("order changed the fingerprint")
	}
	c := []models.Headline{{Title: "A", URL: "u3"}, {Title: "B", URL: "u2"}}
	if Fingerprint(a) == Fingerprint(c) {
		t.Fatal("a new url must be new information")
	}
}

func validCtx() ValidateCtx {
	stop, target := 95.0, 112.0
	_ = stop
	_ = target
	return ValidateCtx{
		Bands: rules.Bands{Min: 90, Max: 120}, ATR: 3, AvailableCash: 50_000, Qty: 100,
		SentHeadlineIDs: []int64{1, 2}, Limits: rules.DefaultLimits(), RequiredProfitPct: 5,
		Pot: 50_000, Now: time.Date(2026, 11, 16, 10, 0, 0, 0, time.UTC),
	}
}

func buy(entry float64, stop, target *float64) *Proposal {
	return &Proposal{
		Action: models.ActionBuy, Entry: entry, Stop: stop, Target: target,
		ValidUntil: "2026-11-20T10:00:00Z", Confidence: ConfHigh, Cites: []int64{1},
	}
}

func TestCheckAcceptsAWellFormedBuy(t *testing.T) {
	stop, target := 95.0, 112.0
	if err := CheckProposal(buy(101, &stop, &target), validCtx()); err != nil {
		t.Fatal(err)
	}
}

func TestCheckRejectsWhatTheRulesForbid(t *testing.T) {
	ctx := validCtx()
	stopHigh := 105.0
	if err := CheckProposal(buy(101, &stopHigh, nil), ctx); err == nil {
		t.Fatal("stop above entry accepted")
	}
	stop, target := 95.0, 112.0
	p := buy(101, &stop, &target)
	p.Cites = nil
	if err := CheckProposal(p, ctx); err == nil || err.(validate.Error).Kind != validate.KindMissingCitation {
		t.Fatalf("missing citation: %v", err)
	}
	p = buy(101, &stop, &target)
	p.Cites = []int64{99}
	if err := CheckProposal(p, ctx); err == nil {
		t.Fatal("unknown headline accepted")
	}
	p = buy(101, &stop, &target)
	p.ValidUntil = "2020-01-01T00:00:00Z"
	if err := CheckProposal(p, ctx); err == nil {
		t.Fatal("expired validity accepted")
	}
	p = buy(101, &stop, &target)
	p.Entry = 200
	if err := CheckProposal(p, ctx); err == nil {
		t.Fatal("entry outside the band accepted")
	}
	farStop := 50.0
	p = buy(101, &farStop, &target)
	if err := CheckProposal(p, ctx); err == nil {
		t.Fatal("stop five ATRs away accepted")
	}
}

func TestCheckShortTermBuyNeedsHighConfidence(t *testing.T) {
	ctx := validCtx()
	ctx.RequiresHighConfidence = true
	ctx.RequiresExit = true
	stop, target := 95.0, 112.0
	p := buy(101, &stop, &target)
	p.Confidence = ConfMedium
	if err := CheckProposal(p, ctx); err == nil || err.(validate.Error).Kind != validate.KindConfidenceTooLow {
		t.Fatalf("medium confidence accepted: %v", err)
	}
}

func TestCheckFundBuyNeedsNoExit(t *testing.T) {
	ctx := validCtx()
	ctx.RequiresExit = false
	if err := CheckProposal(buy(101, nil, nil), ctx); err != nil {
		t.Fatal(err)
	}
}
