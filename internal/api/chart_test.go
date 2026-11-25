package api

import (
	"encoding/json"
	"fmt"
	"net/http"
	"testing"
	"time"
)

// Each range asks for the series that fits it: a month and three months from the daily rows, a year
// from the buckets. Intraday ranges go live and are covered by the market package's own test.
func TestTheChartRangePicksTheRightSeries(t *testing.T) {
	pool, r, id := setup(t)
	base := time.Date(2025, 9, 1, 0, 0, 0, 0, time.UTC)
	for i := 0; i < 90; i++ {
		if _, err := pool.Exec(
			`INSERT INTO bars (instrument_id, ts, open, high, low, close, volume, resolution)
             VALUES (?, ?, ?, ?, ?, ?, 0, 'daily')`,
			id, base.AddDate(0, 0, i).Format("2006-01-02T15:04:05.000000000+00:00"),
			100.0, 101.0, 99.0, 100.0); err != nil {
			t.Fatal(err)
		}
	}
	for i := 0; i < 5; i++ {
		if _, err := pool.Exec(
			`INSERT INTO bars (instrument_id, ts, open, high, low, close, volume, resolution)
             VALUES (?, ?, ?, ?, ?, ?, 13, 'bucket')`,
			id, base.AddDate(0, 0, 200+i*13).Format("2006-01-02T15:04:05.000000000+00:00"),
			90.0, 95.0, 85.0, 90.0); err != nil {
			t.Fatal(err)
		}
	}

	for _, tc := range []struct {
		query      string
		resolution string
		want       int
	}{
		{"range=1m", "daily", 30},
		{"range=3m", "daily", 70},
		{"range=1y", "bucket", 5},
	} {
		code, body := do(t, r, http.MethodGet, "/api/instruments/INFY.NS/chart?"+tc.query, nil, nil)
		if code != http.StatusOK {
			t.Fatalf("%s: %d %s", tc.query, code, body)
		}
		var got struct {
			Resolution string `json:"resolution"`
			Bars       []struct {
				Close float64 `json:"close"`
			} `json:"bars"`
		}
		if err := json.Unmarshal(body, &got); err != nil {
			t.Fatal(err)
		}
		if got.Resolution != tc.resolution || len(got.Bars) != tc.want {
			t.Fatalf("%s: %s with %d points, wanted %s with %d",
				tc.query, got.Resolution, len(got.Bars), tc.resolution, tc.want)
		}
	}

	code, body := do(t, r, http.MethodGet,
		"/api/instruments/INFY.NS/chart?range=custom&from=not-a-date&to=2026-01-01", nil, nil)
	if code != http.StatusInternalServerError {
		t.Fatalf("a bad custom range must say so: %d %s", code, body)
	}
	if msg := fmt.Sprint(string(body)); msg == "" {
		t.Fatal("no message")
	}
}
