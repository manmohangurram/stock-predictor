package api

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"
)

func TestTheMarketReadsTheDataNotACalendar(t *testing.T) {
	state, reason := marketStateFor(false, 600, true)
	if state != "closed" || reason != "Weekend" {
		t.Fatalf("%s %s", state, reason)
	}
	state, _ = marketStateFor(true, 9*60, true)
	if state != "closed" {
		t.Fatal("before the open")
	}
	state, _ = marketStateFor(true, 10*60, true)
	if state != "open" {
		t.Fatal("in session with prices")
	}
	state, _ = marketStateFor(true, 10*60, false)
	if state != "holiday" {
		t.Fatal("in session with no prices is a holiday reading")
	}
	state, reason = marketStateFor(true, 9*60+30, false)
	if state != "open" || reason == "" {
		t.Fatal("the first price has not landed yet")
	}
}

func TestTheMarketEndpointAnswersWithTheAgreedShape(t *testing.T) {
	_, r, _ := setup(t)
	code, body := do(t, r, http.MethodGet, "/api/market", nil, nil)
	if code != http.StatusOK {
		t.Fatalf("%d %s", code, body)
	}
	var v map[string]any
	if err := json.Unmarshal(body, &v); err != nil {
		t.Fatal(err)
	}
	if v["session"] != "09:15–15:30 IST, Mon–Fri" || v["holiday_calendar"] != false {
		t.Fatalf("%v", v)
	}
	if _, ok := v["state"]; !ok {
		t.Fatal("no state")
	}
}

func TestTheScheduleComesFromTheScheduler(t *testing.T) {
	_, r, _ := setup(t)
	code, body := do(t, r, http.MethodGet, "/api/schedule", nil, nil)
	if code != http.StatusOK {
		t.Fatalf("%d %s", code, body)
	}
	var v struct {
		Prompts []map[string]any `json:"prompts"`
	}
	if err := json.Unmarshal(body, &v); err != nil {
		t.Fatal(err)
	}
	if len(v.Prompts) != 3 {
		t.Fatalf("%+v", v.Prompts)
	}
	if v.Prompts[2]["prompt"] != "plan_trade" {
		t.Fatalf("%+v", v.Prompts[2])
	}
}

func TestTheTradeNextIsAlwaysInsideASlot(t *testing.T) {
	if next := tradeNext(time.Now().UTC()); next == nil {
		t.Fatal("no next slot within four days")
	}
}

func TestTheCallsPagerCountsTheFilteredTotal(t *testing.T) {
	pool, r, id := setup(t)
	for i := 0; i < 3; i++ {
		if _, err := pool.Exec(
			`INSERT INTO api_calls (ts, endpoint, purpose, prompt_template)
             VALUES (?, 'chat', 'decide INFY.NS', 'decide_v1')`,
			time.Now().UTC().Format(time.RFC3339)); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := pool.Exec(
		`INSERT INTO api_calls (ts, endpoint, purpose) VALUES (?, 'search', 'search INFY.NS')`,
		time.Now().UTC().Format(time.RFC3339)); err != nil {
		t.Fatal(err)
	}
	_ = id
	req := httptest.NewRequest(http.MethodGet, "/api/calls?call=chat&limit=2", nil)
	w := httptest.NewRecorder()
	r.ServeHTTP(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("%d %s", w.Code, w.Body.String())
	}
	if got := w.Header().Get("x-total-count"); got != "3" {
		t.Fatalf("filtered total %q", got)
	}
	var rows []map[string]any
	if err := json.Unmarshal(w.Body.Bytes(), &rows); err != nil {
		t.Fatal(err)
	}
	if len(rows) != 2 {
		t.Fatalf("page size %d", len(rows))
	}
}

func TestTheFacetsComeFromTheData(t *testing.T) {
	pool, r, _ := setup(t)
	if _, err := pool.Exec(
		`INSERT INTO api_calls (ts, endpoint, purpose, prompt_template)
         VALUES (?, 'chat', 'decide INFY.NS', 'decide_v1')`, time.Now().UTC().Format(time.RFC3339)); err != nil {
		t.Fatal(err)
	}
	code, body := do(t, r, http.MethodGet, "/api/calls/facets", nil, nil)
	if code != http.StatusOK {
		t.Fatalf("%d %s", code, body)
	}
	var v map[string][]string
	if err := json.Unmarshal(body, &v); err != nil {
		t.Fatal(err)
	}
	if len(v["calls"]) != 1 || v["calls"][0] != "chat" {
		t.Fatalf("%v", v)
	}
}
