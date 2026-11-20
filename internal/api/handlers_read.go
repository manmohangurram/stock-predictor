package api

import (
	"database/sql"
	"encoding/json"
	"fmt"
	"log/slog"
	"net/http"
	"strings"
	"time"

	"github.com/gin-gonic/gin"

	"stock-predictor/internal/ledger"
	"stock-predictor/internal/llm"
	"stock-predictor/internal/models"
	"stock-predictor/internal/rules"
	"stock-predictor/internal/runs"
	"stock-predictor/internal/schedule"
	"stock-predictor/internal/trades"
)

type SignalRow struct {
	ID            string   `json:"id"`
	Symbol        string   `json:"symbol"`
	Name          string   `json:"name"`
	Class         string   `json:"class"`
	Action        string   `json:"action"`
	Qty           *float64 `json:"qty"`
	Confidence    *string  `json:"confidence"`
	Entry         *float64 `json:"entry"`
	Stop          *float64 `json:"stop"`
	Target        *float64 `json:"target"`
	ValidUntil    *string  `json:"valid_until"`
	TimingNote    string   `json:"timing_note"`
	Reasons       []string `json:"reasons"`
	Rationale     *string  `json:"rationale"`
	CreatedAt     string   `json:"created_at"`
	Status        string   `json:"status"`
	StatusReason  *string  `json:"status_reason"`
	FilledQty     *float64 `json:"filled_qty"`
	FilledPrice   *float64 `json:"filled_price"`
	PlanID        *string  `json:"plan_id"`
	PlanRationale *string  `json:"plan_rationale"`
	PlanScope     *string  `json:"plan_scope"`
	Amount        *float64 `json:"amount"`
}

type Dashboard struct {
	Currency       string               `json:"currency"`
	Budget         float64              `json:"budget"`
	Committed      float64              `json:"committed"`
	Available      float64              `json:"available"`
	Realised       float64              `json:"realised"`
	Mode           string               `json:"mode"`
	CurrencyLocked bool                 `json:"currency_locked"`
	Positions      []ledger.PositionRow `json:"positions"`
	Signals        []SignalRow          `json:"signals"`
	RecentTrades   []trades.Trade       `json:"recent_trades"`
}

func dashboardHandler(c *gin.Context, st *AppState) {
	mode, err := modeOf(c.Query("mode"))
	if err != nil {
		respondErr(c, err)
		return
	}
	settings, err2 := rules.GetSettings(st.Pool)
	if err2 != nil {
		respondErr(c, internalErr(err2))
		return
	}
	budget, err3 := rules.BudgetTotalIn(st.Pool, mode)
	if err3 != nil {
		respondErr(c, internalErr(err3))
		return
	}
	committed, err4 := rules.CommittedIn(st.Pool, mode)
	if err4 != nil {
		respondErr(c, internalErr(err4))
		return
	}
	positions, err5 := ledger.Positions(st.Pool, mode)
	if err5 != nil {
		respondErr(c, internalErr(err5))
		return
	}
	status := c.DefaultQuery("decisions", "pending")
	signals, err6 := loadSignals(st.Pool, mode, status)
	if err6 != nil {
		respondErr(c, err6)
		return
	}
	etag, err7 := dashboardETag(st.Pool, mode)
	if err7 != nil {
		respondErr(c, internalErr(err7))
		return
	}
	if c.GetHeader("If-None-Match") == etag {
		c.Header("ETag", etag)
		c.Status(http.StatusNotModified)
		return
	}
	recent, err8 := trades.History(st.Pool, nil, 20)
	if err8 != nil {
		respondErr(c, internalErr(err8))
		return
	}
	realised, _ := trades.RealisedIn(st.Pool, mode)
	c.Header("ETag", etag)
	c.JSON(http.StatusOK, Dashboard{
		Currency: settings.Currency, Budget: budget, Committed: committed,
		Available: budget - committed, Realised: realised, Mode: string(mode),
		CurrencyLocked: settings.CurrencyLocked, Positions: positions,
		Signals: signals, RecentTrades: recent,
	})
}

func getSettings(c *gin.Context, st *AppState) {
	s, err := rules.GetSettings(st.Pool)
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	c.JSON(http.StatusOK, s)
}

func putSettings(c *gin.Context, st *AppState) {
	var patch rules.SettingsPatch
	if err := c.ShouldBindJSON(&patch); err != nil {
		respondErr(c, badRequest("invalid body: "+err.Error()))
		return
	}
	if err := rules.SetSettings(st.Pool, patch); err != nil {
		if se, ok := err.(rules.SettingsError); ok {
			status := http.StatusBadRequest
			respondErr(c, &ApiError{Status: status, Message: se.Error()})
			return
		}
		respondErr(c, internalErr(err))
		return
	}
	slog.Info("settings updated")
	getSettings(c, st)
}

func testProxy(c *gin.Context, st *AppState) {
	rt, err := rules.LoadRuntime(st.Pool)
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	if rt.Proxy == nil || *rt.Proxy == "" {
		respondErr(c, badRequest("no proxy configured"))
		return
	}
	client, cerr := httpxClientFor(&rt)
	if cerr != nil {
		respondErr(c, badRequest(cerr.Error()))
		return
	}
	started := time.Now()
	resp, rerr := client.Get("https://api.deepseek.com")
	millis := time.Since(started).Milliseconds()
	statusValue := 0
	errText := ""
	if rerr != nil {
		errText = rerr.Error()
	} else {
		statusValue = resp.StatusCode
		resp.Body.Close()
	}
	c.JSON(http.StatusOK, gin.H{
		"ok": rerr == nil, "status": statusValue, "millis": millis, "error": nullIfEmpty(errText),
	})
}

func nullIfEmpty(s string) *string {
	if s == "" {
		return nil
	}
	return &s
}

type InstrumentRow struct {
	ID              string   `json:"id"`
	RowID           int64    `json:"-"`
	Symbol          string   `json:"symbol"`
	Class           string   `json:"class"`
	Name            string   `json:"name"`
	Currency        string   `json:"currency"`
	TimingNote      string   `json:"timing_note"`
	TargetProfitPct *float64 `json:"target_profit_pct"`
	Active          bool     `json:"active"`
}

type HeadlineRow struct {
	ID          string  `json:"id"`
	Title       string  `json:"title"`
	Publisher   *string `json:"publisher"`
	PublishedAt string  `json:"published_at"`
	URL         string  `json:"url"`
}

type BarPoint struct {
	Ts    string  `json:"ts"`
	Close float64 `json:"close"`
}

type CheckRow struct {
	Name   string `json:"name"`
	Passed bool   `json:"passed"`
	Detail string `json:"detail"`
}

type Analysis struct {
	Instrument InstrumentRow       `json:"instrument"`
	Trailing1Y *float64            `json:"trailing_1y"`
	Bars       []BarPoint          `json:"bars"`
	Indicators map[string]any      `json:"indicators"`
	Checks     []CheckRow          `json:"checks"`
	Position   *ledger.PositionRow `json:"position"`
	Headlines  []HeadlineRow       `json:"headlines"`
	Signals    []SignalRow         `json:"signals"`
	Trades     []trades.Trade      `json:"trades"`
}

func analysisHandler(c *gin.Context, st *AppState) {
	a, err := buildAnalysis(c, st, c.Param("symbol"), true)
	if err != nil {
		respondErr(c, err)
		return
	}
	c.JSON(http.StatusOK, a)
}

func instrumentDetail(c *gin.Context, st *AppState) {
	a, err := buildAnalysis(c, st, c.Param("symbol"), false)
	if err != nil {
		respondErr(c, err)
		return
	}
	c.JSON(http.StatusOK, gin.H{
		"instrument": a.Instrument, "headlines": a.Headlines, "signals": a.Signals,
	})
}

func buildAnalysis(c *gin.Context, st *AppState, symbol string, full bool) (*Analysis, error) {
	var inst InstrumentRow
	var targetProfit sql.NullFloat64
	var archived sql.NullString
	err := st.Pool.QueryRow(
		`SELECT uuid, id, symbol, class, name, currency, target_profit_pct, active, archived_at
         FROM instruments WHERE symbol = ?`, symbol,
	).Scan(&inst.ID, &inst.RowID, &inst.Symbol, &inst.Class, &inst.Name, &inst.Currency,
		&targetProfit, new(int), &archived)
	if err == sql.ErrNoRows {
		return nil, notFound("no instrument " + symbol)
	}
	if err != nil {
		return nil, internalErr(err)
	}
	class, _ := models.ParseAssetClass(inst.Class)
	inst.TimingNote = class.TimingNote()
	inst.Active = !archived.Valid
	if targetProfit.Valid {
		inst.TargetProfitPct = &targetProfit.Float64
	}

	bars, err := loadBarsFor(st.Pool, inst.RowID, 400)
	if err != nil {
		return nil, internalErr(err)
	}
	headlines, err := loadHeadlines(st.Pool, inst.RowID, 10)
	if err != nil {
		return nil, internalErr(err)
	}
	allSignals := []SignalRow{}
	for _, mode := range []models.Mode{models.ModeLong, models.ModeTrade} {
		rows, err := loadSignals(st.Pool, mode, "everything")
		if err != nil {
			return nil, err
		}
		for _, s := range rows {
			if s.Symbol == symbol {
				allSignals = append(allSignals, s)
			}
		}
	}
	position, err := ledger.OpenPositionFor(st.Pool, symbol)
	if err != nil {
		return nil, internalErr(err)
	}
	out := &Analysis{
		Instrument: inst, Headlines: headlines, Signals: allSignals, Position: position,
		Bars: []BarPoint{}, Checks: []CheckRow{},
	}
	if !full {
		return out, nil
	}
	out.Trailing1Y = trailingReturn(bars, 365)
	ind, indErr := rules.ComputeIndicators(bars)
	if indErr == nil {
		out.Indicators = map[string]any{
			"sma50": ind.SMA50, "sma200": ind.SMA200, "rsi14": ind.RSI14,
			"atr14": ind.ATR14, "high52": ind.High52, "low52": ind.Low52,
		}
		var modelPos *models.Position
		if position != nil {
			modelPos = &models.Position{
				ID: position.ID, InstrumentID: inst.RowID, Qty: position.Qty,
				AvgCost: position.AvgCost, Stop: position.Stop, Target: position.Target,
				State: position.State,
			}
		}
		available, _ := rules.Available(st.Pool)
		checks := rules.Diagnose(rules.CandidateInput{
			Instrument: &models.Instrument{
				ID: inst.RowID, Symbol: inst.Symbol, Class: class, Name: inst.Name,
				Currency: inst.Currency, Active: true,
			},
			Bars: bars, Ind: &ind, Position: modelPos, AvailableCash: available,
			Cfg: rules.DefaultRuleConfig(),
		})
		for _, ch := range checks {
			out.Checks = append(out.Checks, CheckRow{Name: ch.Name, Passed: ch.Passed, Detail: ch.Detail})
		}
	} else {
		out.Checks = append(out.Checks, CheckRow{
			Name: "price data", Passed: false,
			Detail: fmt.Sprintf("%d stored bars — the rules need %d", len(bars), rules.MinBars),
		})
	}
	start := len(bars) - 120
	if start < 0 {
		start = 0
	}
	for _, b := range bars[start:] {
		out.Bars = append(out.Bars, BarPoint{Ts: b.Ts, Close: b.Close})
	}
	tradeRows, err := trades.History(st.Pool, &inst.RowID, 200)
	if err != nil {
		return nil, internalErr(err)
	}
	out.Trades = tradeRows
	return out, nil
}

func trailingReturn(bars []models.Bar, days int) *float64 {
	if len(bars) < 2 {
		return nil
	}
	last, err := time.Parse(time.RFC3339, bars[len(bars)-1].Ts)
	if err != nil {
		return nil
	}
	cutoff := last.AddDate(0, 0, -days)
	var base *models.Bar
	for i := range bars {
		if ts, err := time.Parse(time.RFC3339, bars[i].Ts); err == nil && !ts.Before(cutoff) {
			base = &bars[i]
			break
		}
	}
	if base == nil || base.Close <= 0 {
		return nil
	}
	value := (bars[len(bars)-1].Close - base.Close) / base.Close * 100
	return &value
}

func loadBarsFor(pool *sql.DB, instrumentID int64, limit int) ([]models.Bar, error) {
	rows, err := pool.Query(
		`SELECT ts, COALESCE(open, close), COALESCE(high, close), COALESCE(low, close), close,
                COALESCE(volume, 0)
         FROM bars WHERE instrument_id = ? ORDER BY ts DESC LIMIT ?`, instrumentID, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	bars := []models.Bar{}
	for rows.Next() {
		var b models.Bar
		if err := rows.Scan(&b.Ts, &b.Open, &b.High, &b.Low, &b.Close, &b.Volume); err != nil {
			return nil, err
		}
		bars = append(bars, b)
	}
	for i, j := 0, len(bars)-1; i < j; i, j = i+1, j-1 {
		bars[i], bars[j] = bars[j], bars[i]
	}
	return bars, rows.Err()
}

func loadHeadlines(pool *sql.DB, instrumentID int64, limit int) ([]HeadlineRow, error) {
	rows, err := pool.Query(
		`SELECT id, title, publisher, published_at, url FROM news
         WHERE instrument_id = ? ORDER BY published_at DESC LIMIT ?`, instrumentID, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []HeadlineRow{}
	for rows.Next() {
		var h HeadlineRow
		var publisher sql.NullString
		if err := rows.Scan(&h.ID, &h.Title, &publisher, &h.PublishedAt, &h.URL); err != nil {
			return nil, err
		}
		if publisher.Valid {
			h.Publisher = &publisher.String
		}
		out = append(out, h)
	}
	return out, rows.Err()
}

func loadSignals(pool *sql.DB, mode models.Mode, status string) ([]SignalRow, error) {
	var predicate string
	switch status {
	case "everything":
		predicate = "1 = 1"
	case "all":
		predicate = "s.action <> 'BLOCKED'"
	case "approved":
		predicate = "s.action <> 'BLOCKED' AND s.status = 'approved'"
	case "rejected":
		predicate = "s.action <> 'BLOCKED' AND s.status = 'rejected'"
	default:
		predicate = "s.action <> 'BLOCKED' AND s.status = 'pending'"
	}
	rows, err := pool.Query(
		`SELECT s.uuid, i.symbol, i.name, i.class, s.action, s.qty, s.confidence, s.entry, s.stop,
                s.target, s.valid_until, s.reason_json, s.created_at, s.status, s.status_reason,
                s.filled_qty, s.filled_price, (SELECT p.uuid FROM signals p WHERE p.id = s.plan_id)
         FROM signals s JOIN instruments i ON i.id = s.instrument_id
         WHERE `+predicate+` AND s.mode = ? ORDER BY s.id DESC LIMIT 50`, string(mode))
	if err != nil {
		return nil, internalErr(err)
	}
	defer rows.Close()
	out := []SignalRow{}
	for rows.Next() {
		var r SignalRow
		var classRaw, action, createdAt, statusValue string
		var qty, entry, stop, target, filledQty, filledPrice sql.NullFloat64
		var confidence, validUntil, reasonJSON, statusReason sql.NullString
		var planID sql.NullString
		if err := rows.Scan(&r.ID, &r.Symbol, &r.Name, &classRaw, &action, &qty, &confidence,
			&entry, &stop, &target, &validUntil, &reasonJSON, &createdAt, &statusValue,
			&statusReason, &filledQty, &filledPrice, &planID); err != nil {
			return nil, internalErr(err)
		}
		class, _ := models.ParseAssetClass(classRaw)
		reason := map[string]any{}
		if reasonJSON.Valid {
			_ = json.Unmarshal([]byte(reasonJSON.String), &reason)
		}
		r.Class = string(class)
		r.Action = action
		r.TimingNote = class.TimingNote()
		r.CreatedAt = createdAt
		r.Status = statusValue
		if qty.Valid {
			r.Qty = &qty.Float64
		}
		if entry.Valid {
			r.Entry = &entry.Float64
		}
		if stop.Valid {
			r.Stop = &stop.Float64
		}
		if target.Valid {
			r.Target = &target.Float64
		}
		if filledQty.Valid {
			r.FilledQty = &filledQty.Float64
		}
		if filledPrice.Valid {
			r.FilledPrice = &filledPrice.Float64
		}
		if confidence.Valid {
			r.Confidence = &confidence.String
		}
		if validUntil.Valid {
			r.ValidUntil = &validUntil.String
		}
		if statusReason.Valid {
			r.StatusReason = &statusReason.String
		}
		if planID.Valid {
			r.PlanID = &planID.String
		}
		r.Reasons = []string{}
		if list, ok := reason["reasons"].([]any); ok {
			for _, v := range list {
				if s, ok := v.(string); ok {
					r.Reasons = append(r.Reasons, s)
				}
			}
		}
		if detail, ok := reason["detail"].(string); ok {
			r.Reasons = append(r.Reasons, detail)
		}
		if blocked, ok := reason["blocked"].(string); ok {
			cost, hasCost := reason["cost"].(float64)
			available, hasAvail := reason["available"].(float64)
			if hasCost && hasAvail {
				short := cost - available
				if short < 0 {
					short = 0
				}
				r.Reasons = append(r.Reasons, fmt.Sprintf(
					"%s: cost %.2f, available %.2f — short by %.2f", blocked, cost, available, short))
			}
		}
		if v, ok := reason["rationale"].(string); ok {
			r.Rationale = &v
		}
		if v, ok := reason["plan_rationale"].(string); ok {
			r.PlanRationale = &v
		}
		if v, ok := reason["scope"].(string); ok {
			r.PlanScope = &v
		}
		if v, ok := reason["amount"].(float64); ok {
			r.Amount = &v
		}
		out = append(out, r)
	}
	return out, rows.Err()
}

type ArchivedRow struct {
	ID          string  `json:"id"`
	Symbol      string  `json:"symbol"`
	Name        string  `json:"name"`
	Class       string  `json:"class"`
	Fills       int64   `json:"fills"`
	Decisions   int64   `json:"decisions"`
	LastFill    *string `json:"last_fill"`
	Realised    float64 `json:"realised"`
	NetInvested float64 `json:"net_invested"`
}

func archivedInstruments(c *gin.Context, st *AppState) {
	rows, err := st.Pool.Query(
		`SELECT i.uuid, i.symbol, i.name, i.class,
                (SELECT COUNT(*) FROM trades t WHERE t.instrument_id = i.id) AS fills,
                (SELECT COUNT(*) FROM signals s WHERE s.instrument_id = i.id) AS decisions,
                (SELECT MAX(t.traded_at) FROM trades t WHERE t.instrument_id = i.id) AS last_fill,
                (SELECT COALESCE(SUM(t.realised), 0) FROM trades t
                  WHERE t.instrument_id = i.id AND t.side = 'sell') AS realised,
                (SELECT COALESCE(SUM(CASE WHEN t.side = 'buy' THEN t.qty * t.price
                                          ELSE -t.qty * t.price END), 0)
                   FROM trades t WHERE t.instrument_id = i.id) AS net_invested
         FROM instruments i
         WHERE i.archived_at IS NOT NULL
         ORDER BY COALESCE(last_fill, i.archived_at) DESC`)
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	defer rows.Close()
	out := []ArchivedRow{}
	for rows.Next() {
		var r ArchivedRow
		var last sql.NullString
		if err := rows.Scan(&r.ID, &r.Symbol, &r.Name, &r.Class, &r.Fills, &r.Decisions,
			&last, &r.Realised, &r.NetInvested); err != nil {
			respondErr(c, internalErr(err))
			return
		}
		if last.Valid {
			r.LastFill = &last.String
		}
		out = append(out, r)
	}
	c.JSON(http.StatusOK, out)
}

func allTrades(c *gin.Context, st *AppState) {
	rows, err := trades.History(st.Pool, nil, 200)
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	c.JSON(http.StatusOK, rows)
}

func instrumentTrades(c *gin.Context, st *AppState) {
	id, err := instrumentIDBySymbol(st.Pool, c.Param("symbol"))
	if err != nil {
		respondErr(c, err)
		return
	}
	rows, err2 := trades.History(st.Pool, &id, 200)
	if err2 != nil {
		respondErr(c, internalErr(err2))
		return
	}
	c.JSON(http.StatusOK, rows)
}

func listRuns(c *gin.Context, st *AppState) {
	limit := int64(50)
	if raw := c.Query("limit"); raw != "" {
		if v, err := strconvParse(raw); err == nil {
			if v < 1 {
				v = 1
			}
			if v > 200 {
				v = 200
			}
			limit = v
		}
	}
	var rows []runs.Run
	var err error
	if c.Query("latest") == "true" {
		rows, err = runs.LatestByPrompt(st.Pool)
	} else {
		rows, err = runs.Recent(st.Pool, limit)
	}
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	running, _ := runs.Running(st.Pool)
	c.JSON(http.StatusOK, gin.H{"runs": rows, "running": running})
}

func strconvParse(raw string) (int64, error) {
	var v int64
	_, err := fmt.Sscanf(strings.TrimSpace(raw), "%d", &v)
	return v, err
}

var _ = schedule.ISTDay
var _ = llm.DefaultCallGate
