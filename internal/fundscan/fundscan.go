// Package fundscan is best-fund discovery: a curated list of large equity funds, ranked by what
// they actually returned.
//
// Unlike stocks, there is no "gainers" feed for funds. The list is shipped in the repo; each
// fund's NAV history is fetched from mfapi.in (the AMFI mirror), and the top names by trailing
// 3-year and 1-year return are proposed against the mutual-fund earmark. Runs once a day.
package fundscan

import (
	"database/sql"
	"encoding/json"
	"fmt"
	"log/slog"
	"net/http"
	"sort"
	"strings"
	"time"

	"stock-predictor/internal/market"
	"stock-predictor/internal/models"
	"stock-predictor/internal/rules"
	"stock-predictor/internal/schedule"
	"stock-predictor/internal/store"
	"stock-predictor/internal/tstamp"
)

// Curated large-cap / flexi / ELSS equity funds, by AMFI scheme code. Extend this list to widen
// the universe; nothing else needs to change.
var Funds = [][2]string{
	{"122639", "Parag Parikh Flexi Cap Fund"},
	{"118955", "HDFC Flexi Cap Fund"},
	{"119723", "SBI ELSS Tax Saver Fund"},
	{"129046", "Motilal Oswal Flexi Cap Fund"},
	{"120843", "Quant Flexi Cap Fund"},
	{"119076", "DSP Flexi Cap Fund"},
	{"120503", "Axis ELSS Tax Saver Fund"},
	{"119292", "Union Flexi Cap Fund"},
	{"144546", "Tata Flexi Cap Fund"},
	{"118535", "Franklin India Flexi Cap Fund"},
}

const TopN = 3

type Report struct {
	Examined int
	Ranked   int
	Proposed int
	Notes    []string
}

func Run(pool *sql.DB, rt *rules.Runtime, client *http.Client, now time.Time) (Report, error) {
	report := Report{}
	today := schedule.ISTDay(now)
	var last string
	err := pool.QueryRow("SELECT value FROM settings WHERE key = 'fundscan_last'").Scan(&last)
	if err == nil && last == today {
		return report, nil // already ran today
	}
	if err != nil && err != sql.ErrNoRows {
		return report, err
	}

	type ranked struct {
		code    string
		name    string
		score   float64
		history market.FundHistory
	}
	histories := []ranked{}
	for _, fund := range Funds {
		code := fund[0]
		report.Examined++
		h, err := market.FetchFundHistory(client, code)
		if err != nil {
			slog.Warn("fund history unavailable", "code", code, "error", err)
			report.Notes = append(report.Notes, fmt.Sprintf("%s: %v", code, err))
			continue
		}
		if len(h.Navs) < 30 {
			slog.Debug("fund skipped: too little history", "code", code, "points", len(h.Navs))
			report.Notes = append(report.Notes, fmt.Sprintf("%s: only %d NAV points", code, len(h.Navs)))
			continue
		}
		// Rank by 3-year, falling back to 1-year when the scheme is younger.
		if r3, ok := h.TrailingReturn(3.0); ok {
			histories = append(histories, ranked{code, h.Name, r3, h})
		} else if r1, ok := h.TrailingReturn(1.0); ok {
			histories = append(histories, ranked{code, h.Name, r1, h})
		}
	}

	sort.SliceStable(histories, func(i, j int) bool { return histories[i].score > histories[j].score })

	// Only a fund you hold gets a chart: a proposal is not in the book, and storing 3,400 points
	// for one is history nobody asked for.
	held := map[string]int64{}
	rows, err := pool.Query(
		`SELECT DISTINCT i.id, i.symbol FROM positions p
         JOIN instruments i ON i.id = p.instrument_id
         WHERE i.class = 'mutual_fund'`)
	if err != nil {
		return report, err
	}
	for rows.Next() {
		var id int64
		var symbol string
		if err := rows.Scan(&id, &symbol); err != nil {
			rows.Close()
			return report, err
		}
		held[symbol] = id
	}
	rows.Close()
	for _, h := range histories {
		if id, ok := held[h.code]; ok {
			if err := saveNAVHistory(pool, id, h.history); err != nil {
				return report, err
			}
		}
	}
	report.Ranked = len(histories)

	earmark, err := rules.MFAllowance(pool)
	if err != nil {
		return report, err
	}
	if earmark <= 0 {
		report.Notes = append(report.Notes,
			"no mutual-fund allowance configured — the scan is not proposing")
		return report, nil
	}

	heldCategories := []string{}
	catRows, err := pool.Query(
		`SELECT i.name FROM positions p JOIN instruments i ON i.id = p.instrument_id
         WHERE p.mode = 'long' AND p.state <> 'closed' AND i.class = 'mutual_fund'`)
	if err != nil {
		return report, err
	}
	for catRows.Next() {
		var name string
		if err := catRows.Scan(&name); err != nil {
			catRows.Close()
			return report, err
		}
		heldCategories = append(heldCategories, name)
	}
	catRows.Close()

	proposedCategories := []string{}
	limit := TopN
	if len(histories) < limit {
		limit = len(histories)
	}
	for _, h := range histories[:limit] {
		// Overlap is the cheapest real filter a fund can have: a second flexi cap is the same bet.
		category := h.history.Category
		if clashes(category, heldCategories) || clashes(category, proposedCategories) {
			label := "similar"
			if category != "" {
				label = category
			}
			report.Notes = append(report.Notes,
				fmt.Sprintf("%s: skipped — you already hold a %s fund", h.name, label))
			continue
		}
		proposedCategories = append(proposedCategories, h.name)
		// The latest NAV is the entry; a fund decision has no stop or target (NAV execution).
		if len(h.history.Navs) == 0 {
			continue
		}
		nav, _ := h.history.Navs[len(h.history.Navs)-1][1].(float64)
		var already int
		if err := pool.QueryRow(
			`SELECT COUNT(*) FROM signals WHERE kind = 'fundscan' AND reason_json LIKE ? AND created_at >= ?`,
			"%"+h.code+"%", tstamp.From(now),
		).Scan(&already); err != nil {
			return report, err
		}
		if already > 0 {
			continue
		}
		id, err := ensureInstrument(pool, h.code, h.name)
		if err != nil {
			return report, err
		}
		if err := storeSignal(pool, id, h.name, h.code, nav, h.score, now); err != nil {
			return report, err
		}
		report.Proposed++
		report.Notes = append(report.Notes,
			fmt.Sprintf("%s: +%.1f%% over the window, NAV %.2f — proposed", h.name, h.score, nav))
	}

	if _, err := pool.Exec(
		`INSERT INTO settings (key, value, updated_at) VALUES ('fundscan_last', ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at`,
		today, tstamp.From(now),
	); err != nil {
		return report, err
	}
	slog.Info("fund scan complete", "examined", report.Examined, "ranked", report.Ranked,
		"proposed", report.Proposed, "day", today)
	return report, nil
}

func clashes(category string, existing []string) bool {
	if category == "" {
		return false
	}
	c := strings.ToLower(category)
	for _, name := range existing {
		e := strings.ToLower(name)
		for _, kind := range []string{"flexi", "elss", "large cap", "mid cap", "small cap"} {
			if strings.Contains(c, kind) && strings.Contains(e, kind) {
				return true
			}
		}
	}
	return false
}

// The fund's instrument row, created inactive so the scan never starts polling it.
func ensureInstrument(pool *sql.DB, code, name string) (int64, error) {
	symbol := "AMFI:" + code
	var id int64
	err := pool.QueryRow("SELECT id FROM instruments WHERE symbol = ?", symbol).Scan(&id)
	if err == nil {
		return id, nil
	}
	if err != sql.ErrNoRows {
		return 0, err
	}
	err = pool.QueryRow(
		`INSERT INTO instruments (symbol, class, name, currency, active)
         VALUES (?, 'mutual_fund', ?, 'INR', 0) RETURNING id`, symbol, name,
	).Scan(&id)
	return id, err
}

// The NAV series as daily bars. A fund has no OHLC, so every price is the NAV and volume is 0.
func saveNAVHistory(pool *sql.DB, instrumentID int64, h market.FundHistory) error {
	return store.Bars(pool, instrumentID, NAVBars(h))
}

func NAVBars(h market.FundHistory) []models.Bar {
	out := make([]models.Bar, 0, len(h.Navs))
	for _, p := range h.Navs {
		date, _ := p[0].(string)
		nav, _ := p[1].(float64)
		out = append(out, models.Bar{
			Ts: date + "T00:00:00Z", Open: nav, High: nav, Low: nav, Close: nav, Volume: 0,
		})
	}
	return out
}

func storeSignal(pool *sql.DB, instrumentID int64, name, code string, nav, score float64, now time.Time) error {
	reason := mustJSON(map[string]any{
		"reasons":     []string{fmt.Sprintf("%s returned +%.1f%% over the trailing window", name, score)},
		"rationale":   "A six-month-plus commitment: judged on trailing returns, with the fund earmark in mind.",
		"source":      "mfapi",
		"scheme_code": code,
	})
	_, err := pool.Exec(
		`INSERT INTO signals (instrument_id, action, qty, entry, valid_until, reason_json,
            created_at, confidence, mode, kind, status)
         VALUES (?, 'BUY', ?, ?, ?, ?, ?, NULL, 'long', 'fundscan', 'pending')`,
		instrumentID, 0.0, nav, tstamp.From(now.AddDate(0, 0, 7)), reason, tstamp.From(now),
	)
	return err
}

func mustJSON(v any) string {
	b, err := json.Marshal(v)
	if err != nil {
		slog.Warn("fundscan: reason could not be encoded", "error", err)
		return "{}"
	}
	return string(b)
}
