package api

import (
	"database/sql"
	"encoding/json"
	"fmt"
	"log/slog"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/gin-gonic/gin"

	"stock-predictor/internal/fundscan"
	"stock-predictor/internal/httpx"
	"stock-predictor/internal/ids"
	"stock-predictor/internal/market"
	"stock-predictor/internal/models"
	"stock-predictor/internal/rules"
	"stock-predictor/internal/runs"
	"stock-predictor/internal/schedule"
	"stock-predictor/internal/store"
	"stock-predictor/internal/tstamp"
)

type HitRow struct {
	Code  string   `json:"code"`
	Name  string   `json:"name"`
	Class string   `json:"class"`
	Venue string   `json:"venue"`
	Price *float64 `json:"price"`
}

func searchInstruments(c *gin.Context, st *AppState) {
	query := strings.TrimSpace(c.Query("q"))
	if len(query) < 2 {
		c.JSON(http.StatusOK, []HitRow{})
		return
	}
	rt, err := rules.LoadRuntime(st.Pool)
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	client, cerr := market.YahooClient(&rt)
	if cerr != nil {
		respondErr(c, internalErr(cerr))
		return
	}
	var amfiText *string
	if text := st.AMFIText(); text != nil {
		amfiText = text
	}
	hits, serr := market.Search(client, query, amfiText)
	if serr != nil {
		slog.Warn("instrument search unavailable", "error", serr)
	}
	slog.Debug("instrument search", "query", query, "hits", len(hits), "amfi", amfiText != nil)
	rows := make([]HitRow, 0, len(hits))
	for _, h := range hits {
		rows = append(rows, HitRow{
			Code: h.Code, Name: h.Name, Class: string(h.Class), Venue: h.Venue, Price: h.Price,
		})
	}
	c.JSON(http.StatusOK, rows)
}

func addInstrument(c *gin.Context, st *AppState) {
	var body struct {
		Code  string  `json:"code"`
		Name  *string `json:"name"`
		Class *string `json:"class"`
	}
	if err := c.ShouldBindJSON(&body); err != nil {
		respondErr(c, badRequest("invalid body"))
		return
	}
	code := models.NormalizeSymbol(body.Code)
	if code == "" {
		respondErr(c, badRequest("code is required"))
		return
	}
	indianListing := strings.HasSuffix(code, ".NS") || strings.HasSuffix(code, ".BO") ||
		(body.Class != nil && *body.Class == "mutual_fund")
	if !indianListing {
		respondErr(c, badRequest(fmt.Sprintf(
			"%s is not an Indian listing — add an NSE (.NS) or BSE (.BO) symbol, or a fund by its AMFI code",
			code)))
		return
	}
	rt, err := rules.LoadRuntime(st.Pool)
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	client, cerr := market.YahooClient(&rt)
	if cerr != nil {
		respondErr(c, internalErr(cerr))
		return
	}

	class := models.ClassStock
	name := code
	if body.Class != nil && *body.Class == "mutual_fund" {
		class = models.ClassMutualFund
		var hit *market.Hit
		if text := st.AMFIText(); text != nil {
			hit = market.AMFIByCode(*text, code)
		}
		if hit != nil {
			name = hit.Name
		} else {
			known, _ := market.Search(client, code, nil)
			found := false
			for _, h := range known {
				if strings.EqualFold(h.Code, code) {
					name = h.Name
					found = true
					break
				}
			}
			if !found {
				respondErr(c, badRequest(fmt.Sprintf(
					"%s is not in the AMFI scheme list and is not a Yahoo symbol — search for the fund by name and pick a suggestion",
					code)))
				return
			}
		}
	} else if body.Name != nil {
		name = *body.Name
	}

	id, uid, aerr := addOrPromote(st.Pool, code, string(class), name)
	if aerr != nil {
		respondErr(c, aerr)
		return
	}
	// A failed refresh must not fail the add — the instrument exists either way.
	if bars, news, rerr := refreshAny(st.Pool, id, code, string(class), &rt, true); rerr != nil {
		slog.Warn("add-time refresh failed", "symbol", code, "error", rerr)
	} else {
		slog.Info("instrument added", "symbol", code, "class", class, "id", id,
			"bars", bars, "news", news)
	}
	c.JSON(http.StatusOK, InstrumentRow{
		ID: uid, Symbol: code, Class: string(class), Name: name, Currency: "INR",
		TimingNote: class.TimingNote(), Active: true,
	})
}

// Adding is an act of intent, so it makes the row active — including a row the short-term scan
// already created as a candidate. A genuinely archived instrument is not revived by it.
func addOrPromote(pool *sql.DB, code, class, name string) (int64, string, error) {
	var archived sql.NullString
	err := pool.QueryRow("SELECT archived_at FROM instruments WHERE symbol = ?", code).Scan(&archived)
	if err == nil && archived.Valid {
		return 0, "", badRequest(code + " is archived — restore it from its own page")
	}
	if err != nil && err != sql.ErrNoRows {
		return 0, "", internalErr(err)
	}
	if _, err := pool.Exec(
		`INSERT INTO instruments (symbol, class, name, currency, active, uuid)
         VALUES (?, ?, ?, 'INR', 1, ?)
         ON CONFLICT(symbol) DO UPDATE SET active = 1`, code, class, name, ids.New()); err != nil {
		return 0, "", internalErr(err)
	}
	var id int64
	var uuid string
	if err := pool.QueryRow("SELECT id, uuid FROM instruments WHERE symbol = ?", code).Scan(&id, &uuid); err != nil {
		return 0, "", internalErr(err)
	}
	return id, uuid, nil
}

func refreshInstrument(c *gin.Context, st *AppState) {
	symbol := c.Param("symbol")
	var id int64
	var classRaw string
	var active int64
	err := st.Pool.QueryRow("SELECT id, class, active FROM instruments WHERE symbol = ?", symbol).
		Scan(&id, &classRaw, &active)
	if err == sql.ErrNoRows {
		respondErr(c, notFound("no instrument "+symbol))
		return
	}
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	rt, err := rules.LoadRuntime(st.Pool)
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	bars, news, rerr := refreshAny(st.Pool, id, symbol, classRaw, &rt, active != 0)
	if rerr != nil {
		respondErr(c, badRequest(fmt.Sprintf("%s (%s): %v", symbol, classRaw, rerr)))
		return
	}
	slog.Info("instrument refreshed", "symbol", symbol, "bars", bars, "news", news,
		"cached", active != 0)
	c.JSON(http.StatusOK, gin.H{"bars": bars, "headlines": news, "cached": active != 0})
}

// refreshAny fetches prices and headlines. An archived book is not priced, and a read must not
// rebuild what archiving cleared.
func refreshAny(pool *sql.DB, id int64, symbol, class string, rt *rules.Runtime, cache bool) (int, int, error) {
	if class == string(models.ClassMutualFund) {
		bars, err := refreshFund(pool, id, symbol, rt, cache)
		if err == nil {
			return bars, 0, nil
		}
		if !strings.Contains(symbol, ".") {
			return 0, 0, err
		}
	}
	source, err := market.NewYahooSource(rt)
	if err != nil {
		return 0, 0, err
	}
	if !cache {
		fetched, ferr := source.Bars(symbol)
		if ferr != nil {
			return 0, 0, ferr
		}
		return len(fetched), 0, nil
	}
	bars, news, err := store.Refresh(pool, id, symbol, source)
	if err == nil {
		// A forced read must not leave the day-long cache holding the old history.
		market.DropDailyCache(symbol)
	}
	return bars, news, err
}

func refreshFund(pool *sql.DB, id int64, symbol string, rt *rules.Runtime, cache bool) (int, error) {
	code := ""
	if symbol != "" && allDigits(symbol) {
		code = symbol
	} else {
		var stored sql.NullString
		_ = pool.QueryRow(
			`SELECT json_extract(reason_json, '$.scheme_code') FROM signals
             WHERE instrument_id = ? AND json_extract(reason_json, '$.scheme_code') IS NOT NULL
             ORDER BY id DESC LIMIT 1`, id).Scan(&stored)
		if !stored.Valid || stored.String == "" {
			return 0, fmt.Errorf("no AMFI scheme code — add this fund by its AMFI code instead")
		}
		code = stored.String
	}
	client, err := httpx.ClientWith(rt, 20, "Mozilla/5.0 stock-predictor")
	if err != nil {
		return 0, err
	}
	history, err := market.FetchFundHistory(client, code)
	if err != nil {
		return 0, err
	}
	bars := fundscan.NAVBars(history)
	if cache {
		if err := store.Bars(pool, id, bars); err != nil {
			return 0, err
		}
	}
	return len(bars), nil
}

func allDigits(s string) bool {
	for _, r := range s {
		if r < '0' || r > '9' {
			return false
		}
	}
	return s != ""
}

type callRow struct {
	ID               string  `json:"id"`
	Ts               string  `json:"ts"`
	Endpoint         string  `json:"endpoint"`
	DecisionAction   *string `json:"decision_action"`
	DecisionReason   *string `json:"decision_reason"`
	InstrumentName   *string `json:"instrument_name"`
	InstrumentSymbol *string `json:"instrument_symbol"`
	PlanLegs         int64   `json:"plan_legs"`
	Model            *string `json:"model"`
	Trigger          *string `json:"trigger"`
	DecisionID       *string `json:"decision_id"`
	DecisionStatus   *string `json:"decision_status"`
	Purpose          *string `json:"purpose"`
	Round            *int64  `json:"round"`
	PromptTokens     *int64  `json:"prompt_tokens"`
	CompletionTokens *int64  `json:"completion_tokens"`
	ValidatorVerdict *string `json:"validator_verdict"`
	Error            *string `json:"error"`
}

func calls(c *gin.Context, st *AppState) {
	limit := int64(25)
	if raw := c.Query("limit"); raw != "" {
		if v, err := strconv.ParseInt(raw, 10, 64); err == nil {
			limit = v
		}
	} else if raw := c.Query("per_page"); raw != "" {
		if v, err := strconv.ParseInt(raw, 10, 64); err == nil {
			limit = v
		}
	}
	if limit < 1 {
		limit = 1
	}
	if limit > 200 {
		limit = 200
	}
	offset := int64(0)
	if raw := c.Query("offset"); raw != "" {
		offset, _ = strconv.ParseInt(raw, 10, 64)
	} else {
		page := int64(1)
		if raw := c.Query("page"); raw != "" {
			page, _ = strconv.ParseInt(raw, 10, 64)
		}
		if page < 1 {
			page = 1
		}
		offset = (page - 1) * limit
	}

	given := func(key string) *string {
		v := strings.TrimSpace(c.Query(key))
		if v == "" {
			return nil
		}
		return &v
	}
	filters := []struct {
		sql  string
		bind *string
	}{}
	if v := given("prompt"); v != nil {
		filters = append(filters, struct {
			sql  string
			bind *string
		}{" AND (CASE WHEN instr(a.purpose, ' ') > 0 THEN substr(a.purpose, 1, instr(a.purpose, ' ') - 1) ELSE a.purpose END) = ?", v})
	}
	if v := given("call"); v != nil {
		filters = append(filters, struct {
			sql  string
			bind *string
		}{" AND a.endpoint = ?", v})
	}
	if v := given("model"); v != nil {
		filters = append(filters, struct {
			sql  string
			bind *string
		}{" AND a.prompt_template = ?", v})
	}
	switch v := given("review"); {
	case v == nil:
	case *v == "none":
		filters = append(filters, struct {
			sql  string
			bind *string
		}{" AND d.id IS NULL", nil})
	case *v == "blocked":
		filters = append(filters, struct {
			sql  string
			bind *string
		}{" AND d.action = 'BLOCKED'", nil})
	default:
		filters = append(filters, struct {
			sql  string
			bind *string
		}{" AND d.status = ?", v})
	}

	const joins = ` FROM api_calls a
         LEFT JOIN signals d ON d.id = (
             SELECT s.id FROM signals s WHERE s.conversation_id = a.conversation_id
             ORDER BY s.id DESC LIMIT 1)`

	// The WHERE 1 = 1 anchor is load-bearing: without it the first filter lands in the JOIN's ON.
	args := []any{}
	var qb strings.Builder
	qb.WriteString("SELECT COUNT(*)" + joins + " WHERE 1 = 1")
	for _, f := range filters {
		qb.WriteString(f.sql)
		if f.bind != nil {
			args = append(args, *f.bind)
		}
	}
	var total int64
	if err := st.Pool.QueryRow(qb.String(), args...).Scan(&total); err != nil {
		respondErr(c, internalErr(err))
		return
	}

	pageArgs := append([]any{}, args...)
	var page strings.Builder
	page.WriteString(`SELECT a.uuid AS id, a.ts AS ts, a.endpoint AS endpoint, a.purpose AS purpose,
                a.round AS round, a.prompt_tokens AS prompt_tokens,
                a.completion_tokens AS completion_tokens,
                a.validator_verdict AS validator_verdict, a.error AS error,
                a.trigger AS trigger, a.prompt_template AS prompt_template,
                d.uuid AS decision_id, d.status AS decision_status,
                d.action AS decision_action, d.reason_json AS decision_reason,
                COALESCE(i.name, pi.name) AS instrument_name,
                COALESCE(i.symbol, pi.symbol) AS instrument_symbol,
                (SELECT COUNT(*) FROM signals p
                  WHERE p.conversation_id = a.conversation_id AND p.kind = 'plan') AS plan_legs` + joins + `
         LEFT JOIN instruments i ON i.id = d.instrument_id
         LEFT JOIN instruments pi ON pi.symbol = trim(substr(a.purpose, instr(a.purpose, ' ') + 1))
         WHERE 1 = 1`)
	for _, f := range filters {
		page.WriteString(f.sql)
	}
	page.WriteString(" ORDER BY a.id DESC LIMIT ? OFFSET ?")
	pageArgs = append(pageArgs, limit, offset)

	rows, err := st.Pool.Query(page.String(), pageArgs...)
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	defer rows.Close()
	out := []callRow{}
	for rows.Next() {
		var r callRow
		var purpose, trigger, template, verdict, errText, decisionStatus sql.NullString
		var decisionAction, decisionReason, instrumentName, instrumentSymbol sql.NullString
		var round, promptTokens, completionTokens sql.NullInt64
		var decisionID sql.NullString
		var planLegs int64
		if err := rows.Scan(&r.ID, &r.Ts, &r.Endpoint, &purpose, &round, &promptTokens,
			&completionTokens, &verdict, &errText, &trigger, &template, &decisionID,
			&decisionStatus, &decisionAction, &decisionReason, &instrumentName,
			&instrumentSymbol, &planLegs); err != nil {
			respondErr(c, internalErr(err))
			return
		}
		r.Purpose = nullIfEmpty(purpose.String)
		r.Trigger = nullIfEmpty(trigger.String)
		r.Model = nullIfEmpty(template.String)
		r.ValidatorVerdict = nullIfEmpty(verdict.String)
		r.Error = nullIfEmpty(errText.String)
		r.DecisionStatus = nullIfEmpty(decisionStatus.String)
		r.DecisionReason = blockedReason(decisionReason.String)
		if round.Valid {
			r.Round = &round.Int64
		}
		if promptTokens.Valid {
			r.PromptTokens = &promptTokens.Int64
		}
		if completionTokens.Valid {
			r.CompletionTokens = &completionTokens.Int64
		}
		if decisionID.Valid {
			r.DecisionID = &decisionID.String
		}
		r.PlanLegs = planLegs
		if planLegs > 0 && (!decisionAction.Valid || decisionAction.String != "BLOCKED") {
			plan := "PLAN"
			r.DecisionAction = &plan
		} else if decisionAction.Valid {
			v := decisionAction.String
			r.DecisionAction = &v
		}
		if planLegs == 0 {
			r.InstrumentName = nullIfEmpty(instrumentName.String)
			r.InstrumentSymbol = nullIfEmpty(instrumentSymbol.String)
		}
		out = append(out, r)
	}
	if err := rows.Err(); err != nil {
		respondErr(c, internalErr(err))
		return
	}
	c.Header("x-total-count", strconv.FormatInt(total, 10))
	c.Header("x-offset", strconv.FormatInt(offset, 10))
	c.Header("x-limit", strconv.FormatInt(limit, 10))
	c.JSON(http.StatusOK, out)
}

func blockedReason(raw string) *string {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return nil
	}
	var v map[string]any
	if err := jsonUnmarshal([]byte(raw), &v); err != nil {
		return nil
	}
	kind, _ := v["blocked"].(string)
	if kind == "" {
		return nil
	}
	if detail, ok := v["detail"].(string); ok && detail != "" {
		s := kind + ": " + detail
		return &s
	}
	if reasons, ok := v["reasons"].([]any); ok && len(reasons) > 0 {
		if first, ok := reasons[0].(string); ok {
			return &first
		}
	}
	cost, hasCost := v["cost"].(float64)
	available, hasAvail := v["available"].(float64)
	var s string
	if hasCost && hasAvail {
		s = fmt.Sprintf("%s: cost %.2f, available %.2f", kind, cost, available)
	} else {
		s = kind
	}
	return &s
}

func callDetail(c *gin.Context, st *AppState) {
	id, err := pathRowID(st.Pool, c, "api_calls")
	if err != nil {
		respondErr(c, err)
		return
	}
	var ts, endpoint string
	var purpose, trigger, template, requestJSON, responseJSON, verdict, errText, decisionReason sql.NullString
	var instrumentName, instrumentSymbol sql.NullString
	var round, promptTokens, completionTokens sql.NullInt64
	var planLegs int64
	err = st.Pool.QueryRow(
		`SELECT a.id, a.ts, a.endpoint, a.purpose, a.round, a.trigger, a.prompt_template,
                a.request_json, a.response_json, a.prompt_tokens, a.completion_tokens,
                a.validator_verdict, a.error, d.reason_json AS decision_reason,
                COALESCE(i.name, pi.name) AS instrument_name,
                COALESCE(i.symbol, pi.symbol) AS instrument_symbol,
                (SELECT COUNT(*) FROM signals p
                  WHERE p.conversation_id = a.conversation_id AND p.kind = 'plan') AS plan_legs
         FROM api_calls a
         LEFT JOIN signals d ON d.id = (
             SELECT s.id FROM signals s WHERE s.conversation_id = a.conversation_id
             ORDER BY s.id DESC LIMIT 1)
         LEFT JOIN instruments i ON i.id = d.instrument_id
         LEFT JOIN instruments pi ON pi.symbol = trim(substr(a.purpose, instr(a.purpose, ' ') + 1))
         WHERE a.id = ?`, id,
	).Scan(&id, &ts, &endpoint, &purpose, &round, &trigger, &template, &requestJSON,
		&responseJSON, &promptTokens, &completionTokens, &verdict, &errText, &decisionReason,
		&instrumentName, &instrumentSymbol, &planLegs)
	if err == sql.ErrNoRows {
		respondErr(c, notFound(fmt.Sprintf("no call %d", id)))
		return
	}
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	names := map[string]string{}
	rows, err := st.Pool.Query(
		`SELECT i.symbol, i.name FROM signals s JOIN instruments i ON i.id = s.instrument_id
         WHERE s.conversation_id = (SELECT conversation_id FROM api_calls WHERE id = ?)
           AND s.kind = 'plan'`, id)
	if err == nil {
		defer rows.Close()
		for rows.Next() {
			var symbol, name string
			if rows.Scan(&symbol, &name) == nil {
				names[symbol] = name
			}
		}
	}
	out := gin.H{
		"id": c.Param("id"), "ts": ts, "endpoint": endpoint, "purpose": nullIfEmpty(purpose.String),
		"round": nullInt(round), "trigger": nullIfEmpty(trigger.String),
		"decision_reason":   blockedReason(decisionReason.String),
		"instrument_name":   nullIfEmpty(instrumentName.String),
		"instrument_symbol": nullIfEmpty(instrumentSymbol.String),
		"prompt_template":   nullIfEmpty(template.String),
		"request_json":      nullIfEmpty(requestJSON.String),
		"response_json":     nullIfEmpty(responseJSON.String),
		"prompt_tokens":     nullInt(promptTokens), "completion_tokens": nullInt(completionTokens),
		"validator_verdict": nullIfEmpty(verdict.String), "error": nullIfEmpty(errText.String),
		"plan_names": names,
	}
	c.JSON(http.StatusOK, out)
}

func nullInt(v sql.NullInt64) *int64 {
	if v.Valid {
		return &v.Int64
	}
	return nil
}

func callFacets(c *gin.Context, st *AppState) {
	distinct := func(q string) []string {
		rows, err := st.Pool.Query(q)
		if err != nil {
			return []string{}
		}
		defer rows.Close()
		out := []string{}
		for rows.Next() {
			var v sql.NullString
			if rows.Scan(&v) == nil && v.Valid {
				out = append(out, v.String)
			}
		}
		return out
	}
	c.JSON(http.StatusOK, gin.H{
		"prompts": distinct(`SELECT DISTINCT trim(substr(purpose, 1, instr(purpose, ' ') - 1)) AS v
             FROM api_calls WHERE purpose IS NOT NULL AND instr(purpose, ' ') > 0 ORDER BY v`),
		"calls": distinct("SELECT DISTINCT endpoint FROM api_calls ORDER BY endpoint"),
		"models": distinct(`SELECT DISTINCT prompt_template FROM api_calls
             WHERE prompt_template IS NOT NULL ORDER BY prompt_template`),
	})
}

func marketState(c *gin.Context, st *AppState) {
	now := time.Now().UTC()
	ist := now.Add(330 * time.Minute)
	weekday := ist.Weekday() != time.Saturday && ist.Weekday() != time.Sunday
	minutes := ist.Hour()*60 + ist.Minute()

	// Only a traded price says the exchange is working; a fund NAV is published after the close.
	var latest sql.NullString
	_ = st.Pool.QueryRow(
		`SELECT MAX(b.ts) FROM bars b JOIN instruments i ON i.id = b.instrument_id
         WHERE i.class = 'stock'`).Scan(&latest)
	pricedToday := false
	if latest.Valid {
		if d, err := time.Parse(time.RFC3339, latest.String); err == nil {
			istDay := d.Add(330 * time.Minute).Format("2006-01-02")
			pricedToday = istDay == ist.Format("2006-01-02")
		}
	}
	state, reason := marketStateFor(weekday, minutes, pricedToday)
	nextOpen := nextOpenAt(now, ist)
	var closesAt *string
	if state == "open" {
		at := time.Date(ist.Year(), ist.Month(), ist.Day(), 15, 30, 0, 0,
			time.FixedZone("IST", 330*60)).UTC()
		s := tstamp.From(at)
		closesAt = &s
	}
	c.JSON(http.StatusOK, gin.H{
		"state": state, "reason": reason, "as_of": tstamp.From(now),
		"session":      "09:15–15:30 IST, Mon–Fri",
		"last_bar_at":  nullIfEmpty(latest.String),
		"next_open_at": nextOpen, "closes_at": closesAt, "holiday_calendar": false,
	})
}

func marketStateFor(weekday bool, minutes int, pricedToday bool) (string, string) {
	if !weekday {
		return "closed", "Weekend"
	}
	if minutes < 9*60+15 || minutes > 15*60+30 {
		if minutes < 9*60+15 {
			return "closed", "Before the open"
		}
		return "closed", "After the close"
	}
	if pricedToday {
		return "open", "Trading"
	}
	// A daily candle is not published until the close, so the first price of a session never lands
	// at 09:15. Only after the grace period is "nothing has priced" evidence of a holiday.
	if minutes < 9*60+45 {
		return "open", "Trading — the first price has not landed yet"
	}
	return "holiday", "No prices recorded today — the exchange is probably closed for a holiday"
}

func nextOpenAt(now, istNow time.Time) *string {
	day := time.Date(istNow.Year(), istNow.Month(), istNow.Day(), 0, 0, 0, 0,
		time.FixedZone("IST", 330*60))
	for i := 0; i < 9; i++ {
		if day.Weekday() != time.Saturday && day.Weekday() != time.Sunday {
			at := day.Add(9*time.Hour + 15*time.Minute).UTC()
			if at.After(now) {
				s := tstamp.From(at)
				return &s
			}
		}
		day = day.AddDate(0, 0, 1)
	}
	return nil
}

func scheduleView(c *gin.Context, st *AppState) {
	rows, err := runs.LatestByPrompt(st.Pool)
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	latest := func(prompt string) *runs.Run {
		for i := range rows {
			if rows[i].Prompt == prompt {
				return &rows[i]
			}
		}
		return nil
	}
	now := time.Now().UTC()
	today := schedule.ISTDay(now)
	marker := func(key string) *string {
		var v string
		if err := st.Pool.QueryRow("SELECT value FROM settings WHERE key = ?", key).Scan(&v); err == nil {
			return &v
		}
		return nil
	}
	dailyNext := func(ranToday bool) *string {
		if !ranToday {
			s := tstamp.From(now)
			return &s
		}
		ist := now.Add(330 * time.Minute)
		nextMidnight := time.Date(ist.Year(), ist.Month(), ist.Day(), 0, 5, 0, 0,
			time.FixedZone("IST", 330*60)).AddDate(0, 0, 1)
		s := tstamp.From(nextMidnight.UTC())
		return &s
	}
	stockRan := marker("stock_plan_last") != nil && *marker("stock_plan_last") == today
	fundRan := marker("mf_plan_last") != nil && *marker("mf_plan_last") == today

	entry := func(prompt, label, rule string, next *string) gin.H {
		last := latest(prompt)
		var lastValue any
		if last != nil {
			lastValue = gin.H{
				"status": last.Status, "trigger": last.Trigger, "at": last.StartedAt,
				"finished_at": last.FinishedAt, "summary": last.Summary, "error": last.Error,
			}
		}
		return gin.H{
			"prompt": prompt, "label": label, "rule": rule,
			"next_due_at": next, "last": lastValue,
		}
	}
	c.JSON(http.StatusOK, gin.H{
		"now": tstamp.From(now),
		"prompts": []gin.H{
			entry("plan_stock", "Stock plan", "once a day, first tick after midnight IST",
				dailyNext(stockRan)),
			entry("plan_fund", "Fund plan", "once a day, after the fund scan", dailyNext(fundRan)),
			entry("plan_trade", "Trade plan",
				"every two hours in session — 09:15, 11:15, 13:15 IST", tradeNext(now)),
		},
	})
}

// The next trade slot, from the same function the scheduler uses.
func tradeNext(now time.Time) *string {
	// Four days, so a Friday evening still reaches Monday's open.
	for step := 1; step < 60*24*4; step++ {
		at := now.Add(time.Duration(step) * time.Minute)
		if _, ok := schedule.TradeSlot(at); ok {
			s := tstamp.From(at)
			return &s
		}
	}
	return nil
}

func jsonUnmarshal(data []byte, v any) error { return json.Unmarshal(data, v) }

var _ = models.ModeLong
