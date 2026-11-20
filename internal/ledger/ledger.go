// Package ledger answers what the holdings tables show.
//
// A position that has been sold down to zero is still part of the record, so it stays in the
// ledger until its instrument is archived — the row then reads 0 units instead of disappearing
// the moment the last unit is sold.
package ledger

import (
	"database/sql"
	"strings"

	"stock-predictor/internal/models"
)

type PositionRow struct {
	ID              string   `json:"id"`
	RowID           int64    `json:"-"`
	Symbol          string   `json:"symbol"`
	Name            string   `json:"name"`
	Class           string   `json:"class"`
	Qty             float64  `json:"qty"`
	AvgCost         float64  `json:"avg_cost"`
	LTP             *float64 `json:"ltp"`
	Value           *float64 `json:"value"`
	Stop            *float64 `json:"stop"`
	Target          *float64 `json:"target"`
	TargetProfitPct *float64 `json:"target_profit_pct"`
	State           string   `json:"state"`
	TimingNote      string   `json:"timing_note"`
}

// Positions returns open and closed rows. Archiving an instrument is the only thing that
// removes a row from the ledger; a full sale only flips its state.
func Positions(pool *sql.DB, mode models.Mode) ([]PositionRow, error) {
	rows, err := pool.Query(
		`SELECT p.uuid AS id, p.id AS row_id, i.symbol AS symbol, i.name AS name, i.class AS class, p.qty AS qty,
                p.avg_cost AS avg_cost, p.stop AS stop, p.target AS target,
                i.target_profit_pct AS target_profit_pct, p.state AS state,
                (SELECT b.close FROM bars b WHERE b.instrument_id = p.instrument_id
                 ORDER BY b.ts DESC LIMIT 1) AS ltp
         FROM positions p JOIN instruments i ON i.id = p.instrument_id
         WHERE p.mode = ? AND i.active = 1 AND p.state IN ('open', 'closed')
         ORDER BY p.id DESC`,
		string(mode),
	)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	out := []PositionRow{}
	for rows.Next() {
		var r PositionRow
		var classRaw string
		var ltp, stop, target, targetProfit sql.NullFloat64
		if err := rows.Scan(&r.ID, &r.RowID, &r.Symbol, &r.Name, &classRaw, &r.Qty, &r.AvgCost,
			&stop, &target, &targetProfit, &r.State, &ltp); err != nil {
			return nil, err
		}
		class, err := models.ParseAssetClass(classRaw)
		if err != nil {
			class = models.ClassStock
		}
		r.Class = string(class)
		r.TimingNote = class.TimingNote()
		if stop.Valid {
			r.Stop = &stop.Float64
		}
		if target.Valid {
			r.Target = &target.Float64
		}
		if targetProfit.Valid {
			r.TargetProfitPct = &targetProfit.Float64
		}
		if ltp.Valid {
			r.LTP = &ltp.Float64
			value := ltp.Float64 * r.Qty
			r.Value = &value
		}
		out = append(out, r)
	}
	return out, rows.Err()
}

// OpenPositionFor finds the live holding for a symbol in either pot, which is what an
// instrument page needs: the same name can be held long-term or short-term.
func OpenPositionFor(pool *sql.DB, symbol string) (*PositionRow, error) {
	for _, mode := range []models.Mode{models.ModeLong, models.ModeTrade} {
		rows, err := Positions(pool, mode)
		if err != nil {
			return nil, err
		}
		for i := range rows {
			if strings.EqualFold(rows[i].Symbol, symbol) && rows[i].State == "open" {
				return &rows[i], nil
			}
		}
	}
	return nil, nil
}
