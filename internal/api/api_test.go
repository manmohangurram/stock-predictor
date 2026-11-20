package api

import (
	"bytes"
	"database/sql"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"testing"
	"time"

	"github.com/gin-gonic/gin"

	"stock-predictor/internal/config"
	"stock-predictor/internal/db"
	"stock-predictor/internal/ids"
	"stock-predictor/internal/models"
	"stock-predictor/internal/rules"
	"stock-predictor/internal/trades"
)

func setup(t *testing.T) (*sql.DB, *gin.Engine, int64) {
	t.Helper()
	gin.SetMode(gin.TestMode)
	pool, err := db.ConnectAndMigrate(t.Context(), filepath.Join(t.TempDir(), "t.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { pool.Close() })
	res, err := pool.Exec(
		`INSERT INTO instruments (symbol, class, name, currency, active, uuid)
         VALUES ('INFY.NS', 'stock', 'Infosys', 'INR', 1, ?)`, ids.New())
	if err != nil {
		t.Fatal(err)
	}
	id, _ := res.LastInsertId()
	if _, err := rules.EnsureMonth(pool, models.ModeLong, "", 100_000, time.Now().UTC()); err != nil {
		t.Fatal(err)
	}
	st := NewState(pool, &config.Config{PublicURL: "https://example.in"})
	st.Manual = make(chan ManualRequest, 8)
	return pool, Router(st), id
}

func do(t *testing.T, r *gin.Engine, method, path string, body any, headers map[string]string) (int, []byte) {
	t.Helper()
	var buf bytes.Buffer
	if body != nil {
		if err := json.NewEncoder(&buf).Encode(body); err != nil {
			t.Fatal(err)
		}
	}
	req := httptest.NewRequest(method, path, &buf)
	req.Header.Set("Content-Type", "application/json")
	for k, v := range headers {
		req.Header.Set(k, v)
	}
	w := httptest.NewRecorder()
	r.ServeHTTP(w, req)
	return w.Code, w.Body.Bytes()
}

func TestTheInstrumentPagePathIsTheOneTheUiCalls(t *testing.T) {
	_, r, _ := setup(t)
	code, body := do(t, r, http.MethodGet, "/api/instruments/INFY.NS/analysis", nil, nil)
	if code != http.StatusOK {
		t.Fatalf("%d %s", code, body)
	}
	var v map[string]any
	if err := json.Unmarshal(body, &v); err != nil {
		t.Fatalf("not JSON: %v", err)
	}
	if _, ok := v["instrument"]; !ok {
		t.Fatalf("%v", v)
	}
	// No bars means no indicators: null, never an empty object the page treats as present.
	if v["indicators"] != nil {
		t.Fatalf("indicators without bars must be null: %v", v["indicators"])
	}
}

func TestHealth(t *testing.T) {
	_, r, _ := setup(t)
	code, _ := do(t, r, http.MethodGet, "/api/health", nil, nil)
	if code != http.StatusOK {
		t.Fatalf("%d", code)
	}
}

func TestTheDashboardCarriesTheBookAndAnswers304(t *testing.T) {
	pool, r, id := setup(t)
	if err := trades.Record(pool, id, trades.SideBuy, 10, 100, nil, nil, models.ModeLong); err != nil {
		t.Fatal(err)
	}
	pending := ids.New()
	if _, err := pool.Exec(
		`INSERT INTO signals (id, instrument_id, action, qty, entry, valid_until, reason_json,
            created_at, status, mode)
         VALUES (?, ?, 'BUY', 5, 100, '2030-01-01T00:00:00Z', '{}', ?, 'pending', 'long')`,
		pending, id, time.Now().UTC().Format(time.RFC3339)); err != nil {
		t.Fatal(err)
	}
	code, body := do(t, r, http.MethodGet, "/api/dashboard?mode=long", nil, nil)
	if code != http.StatusOK {
		t.Fatalf("%d %s", code, body)
	}
	var dash Dashboard
	if err := json.Unmarshal(body, &dash); err != nil {
		t.Fatal(err)
	}
	if len(dash.Positions) != 1 || dash.Positions[0].Qty != 10 {
		t.Fatalf("%+v", dash.Positions)
	}
	if len(dash.Signals) != 1 || dash.Signals[0].ID != pending {
		t.Fatalf("the pending decision did not come back by its uuid: %+v", dash.Signals)
	}
	if dash.Budget != 100_000 || dash.Available != 99_000 {
		t.Fatalf("budget %v available %v", dash.Budget, dash.Available)
	}

	req := httptest.NewRequest(http.MethodGet, "/api/dashboard?mode=long", nil)
	req.Header.Set("If-None-Match", "")
	w := httptest.NewRecorder()
	r.ServeHTTP(w, req)
	etag := w.Header().Get("ETag")
	if etag == "" {
		t.Fatal("no etag")
	}
	code, body = do(t, r, http.MethodGet, "/api/dashboard?mode=long", nil,
		map[string]string{"If-None-Match": etag})
	if code != http.StatusNotModified || len(body) != 0 {
		t.Fatalf("%d %q", code, body)
	}
	// An in-place change must move the tag: an edit changes no id.
	if err := trades.Record(pool, id, trades.SideBuy, 1, 100, nil, nil, models.ModeLong); err != nil {
		t.Fatal(err)
	}
	code, _ = do(t, r, http.MethodGet, "/api/dashboard?mode=long", nil,
		map[string]string{"If-None-Match": etag})
	if code != http.StatusOK {
		t.Fatalf("a changed book answered %d", code)
	}
}

func TestApprovingASignalRecordsTheFillAndItAppearsImmediately(t *testing.T) {
	pool, r, id := setup(t)
	signalUUID := ids.New()
	_, err := pool.Exec(
		`INSERT INTO signals (id, instrument_id, action, qty, entry, stop, target, valid_until,
            reason_json, created_at, status, mode)
         VALUES (?, ?, 'BUY', 10, 100, 95, 110, '2030-01-01T00:00:00Z', '{}', ?, 'pending', 'long')`,
		signalUUID, id, time.Now().UTC().Format(time.RFC3339))
	if err != nil {
		t.Fatal(err)
	}
	code, body := do(t, r, http.MethodPost, "/api/signals/"+signalUUID+"/approve",
		map[string]any{"qty": 10, "price": 100}, nil)
	if code != http.StatusOK {
		t.Fatalf("%d %s", code, body)
	}
	_, body = do(t, r, http.MethodGet, "/api/trades", nil, nil)
	var rows []trades.Trade
	if err := json.Unmarshal(body, &rows); err != nil {
		t.Fatal(err)
	}
	if len(rows) != 1 || rows[0].Qty != 10 {
		t.Fatalf("%+v", rows)
	}
	var stop sql.NullFloat64
	var target sql.NullFloat64
	if err := pool.QueryRow("SELECT stop, target FROM positions").Scan(&stop, &target); err != nil {
		t.Fatal(err)
	}
	if !stop.Valid || stop.Float64 != 95 || !target.Valid || target.Float64 != 110 {
		t.Fatalf("the exit plan did not follow the fill: %+v %+v", stop, target)
	}
}

func TestAFullySoldHoldingStaysVisibleUntilArchived(t *testing.T) {
	pool, r, id := setup(t)
	if err := trades.Record(pool, id, trades.SideBuy, 10, 100, nil, nil, models.ModeLong); err != nil {
		t.Fatal(err)
	}
	if err := trades.Record(pool, id, trades.SideSell, 10, 120, nil, nil, models.ModeLong); err != nil {
		t.Fatal(err)
	}
	_, body := do(t, r, http.MethodGet, "/api/dashboard?mode=long", nil, nil)
	var dash Dashboard
	_ = json.Unmarshal(body, &dash)
	if len(dash.Positions) != 1 || dash.Positions[0].Qty != 0 {
		t.Fatalf("the sold-out row must stay: %+v", dash.Positions)
	}
	code, body := do(t, r, http.MethodDelete, "/api/instruments/INFY.NS", nil, nil)
	if code != http.StatusOK {
		t.Fatalf("%d %s", code, body)
	}
	_, body = do(t, r, http.MethodGet, "/api/dashboard?mode=long", nil, nil)
	_ = json.Unmarshal(body, &dash)
	if len(dash.Positions) != 0 {
		t.Fatalf("archiving is what removes it: %+v", dash.Positions)
	}
	_, body = do(t, r, http.MethodGet, "/api/archived", nil, nil)
	var archived []ArchivedRow
	_ = json.Unmarshal(body, &archived)
	if len(archived) != 1 || archived[0].Symbol != "INFY.NS" {
		t.Fatalf("%+v", archived)
	}
}

func TestArchivingAHeldInstrumentIsRefused(t *testing.T) {
	pool, r, id := setup(t)
	if err := trades.Record(pool, id, trades.SideBuy, 10, 100, nil, nil, models.ModeLong); err != nil {
		t.Fatal(err)
	}
	code, body := do(t, r, http.MethodDelete, "/api/instruments/INFY.NS", nil, nil)
	if code != http.StatusBadRequest {
		t.Fatalf("%d %s", code, body)
	}
}

func TestRejectingOneLegRejectsThePlan(t *testing.T) {
	pool, r, id := setup(t)
	insert := func(action string, planID string) (string, string) {
		uid := ids.New()
		_, err := pool.Exec(
			`INSERT INTO signals (id, instrument_id, action, qty, entry, valid_until, reason_json,
                created_at, status, mode, plan_id)
             VALUES (?, ?, ?, 1, 100, '2030-01-01T00:00:00Z', '{}', ?, 'pending', 'long', ?)`,
			uid, id, action, time.Now().UTC().Format(time.RFC3339), planID)
		if err != nil {
			t.Fatal(err)
		}
		return uid, uid
	}
	first, firstUUID := insert("SELL", "")
	if _, err := pool.Exec("UPDATE signals SET plan_id = ? WHERE id = ?", firstUUID, first); err != nil {
		t.Fatal(err)
	}
	_, secondUUID := insert("BUY", firstUUID)
	code, body := do(t, r, http.MethodPost, "/api/signals/"+secondUUID+"/reject",
		map[string]any{"reason": "not today"}, nil)
	if code != http.StatusOK {
		t.Fatalf("%d %s", code, body)
	}
	var pending int
	if err := pool.QueryRow("SELECT COUNT(*) FROM signals WHERE status = 'pending'").Scan(&pending); err != nil {
		t.Fatal(err)
	}
	if pending != 0 {
		t.Fatalf("%d legs still pending", pending)
	}
}

func itoa(v int64) string {
	return fmt.Sprint(v)
}
