package api

import (
	"encoding/json"
	"net/http"
	"testing"

	"stock-predictor/internal/ids"
	"stock-predictor/internal/tstamp"
)

// A plan call carries no decision and no instrument of its own. It still has to be a row on the
// audit list, or a run that called the model looks like a run that did not.
func TestAPlanCallIsOnTheAuditList(t *testing.T) {
	pool, r, _ := setup(t)
	if _, err := pool.Exec(
		`INSERT INTO api_calls (id, ts, conversation_id, round, endpoint, purpose, prompt_template,
            request_json, response_json, http_status, prompt_tokens, completion_tokens, latency_ms,
            trigger)
         VALUES (?, ?, 'plan-1795299614456', 1, 'chat', 'plan trades', 'plan_trade_v1', '{}', '{}',
            200, 10, 5, 860, 'trade plan: 0 holdings')`,
		ids.New(), tstamp.Now()); err != nil {
		t.Fatal(err)
	}
	code, body := do(t, r, http.MethodGet, "/api/calls?limit=10", nil, nil)
	if code != http.StatusOK {
		t.Fatalf("%d %s", code, body)
	}
	var rows []map[string]any
	if err := json.Unmarshal(body, &rows); err != nil {
		t.Fatalf("%v: %s", err, body)
	}
	if len(rows) != 1 {
		t.Fatalf("%d rows: %s", len(rows), body)
	}
	if rows[0]["purpose"] != "plan trades" {
		t.Fatalf("%v", rows[0])
	}
}
