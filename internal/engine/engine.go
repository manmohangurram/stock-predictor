// Package engine is one tick of the app: gather, gate, decide, record, push.
package engine

import (
	"database/sql"
	"encoding/json"
	"fmt"
	"log/slog"
	"math"
	"net/http"
	"strings"
	"time"

	"stock-predictor/internal/httpx"
	"stock-predictor/internal/ids"
	"stock-predictor/internal/llm"
	"stock-predictor/internal/market"
	"stock-predictor/internal/models"
	"stock-predictor/internal/notify"
	"stock-predictor/internal/rules"
	"stock-predictor/internal/runs"
	"stock-predictor/internal/schedule"
	"stock-predictor/internal/store"
	"stock-predictor/internal/tstamp"
	"stock-predictor/internal/validate"
)

type EngineDeps struct {
	Pool *sql.DB
	// Read at the top of every tick, so an edit in the UI applies without a restart.
	Runtime   rules.Runtime
	Market    market.MarketSource
	Transport llm.Transport
	Notifier  notify.Notifier
	Rules     rules.RuleConfig
	// Skip the quiet-tick dedupe. Used by --predict, where a human asked for an answer right
	// now and "nothing changed since last time" is not an answer.
	Force bool

	// CandidateBlock is the trade plan's list (trend.rs). Nil leaves it empty.
	CandidateBlock func(pool *sql.DB, m market.MarketSource, t llm.Transport, now time.Time) string
	// SweepGlances drops the names a trade plan looked at and did not use (trend.rs).
	SweepGlances func(pool *sql.DB) (int64, error)
	// DailyList is the stock plan's benchmark list (benchmark.rs).
	DailyList func(pool *sql.DB, m market.MarketSource, now time.Time) (string, error)
	// NewsSearch is the morning pass's web search (market/googlenews.rs).
	NewsSearch func(client *http.Client, name string, limit int) ([]models.Headline, error)
	// Fundamentals is the financials block (market/fundamentals.rs). Nil reads "unavailable".
	Fundamentals func(deps *EngineDeps, symbol string, year int) string
}

type PollReport struct {
	// One human-readable line per decision, for --predict.
	Notes         []string
	Candidates    int
	LLMCalls      int64
	Signals       int
	Notifications int
	Blocked       int
	// Instruments this tick looked at: the denominator for "2 of 6 instruments asked".
	Polled int
	// Instruments the model was actually asked about this tick (ruling and re-priced alike).
	Asked int
	// Instruments the gate left alone this tick: no price move, no new headline.
	NotAsked []string
	// Fresh news pulled this tick because a holding left its band, for the plan to read.
	Fresh string
	// Plan calls that produced a plan. Zero is the normal answer.
	Plans  int
	Errors []string
}

type resolved struct {
	action     models.Action
	entry      float64
	stop       *float64
	target     *float64
	validUntil *string
	qty        float64
	reasons    []string
	rationale  *string
	confidence *string
}

func PollOnce(deps *EngineDeps, now time.Time) (PollReport, error) {
	report := PollReport{}
	instruments, err := loadInstruments(deps.Pool)
	if err != nil {
		return report, err
	}
	slog.Info("poll starting", "instruments", len(instruments), "force", deps.Force,
		"daily_cap", deps.Runtime.DailyCap)

	for i := range instruments {
		if err := pollInstrument(deps, instruments[i], now, &report); err != nil {
			slog.Warn("instrument poll failed", "symbol", instruments[i].Symbol, "error", err)
			report.Errors = append(report.Errors, fmt.Sprintf("%s: %v", instruments[i].Symbol, err))
		}
	}

	fresh := report.Fresh
	report.Fresh = ""
	if fresh != "" {
		ok, err := planBookWith(deps, now, llm.ScopeStock, nil, fresh, true)
		switch {
		case err != nil:
			report.Errors = append(report.Errors, fmt.Sprintf("band plan: %v", err))
		case ok:
			report.Plans++
		}
	}

	// The stock plan is a daily question, not a side effect of a per-stock call: the marker
	// decides whether it has been asked today, whatever this tick did.
	if ok, err := planBook(deps, now, llm.ScopeStock, nil); err != nil {
		report.Errors = append(report.Errors, fmt.Sprintf("plan: %v", err))
	} else if ok {
		report.Plans++
	}
	report.LLMCalls, _ = llm.DailyChatCalls(deps.Pool)
	return report, nil
}

// The headlines the model reads: the last week, newest first. So today's search is preferred
// simply by being newest, and a holiday week falls back to whatever that week did carry.
func storedHeadlines(pool *sql.DB, instrumentID string, now time.Time) ([]models.Headline, error) {
	since := tstamp.From(now.AddDate(0, 0, -7))
	rows, err := pool.Query(
		`SELECT title, publisher, published_at, url FROM news
         WHERE instrument_id = ? AND published_at >= ?
         ORDER BY published_at DESC LIMIT 8`, instrumentID, since,
	)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []models.Headline{}
	for rows.Next() {
		var title, publishedAt, url string
		var publisher sql.NullString
		if err := rows.Scan(&title, &publisher, &publishedAt, &url); err != nil {
			return nil, err
		}
		out = append(out, models.Headline{
			ID: int64(len(out)) + 1, Title: title, Publisher: publisher.String,
			PublishedAt: publishedAt, URL: url,
		})
	}
	return out, rows.Err()
}

// The headline must name the company, by name or by its first word.
func namesCompany(h models.Headline, name string) bool {
	title := lower(h.Title)
	first := lower(firstWord(name))
	return first != "" && contains(title, first)
}

// The morning news pass: one web search per active instrument, once an IST day, stored as
// headlines.
func RefreshNews(deps *EngineDeps, now time.Time) (int, error) {
	if deps.NewsSearch == nil {
		return 0, nil
	}
	day := schedule.ISTDay(now)
	last, err := getSetting(deps.Pool, "news_search_last")
	if err != nil {
		return 0, err
	}
	if last != nil && *last == day {
		return 0, nil
	}
	client, err := httpx.ClientWith(&deps.Runtime, 20, "Mozilla/5.0 stock-predictor")
	if err != nil {
		return 0, err
	}
	instruments, err := loadInstruments(deps.Pool)
	if err != nil {
		return 0, err
	}
	stored := 0
	for _, instrument := range instruments {
		rows, err := deps.NewsSearch(client, instrument.Name, 2)
		if err != nil {
			continue
		}
		kept := []models.Headline{}
		for _, h := range rows {
			if !hasIdeographic(h.Title) && namesCompany(h, instrument.Name) {
				kept = append(kept, h)
			}
		}
		for i := range kept {
			kept[i].ID = int64(i) + 1
		}
		stored += len(kept)
		if err := store.Headlines(deps.Pool, instrument.ID, kept); err != nil {
			return stored, err
		}
	}
	if err := putSetting(deps.Pool, "news_search_last", day, now); err != nil {
		return stored, err
	}
	return stored, nil
}

// True once per IST day per reason: the same band edge must not buy ninety-six calls.
func bandTriggerFires(pool *sql.DB, instrument models.Instrument, reason string, now time.Time) (bool, error) {
	marker := schedule.ISTDay(now) + "-" + reason
	key := fmt.Sprintf("stock_band_trigger_%s", instrument.ID)
	v, err := getSetting(pool, key)
	if err != nil {
		return false, err
	}
	if v != nil && *v == marker {
		return false, nil
	}
	if err := putSetting(pool, key, marker, now); err != nil {
		return false, err
	}
	return true, nil
}

// One book call: the model sees every holding, the cash and today's list, and may answer with
// up to three moves that stand or fall together. Returns true only when a plan was stored.
func planBook(deps *EngineDeps, now time.Time, scope llm.Scope, run *string) (bool, error) {
	return planBookWith(deps, now, scope, run, "", false)
}

// extra is fresh context for this one call — news pulled because a holding left its band.
func planBookWith(deps *EngineDeps, now time.Time, scope llm.Scope, run *string, extra string, force bool) (bool, error) {
	manual := run != nil
	var slot string
	if scope.IsTrade() {
		s, ok := schedule.TradeSlot(now)
		if !ok {
			if !manual {
				return false, nil // outside the session: nothing can be done anyway
			}
			slot = ""
		} else {
			slot = s
		}
	} else {
		slot = schedule.ISTDay(now)
	}
	marker := "stock_plan_last"
	switch scope {
	case llm.ScopeFund:
		marker = "mf_plan_last"
	case llm.ScopeTrade:
		marker = "trade_plan_last_slot"
	}
	if !manual && !force {
		last, err := getSetting(deps.Pool, marker)
		if err != nil {
			return false, err
		}
		if last != nil && *last == slot {
			slog.Debug("plan skipped: already asked this slot", "scope", scope, "slot", slot)
			return false, nil
		}
	}
	funds, err := fundSymbols(deps.Pool)
	if err != nil {
		return false, err
	}
	mode := models.ModeLong
	if scope.IsTrade() {
		mode = models.ModeTrade
	}
	allHoldings, err := loadHoldings(deps.Pool, mode)
	if err != nil {
		return false, err
	}
	book := []llm.HoldingLine{}
	for _, h := range allHoldings {
		if scope.IsTrade() || containsStr(funds, h.Symbol) == scope.IsFund() {
			book = append(book, h)
		}
	}
	// A stock plan may open the very first position; a fund plan needs the fund book to weigh.
	if len(book) == 0 && scope.IsFund() {
		slog.Debug("plan skipped: fund book is empty", "scope", scope)
		return false, nil
	}
	settings, err := rules.GetSettings(deps.Pool)
	if err != nil {
		return false, err
	}
	var available float64
	switch {
	case scope.IsFund():
		available, err = rules.AvailableMF(deps.Pool)
	case scope.IsTrade():
		available, err = rules.AvailableIn(deps.Pool, models.ModeTrade)
	default:
		available, err = rules.AvailableIn(deps.Pool, models.ModeLong)
	}
	if err != nil {
		return false, err
	}
	calls, err := llm.DailyChatCalls(deps.Pool)
	if err != nil {
		return false, err
	}
	if calls >= int64(deps.Runtime.DailyCap) {
		slog.Info("daily call cap reached — plan not attempted", "scope", scope, "calls", calls,
			"cap", deps.Runtime.DailyCap)
		return false, nil
	}

	var candidates string
	switch {
	case scope.IsTrade():
		if deps.CandidateBlock != nil {
			candidates = deps.CandidateBlock(deps.Pool, deps.Market, deps.Transport, now)
		}
	case scope.IsFund():
		candidates, err = candidatesForPlan(deps.Pool, settings.Currency, scope)
		if err != nil {
			return false, err
		}
	default:
		if deps.DailyList != nil {
			list, derr := deps.DailyList(deps.Pool, deps.Market, now)
			if derr != nil {
				slog.Warn("nifty 100 list unavailable", "error", derr)
				candidates = fmt.Sprintf("the benchmark list could not be built today (%v)\n", derr)
			} else {
				candidates = list
			}
		} else {
			candidates = "no candidate cleared a positive five-session return today\n"
		}
	}
	if scope.IsTrade() && len(book) == 0 && trimSpace(candidates) == "" {
		return false, nil // nothing held, nothing offered: no call worth paying for
	}
	if trimSpace(extra) != "" {
		candidates = candidates + "\n" + extra + "\n"
	}
	var limits string
	switch {
	case scope.IsFund():
		limits = fmt.Sprintf(
			"the whole plan may spend at most %.2f %s, which is everything the fund purse holds; there is no per-trade ceiling for funds",
			available, settings.Currency)
	case scope.IsTrade():
		limits = fmt.Sprintf(
			"spend at most %.2f %s across the plan; no single buy above %.2f; a take-profit between %.0f%% and %.0f%% — and every move carries an expiry",
			settings.LimitsTrade.MaxTradeAmount, settings.Currency,
			settings.LimitsTrade.MaxTradeAmount,
			settings.LimitsTrade.MinProfitPct, settings.LimitsTrade.MaxProfitPct)
	default:
		limits = fmt.Sprintf(
			"spend at most %.2f %s across the plan; no single buy above %.2f; a take-profit between %.0f%% and %.0f%%",
			settings.LimitsLong.MaxTradeAmount, settings.Currency,
			settings.LimitsLong.MaxTradeAmount,
			settings.LimitsLong.MinProfitPct, settings.LimitsLong.MaxProfitPct)
	}
	rendered, err := llm.RenderPlan(scope, llm.BookBlock(book, settings.Currency),
		llm.CapLine(available, settings.Currency), limits, candidates, available, settings.Currency)
	if err != nil {
		return false, err
	}
	model, err := rules.ModelConfigFor(deps.Pool)
	if err != nil {
		return false, err
	}
	if strings.TrimSpace(model.APIKey) == "" {
		slog.Info("plan skipped: no model key configured", "scope", scope)
		return false, nil
	}
	runID := run
	if runID == nil {
		prompt := runs.PlanStock
		switch scope {
		case llm.ScopeFund:
			prompt = runs.PlanFund
		case llm.ScopeTrade:
			prompt = runs.PlanTrade
		}
		id, err := runs.Start(deps.Pool, prompt, runs.TriggerSchedule, nil)
		if err != nil {
			return false, err
		}
		runID = &id
	}
	conversation := fmt.Sprintf("plan-%d", now.UnixMilli())
	trigger := fmt.Sprintf("%s plan: %d holdings, %.2f %s free — looked for moves that fit together",
		scope.PurseName(), len(book), available, settings.Currency)
	slog.Info("plan call", "scope", scope, "holdings", len(book), "available", available,
		"conversation", conversation, "run", *runID)
	reply, err := llm.Chat(deps.Pool, &model, deps.Transport, llm.ChatRequest{
		ConversationID: conversation,
		Round:          1,
		Prompt:         rendered.Text,
		PromptTemplate: rendered.Template,
		PromptHash:     rendered.Hash,
		Trigger:        &trigger,
		Purpose:        "plan " + scope.PurseName() + "s",
	})
	if err != nil {
		// A failed call looked at a list too; the glances go back before the error does.
		if scope.IsTrade() && deps.SweepGlances != nil {
			_, _ = deps.SweepGlances(deps.Pool)
		}
		// A failed call must close its run, or the page shows a spinner that will not stop.
		_ = runs.Fail(deps.Pool, *runID, err.Error())
		return false, err
	}

	if !manual && !force {
		if err := putSetting(deps.Pool, marker, slot, now); err != nil {
			return false, err
		}
	}

	answer, err := llm.ParsePlan(reply.Content)
	if err != nil {
		slog.Warn("plan reply unreadable", "error", err)
		_ = runs.Fail(deps.Pool, *runID, fmt.Sprintf("the reply did not parse: %v", err))
		return false, nil
	}
	if answer.Plan == nil {
		slog.Info("plan passed", "scope", scope, "why", answer.Why)
		_ = runs.Finish(deps.Pool, *runID, "nothing worth doing — the model passed", &conversation)
		// A trade plan may have looked at twenty tickers to make two moves; the rest are gone.
		if scope.IsTrade() && deps.SweepGlances != nil {
			_, _ = deps.SweepGlances(deps.Pool)
		}
		return false, nil
	}
	plan := *answer.Plan

	// One purse per plan: the other is reported as zero, so a mixed plan cannot balance.
	cash := llm.Cash{}
	switch scope {
	case llm.ScopeFund:
		cash.Fund = available
	case llm.ScopeTrade:
		cash.Trade = available
	default:
		cash.Stock = available
	}
	limitsFor := settings.LimitsLong
	if scope.IsTrade() {
		limitsFor = settings.LimitsTrade
	}
	if err := llm.ValidatePlan(plan, book, cash, limitsFor, settings.Currency, now, funds, scope); err != nil {
		slog.Warn("plan refused by the validator", "error", err)
		if err := recordPlanRefusal(deps.Pool, err.Error(), now, &conversation, scope); err != nil {
			return false, err
		}
		_ = runs.Finish(deps.Pool, *runID, "refused: "+err.Error(), &conversation)
		return false, nil
	}

	var planID *string
	for _, m := range plan.Moves {
		reasonJSON := mustJSON(map[string]any{
			"reasons":        []string{},
			"rationale":      m.Why,
			"plan_rationale": plan.Rationale,
			"plan":           true,
			"scope":          scope.PurseName(),
			"amount":         m.Amount,
		})
		validUntil := now.Add(18 * time.Hour)
		if m.ValidUntil != nil {
			if parsed, err := time.Parse(time.RFC3339, *m.ValidUntil); err == nil {
				validUntil = parsed
			}
		}
		moveMode := models.ModeLong
		if scope.IsTrade() {
			moveMode = models.ModeTrade
		}
		var id string
		err := deps.Pool.QueryRow(
			`INSERT INTO signals (id, instrument_id, action, qty, entry, stop, target, valid_until,
                reason_json, created_at, confidence, mode, kind, status, plan_id, conversation_id)
             SELECT ?, id, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, 'plan', 'pending', ?, ?
             FROM instruments WHERE symbol = ? RETURNING id`,
			ids.New(), string(m.Action), m.Qty, m.Price, m.Stop, m.Target, tstamp.From(validUntil),
			reasonJSON, tstamp.From(now), string(moveMode), planID, conversation, m.Symbol,
		).Scan(&id)
		if err == sql.ErrNoRows {
			why := fmt.Sprintf("%s is not in the book, so the plan cannot be stored", m.Symbol)
			if err := recordPlanRefusal(deps.Pool, why, now, &conversation, scope); err != nil {
				return false, err
			}
			_ = runs.Fail(deps.Pool, *runID, why)
			return false, nil
		}
		if err != nil {
			return false, err
		}
		// The first leg is its own group: every later leg points at it, and so does it.
		if planID == nil {
			if _, err := deps.Pool.Exec("UPDATE signals SET plan_id = ? WHERE id = ?", id, id); err != nil {
				return false, err
			}
			planID = &id
		}
	}
	if scope.IsTrade() && deps.SweepGlances != nil {
		if dropped, err := deps.SweepGlances(deps.Pool); err == nil && dropped > 0 {
			slog.Info("trade plan swept its glances", "dropped", dropped)
		}
	}
	noun := "moves"
	if len(plan.Moves) == 1 {
		noun = "move"
	}
	slog.Info("plan stored", "scope", scope, "moves", len(plan.Moves), "plan_id", *planID,
		"conversation", conversation)
	_ = runs.Finish(deps.Pool, *runID, fmt.Sprintf("%d %s stored", len(plan.Moves), noun), &conversation)
	return true, nil
}

// A refused plan is a decision too: it is recorded with the reason, so the ledger can say why
// nothing happened, the way a blocked single proposal already does.
func recordPlanRefusal(pool *sql.DB, why string, now time.Time, conversationID *string, scope llm.Scope) error {
	reason := mustJSON(map[string]any{"blocked": "plan_refused", "reasons": []string{why}})
	mode := models.ModeLong
	if scope.IsTrade() {
		mode = models.ModeTrade
	}
	fund := 0
	if scope.IsFund() {
		fund = 1
	}
	_, err := pool.Exec(
		`INSERT INTO signals (id, instrument_id, action, qty, entry, valid_until, reason_json,
            created_at, mode, kind, status, conversation_id)
         SELECT ?, id, 'BLOCKED', 0, 0, ?, ?, ?, ?, 'plan', 'pending', ?
         FROM instruments WHERE (class = 'mutual_fund') = ? AND active = 1
         ORDER BY id LIMIT 1`,
		ids.New(), tstamp.From(now.Add(18*time.Hour)), reason, tstamp.From(now), string(mode),
		conversationID, fund,
	)
	return err
}

// The stock plan on its own, for a manual run: the same call the poll makes, asked now.
func PlanStock(deps *EngineDeps, now time.Time, run *string) (bool, error) {
	return planBook(deps, now, llm.ScopeStock, run)
}

// One instrument, asked on its own — the precise version of a poll.
func DecideOne(deps *EngineDeps, symbol string, now time.Time) (*string, error) {
	instruments, err := loadInstruments(deps.Pool)
	if err != nil {
		return nil, err
	}
	for i := range instruments {
		if instruments[i].Symbol == symbol {
			return Advise(deps, instruments[i], now)
		}
	}
	return nil, fmt.Errorf("%s is not an active instrument", symbol)
}

// The trade book's clock. An idea that has passed its own expiry is closed by the app, not by
// the model's opinion of it.
//
// One SELL per open position, and none while one is already waiting to be approved.
func ExpireTradePositions(pool *sql.DB, now time.Time) ([]string, error) {
	rows, err := pool.Query(
		`SELECT p.instrument_id, i.symbol, i.name, p.qty,
                (SELECT b.close FROM bars b WHERE b.instrument_id = p.instrument_id
                 ORDER BY b.ts DESC LIMIT 1) AS close
           FROM positions p
           JOIN instruments i ON i.id = p.instrument_id
          WHERE p.mode = 'trade' AND p.state = 'open'
            AND EXISTS (
                SELECT 1 FROM signals s
                 WHERE s.instrument_id = p.instrument_id
                   AND s.mode = 'trade' AND s.action = 'BUY'
                   AND s.status = 'approved'
                   AND s.valid_until IS NOT NULL
                   AND s.valid_until <= ?)
            AND NOT EXISTS (
                SELECT 1 FROM signals s2
                 WHERE s2.instrument_id = p.instrument_id
                   AND s2.mode = 'trade' AND s2.action = 'SELL'
                   AND s2.status = 'pending')`, tstamp.From(now),
	)
	if err != nil {
		return nil, err
	}
	type expired struct {
		instrumentID string
		symbol       string
		name         string
		qty          float64
		close        *float64
	}
	items := []expired{}
	for rows.Next() {
		var e expired
		var close sql.NullFloat64
		if err := rows.Scan(&e.instrumentID, &e.symbol, &e.name, &e.qty, &close); err != nil {
			rows.Close()
			return nil, err
		}
		if close.Valid {
			e.close = &close.Float64
		}
		items = append(items, e)
	}
	rows.Close()
	if err := rows.Err(); err != nil {
		return nil, err
	}

	out := []string{}
	for _, e := range items {
		reason := mustJSON(map[string]any{
			"reasons": []string{"the idea's own expiry has passed — neither target nor stop was touched"},
			"expired": true,
		})
		entry := 0.0
		if e.close != nil {
			entry = *e.close
		}
		if _, err := pool.Exec(
			`INSERT INTO signals (id, instrument_id, action, qty, entry, valid_until, reason_json,
                created_at, mode, kind, status)
             VALUES (?, ?, 'SELL', ?, ?, ?, ?, ?, 'trade', 'expiry', 'pending')`,
			ids.New(), e.instrumentID, e.qty, entry, tstamp.From(now.Add(18*time.Hour)), reason, tstamp.From(now),
		); err != nil {
			return out, err
		}
		out = append(out, fmt.Sprintf("%s · %.0f units — %s expired", e.symbol, e.qty, e.name))
	}
	return out, nil
}

func PlanTrade(deps *EngineDeps, now time.Time, run *string) (bool, error) {
	planned, err := planBook(deps, now, llm.ScopeTrade, run)
	if err != nil {
		return false, err
	}
	lines, err := ExpireTradePositions(deps.Pool, now)
	if err != nil {
		slog.Warn("expiry sweep failed", "error", err)
		return planned, nil
	}
	for _, line := range lines {
		symbol := firstToken(line)
		n := notify.Notification{
			Title:     "SELL · " + symbol,
			Body:      line,
			Priority:  notify.PriorityHigh,
			DedupeKey: fmt.Sprintf("expiry-%s-%d", line, now.Unix()/900),
			Symbol:    symbol,
		}
		if _, err := notify.Dispatch(deps.Pool, deps.Notifier, n); err != nil {
			slog.Warn("expiry push failed", "error", err)
		}
		slog.Info("trade idea expired", "line", line)
	}
	return planned, nil
}

// The fund plan, run once a day after the arithmetic scan has ranked today's funds.
func PlanFunds(deps *EngineDeps, now time.Time, run *string) (bool, error) {
	return planBook(deps, now, llm.ScopeFund, run)
}

// The symbols that are mutual funds, so the plan's money can be counted per purse.
func fundSymbols(pool *sql.DB) ([]string, error) {
	rows, err := pool.Query("SELECT symbol FROM instruments WHERE class = 'mutual_fund'")
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []string{}
	for rows.Next() {
		var s string
		if err := rows.Scan(&s); err != nil {
			return nil, err
		}
		out = append(out, s)
	}
	return out, rows.Err()
}

// The fund plan's list: the active funds and their last two NAVs.
func candidatesForPlan(pool *sql.DB, currency string, scope llm.Scope) (string, error) {
	fund := 0
	if scope.IsFund() {
		fund = 1
	}
	rows, err := pool.Query(
		`SELECT i.symbol AS symbol, i.name AS name,
                (SELECT b.close FROM bars b WHERE b.instrument_id = i.id
                 ORDER BY b.ts DESC LIMIT 1) AS last,
                (SELECT b.close FROM bars b WHERE b.instrument_id = i.id
                 ORDER BY b.ts DESC LIMIT 1 OFFSET 1) AS prev
         FROM instruments i
         WHERE i.active = 1 AND (i.class = 'mutual_fund') = ?
         ORDER BY i.symbol`, fund,
	)
	if err != nil {
		return "", err
	}
	defer rows.Close()
	out := ""
	for rows.Next() {
		var symbol, name string
		var last float64
		var prev sql.NullFloat64
		if err := rows.Scan(&symbol, &name, &last, &prev); err != nil {
			return "", err
		}
		change := "no prior close"
		if prev.Valid && prev.Float64 > 0 {
			change = fmt.Sprintf("%+.2f%% today", (last-prev.Float64)/prev.Float64*100)
		}
		out += fmt.Sprintf("%s %s at %.2f %s, %s\n", symbol, name, last, currency, change)
	}
	if out == "" {
		out = "nothing on offer today\n"
	}
	return out, rows.Err()
}

type gathered struct {
	bars               []models.Bar
	news               []models.Headline
	ind                rules.Indicators
	position           *models.Position
	available          float64
	limits             rules.Limits
	requiredProfit     float64
	pot                float64
	instrumentExposure float64
	priceWindow        string
	tape               []models.Bar
	dailySummary       string
	positionsText      string
	recentTrades       string
	lastDecision       string
}

func (g gathered) lastClose() float64 {
	if len(g.bars) == 0 {
		return 0
	}
	return g.bars[len(g.bars)-1].Close
}

func gather(deps *EngineDeps, instrument models.Instrument, now time.Time) (*gathered, error) {
	bars, err := loadDaily(deps, instrument.Symbol)
	if err != nil {
		return nil, err
	}
	priceWindow := "(intraday tape unavailable)"
	tape := []models.Bar{}
	if t, err := deps.Market.Intraday(instrument.Symbol, "15m", "5d"); err == nil {
		tape = t
		priceWindow = formatWindow(tape, deps.Runtime.PromptWindowHours, now)
	} else {
		slog.Debug("no intraday tape", "symbol", instrument.Symbol, "error", err)
	}
	// The daily series is fetched once a day; today's candle is what the tape says it is.
	bars = mergeTape(bars, tape, now)
	ind, err := rules.ComputeIndicators(bars)
	if err != nil {
		slog.Debug("not enough history for indicators", "symbol", instrument.Symbol)
		return nil, nil
	}
	if err := store.Bars(deps.Pool, instrument.ID, bars); err != nil {
		return nil, err
	}
	// The morning's search already stored what the web said; a tick reads it, it does not re-ask.
	news, err := storedHeadlines(deps.Pool, instrument.ID, now)
	if err != nil {
		return nil, err
	}
	position, err := loadPosition(deps.Pool, instrument.ID)
	if err != nil {
		return nil, err
	}
	available, err := rules.AvailableIn(deps.Pool, models.ModeLong)
	if err != nil {
		return nil, err
	}
	limits, err := rules.LimitsFor(deps.Pool, models.ModeLong)
	if err != nil {
		return nil, err
	}
	pot, err := rules.BudgetTotalIn(deps.Pool, models.ModeLong)
	if err != nil {
		return nil, err
	}
	exposure, err := loadExposure(deps.Pool, instrument.ID)
	if err != nil {
		return nil, err
	}
	targetProfit, err := loadTargetProfit(deps.Pool, instrument.ID)
	if err != nil {
		return nil, err
	}
	required := limits.MinProfitPct
	if targetProfit != nil {
		required = math.Min(math.Max(*targetProfit, limits.MinProfitPct), limits.MaxProfitPct)
	}
	holdings, err := loadHoldings(deps.Pool, models.ModeLong)
	if err != nil {
		return nil, err
	}
	recent, err := loadRecentTrades(deps.Pool, instrument.ID, now)
	if err != nil {
		return nil, err
	}
	lastDecision, err := loadLastDecision(deps.Pool, instrument.ID)
	if err != nil {
		return nil, err
	}
	return &gathered{
		bars: bars, news: news, ind: ind, position: position, available: available,
		limits: limits, requiredProfit: required, pot: pot, instrumentExposure: exposure,
		priceWindow: priceWindow, tape: tape,
		dailySummary: formatDaily(bars, 10), positionsText: FormatBook(holdings, instrument.Symbol),
		recentTrades: recent, lastDecision: lastDecision,
	}, nil
}

// loadDaily uses the day-long cache when the source has one, so a poll does not re-download a year
// of history for every instrument.
func loadDaily(deps *EngineDeps, symbol string) ([]models.Bar, error) {
	if cached, ok := deps.Market.(interface {
		CachedBars(string) ([]models.Bar, error)
	}); ok {
		return cached.CachedBars(symbol)
	}
	return deps.Market.Bars(symbol)
}

// mergeTape folds today's tape into the daily series: replace today's candle when the daily fetch
// already carried one, append it otherwise. After this, indicators and the prompt see the price
// the tape shows without a second daily request.
func mergeTape(daily, tape []models.Bar, now time.Time) []models.Bar {
	if len(tape) == 0 {
		return daily
	}
	today := schedule.ISTDay(now)
	var today_ []models.Bar
	for _, b := range tape {
		if len(b.Ts) >= 10 && b.Ts[:10] == today {
			today_ = append(today_, b)
		}
	}
	if len(today_) == 0 {
		return daily
	}
	synthetic := models.Bar{
		Ts: today + "T00:00:00.000000000+00:00", Open: today_[0].Open,
		High: today_[0].High, Low: today_[0].Low, Close: today_[0].Close,
	}
	for _, b := range today_[1:] {
		if b.High > synthetic.High {
			synthetic.High = b.High
		}
		if b.Low < synthetic.Low {
			synthetic.Low = b.Low
		}
		synthetic.Close = b.Close
		synthetic.Volume += b.Volume
	}
	merged := make([]models.Bar, len(daily), len(daily)+1)
	copy(merged, daily)
	if len(daily) > 0 && len(daily[len(daily)-1].Ts) >= 10 &&
		daily[len(daily)-1].Ts[:10] == today {
		merged[len(merged)-1] = synthetic
		return merged
	}
	return append(merged, synthetic)
}

func pollInstrument(deps *EngineDeps, instrument models.Instrument, now time.Time, report *PollReport) error {
	if instrument.Class == models.ClassMutualFund && !contains(instrument.Symbol, ".") {
		report.Notes = append(report.Notes, fmt.Sprintf(
			"%s: AMFI scheme code, no price series — hold it, but add the Yahoo code to get advice",
			instrument.Symbol))
		return nil
	}
	g, err := gather(deps, instrument, now)
	if err != nil {
		return err
	}
	if g == nil {
		return nil
	}
	report.Polled++

	// A winner's stop follows the price up, and the book in memory moves with it: the rules
	// below must judge the stop that is now recorded, not the one this tick began with.
	if g.position != nil && len(g.bars) > 0 {
		close := g.bars[len(g.bars)-1].Close
		next := rules.TrailedStop(g.position.Stop, g.position.AvgCost, close, g.ind.ATR14)
		if !samePtr(next, g.position.Stop) {
			if next != nil {
				if _, err := deps.Pool.Exec("UPDATE positions SET stop = ? WHERE id = ?", *next, g.position.ID); err != nil {
					return err
				}
			}
			g.position.Stop = next
		}
	}

	input := rules.CandidateInput{
		Instrument: &instrument, Bars: g.bars, Ind: &g.ind, Position: g.position,
		AvailableCash: g.available, Cfg: deps.Rules,
	}
	rulesCandidate := rules.Candidate(input)
	hadRulesCandidate := rulesCandidate != nil
	var candidate models.Candidate
	if rulesCandidate != nil {
		candidate = *rulesCandidate
	} else {
		// Say why, per instrument: "the ledger is empty" is not an answer on its own.
		failed := []string{}
		for _, c := range rules.Diagnose(input) {
			if !c.Passed {
				failed = append(failed, c.Name+" — "+c.Detail)
			}
		}
		why := "no rule fired"
		if len(failed) > 0 {
			why = join(failed, "; ")
		}
		report.Notes = append(report.Notes, fmt.Sprintf(
			"%s: rules see no entry (%s) — asking the model on prices alone", instrument.Symbol, why))
		last := g.lastClose()
		qty := math.Floor(g.available / math.Max(last, 1e-9))
		candidate = models.Candidate{
			InstrumentID: instrument.ID, Action: models.ActionHold,
			Entry: last, Qty: qty, TimingNote: instrument.Class.TimingNote(),
		}
	}
	clamped, ok := clampToLimits(candidate, g.limits, g.pot, g.instrumentExposure)
	if !ok {
		report.Notes = append(report.Notes, fmt.Sprintf(
			"%s: no affordable size inside the limits (%.2f to %.2f per trade)",
			instrument.Symbol, g.limits.MinTradeAmount, g.limits.MaxTradeAmount))
		return nil
	}
	candidate = clamped

	// The rules already act on a breached stop or a reached target. This fires where they do
	// not: a holding that has run past the pot's ceiling without touching its own recorded
	// target.
	if !hadRulesCandidate && g.position != nil && len(g.bars) > 0 {
		close := g.bars[len(g.bars)-1].Close
		settings, err := rules.GetSettings(deps.Pool)
		if err != nil {
			return err
		}
		reason := rules.BandReason(g.position.Stop, g.position.AvgCost, close, settings.LimitsLong.MaxProfitPct)
		if reason == "ceiling" {
			fires, err := bandTriggerFires(deps.Pool, instrument, "ceiling", now)
			if err != nil {
				return err
			}
			if fires {
				// The morning's copy is what the answer should not rest on: read the feed now.
				hits := []models.Headline{}
				if deps.NewsSearch != nil {
					if client, err := httpx.ClientWith(&deps.Runtime, 20, "Mozilla/5.0 stock-predictor"); err == nil {
						if rows, err := deps.NewsSearch(client, instrument.Name, 2); err == nil {
							hits = rows
						}
					}
				}
				report.Notes = append(report.Notes, fmt.Sprintf(
					"%s: past the %.0f%% ceiling with no exit from the rules — news pulled, plan asked",
					instrument.Symbol, settings.LimitsLong.MaxProfitPct))
				searchHits := []llm.SearchHit{}
				for _, h := range hits {
					if hasIdeographic(h.Title) {
						continue
					}
					date := h.PublishedAt
					searchHits = append(searchHits, llm.SearchHit{
						Title: h.Title, Site: h.Publisher, URL: h.URL, Date: &date,
					})
				}
				stop := "none"
				if g.position.Stop != nil {
					stop = fmt.Sprintf("%.2f", *g.position.Stop)
				}
				report.Fresh += fmt.Sprintf("## %s is past the pot's %.0f%% ceiling\nlast %.2f, paid %.2f, stop %s\n%s\n",
					instrument.Symbol, settings.LimitsLong.MaxProfitPct, close, g.position.AvgCost,
					stop, llm.RenderWebHits(searchHits))
			}
		}
	}

	if hadRulesCandidate {
		report.Candidates++
	}

	headlineFP := llm.Fingerprint(g.news)
	unchanged, err := llm.Unchanged(deps.Pool, instrument.ID, headlineFP)
	if err != nil {
		unchanged = false
	}
	headlinesChanged := !unchanged
	slog.Debug("instrument gathered", "symbol", instrument.Symbol, "bars", len(g.bars),
		"news", len(g.news), "held", heldQty(g.position), "available", g.available,
		"candidate", candidate.Action, "entry", candidate.Entry, "qty", candidate.Qty,
		"headlines_changed", headlinesChanged)

	shortContext := shortCard(instrument, g.bars, g.tape, now)

	// Why this call is being made, in words a person can read later in the audit trail.
	var previousPrice sql.NullFloat64
	_ = deps.Pool.QueryRow("SELECT last_price FROM llm_memos WHERE instrument_id = ?", instrument.ID).Scan(&previousPrice)
	var gateReason *string
	switch {
	case deps.Force:
		r := "asked by hand"
		gateReason = &r
	case headlinesChanged:
		r := "new headline on " + instrument.Symbol
		gateReason = &r
	case previousPrice.Valid:
		if math.Abs(previousPrice.Float64-candidate.Entry) >= deps.Runtime.MinPriceMove {
			r := fmt.Sprintf("price moved %.2f → %.2f", previousPrice.Float64, candidate.Entry)
			gateReason = &r
		}
	default:
		r := "never asked about this instrument"
		gateReason = &r
	}

	// Do not spend a call on a price that has not moved, and keep quiet otherwise.
	if !deps.Force {
		gate := llm.DefaultCallGate()
		gate.MinMove = deps.Runtime.MinPriceMove
		should, err := llm.ShouldCall(deps.Pool, instrument.ID, candidate.Entry, now, gate, headlinesChanged)
		if err != nil {
			return err
		}
		if !should {
			report.Notes = append(report.Notes, fmt.Sprintf(
				"%s: price moved less than %.2f and no new headlines — model not asked",
				instrument.Symbol, gate.MinMove))
			report.NotAsked = append(report.NotAsked, instrument.Symbol)
			// The decision stands, but its price should not age: refresh the entry only.
			_, err := reuseModelSignal(deps.Pool, instrument.ID, candidate.Entry, g.ind.ATR14, now)
			return err
		}
	}

	// No key configured means no model layer at all — not a failed call on every tick.
	dsKey, err := rules.DeepseekKey(deps.Pool)
	if err != nil {
		return err
	}
	if dsKey == nil {
		broken, _ := unreadableKey(deps.Pool, "deepseek_key")
		note := instrument.Symbol + ": no DeepSeek key — rules only"
		if broken {
			note = instrument.Symbol + ": the stored key cannot be opened with the current SECRET_KEY — re-enter it in Settings"
		}
		report.Notes = append(report.Notes, note)
		valid := tstamp.From(now.Add(18 * time.Hour))
		r := resolved{
			action: candidate.Action, entry: candidate.Entry, stop: candidate.Stop,
			target: candidate.Target, validUntil: &valid, qty: candidate.Qty, reasons: candidate.Reasons,
		}
		return finish(deps, instrument, finished{
			resolved: r, sizing: sizing{available: g.available, atr: g.ind.ATR14}, now: now,
		}, report)
	}

	if !deps.Force {
		pair, err := rotateForCash(deps, instrument, candidate, now)
		if err != nil {
			slog.Warn("rotation attempt failed", "error", err)
		} else if pair != nil {
			report.Signals += 2
			report.Notes = append(report.Notes, fmt.Sprintf(
				"%s: swap proposed — sell (decision %d) to fund this (decision %d)",
				instrument.Symbol, pair[0], pair[1]))
			return nil
		}
	}

	// Only a call that will actually happen pays for financials; they change quarterly, not per tick.
	fundamentals := "unavailable"
	if deps.Fundamentals != nil {
		fundamentals = deps.Fundamentals(deps, instrument.Symbol, now.Year())
	}

	llmDeps := llm.LlmDeps{Pool: deps.Pool, Runtime: deps.Runtime, Transport: deps.Transport, Market: deps.Market}
	decision, err := llm.Run(&llmDeps, llm.RunInput{
		Instrument: &instrument, Ind: &g.ind, Candidate: candidate,
		CallReason: gateReason, Fundamentals: fundamentals, ShortContext: shortContext,
		Mode: models.ModeLong, AvailableCash: g.available,
		HeldQty: heldQty(g.position), Limits: g.limits, RequiredProfitPct: g.requiredProfit,
		InstrumentExposure: g.instrumentExposure, Pot: g.pot,
		WindowHours: deps.Runtime.PromptWindowHours, Headlines: g.news,
		PriceWindow: g.priceWindow, DailySummary: g.dailySummary, Positions: g.positionsText,
		RecentTrades: g.recentTrades, LastDecision: g.lastDecision, Now: now,
	})
	if err != nil {
		return err
	}

	// The model answered for this headline set; do not ask it about the same news again.
	if decision.Outcome.Kind != llm.OutcomeRules {
		if err := llm.Remember(deps.Pool, instrument.ID, headlineFP); err != nil {
			return err
		}
	}
	// Recorded whatever the answer was, so a failing key cannot loop on every tick.
	if err := llm.RecordMemoCall(deps.Pool, instrument.ID, candidate.Entry, now, headlineFP); err != nil {
		return err
	}
	report.Asked++

	var refused *llm.Outcome
	if decision.Outcome.Kind == llm.OutcomeRejected && decision.Outcome.Error != nil &&
		decision.Outcome.Error.Kind != validate.KindInsufficientCash {
		refused = &decision.Outcome
	}
	res, ok := resolve(candidate, decision.Outcome, now)
	if !ok {
		return nil
	}
	if res.action != models.ActionBuy && res.action != models.ActionSell && refused != nil {
		if err := recordRefused(deps.Pool, instrument, refused.Proposal, refused.Error.Error(), now, &decision.ConversationID); err != nil {
			return err
		}
		report.Blocked++
		report.Notes = append(report.Notes, fmt.Sprintf(
			"%s: model proposed %s at %.2f — refused: %s",
			instrument.Symbol, refused.Proposal.Action, refused.Proposal.Entry, refused.Error.Error()))
		return nil
	}

	return finish(deps, instrument, finished{
		resolved: res, sizing: sizing{available: g.available, atr: g.ind.ATR14},
		now: now, conversationID: &decision.ConversationID,
	}, report)
}

// Ask the model about one instrument on demand, bypassing the rules gate, and write nothing.
func Advise(deps *EngineDeps, instrument models.Instrument, now time.Time) (*string, error) {
	g, err := gather(deps, instrument, now)
	if err != nil {
		return nil, err
	}
	if g == nil {
		s := "not enough stored history for indicators"
		return &s, nil
	}
	neutral := models.Candidate{
		InstrumentID: instrument.ID, Action: models.ActionHold,
		Entry:      g.lastClose(),
		Qty:        math.Max(math.Floor(g.available/math.Max(g.lastClose(), 1e-9)), 0),
		TimingNote: instrument.Class.TimingNote(),
	}
	llmDeps := llm.LlmDeps{Pool: deps.Pool, Runtime: deps.Runtime, Transport: deps.Transport, Market: deps.Market}
	decision, err := llm.Run(&llmDeps, llm.RunInput{
		Instrument: &instrument, Ind: &g.ind, Candidate: neutral,
		Fundamentals: "", ShortContext: "", Mode: models.ModeLong,
		AvailableCash: g.available, HeldQty: heldQty(g.position),
		Limits: g.limits, RequiredProfitPct: g.requiredProfit,
		InstrumentExposure: g.instrumentExposure, Pot: g.pot,
		WindowHours: deps.Runtime.PromptWindowHours, Headlines: g.news,
		PriceWindow: g.priceWindow, DailySummary: g.dailySummary, Positions: g.positionsText,
		RecentTrades: g.recentTrades, LastDecision: g.lastDecision, Now: now,
	})
	if err != nil {
		return nil, err
	}
	var line string
	switch decision.Outcome.Kind {
	case llm.OutcomeLlm:
		p := decision.Outcome.Proposal
		line = fmt.Sprintf("%s %s entry %.2f stop %s target %s confidence %s · %s",
			instrument.Symbol, p.Action, p.Entry, fmtPtr(p.Stop), fmtPtr(p.Target), p.Confidence, p.Rationale)
	case llm.OutcomePass:
		line = fmt.Sprintf("%s PASS · %s", instrument.Symbol, decision.Outcome.Why)
	case llm.OutcomeRejected:
		p := decision.Outcome.Proposal
		line = fmt.Sprintf("%s model proposed %s entry %.2f stop %s target %s but it was rejected: %s",
			instrument.Symbol, p.Action, p.Entry, fmtPtr(p.Stop), fmtPtr(p.Target), decision.Outcome.Error.Error())
	default:
		line = fmt.Sprintf("%s no usable model answer, rules unchanged", instrument.Symbol)
	}
	return &line, nil
}

type sizing struct {
	available float64
	atr       float64
}

type finished struct {
	resolved       resolved
	sizing         sizing
	now            time.Time
	conversationID *string
}

func finish(deps *EngineDeps, instrument models.Instrument, done finished, report *PollReport) error {
	r := done.resolved
	// A HOLD is the absence of a decision, not a decision to record.
	if r.action != models.ActionBuy && r.action != models.ActionSell {
		return nil
	}
	if r.action == models.ActionBuy {
		cost := r.qty * r.entry
		fits, err := rules.Fits(deps.Pool, cost)
		if err != nil {
			return err
		}
		if !fits {
			if err := recordBlocked(deps.Pool, instrument, r, done.sizing.available, done.now, done.conversationID); err != nil {
				return err
			}
			report.Blocked++
			return nil
		}
	}

	rationale := "rules only"
	if r.rationale != nil {
		rationale = *r.rationale
	}
	until := "-"
	if r.validUntil != nil {
		until = *r.validUntil
	}
	report.Notes = append(report.Notes, fmt.Sprintf(
		"%s %s qty %v entry %.2f stop %s target %s until %s :: %s",
		instrument.Symbol, r.action, r.qty, r.entry, fmtPtr(r.stop), fmtPtr(r.target), until, rationale))

	if r.rationale == nil {
		reused, err := reuseModelSignal(deps.Pool, instrument.ID, r.entry, done.sizing.atr, done.now)
		if err != nil {
			return err
		}
		if reused {
			return nil
		}
	}

	if !deps.Force {
		prev, err := latestSignal(deps.Pool, instrument.ID)
		if err != nil {
			return err
		}
		if prev != nil {
			sameShape := prev.Action == string(r.action) &&
				stopRound(prev.Stop) == stopRound(r.stop) &&
				stopRound(prev.Target) == stopRound(r.target)
			if sameShape {
				if prev.Entry != nil && math.Abs(*prev.Entry-r.entry) >= 0.005 {
					if _, err := deps.Pool.Exec("UPDATE signals SET entry = ? WHERE id = ?", r.entry, prev.ID); err != nil {
						return err
					}
				}
				return nil
			}
		}
	}

	signalID, err := insertSignal(deps.Pool, instrument.ID, r, done.now, done.conversationID)
	if err != nil {
		return err
	}
	report.Signals++
	slog.Info("signal stored", "symbol", instrument.Symbol, "action", r.action, "qty", r.qty,
		"entry", r.entry, "stop", fmtPtr(r.stop), "target", fmtPtr(r.target),
		"signal_id", signalID)

	asCandidate := models.Candidate{
		InstrumentID: instrument.ID, Action: r.action, Entry: r.entry, Stop: r.stop,
		Target: r.target, Qty: r.qty, TimingNote: instrument.Class.TimingNote(),
	}
	if n := notify.FromCandidate(instrument.Symbol, instrument.Name, instrument.Currency, asCandidate); n != nil {
		n.SignalID = &signalID
		sent, err := notify.Dispatch(deps.Pool, deps.Notifier, *n)
		switch {
		case err != nil:
			report.Errors = append(report.Errors, fmt.Sprintf("%s: push failed: %v", instrument.Symbol, err))
		case sent:
			report.Notifications++
		case !deps.Notifier.Configured():
			report.Notes = append(report.Notes, fmt.Sprintf("%s: no ntfy topic — recorded without a push", instrument.Symbol))
		}
	}
	return nil
}

// Rules candidate is the spine; the LLM may change direction and prices, never quantity.
func resolve(candidate models.Candidate, outcome llm.Outcome, now time.Time) (resolved, bool) {
	validUntil := tstamp.From(now.Add(18 * time.Hour))
	switch outcome.Kind {
	case llm.OutcomeRules:
		c := outcome.Candidate
		return resolved{
			action: c.Action, entry: c.Entry, stop: c.Stop, target: c.Target,
			validUntil: &validUntil, qty: c.Qty, reasons: c.Reasons,
		}, true
	case llm.OutcomeLlm:
		r := fromProposal(candidate, outcome.Proposal, validUntil)
		if candidate.Urgent {
			r.action = candidate.Action
			r.entry = candidate.Entry
			r.qty = candidate.Qty
			r.stop = nil
			r.target = candidate.Target
		}
		return r, true
	case llm.OutcomeRejected:
		if outcome.Error != nil && outcome.Error.Kind == validate.KindInsufficientCash {
			return fromProposal(candidate, outcome.Proposal, validUntil), true
		}
		return resolved{
			action: candidate.Action, entry: candidate.Entry, stop: candidate.Stop,
			target: candidate.Target, validUntil: &validUntil, qty: candidate.Qty,
			reasons: candidate.Reasons,
		}, true
	default:
		return resolved{}, false
	}
}

func fromProposal(candidate models.Candidate, p *llm.Proposal, fallbackUntil string) resolved {
	qty := candidate.Qty
	if p.Qty != nil {
		qty = *p.Qty
	}
	validUntil := p.ValidUntil
	if validUntil == "" {
		validUntil = fallbackUntil
	}
	rationale := p.Rationale
	confidence := string(p.Confidence)
	return resolved{
		action: p.Action, qty: qty, entry: p.Entry, stop: p.Stop, target: p.Target,
		validUntil: &validUntil, reasons: candidate.Reasons, rationale: &rationale,
		confidence: &confidence,
	}
}

func loadInstruments(pool *sql.DB) ([]models.Instrument, error) {
	rows, err := pool.Query(
		"SELECT id, symbol, class, name, currency, active FROM instruments WHERE active = 1 ORDER BY id")
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []models.Instrument{}
	for rows.Next() {
		var i models.Instrument
		var classRaw string
		var active int64
		if err := rows.Scan(&i.ID, &i.Symbol, &classRaw, &i.Name, &i.Currency, &active); err != nil {
			return nil, err
		}
		class, err := models.ParseAssetClass(classRaw)
		if err != nil {
			return nil, err
		}
		i.Class = class
		i.Active = active != 0
		out = append(out, i)
	}
	return out, rows.Err()
}

// Cap a candidate's size to the pot's limits, or drop it if nothing usable is left.
func clampToLimits(c models.Candidate, limits rules.Limits, pot, exposure float64) (models.Candidate, bool) {
	if c.Action != models.ActionBuy && c.Action != models.ActionSell {
		return c, true
	}
	maxFill := limits.MaxFill(pot, exposure)
	if maxFill <= 0 {
		return c, false
	}
	maxUnits := maxFill / math.Max(c.Entry, 1e-9)
	if c.Qty > maxUnits {
		if c.Action == models.ActionSell {
			c.Qty = math.Min(c.Qty, maxUnits)
		} else {
			c.Qty = math.Floor(maxUnits)
		}
	}
	if c.Qty <= 0 {
		return c, false
	}
	// A buy below the minimum is not worth its fees; a sale of a tiny remainder still is.
	if c.Action == models.ActionBuy && c.Qty*c.Entry < limits.MinTradeAmount {
		return c, false
	}
	return c, true
}

func loadExposure(pool *sql.DB, instrumentID string) (float64, error) {
	var sum sql.NullFloat64
	err := pool.QueryRow(
		`SELECT SUM(qty * avg_cost) FROM positions
         WHERE instrument_id = ? AND mode = 'long' AND state <> 'closed'`, instrumentID,
	).Scan(&sum)
	return sum.Float64, err
}

func loadTargetProfit(pool *sql.DB, instrumentID string) (*float64, error) {
	var v sql.NullFloat64
	err := pool.QueryRow("SELECT target_profit_pct FROM instruments WHERE id = ?", instrumentID).Scan(&v)
	if err == sql.ErrNoRows || !v.Valid {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	return &v.Float64, nil
}

func loadPosition(pool *sql.DB, instrumentID string) (*models.Position, error) {
	var p models.Position
	var stop, target sql.NullFloat64
	err := pool.QueryRow(
		`SELECT id, instrument_id, qty, avg_cost, stop, target, state FROM positions
         WHERE instrument_id = ? AND state = 'open' ORDER BY id DESC LIMIT 1`, instrumentID,
	).Scan(&p.ID, &p.InstrumentID, &p.Qty, &p.AvgCost, &stop, &target, &p.State)
	if err == sql.ErrNoRows {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	if stop.Valid {
		p.Stop = &stop.Float64
	}
	if target.Valid {
		p.Target = &target.Float64
	}
	return &p, nil
}

// Refresh the entry on the last signal when it is still a live model decision. Returns false
// when the stored signal was rules-only, has expired, or the price left the band.
func reuseModelSignal(pool *sql.DB, instrumentID string, price, atr float64, now time.Time) (bool, error) {
	var id int64
	var entry sql.NullFloat64
	var validUntil, reasonJSON sql.NullString
	err := pool.QueryRow(
		`SELECT id, entry, valid_until, reason_json FROM signals
         WHERE instrument_id = ? AND action <> 'BLOCKED' ORDER BY id DESC LIMIT 1`, instrumentID,
	).Scan(&id, &entry, &validUntil, &reasonJSON)
	if err == sql.ErrNoRows {
		return false, nil
	}
	if err != nil {
		return false, err
	}
	fromModel := false
	if reasonJSON.Valid {
		var v map[string]any
		if json.Unmarshal([]byte(reasonJSON.String), &v) == nil {
			_, fromModel = v["rationale"]
		}
	}
	stillValid := false
	if validUntil.Valid {
		if d, err := time.Parse(time.RFC3339, validUntil.String); err == nil && d.After(now) {
			stillValid = true
		}
	}
	insideBand := entry.Valid && math.Abs(price-entry.Float64) <= math.Max(math.Abs(atr), 1e-9)
	if fromModel && stillValid && insideBand {
		_, err := pool.Exec("UPDATE signals SET entry = ? WHERE id = ?", price, id)
		return err == nil, err
	}
	return false, nil
}

func stopRound(v *float64) *int64 {
	if v == nil {
		return nil
	}
	r := int64(math.Round(*v * 100))
	return &r
}

type lastSignal struct {
	ID     int64
	Action string
	Entry  *float64
	Stop   *float64
	Target *float64
}

func latestSignal(pool *sql.DB, instrumentID string) (*lastSignal, error) {
	var s lastSignal
	var entry, stop, target sql.NullFloat64
	err := pool.QueryRow(
		`SELECT id, action, entry, stop, target FROM signals
         WHERE instrument_id = ? ORDER BY id DESC LIMIT 1`, instrumentID,
	).Scan(&s.ID, &s.Action, &entry, &stop, &target)
	if err == sql.ErrNoRows {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	if entry.Valid {
		s.Entry = &entry.Float64
	}
	if stop.Valid {
		s.Stop = &stop.Float64
	}
	if target.Valid {
		s.Target = &target.Float64
	}
	return &s, nil
}

func insertSignal(pool *sql.DB, instrumentID string, r resolved, now time.Time, conversationID *string) (string, error) {
	reason := mustJSON(map[string]any{"reasons": r.reasons, "rationale": r.rationale})
	var id string
	err := pool.QueryRow(
		`INSERT INTO signals (id, instrument_id, action, qty, entry, stop, target, valid_until,
            reason_json, created_at, confidence, conversation_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id`,
		ids.New(), instrumentID, string(r.action), r.qty, r.entry, r.stop, r.target, r.validUntil,
		reason, tstamp.From(now), r.confidence, conversationID,
	).Scan(&id)
	return id, err
}

func recordBlocked(pool *sql.DB, instrument models.Instrument, r resolved, available float64, now time.Time, conversationID *string) error {
	cost := r.qty * r.entry
	reason := mustJSON(map[string]any{
		"blocked": "insufficient_cash", "cost": cost, "available": available,
		"shortfall": cost - available, "reasons": r.reasons,
	})
	_, err := pool.Exec(
		`INSERT INTO signals (id, instrument_id, action, qty, entry, stop, target, valid_until,
            reason_json, created_at, conversation_id)
         VALUES (?, ?, 'BLOCKED', ?, ?, ?, ?, ?, ?, ?, ?)`,
		ids.New(), instrument.ID, r.qty, r.entry, r.stop, r.target, r.validUntil, reason,
		tstamp.From(now), conversationID,
	)
	return err
}

// A proposal the validator refused: recorded as not-actionable with the reason the model was
// told, so Audit and the ledger agree about what happened. No push — nothing to approve.
func recordRefused(pool *sql.DB, instrument models.Instrument, p *llm.Proposal, why string, now time.Time, conversationID *string) error {
	reason := mustJSON(map[string]any{
		"blocked": "the model's proposal was refused",
		"detail":  why,
		"proposed": map[string]any{
			"action": string(p.Action), "qty": p.Qty, "entry": p.Entry,
			"stop": p.Stop, "target": p.Target, "rationale": p.Rationale,
		},
	})
	_, err := pool.Exec(
		`INSERT INTO signals (id, instrument_id, action, qty, entry, stop, target, valid_until,
            reason_json, created_at, confidence, conversation_id)
         VALUES (?, ?, 'BLOCKED', ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
		ids.New(), instrument.ID, p.Qty, p.Entry, p.Stop, p.Target, p.ValidUntil, reason,
		tstamp.From(now), string(p.Confidence), conversationID,
	)
	return err
}

// 15-minute tape, oldest first, timestamps in IST.
func formatWindow(bars []models.Bar, hours uint32, now time.Time) string {
	cutoff := now.Add(-time.Duration(hours) * time.Hour)
	ist := time.FixedZone("IST", 330*60)
	lines := []string{}
	for _, b := range bars {
		d, err := time.Parse(time.RFC3339, b.Ts)
		if err != nil || d.Before(cutoff) {
			continue
		}
		lines = append(lines, fmt.Sprintf("%s %.2f %.2f %.2f %.2f %d",
			d.In(ist).Format("02 15:04"), b.Open, b.High, b.Low, b.Close, b.Volume))
	}
	if len(lines) == 0 {
		return "(no bars in the last window)"
	}
	return join(lines, "\n")
}

func formatDaily(bars []models.Bar, days int) string {
	tail := []string{}
	start := len(bars) - days
	if start < 0 {
		start = 0
	}
	for _, b := range bars[start:] {
		d := b.Ts
		if len(d) > 10 {
			d = d[:10]
		}
		tail = append(tail, fmt.Sprintf("%s %.2f", d, b.Close))
	}
	if len(tail) == 0 {
		return "(none)"
	}
	return join(tail, ", ")
}

// How many holdings a prompt will enumerate before summarising the rest.
const bookLines = 12

// One line per holding, richest first, with the instrument under review marked.
func FormatBook(rows []llm.HoldingLine, current string) string {
	if len(rows) == 0 {
		return "empty — nothing is held"
	}
	sorted := make([]llm.HoldingLine, len(rows))
	copy(sorted, rows)
	value := func(h llm.HoldingLine) float64 {
		if h.LTP != nil {
			return h.Qty * *h.LTP
		}
		return h.Qty * h.AvgCost
	}
	for i := 1; i < len(sorted); i++ {
		for j := i; j > 0 && value(sorted[j]) > value(sorted[j-1]); j-- {
			sorted[j], sorted[j-1] = sorted[j-1], sorted[j]
		}
	}
	out := ""
	for i, h := range sorted {
		if i == bookLines {
			break
		}
		ltp := "—"
		if h.LTP != nil {
			ltp = fmt.Sprintf("%.2f", *h.LTP)
		}
		pct := "P&L unknown"
		if p, ok := h.ProfitPct(); ok {
			pct = fmt.Sprintf("%+.1f%%", p)
		}
		out += fmt.Sprintf("%s %s: %.3f units at %.2f, now %s, %s, stop %s, target %s",
			h.Symbol, h.Name, h.Qty, h.AvgCost, ltp, pct, fmtOrNone(h.Stop), fmtOrNone(h.Target))
		if h.Symbol == current {
			out += "  ← this one"
		}
		out += "\n"
	}
	if len(sorted) > bookLines {
		out += fmt.Sprintf("+%d more not shown\n", len(sorted)-bookLines)
	}
	return out
}

func loadRecentTrades(pool *sql.DB, instrumentID string, now time.Time) (string, error) {
	cutoff := tstamp.From(now.AddDate(0, 0, -7))
	rows, err := pool.Query(
		`SELECT qty, avg_cost, opened_at, state FROM positions
         WHERE instrument_id = ? AND opened_at >= ? ORDER BY opened_at DESC`, instrumentID, cutoff,
	)
	if err != nil {
		return "", err
	}
	defer rows.Close()
	parts := []string{}
	for rows.Next() {
		var qty, cost float64
		var at, state string
		if err := rows.Scan(&qty, &cost, &at, &state); err != nil {
			return "", err
		}
		if len(at) > 10 {
			at = at[:10]
		}
		parts = append(parts, fmt.Sprintf("%s BUY %.3f @ %.2f (%s)", at, qty, cost, state))
	}
	if len(parts) == 0 {
		return "none in the last 7 days", nil
	}
	return join(parts, "; "), rows.Err()
}

func loadLastDecision(pool *sql.DB, instrumentID string) (string, error) {
	var action, at string
	var entry sql.NullFloat64
	var reason sql.NullString
	err := pool.QueryRow(
		`SELECT action, entry, created_at, reason_json FROM signals
         WHERE instrument_id = ? ORDER BY id DESC LIMIT 1`, instrumentID,
	).Scan(&action, &entry, &at, &reason)
	if err == sql.ErrNoRows {
		return "none", nil
	}
	if err != nil {
		return "", err
	}
	rationale := "rules only"
	if reason.Valid {
		var v map[string]any
		if json.Unmarshal([]byte(reason.String), &v) == nil {
			if s, ok := v["rationale"].(string); ok && s != "" {
				rationale = s
			}
		}
	}
	if len(at) > 10 {
		at = at[:10]
	}
	entryText := "-"
	if entry.Valid {
		entryText = fmt.Sprintf("%.2f", entry.Float64)
	}
	return fmt.Sprintf("%s %s @ %s — %s", at, action, entryText, rationale), nil
}

// The short-horizon card: computed from the daily bars and today's tape we already hold.
func shortCard(instrument models.Instrument, daily, intraday []models.Bar, now time.Time) string {
	if instrument.Class != models.ClassStock {
		return "" // a fund has no intraday horizon
	}
	card := rules.ComputeShortContext(daily, intraday, now)
	return card.Render()
}

func loadHoldings(pool *sql.DB, mode models.Mode) ([]llm.HoldingLine, error) {
	rows, err := pool.Query(
		`SELECT i.symbol, i.name, p.qty, p.avg_cost, p.stop, p.target,
                (SELECT b.close FROM bars b WHERE b.instrument_id = p.instrument_id
                 ORDER BY b.ts DESC LIMIT 1) AS ltp
         FROM positions p JOIN instruments i ON i.id = p.instrument_id
         WHERE p.state = 'open' AND p.mode = ?`, string(mode),
	)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []llm.HoldingLine{}
	for rows.Next() {
		var h llm.HoldingLine
		var stop, target, ltp sql.NullFloat64
		if err := rows.Scan(&h.Symbol, &h.Name, &h.Qty, &h.AvgCost, &stop, &target, &ltp); err != nil {
			return nil, err
		}
		if stop.Valid {
			h.Stop = &stop.Float64
		}
		if target.Valid {
			h.Target = &target.Float64
		}
		if ltp.Valid {
			h.LTP = &ltp.Float64
		}
		out = append(out, h)
	}
	return out, rows.Err()
}

// Ask for a swap when a buy is right but the cash is not there.
func rotateForCash(deps *EngineDeps, instrument models.Instrument, candidate models.Candidate, now time.Time) ([]int64, error) {
	if candidate.Action != models.ActionBuy {
		return nil, nil
	}
	available, err := rules.AvailableIn(deps.Pool, models.ModeLong)
	if err != nil {
		return nil, err
	}
	cost := rules.Cost(candidate.Qty, candidate.Entry, instrument.Class)
	if cost <= available+1e-9 {
		return nil, nil // it fits; nothing to sell
	}
	holdings, err := loadHoldings(deps.Pool, models.ModeLong)
	if err != nil {
		return nil, err
	}
	settings, err := rules.GetSettings(deps.Pool)
	if err != nil {
		return nil, err
	}
	profitable := false
	for _, h := range holdings {
		if p, ok := h.ProfitPct(); ok && p >= settings.LimitsLong.MinProfitPct {
			profitable = true
			break
		}
	}
	if !profitable {
		return nil, nil // nothing worth selling, so do not spend a call on it
	}
	table := llm.HoldingsTable(holdings, available, settings.Currency)
	wanted := fmt.Sprintf("%s (%s) at %.2f, %v units = %.2f %s",
		instrument.Symbol, instrument.Name, candidate.Entry, candidate.Qty, cost, settings.Currency)
	rendered, err := llm.RenderRotation(table, wanted)
	if err != nil {
		return nil, err
	}
	model, err := rules.ModelConfigFor(deps.Pool)
	if err != nil {
		return nil, err
	}
	calls, err := llm.DailyChatCalls(deps.Pool)
	if err != nil {
		return nil, err
	}
	if calls >= int64(deps.Runtime.DailyCap) {
		slog.Info("daily call cap reached — rotation not attempted")
		return nil, nil
	}
	trigger := fmt.Sprintf("rotation: %v units at %.2f costs %.2f, cash %.2f — looked for a profit to sell",
		candidate.Qty, candidate.Entry, cost, available)
	reply, err := llm.Chat(deps.Pool, &model, deps.Transport, llm.ChatRequest{
		ConversationID: fmt.Sprintf("rotate-%s-%d", instrument.Symbol, now.UnixMilli()),
		Round:          1,
		Prompt:         rendered.Text,
		PromptTemplate: rendered.Template,
		PromptHash:     rendered.Hash,
		Trigger:        &trigger,
		Purpose:        "rotate " + instrument.Symbol,
	})
	if err != nil {
		return nil, err
	}
	rotation, err := llm.ParseRotation(reply.Content)
	if err != nil {
		slog.Warn("rotation reply unreadable", "error", err)
		return nil, nil
	}
	if rotation.Pass {
		slog.Info("rotation declined", "why", rotation.Why)
		return nil, nil
	}
	if err := llm.ValidateRotation(rotation, holdings, available, settings.LimitsLong, models.ModeLong); err != nil {
		slog.Warn("rotation refused by the validator", "error", err)
		return nil, nil
	}
	swap := rotation.Swap
	sell, buy := swap.Sell, swap.Buy
	validUntil := tstamp.From(now.Add(18 * time.Hour))
	why := ""
	if sell.Why != nil {
		why = *sell.Why
	}
	var sellID int64
	err = deps.Pool.QueryRow(
		`INSERT INTO signals (id, instrument_id, action, qty, entry, valid_until, reason_json,
            created_at, confidence, mode, kind, status)
         SELECT ?, id, 'SELL', ?, ?, ?, ?, ?, ?, 'long', 'rotation', 'pending'
         FROM instruments WHERE symbol = ? RETURNING id`,
		sell.Qty, sell.Price, validUntil,
		mustJSON(map[string]any{"reasons": []string{why}}),
		tstamp.From(now), sell.Confidence, ids.New(), sell.Symbol,
	).Scan(&sellID)
	if err != nil {
		return nil, err
	}
	var buyID int64
	err = deps.Pool.QueryRow(
		`INSERT INTO signals (id, instrument_id, action, qty, entry, stop, target, valid_until,
            reason_json, created_at, confidence, mode, kind, status, pair_id)
         SELECT ?, id, 'BUY', ?, ?, ?, ?, ?, ?, ?, ?, 'long', 'rotation', 'pending', ?
         FROM instruments WHERE symbol = ? RETURNING id`,
		buy.Qty, buy.Price, buy.Stop, buy.Target, validUntil,
		mustJSON(map[string]any{
			"reasons": []string{}, "rationale": buy.Rationale, "paired_with": sell.Symbol,
		}),
		tstamp.From(now), buy.Confidence, sellID, ids.New(), buy.Symbol,
	).Scan(&buyID)
	if err != nil {
		return nil, err
	}
	return []int64{sellID, buyID}, nil
}

// Pos is re-exported so callers can render the ledger without importing ledger directly.
