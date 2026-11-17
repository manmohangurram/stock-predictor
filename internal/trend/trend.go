// Package trend is short-term mode's evidence: what moved today, and whether it keeps moving.
//
// The list itself is asked once per IST day from the exchange and stored in trend_candidates:
// the day's gainers and the Nifty 50's own movers, both direct NSE APIs. A tick reads that list
// rather than re-asking the exchange. Duplicates are one row — the first source that names a
// symbol keeps it, and the exchange's row wins because it carries a price.
package trend

import (
	"database/sql"
	"fmt"
	"log/slog"
	"net/http"
	"sort"
	"strings"
	"time"

	"stock-predictor/internal/httpx"
	"stock-predictor/internal/llm"
	"stock-predictor/internal/market"
	"stock-predictor/internal/models"
	"stock-predictor/internal/rules"
	"stock-predictor/internal/schedule"
	"stock-predictor/internal/store"
	"stock-predictor/internal/tstamp"
)

const TopN = 10

// How old a search result may be and still count as news. The API has no date filter — the only
// lever is the date on each result — so this is where "today's reason" is separated from a page
// that merely has the stock's name in its title.
const NewsWindowDays = 3

// The one query the daily scan searches. Broad on purpose — the model reads the results and picks.

type Report struct {
	Examined int
	Called   int
	Proposed int
	// Names the model looked at and refused. A decision with nothing to approve.
	Passed int
	// One line per proposal, ready to put in a notification.
	Ideas    []string
	Notes    []string
	Notified int
}

// One name on today's list, from whichever of the three sources named it first.
type candidate struct {
	// Yahoo form: TRANSWORLD.NS.
	symbol string
	source string
	// One line: why it is on the list. The exchange's sources have none, the search has.
	why           string
	ltp           *float64
	changePct     *float64
	previousClose *float64
	turnover      *float64
}

// What the scan may offer, before the model is asked anything. Three numbers from Settings: a
// price floor, a turnover floor, and the locked-band cut-off.
func worthOffering(c candidate, minPrice, minTurnover, lockedPct float64) bool {
	// A name with no exchange price yet is left to the model: it has no numbers to judge by here.
	if c.ltp == nil || c.turnover == nil || c.changePct == nil {
		return true
	}
	change := *c.changePct
	if change < 0 {
		change = -change
	}
	return *c.ltp >= minPrice && *c.turnover >= minTurnover && change < lockedPct
}

func fromGainer(g market.Gainer, source string) candidate {
	turnover := g.Turnover
	return candidate{
		symbol: g.YahooSymbol(), source: source,
		ltp: &g.LTP, changePct: &g.PerChange, previousClose: &g.PreviousClose,
		turnover: turnover,
	}
}

// The bare NSE ticker: what NSE writes, and what a search query wants.
func (c candidate) shortName() string {
	if idx := strings.Index(c.symbol, "."); idx >= 0 {
		return c.symbol[:idx]
	}
	return c.symbol
}

// The one line the prompt opens with. Exchange names have prices; discovered ones do not.
func (c candidate) describe() string {
	if c.ltp != nil && c.changePct != nil && c.previousClose != nil {
		turnover := 0.0
		if c.turnover != nil {
			turnover = *c.turnover
		}
		return fmt.Sprintf("%.2f on the exchange feed, %+.2f%% today, previous close %.2f, turnover %.0f",
			*c.ltp, *c.changePct, *c.previousClose, turnover)
	}
	return "on today's scan; no exchange price for it, so the tape below is the price"
}

// True when the feed's price is not in the same world as our own last close. A stale snapshot and
// a genuine move look identical to a model reading two numbers, so the prompt says which of the
// two is suspect instead of letting it conclude the stock "is" that price.
func (c candidate) disagreesWith(lastClose *float64) string {
	if c.ltp == nil || lastClose == nil || *lastClose <= 0 {
		return ""
	}
	feed, close := *c.ltp, *lastClose
	gap := abs(feed-close) / close
	if gap <= 0.05 {
		return ""
	}
	return fmt.Sprintf(
		"the feed says %.2f while our last close was %.2f (%+.1f%%) — one of the two is stale; trust the tape, and pass if the setup depends on the feed's number",
		feed, close, (feed/close-1)*100)
}

// Remove the names a plan call looked at and never used: inactive, never archived, and with no
// decision, no fill and no position.
func SweepGlances(pool *sql.DB) (int64, error) {
	rows, err := pool.Query(
		`SELECT i.id FROM instruments i
          WHERE i.active = 0
            AND i.archived_at IS NULL
            AND NOT EXISTS (SELECT 1 FROM signals   s WHERE s.instrument_id = i.id)
            AND NOT EXISTS (SELECT 1 FROM trades    t WHERE t.instrument_id = i.id)
            AND NOT EXISTS (SELECT 1 FROM positions p WHERE p.instrument_id = i.id)`)
	if err != nil {
		return 0, err
	}
	ids := []int64{}
	for rows.Next() {
		var id int64
		if err := rows.Scan(&id); err != nil {
			rows.Close()
			return 0, err
		}
		ids = append(ids, id)
	}
	rows.Close()
	if err := rows.Err(); err != nil {
		return 0, err
	}
	dropped := int64(0)
	for _, id := range ids {
		tx, err := pool.Begin()
		if err != nil {
			return dropped, err
		}
		for _, q := range []string{
			"DELETE FROM bars WHERE instrument_id = ?",
			"DELETE FROM news WHERE instrument_id = ?",
			"DELETE FROM llm_memos WHERE instrument_id = ?",
			"DELETE FROM trend_candidates WHERE symbol = (SELECT symbol FROM instruments WHERE id = ?)",
			"DELETE FROM instruments WHERE id = ?",
		} {
			if _, err := tx.Exec(q, id); err != nil {
				tx.Rollback()
				return dropped, err
			}
		}
		if err := tx.Commit(); err != nil {
			return dropped, err
		}
		dropped++
	}
	if dropped == 0 {
		return 0, nil
	}
	slog.Info("swept unused glances", "dropped", dropped)
	return dropped, nil
}

// The trade plan's list: what moved today, with the tape and today's dated web evidence for the
// few names worth a look. One call, one book — the shape in which "sell A to buy B" is possible.
//
// Evidence for four names, not twenty: the model can only act on two moves.
func CandidateBlock(pool *sql.DB, m market.MarketSource, transport llm.Transport, now time.Time) string {
	const evidence = 4
	rt, err := rules.LoadRuntime(pool)
	if err != nil {
		rt = rules.DefaultRuntime()
	}
	client, err := httpx.ClientWith(&rt, 20, "Mozilla/5.0 stock-predictor")
	if err != nil {
		return "no list today — the exchange could not be reached\n"
	}
	report := Report{}
	filters := rt
	list, err := dailyList(pool, client, now, &report)
	if err != nil {
		list = nil
	}
	kept := []candidate{}
	held := tradeState(pool)
	for _, c := range list {
		keep := worthOffering(c, filters.ScanMinPrice, filters.ScanMinTurnover, filters.ScanLockedPct)
		if !keep {
			report.Notes = append(report.Notes, fmt.Sprintf(
				"%s left out: under %.0f rupees, under %.0f turnover, or locked at the band",
				c.symbol, filters.ScanMinPrice, filters.ScanMinTurnover))
			continue
		}
		// A name already held, or already waiting to be bought, is not a new idea. Offering it
		// again is how one up-move becomes three buys at higher entries and unreachable targets.
		if held[c.symbol] {
			report.Notes = append(report.Notes, c.symbol+" left out: already held or under review")
			continue
		}
		kept = append(kept, c)
	}
	if len(kept) == 0 {
		notes := ""
		if len(report.Notes) > 0 {
			notes = " (" + strings.Join(report.Notes, "; ") + ")"
		}
		slog.Info("trade candidates: none", "listed", len(list), "notes", notes)
		return "no candidates today" + notes + "\n"
	}
	slog.Info("trade candidates", "listed", len(list), "kept", len(kept), "notes", len(report.Notes))
	out := ""
	for i, g := range kept {
		out += fmt.Sprintf("%d. %s · %s · %s\n", i+1, g.symbol, g.describe(), scanNote(g.why))
		if i >= evidence {
			continue
		}
		// The evidence only for the ones the model might actually act on.
		id, ok, err := ensureInstrument(pool, g.symbol, g.shortName())
		if err != nil || !ok {
			continue
		}
		if !hasBarsToday(pool, id, now) {
			if _, _, err := store.Refresh(pool, id, g.symbol, m); err != nil {
				continue
			}
		}
		var knownName *string
		var name string
		if err := pool.QueryRow("SELECT name FROM instruments WHERE id = ?", id).Scan(&name); err == nil {
			if strings.TrimSpace(name) != "" && !strings.EqualFold(g.shortName(), strings.TrimSpace(name)) {
				knownName = &name
			}
		}
		bars, err := loadBars(pool, id)
		if err != nil {
			continue
		}
		out += fmt.Sprintf("   tape: %s\n", strings.ReplaceAll(tape(bars), "\n", "; "))
		var lastClose *float64
		if len(bars) > 0 {
			last := bars[len(bars)-1].Close
			lastClose = &last
		}
		if warn := g.disagreesWith(lastClose); warn != "" {
			out += fmt.Sprintf("   feed vs tape: %s\n", warn)
		}
		news, _ := recentHeadlines(pool, id)
		if len(news) > 0 {
			out += fmt.Sprintf("   headlines: %s\n", strings.ReplaceAll(renderHeadlines(news), "\n", " | "))
		}
		if model, err := rules.ModelConfigFor(pool); err == nil {
			query := datedQuery(newsQuery(g.shortName(), knownName), now)
			if hits, err := llm.SearchFor(pool, model.BaseURL, transport, query, "search "+g.symbol); err == nil {
				recent := relevant(recentHits(hits, now), g.shortName(), knownName)
				if len(recent) > 0 {
					out += fmt.Sprintf("   web: %s\n", strings.ReplaceAll(llm.RenderWebHits(recent), "\n", " "))
				}
			}
		}
	}
	out += fmt.Sprintf("\n(evidence supplied for the first %d; the rest are prices only)\n", evidence)
	return out
}

// Today's list: read from the table if it was asked, otherwise asked and stored.
//
// Once per IST day, and only once: two exchange calls, and a tick every fifteen minutes must not
// pay that repeatedly. A failed source is a note, never a reason to lose the whole list.
func dailyList(pool *sql.DB, client *http.Client, now time.Time, report *Report) ([]candidate, error) {
	today := schedule.ISTDay(now)
	stored, err := loadCandidates(pool, today)
	if err != nil {
		return nil, err
	}
	if len(stored) > 0 {
		return stored, nil
	}

	out := []candidate{}
	if gainers, index, err := market.FetchToday(client, TopN); err == nil {
		slog.Info("exchange movers fetched", "gainers", len(gainers), "nifty50", len(index))
		for _, g := range gainers {
			out = append(out, fromGainer(g, "gainers"))
		}
		for _, g := range index {
			out = append(out, fromGainer(g, "nifty50"))
		}
	} else {
		slog.Warn("exchange lists unavailable", "error", err)
		report.Notes = append(report.Notes, fmt.Sprintf("exchange lists unavailable: %v", err))
	}

	out = dedupe(out)
	slog.Info("daily trade list built", "day", today, "candidates", len(out))
	for _, c := range out {
		if err := storeCandidate(pool, today, c, now); err != nil {
			return nil, err
		}
	}
	if err := setMarker(pool, "trend_list_last", today); err != nil {
		return nil, err
	}
	return out, nil
}

// One row per symbol, first source wins: the exchange's rows carry a price, so they are pushed
// first, and a search hit that the exchange already named adds nothing.
func dedupe(rows []candidate) []candidate {
	seen := map[string]bool{}
	out := []candidate{}
	for _, c := range rows {
		if seen[c.symbol] {
			continue
		}
		seen[c.symbol] = true
		out = append(out, c)
	}
	sort.SliceStable(out, func(i, j int) bool {
		return changeOf(out[i]) > changeOf(out[j])
	})
	if len(out) > TopN*3 {
		out = out[:TopN*3]
	}
	return out
}

func changeOf(c candidate) float64 {
	if c.changePct == nil {
		return -999.0
	}
	return *c.changePct
}

// tradeState is the set of symbols the trade book already holds, or already has a pending BUY
// decision for. Neither is a new idea, and offering it again invites a second entry at a worse
// price.
func tradeState(pool *sql.DB) map[string]bool {
	out := map[string]bool{}
	rows, err := pool.Query(
		`SELECT i.symbol FROM positions p JOIN instruments i ON i.id = p.instrument_id
         WHERE p.mode = 'trade' AND p.state = 'open'
         UNION
         SELECT i.symbol FROM signals s JOIN instruments i ON i.id = s.instrument_id
         WHERE s.mode = 'trade' AND s.action = 'BUY' AND s.status = 'pending'`)
	if err != nil {
		return out
	}
	defer rows.Close()
	for rows.Next() {
		var s string
		if rows.Scan(&s) == nil {
			out[s] = true
		}
	}
	return out
}

// Keep only results dated inside the window. Undated pages go too: a quote page or a filings
// index has no date because it is not an article, and a blank date reads as "current".
func recentHits(hits []llm.SearchHit, now time.Time) []llm.SearchHit {
	cutoff := now.AddDate(0, 0, -int(NewsWindowDays)).Format("2006-01-02")
	out := []llm.SearchHit{}
	for _, h := range hits {
		if h.Date == nil {
			continue
		}
		day := dayOf(*h.Date)
		if day != "" && day >= cutoff {
			out = append(out, h)
		}
	}
	return out
}

// What to search for, given a ticker and (when known) the company's name.
//
// The ticker alone is not enough: search engines split ANTGRAPHIC into "ant" and hand back
// Anthropic's IPO. The name, quoted, is what a news page actually writes.
func newsQuery(ticker string, name *string) string {
	if name != nil && strings.TrimSpace(*name) != "" {
		return fmt.Sprintf("%q OR %s share news", strings.TrimSpace(*name), ticker)
	}
	return ticker + " share news"
}

// Keep only the hits that are about this stock: the ticker or the company's name must appear in
// the title or the snippet. A result that mentions neither is a different company.
func relevant(hits []llm.SearchHit, ticker string, name *string) []llm.SearchHit {
	needle := func(haystack string) bool {
		lower := strings.ToLower(haystack)
		if strings.Contains(lower, strings.ToLower(ticker)) {
			return true
		}
		if name != nil {
			words := strings.Fields(*name)
			if len(words) > 2 {
				words = words[:2]
			}
			for _, word := range words {
				if len(word) >= 4 && strings.Contains(lower, strings.ToLower(word)) {
					return true
				}
			}
		}
		return false
	}
	out := []llm.SearchHit{}
	for _, h := range hits {
		if needle(h.Title) || needle(h.Snippet) {
			out = append(out, h)
		}
	}
	return out
}

// The window, pushed into the query itself: after: and before: are real operators. lang:en because
// a source that is not in English is not one this book can read.
func datedQuery(query string, now time.Time) string {
	from := now.AddDate(0, 0, -int(NewsWindowDays)).Format("2006-01-02")
	return query + " after:" + from + " lang:en"
}

// 2026-11-08T16:00:00+00:00 → 2026-11-08. The time of day is not the question; the day is.
func dayOf(raw string) string {
	if len(raw) < 10 {
		return ""
	}
	if _, err := time.Parse("2006-01-02", raw[:10]); err != nil {
		return ""
	}
	return raw[:10]
}

// The scan's line, marked as what it is: a reading of the web by another model, which may quote a
// price that is days old. The tape below it is ours and is the only price that counts.
func scanNote(why string) string {
	if strings.TrimSpace(why) == "" {
		return "no note from the scan"
	}
	return "scan's note, unverified — do not take a price from it: " + why
}

// hasBarsToday is the evidence cache: a candidate whose prices were fetched since the open needs
// no second chart request for the same slot.
func hasBarsToday(pool *sql.DB, instrumentID int64, now time.Time) bool {
	var ts string
	if err := pool.QueryRow(
		"SELECT ts FROM bars WHERE instrument_id = ? ORDER BY ts DESC LIMIT 1", instrumentID,
	).Scan(&ts); err != nil {
		return false
	}
	return len(ts) >= 10 && ts[:10] == schedule.ISTDay(now)
}

func loadCandidates(pool *sql.DB, day string) ([]candidate, error) {
	rows, err := pool.Query(
		`SELECT symbol, source, why, ltp, change_pct, previous_close, turnover
         FROM trend_candidates WHERE day = ?
         ORDER BY COALESCE(change_pct, -999) DESC, symbol`, day)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []candidate{}
	for rows.Next() {
		var c candidate
		var ltp, change, prev, turnover sql.NullFloat64
		if err := rows.Scan(&c.symbol, &c.source, &c.why, &ltp, &change, &prev, &turnover); err != nil {
			return nil, err
		}
		if ltp.Valid {
			c.ltp = &ltp.Float64
		}
		if change.Valid {
			c.changePct = &change.Float64
		}
		if prev.Valid {
			c.previousClose = &prev.Float64
		}
		if turnover.Valid {
			c.turnover = &turnover.Float64
		}
		out = append(out, c)
	}
	return out, rows.Err()
}

func storeCandidate(pool *sql.DB, day string, c candidate, now time.Time) error {
	_, err := pool.Exec(
		`INSERT INTO trend_candidates (day, symbol, source, why, ltp, change_pct, previous_close, turnover, added_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(day, symbol) DO NOTHING`,
		day, c.symbol, c.source, c.why, c.ltp, c.changePct, c.previousClose, c.turnover, tstamp.From(now),
	)
	return err
}

func setMarker(pool *sql.DB, key, value string) error {
	_, err := pool.Exec(
		`INSERT INTO settings (key, value, updated_at) VALUES (?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at`,
		key, value, tstamp.Now(),
	)
	return err
}

// The instrument for a discovered symbol: found, or created for this tick only. The bool says the
// row is available to use; a symbol already held in the trade pot returns false.
func ensureInstrument(pool *sql.DB, yahooSymbol, shortName string) (int64, bool, error) {
	var id int64
	err := pool.QueryRow("SELECT id FROM instruments WHERE symbol = ?", yahooSymbol).Scan(&id)
	if err == nil {
		var held sql.NullFloat64
		if err := pool.QueryRow(
			`SELECT CAST(COALESCE(SUM(qty),0) AS REAL) FROM positions
             WHERE instrument_id = ? AND state <> 'closed' AND mode = 'trade'`, id,
		).Scan(&held); err != nil {
			return 0, false, err
		}
		return id, !held.Valid || held.Float64 <= 0, nil
	}
	if err != sql.ErrNoRows {
		return 0, false, err
	}
	err = pool.QueryRow(
		`INSERT INTO instruments (symbol, class, name, currency, active)
         VALUES (?, 'stock', ?, 'INR', 0) RETURNING id`, yahooSymbol, shortName,
	).Scan(&id)
	if err != nil {
		return 0, false, err
	}
	return id, true, nil
}

func loadBars(pool *sql.DB, instrumentID int64) ([]models.Bar, error) {
	rows, err := pool.Query(
		`SELECT ts, COALESCE(open, close), COALESCE(high, close), COALESCE(low, close), close,
                COALESCE(volume, 0)
         FROM bars WHERE instrument_id = ? ORDER BY ts DESC LIMIT 60`, instrumentID)
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
	// Oldest first.
	for i, j := 0, len(bars)-1; i < j; i, j = i+1, j-1 {
		bars[i], bars[j] = bars[j], bars[i]
	}
	return bars, rows.Err()
}

func recentHeadlines(pool *sql.DB, instrumentID int64) ([][2]string, error) {
	rows, err := pool.Query(
		`SELECT title, published_at FROM news WHERE instrument_id = ?
         ORDER BY published_at DESC LIMIT 4`, instrumentID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := [][2]string{}
	for rows.Next() {
		var title, at string
		if err := rows.Scan(&title, &at); err != nil {
			return nil, err
		}
		out = append(out, [2]string{title, at})
	}
	return out, rows.Err()
}

func renderHeadlines(rows [][2]string) string {
	if len(rows) == 0 {
		return "(no headlines)"
	}
	parts := make([]string, len(rows))
	for i, row := range rows {
		at := row[1]
		if len(at) > 10 {
			at = at[:10]
		}
		parts[i] = "[" + at + "] " + row[0]
	}
	return strings.Join(parts, "\n")
}

// Newest first, at most eight daily closes.
func tape(bars []models.Bar) string {
	parts := []string{}
	for i := len(bars) - 1; i >= 0 && len(parts) < 8; i-- {
		day := bars[i].Ts
		if len(day) > 10 {
			day = day[:10]
		}
		parts = append(parts, fmt.Sprintf("%s close %.2f", day, bars[i].Close))
	}
	return strings.Join(parts, "\n")
}

func abs(v float64) float64 {
	if v < 0 {
		return -v
	}
	return v
}
