// Package trades records real fills: the history row, the position projection and the cash
// movement all move together or not at all.
package trades

import (
	"database/sql"
	"fmt"
	"log/slog"
	"math"
	"strings"
	"time"

	"stock-predictor/internal/ids"
	"stock-predictor/internal/models"
	"stock-predictor/internal/rules"
	"stock-predictor/internal/tstamp"
)

type Side string

const (
	SideBuy  Side = "buy"
	SideSell Side = "sell"
)

func ParseSide(s string) (Side, error) {
	switch strings.ToLower(strings.TrimSpace(s)) {
	case "buy":
		return SideBuy, nil
	case "sell":
		return SideSell, nil
	}
	return "", fmt.Errorf("side must be buy or sell, got %q", s)
}

type Trade struct {
	ID          string   `json:"id"`
	Symbol      string   `json:"symbol"`
	Side        string   `json:"side"`
	Qty         float64  `json:"qty"`
	Price       float64  `json:"price"`
	Realised    *float64 `json:"realised"`
	TradedAt    string   `json:"traded_at"`
	Note        *string  `json:"note"`
	Mode        string   `json:"mode"`
	MarketClose *float64 `json:"market_close"`
}

// When a fill happened. A bare YYYY-MM-DD means that trading day, stamped at the 15:30 IST
// close, so a backfilled record sorts after the morning and before the next day.
func ParseWhen(input string, now time.Time) (time.Time, error) {
	raw := strings.TrimSpace(input)
	if raw == "" {
		return now, nil
	}
	parsed, err := time.Parse(time.RFC3339, raw)
	if err != nil {
		date, derr := time.Parse("2006-01-02", raw)
		if derr != nil {
			return time.Time{}, fmt.Errorf("date must be YYYY-MM-DD or RFC3339, got %q", raw)
		}
		// 15:30 IST == 10:00 UTC; NSE has no DST, so a fixed offset is exact.
		parsed = time.Date(date.Year(), date.Month(), date.Day(), 15, 30, 0, 0, time.FixedZone("IST", 330*60))
	}
	parsed = parsed.UTC()
	if parsed.After(now.Add(24 * time.Hour)) {
		return time.Time{}, fmt.Errorf("cannot record a fill in the future")
	}
	if parsed.Before(time.Date(1990, 1, 1, 0, 0, 0, 0, time.UTC)) {
		return time.Time{}, fmt.Errorf("that date is before any market this app knows about")
	}
	return parsed, nil
}

func Record(pool *sql.DB, instrumentID string, side Side, qty, price float64, note *string, at *time.Time, mode models.Mode) error {
	tx, err := pool.Begin()
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if err := RecordOn(tx, instrumentID, side, qty, price, note, at, mode); err != nil {
		slog.Warn("fill refused", "instrument", instrumentID, "side", side, "qty", qty,
			"price", price, "mode", mode, "error", err)
		return err
	}
	if err := tx.Commit(); err != nil {
		return err
	}
	slog.Info("fill recorded", "instrument", instrumentID, "side", side, "qty", qty,
		"price", price, "mode", mode, "note", note)
	return nil
}

func RecordOn(tx *sql.Tx, instrumentID string, side Side, qty, price float64, note *string, at *time.Time, mode models.Mode) error {
	if !finite(qty) || qty <= 0 {
		return fmt.Errorf("qty must be positive")
	}
	if !finite(price) || price <= 0 {
		return fmt.Errorf("price must be positive")
	}

	// A mutual fund has no intraday exit, so it cannot be a trading-pot fill.
	var classRaw string
	if err := tx.QueryRow("SELECT class FROM instruments WHERE id = ?", instrumentID).Scan(&classRaw); err != nil {
		return err
	}
	class, err := models.ParseAssetClass(classRaw)
	if err != nil {
		return err
	}
	if !class.PotAllowed(mode) {
		return fmt.Errorf("%s cannot be held in the trading pot — mutual funds are long-term only", class)
	}

	when := time.Now().UTC()
	if at != nil {
		when = *at
	}
	stamp := tstamp.From(when)

	var posID string
	var held, cost sql.NullFloat64
	var openedAt sql.NullString
	err = tx.QueryRow(
		`SELECT id, qty, avg_cost, opened_at FROM positions
		 WHERE instrument_id = ? AND state = 'open' AND mode = ?
		 ORDER BY id DESC LIMIT 1`, instrumentID, string(mode),
	).Scan(&posID, &held, &cost, &openedAt)
	hasOpen := err == nil
	if err != nil && err != sql.ErrNoRows {
		return err
	}

	switch side {
	case SideBuy:
		value := qty * price
		purse := rules.BucketFor(class)
		available, err := rules.CashInBucketOn(tx, mode, purse)
		if err != nil {
			return err
		}
		if value > available+1e-9 {
			return fmt.Errorf("cost %.2f exceeds available cash %.2f", value, available)
		}

		if hasOpen {
			newQty := held.Float64 + qty
			newCost := (held.Float64*cost.Float64 + value) / newQty
			opened := when
			if openedAt.Valid {
				if parsed, err := time.Parse(time.RFC3339, openedAt.String); err == nil {
					opened = parsed.UTC()
				}
			}
			started := opened
			if when.Before(started) {
				started = when
			}
			if _, err := tx.Exec(
				"UPDATE positions SET qty = ?, avg_cost = ?, opened_at = ? WHERE id = ?",
				newQty, newCost, tstamp.From(started), posID,
			); err != nil {
				return err
			}
		} else {
			if _, err := tx.Exec(
				`INSERT INTO positions (id, instrument_id, qty, avg_cost, opened_at, state, mode)
				 VALUES (?, ?, ?, ?, ?, 'open', ?)`,
				ids.New(), instrumentID, qty, price, stamp, string(mode),
			); err != nil {
				return err
			}
		}

		if err := insertTrade(tx, instrumentID, side, qty, price, nil, stamp, note, mode); err != nil {
			return err
		}
		ref := fmt.Sprintf("buy %v @ %v", qty, price)
		return rules.RecordOn(tx, rules.NewCashEntry{
			Kind: rules.CashBuy, Amount: -value, Reference: ref, Bucket: purse,
		}, stamp, mode)

	case SideSell:
		if !hasOpen {
			return fmt.Errorf("nothing is held to sell")
		}
		if qty > held.Float64+1e-9 {
			return fmt.Errorf("cannot sell %v, only %v held", qty, held.Float64)
		}
		if openedAt.Valid {
			if opened, err := time.Parse(time.RFC3339, openedAt.String); err == nil && when.Before(opened.UTC()) {
				return fmt.Errorf("sell date is before the position was opened (%s)", openedAt.String)
			}
		}

		realised := qty * (price - cost.Float64)
		remaining := held.Float64 - qty
		if remaining <= 1e-9 {
			// A full exit keeps the row at 0 units and flips the state: the ledger still
			// shows that this holding existed, and the trade history stays whole.
			if _, err := tx.Exec("UPDATE positions SET qty = 0, state = 'closed' WHERE id = ?", posID); err != nil {
				return err
			}
		} else {
			if _, err := tx.Exec("UPDATE positions SET qty = ? WHERE id = ?", remaining, posID); err != nil {
				return err
			}
		}

		if err := insertTrade(tx, instrumentID, side, qty, price, &realised, stamp, note, mode); err != nil {
			return err
		}
		ref := fmt.Sprintf("sell %v @ %v", qty, price)
		return rules.RecordOn(tx, rules.NewCashEntry{
			Kind: rules.CashSell, Amount: qty * price, Reference: ref, Bucket: rules.BucketFor(class),
		}, stamp, mode)
	}
	return fmt.Errorf("unknown side %q", side)
}

// Edit corrects a recorded fill.
//
// The trade row is rewritten, the position moves by the difference, and the cash difference
// is posted as its own adjust entry — the ledger stays append-only, so a correction is a
// visible event rather than a silent rewrite of history.
//
// An edit on an archived instrument brings it back into the book: correcting a fill is working
// the name again, and leaving it archived would hide the very row that was just corrected.
func Edit(pool *sql.DB, id string, qty, price *float64, at *time.Time) error {
	tx, err := pool.Begin()
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if err := EditOn(tx, id, qty, price, at); err != nil {
		slog.Warn("fill edit refused", "id", id, "error", err)
		return err
	}
	if err := tx.Commit(); err != nil {
		return err
	}
	slog.Info("fill edited", "id", id)
	return nil
}

func EditOn(tx *sql.Tx, id string, qty, price *float64, at *time.Time) error {
	var instrumentID string
	var sideRaw, tradedAt, modeRaw string
	var oldQty, oldPrice float64
	err := tx.QueryRow(
		"SELECT instrument_id, side, qty, price, traded_at, mode FROM trades WHERE id = ?", id,
	).Scan(&instrumentID, &sideRaw, &oldQty, &oldPrice, &tradedAt, &modeRaw)
	if err == sql.ErrNoRows {
		return fmt.Errorf("no fill %s", id)
	}
	if err != nil {
		return err
	}
	mode, err := models.ParseMode(modeRaw)
	if err != nil {
		return err
	}
	side, err := ParseSide(sideRaw)
	if err != nil {
		return err
	}
	newQty, newPrice := oldQty, oldPrice
	if qty != nil {
		newQty = *qty
	}
	if price != nil {
		newPrice = *price
	}
	if !finite(newQty) || newQty <= 0 {
		return fmt.Errorf("qty must be positive")
	}
	if !finite(newPrice) || newPrice <= 0 {
		return fmt.Errorf("price must be positive")
	}
	when := time.Now().UTC()
	if at != nil {
		when = *at
	} else if parsed, err := time.Parse(time.RFC3339, tradedAt); err == nil {
		when = parsed.UTC()
	}

	var posID string
	var held, cost sql.NullFloat64
	err = tx.QueryRow(
		`SELECT id, qty, avg_cost FROM positions
		 WHERE instrument_id = ? AND mode = ? ORDER BY state = 'open' DESC, id DESC LIMIT 1`,
		instrumentID, string(mode),
	).Scan(&posID, &held, &cost)
	if err != nil && err != sql.ErrNoRows {
		return err
	}

	oldValue := oldQty * oldPrice
	newValue := newQty * newPrice
	// A fill is a movement of units and of money; an edit moves the difference.
	var deltaUnits, deltaCash float64
	if side == SideBuy {
		deltaUnits = newQty - oldQty
		deltaCash = oldValue - newValue
	} else {
		deltaUnits = oldQty - newQty
		deltaCash = newValue - oldValue
	}
	if held.Float64+deltaUnits < -1e-9 {
		return fmt.Errorf("that edit would leave %.3f units held, which is less than nothing", held.Float64+deltaUnits)
	}

	if posID != "" {
		newHeld := held.Float64 + deltaUnits
		basis := held.Float64 * cost.Float64
		if side == SideBuy {
			basis += newValue - oldValue
		}
		newCost := cost.Float64
		if newHeld > 1e-9 {
			newCost = basis / newHeld
		}
		state := "closed"
		if newHeld > 1e-9 {
			state = "open"
		}
		if _, err := tx.Exec(
			"UPDATE positions SET qty = ?, avg_cost = ?, state = ? WHERE id = ?",
			newHeld, newCost, state, posID,
		); err != nil {
			return err
		}
	}

	var realised *float64
	if side == SideSell {
		r := newQty * (newPrice - cost.Float64)
		realised = &r
	}
	if _, err := tx.Exec(
		"UPDATE trades SET qty = ?, price = ?, realised = ?, traded_at = ? WHERE id = ?",
		newQty, newPrice, realised, tstamp.From(when), id,
	); err != nil {
		return err
	}

	if math.Abs(deltaCash) > 1e-9 {
		// Money back when the fill was overstated, more when it was understated.
		if err := rules.RecordOn(tx, rules.NewCashEntry{
			Kind: rules.CashAdjust, Amount: deltaCash, Reference: fmt.Sprintf("correct fill %s", id),
		}, tstamp.From(when), mode); err != nil {
			return err
		}
	}

	// Correcting a fill means the book is being worked again, so the instrument comes back.
	if _, err := tx.Exec(
		"UPDATE instruments SET active = 1, archived_at = NULL WHERE id = ? AND archived_at IS NOT NULL",
		instrumentID,
	); err != nil {
		return err
	}
	return nil
}

// Delete removes a fill entirely: the position moves back by exactly what the fill moved it,
// the money difference is posted as its own adjust entry, and a decision the fill satisfied
// returns to pending.
func Delete(pool *sql.DB, id string) error {
	tx, err := pool.Begin()
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if err := DeleteOn(tx, id); err != nil {
		slog.Warn("fill delete refused", "id", id, "error", err)
		return err
	}
	if err := tx.Commit(); err != nil {
		return err
	}
	slog.Info("fill deleted", "id", id)
	return nil
}

func DeleteOn(tx *sql.Tx, id string) error {
	var instrumentID string
	var sideRaw, modeRaw string
	var qty, price float64
	var note *string
	err := tx.QueryRow(
		"SELECT instrument_id, side, qty, price, note, mode FROM trades WHERE id = ?", id,
	).Scan(&instrumentID, &sideRaw, &qty, &price, &note, &modeRaw)
	if err == sql.ErrNoRows {
		return fmt.Errorf("no fill %s", id)
	}
	if err != nil {
		return err
	}
	mode, err := models.ParseMode(modeRaw)
	if err != nil {
		return err
	}
	side, err := ParseSide(sideRaw)
	if err != nil {
		return err
	}
	value := qty * price

	var posID string
	var held, cost sql.NullFloat64
	err = tx.QueryRow(
		`SELECT id, qty, avg_cost FROM positions
		 WHERE instrument_id = ? AND mode = ? ORDER BY state = 'open' DESC, id DESC LIMIT 1`,
		instrumentID, string(mode),
	).Scan(&posID, &held, &cost)
	if err != nil && err != sql.ErrNoRows {
		return err
	}

	// Undo: a buy gives its units back, a sell returns the units it took.
	deltaUnits := -qty
	if side == SideSell {
		deltaUnits = qty
	}
	if held.Float64+deltaUnits < -1e-9 {
		return fmt.Errorf("deleting this fill would leave %.3f units held — sell or correct the later fills first", held.Float64+deltaUnits)
	}

	if posID != "" {
		newHeld := held.Float64 + deltaUnits
		basis := held.Float64 * cost.Float64
		if side == SideBuy {
			basis -= value
		}
		newCost := cost.Float64
		if newHeld > 1e-9 {
			newCost = math.Max(basis/newHeld, 0)
		}
		state := "closed"
		if newHeld > 1e-9 {
			state = "open"
		}
		if _, err := tx.Exec(
			"UPDATE positions SET qty = ?, avg_cost = ?, state = ? WHERE id = ?",
			newHeld, newCost, state, posID,
		); err != nil {
			return err
		}
	}

	// The money comes back the way it went out: a deleted buy refunds, a deleted sell charges.
	cash := value
	if side == SideSell {
		cash = -value
	}
	if math.Abs(cash) > 1e-9 {
		if err := rules.RecordOn(tx, rules.NewCashEntry{
			Kind: rules.CashAdjust, Amount: cash, Reference: fmt.Sprintf("delete fill %s", id),
		}, tstamp.Now(), mode); err != nil {
			return err
		}
	}

	if _, err := tx.Exec("DELETE FROM trades WHERE id = ?", id); err != nil {
		return err
	}

	// A fill that satisfied a decision no longer exists, so the decision is pending again.
	if note != nil {
		if rest, ok := strings.CutPrefix(*note, "decision "); ok {
			if decision, err := parseInt64(strings.TrimSpace(rest)); err == nil {
				if _, err := tx.Exec(
					`UPDATE signals SET status = 'pending', status_at = NULL, filled_qty = NULL,
                        filled_price = NULL WHERE id = ?`, decision,
				); err != nil {
					return err
				}
			}
		}
	}
	return nil
}

func insertTrade(tx *sql.Tx, instrumentID string, side Side, qty, price float64, realised *float64, at string, note *string, mode models.Mode) error {
	_, err := tx.Exec(
		`INSERT INTO trades (id, instrument_id, side, qty, price, realised, traded_at, note, mode)
		 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
		ids.New(), instrumentID, string(side), qty, price, realised, at, note, string(mode),
	)
	return err
}

// History is the ledger list. A mode of nil means every book; the dashboard asks for one book, so
// its recent fills are already the ones the reader is looking at.
func History(pool *sql.DB, instrumentID *string, mode *models.Mode, limit int64) ([]Trade, error) {
	const selectSQL = `SELECT t.id AS id, i.symbol AS symbol, t.side AS side, t.qty AS qty,
            t.price AS price, t.realised AS realised, t.traded_at AS traded_at, t.note AS note,
            t.mode AS mode,
            (SELECT b.close FROM bars b
             WHERE b.instrument_id = t.instrument_id
               AND substr(b.ts, 1, 10) = substr(t.traded_at, 1, 10)
             ORDER BY b.ts DESC LIMIT 1) AS market_close
        FROM trades t JOIN instruments i ON i.id = t.instrument_id`

	where := []string{}
	args := []any{}
	if instrumentID != nil {
		where = append(where, "t.instrument_id = ?")
		args = append(args, *instrumentID)
	}
	if mode != nil {
		where = append(where, "t.mode = ?")
		args = append(args, string(*mode))
	}
	query := selectSQL
	if len(where) > 0 {
		query += " WHERE " + strings.Join(where, " AND ")
	}
	args = append(args, limit)
	rows, err := pool.Query(query+" ORDER BY t.traded_at DESC LIMIT ?", args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	out := []Trade{}
	for rows.Next() {
		var t Trade
		var realised, marketClose sql.NullFloat64
		var note sql.NullString
		if err := rows.Scan(&t.ID, &t.Symbol, &t.Side, &t.Qty, &t.Price, &realised,
			&t.TradedAt, &note, &t.Mode, &marketClose); err != nil {
			return nil, err
		}
		if realised.Valid {
			t.Realised = &realised.Float64
		}
		if note.Valid {
			t.Note = &note.String
		}
		if marketClose.Valid {
			t.MarketClose = &marketClose.Float64
		}
		out = append(out, t)
	}
	return out, rows.Err()
}

func RealisedIn(pool *sql.DB, mode models.Mode) (float64, error) {
	var sum sql.NullFloat64
	err := pool.QueryRow(
		"SELECT SUM(realised) FROM trades WHERE side = 'sell' AND mode = ?", string(mode),
	).Scan(&sum)
	if err != nil {
		return 0, err
	}
	return sum.Float64, nil
}

func finite(v float64) bool {
	return !math.IsNaN(v) && !math.IsInf(v, 0)
}

func parseInt64(s string) (int64, error) {
	var n int64
	_, err := fmt.Sscanf(s, "%d", &n)
	return n, err
}
