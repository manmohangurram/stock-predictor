package llm

import (
	"database/sql"
	"sort"
	"strings"
	"time"

	"stock-predictor/internal/models"
	"stock-predictor/internal/tstamp"
)

// Order-independent fingerprint of what the model would actually read: title + url. A
// re-published title under a new url is new information; the same url is not.
func Fingerprint(headlines []models.Headline) string {
	parts := make([]string, len(headlines))
	for i, h := range headlines {
		parts[i] = strings.TrimSpace(h.Title) + "\x1f" + strings.TrimSpace(h.URL)
	}
	sort.Strings(parts)
	var b strings.Builder
	for _, p := range parts {
		b.WriteString(p)
		b.WriteString("\n")
	}
	return hex(sha256sum(b.String()))
}

// True when the model has already answered for exactly this headline set.
func Unchanged(pool *sql.DB, instrumentID string, fp string) (bool, error) {
	var stored string
	err := pool.QueryRow("SELECT headline_fp FROM llm_memos WHERE instrument_id = ?", instrumentID).Scan(&stored)
	if err == sql.ErrNoRows {
		return false, nil
	}
	if err != nil {
		return false, err
	}
	return stored == fp, nil
}

// How the per-instrument call gate decides.
type CallGate struct {
	MinMove float64
}

func DefaultCallGate() CallGate {
	return CallGate{MinMove: 2.0}
}

// True when the model should be consulted for this instrument at this price and time.
func ShouldCall(pool *sql.DB, instrumentID string, price float64, now time.Time, gate CallGate, headlinesChanged bool) (bool, error) {
	if headlinesChanged {
		return true, nil
	}
	var lastPrice sql.NullFloat64
	var lastCallAt sql.NullString
	err := pool.QueryRow(
		"SELECT last_price, last_call_at FROM llm_memos WHERE instrument_id = ?", instrumentID,
	).Scan(&lastPrice, &lastCallAt)
	if err == sql.ErrNoRows {
		return true, nil // never asked: ask
	}
	if err != nil {
		return false, err
	}
	if !lastPrice.Valid {
		return true, nil
	}
	diff := price - lastPrice.Float64
	if diff < 0 {
		diff = -diff
	}
	return diff >= gate.MinMove, nil
}

// Recorded after every attempt, success or failure: a failing call must not loop every
// tick.
func RecordMemoCall(pool *sql.DB, instrumentID string, price float64, now time.Time, headlineFP string) error {
	_, err := pool.Exec(
		`INSERT INTO llm_memos (instrument_id, headline_fp, decided_at, last_price, last_call_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(instrument_id) DO UPDATE SET
             headline_fp = excluded.headline_fp,
             last_price = excluded.last_price, last_call_at = excluded.last_call_at`,
		instrumentID, headlineFP, tstamp.From(now), price, tstamp.From(now),
	)
	return err
}

func Remember(pool *sql.DB, instrumentID string, fp string) error {
	_, err := pool.Exec(
		`INSERT INTO llm_memos (instrument_id, headline_fp, decided_at) VALUES (?, ?, ?)
         ON CONFLICT(instrument_id) DO UPDATE SET headline_fp = excluded.headline_fp,
             decided_at = excluded.decided_at`,
		instrumentID, fp, tstamp.Now(),
	)
	return err
}
