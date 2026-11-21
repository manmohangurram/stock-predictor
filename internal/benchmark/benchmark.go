// Package benchmark is the long-term book's candidate list: the Nifty 100, ranked by their last
// five sessions.
//
// NSE refuses equity-stockIndices to a non-browser client, so the index cannot be asked for its
// own performance. What is available is the constituent CSV and one daily chart per symbol — a
// hundred requests once an IST day, kept in plan_candidates so a retry costs nothing.
package benchmark

import (
	"database/sql"
	"fmt"
	"log/slog"
	"sort"
	"strings"
	"time"

	"stock-predictor/internal/market"
	"stock-predictor/internal/schedule"
	"stock-predictor/internal/tstamp"
)

// The sessions the ranking is measured over. Five is a week of Indian trading, long enough that
// a single gap-up cannot carry a name into the list on its own.
const Sessions = 5

// How many names the plan is offered.
const Top = 10

type Pick struct {
	Symbol string
	Name   string
	Close  float64
	Ret5   float64
}

// The one line the plan prompt reads.
func (p Pick) Line(currency string) string {
	return fmt.Sprintf("%s %s at %.2f %s, %+.2f%% over %d sessions",
		p.Symbol, p.Name, p.Close, currency, p.Ret5*100.0, Sessions)
}

// Company Name,Industry,Symbol,Series,ISIN Code — the symbol is bare (ABB), Yahoo wants .NS.
func ParseConstituents(csv string) []market.Constituent {
	out := []market.Constituent{}
	lines := strings.Split(csv, "\n")
	for i, line := range lines {
		if i == 0 {
			continue
		}
		cells := strings.Split(line, ",")
		if len(cells) < 3 {
			continue
		}
		name := strings.TrimSpace(cells[0])
		symbol := strings.TrimSpace(cells[2])
		if symbol == "" || strings.EqualFold(symbol, "symbol") {
			continue
		}
		out = append(out, market.Constituent{Symbol: symbol + ".NS", Name: name})
	}
	return out
}

// Return over the last sessions closes, or false when there is not enough history. A name with
// four closes is not ranked on three of them and called a trend.
func ReturnOver(closes []float64, sessions int) (float64, bool) {
	if len(closes) <= sessions {
		return 0, false
	}
	last := closes[len(closes)-1]
	window := closes[len(closes)-1-sessions:]
	if isNaN(last) || isInf(last) {
		return 0, false
	}
	for _, c := range window {
		if isNaN(c) || isInf(c) || c <= 0 {
			return 0, false
		}
	}
	return last/window[0] - 1.0, true
}

// Fetch the closes and compute the return. A symbol whose chart fails is skipped, not fatal: one
// suspended name must not cost the whole day's list.
func Measure(m market.MarketSource, symbol, name string) (Pick, bool) {
	bars, err := m.Bars(symbol)
	if err != nil || len(bars) == 0 {
		return Pick{}, false
	}
	closes := make([]float64, len(bars))
	for i, b := range bars {
		closes[i] = b.Close
	}
	ret, ok := ReturnOver(closes, Sessions)
	if !ok {
		return Pick{}, false
	}
	return Pick{Symbol: symbol, Name: name, Close: closes[len(closes)-1], Ret5: ret}, true
}

// Positive first, then ranked, then cut to top.
func Rank(picks []Pick, top int) []Pick {
	kept := []Pick{}
	for _, p := range picks {
		if p.Ret5 > 0 && p.Close > 0 {
			kept = append(kept, p)
		}
	}
	sort.SliceStable(kept, func(i, j int) bool { return kept[i].Ret5 > kept[j].Ret5 })
	if len(kept) > top {
		kept = kept[:top]
	}
	return kept
}

// Today's list: from the table if it was built, otherwise built and stored.
//
// A pass where every symbol failed is not "the market fell": it is a data outage, and the day is
// retried rather than recorded as an empty list.
func DailyList(pool *sql.DB, m market.MarketSource, now time.Time) ([]Pick, error) {
	day := schedule.ISTDay(now)
	stored, err := load(pool, day)
	if err != nil {
		return nil, err
	}
	if len(stored) > 0 {
		slog.Debug("benchmark list from cache", "day", day, "picks", len(stored))
		return stored, nil
	}
	// The list was built today and legitimately came back empty; asking the exchange again
	// cannot improve on that.
	if marker := getMarker(pool); marker == day {
		slog.Debug("benchmark list already built today and empty", "day", day)
		return []Pick{}, nil
	}

	measured := []Pick{}
	constituents, err := m.Constituents()
	if err != nil {
		slog.Warn("nifty 100 constituents unavailable", "error", err)
		return nil, err
	}
	slog.Info("benchmark sweep starting", "constituents", len(constituents), "day", day)
	started := time.Now()
	if len(constituents) == 0 {
		return nil, fmt.Errorf("the benchmark list was empty — the exchange returned no constituents")
	}
	if batch, ok := m.(interface {
		Closes([]string) (map[string][]float64, error)
	}); ok {
		symbols := make([]string, len(constituents))
		for i, c := range constituents {
			symbols[i] = c.Symbol
		}
		closes, err := batch.Closes(symbols)
		if err != nil {
			return nil, err
		}
		for _, c := range constituents {
			series := closes[c.Symbol]
			ret, ok := ReturnOver(series, Sessions)
			if !ok {
				continue
			}
			measured = append(measured, Pick{
				Symbol: c.Symbol, Name: c.Name,
				Close: series[len(series)-1], Ret5: ret,
			})
		}
	} else {
		for i, c := range constituents {
			if i > 0 {
				// A hundred back-to-back charts is a burst; a small pause keeps the exchange from
				// treating the sweep as abuse.
				time.Sleep(120 * time.Millisecond)
			}
			if p, ok := Measure(m, c.Symbol, c.Name); ok {
				measured = append(measured, p)
			}
		}
	}
	if len(measured) == 0 {
		slog.Warn("benchmark sweep measured nothing", "constituents", len(constituents),
			"took", time.Since(started).Round(time.Millisecond))
		return nil, fmt.Errorf("no price history for any of the %d constituents — the price source is throttling or unavailable", len(constituents))
	}
	picks := Rank(measured, Top)
	for _, p := range picks {
		if err := store(pool, day, p, now); err != nil {
			return nil, err
		}
	}
	// Set even when empty: an empty day is an answer, and a retry must not pay for it again.
	if err := setMarker(pool, day, now); err != nil {
		return nil, err
	}
	slog.Info("benchmark list built", "day", day, "measured", len(measured), "picks", len(picks),
		"took", time.Since(started).Round(time.Millisecond))
	return picks, nil
}

func getMarker(pool *sql.DB) string {
	var v string
	if err := pool.QueryRow("SELECT value FROM settings WHERE key = 'plan_candidates_last'").Scan(&v); err != nil {
		return ""
	}
	return v
}

func load(pool *sql.DB, day string) ([]Pick, error) {
	rows, err := pool.Query(
		"SELECT symbol, name, close, ret5 FROM plan_candidates WHERE day = ? ORDER BY ret5 DESC", day)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []Pick{}
	for rows.Next() {
		var p Pick
		if err := rows.Scan(&p.Symbol, &p.Name, &p.Close, &p.Ret5); err != nil {
			return nil, err
		}
		out = append(out, p)
	}
	return out, rows.Err()
}

func store(pool *sql.DB, day string, p Pick, now time.Time) error {
	_, err := pool.Exec(
		`INSERT INTO plan_candidates (day, symbol, name, close, ret5, added_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(day, symbol) DO NOTHING`,
		day, p.Symbol, p.Name, p.Close, p.Ret5, tstamp.From(now),
	)
	return err
}

func setMarker(pool *sql.DB, day string, now time.Time) error {
	_, err := pool.Exec(
		`INSERT INTO settings (key, value, updated_at) VALUES ('plan_candidates_last', ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at`,
		day, tstamp.From(now),
	)
	return err
}

func isNaN(v float64) bool { return v != v }
func isInf(v float64) bool {
	return v > 1.7976931348623157e308 || v < -1.7976931348623157e308
}
