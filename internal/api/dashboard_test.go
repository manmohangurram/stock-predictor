package api

import (
	"encoding/json"
	"net/http"
	"testing"

	"stock-predictor/internal/models"
	"stock-predictor/internal/trades"
)

func TestTheDashboardCarriesTheInstrumentNameOnEveryFill(t *testing.T) {
	pool, r, id := setup(t)
	if err := trades.Record(pool, id, trades.SideBuy, 10, 100, nil, nil, models.ModeLong); err != nil {
		t.Fatal(err)
	}

	code, body := do(t, r, http.MethodGet, "/api/dashboard?mode=long", nil, nil)
	if code != http.StatusOK {
		t.Fatalf("%d %s", code, body)
	}
	var dash struct {
		RecentTrades []map[string]any `json:"recent_trades"`
	}
	if err := json.Unmarshal(body, &dash); err != nil {
		t.Fatalf("%v: %s", err, body)
	}
	if len(dash.RecentTrades) != 1 {
		t.Fatalf("%d fills: %s", len(dash.RecentTrades), body)
	}
	fill := dash.RecentTrades[0]
	if fill["symbol"] != "INFY.NS" {
		t.Fatalf("symbol: %v", fill["symbol"])
	}
	if fill["name"] != "Infosys" {
		t.Fatalf("name: %v (want Infosys) — a build without it predates v1.0.5", fill["name"])
	}
}
