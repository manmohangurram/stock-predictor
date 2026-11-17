// Package seed creates a small real watchlist. Everything downstream uses live Yahoo data for
// these; nothing here invents a number.
package seed

import (
	"database/sql"
	"fmt"
	"log/slog"
	"net/http"
	"time"

	"stock-predictor/internal/market"
	"stock-predictor/internal/models"
	"stock-predictor/internal/rules"
	"stock-predictor/internal/trades"
)

var Stocks = [][2]string{
	{"INFY.NS", "Infosys"},
	{"TCS.NS", "Tata Consultancy Services"},
	{"HDFCBANK.NS", "HDFC Bank"},
	{"RELIANCE.NS", "Reliance Industries"},
}

const MFSScheme = "120503"
const spendPerStock = 20_000.0
const spendPerFund = 25_000.0

type Report struct {
	InstrumentsAdded int
	PositionsAdded   int
	Skipped          []string
}

// Seeds only what it can price from a live source: a position at the real last close, or at the
// scheme's real NAV.
func Run(pool *sql.DB, client *http.Client, withPositions bool) (Report, error) {
	report := Report{}
	for _, stock := range Stocks {
		symbol, name := stock[0], stock[1]
		added, err := insertInstrument(pool, symbol, models.ClassStock, name)
		if err != nil {
			return report, err
		}
		report.InstrumentsAdded += added
		if added > 0 {
			slog.Info("seed instrument", "symbol", symbol, "name", name, "class", models.ClassStock)
		}
		if !withPositions {
			continue
		}
		id, err := instrumentID(pool, symbol)
		if err != nil {
			return report, err
		}
		has, err := hasPosition(pool, id)
		if err != nil {
			return report, err
		}
		if has {
			continue
		}
		bars, err := market.FetchBars(client, symbol)
		if err != nil {
			report.Skipped = append(report.Skipped, fmt.Sprintf("%s: %v", symbol, err))
			continue
		}
		if len(bars) == 0 {
			report.Skipped = append(report.Skipped, symbol+": no price data")
			continue
		}
		closeValue := bars[len(bars)-1].Close
		qty := float64(int64(spendPerStock / closeValue))
		if qty < 1 {
			report.Skipped = append(report.Skipped, symbol+": price too high for the stake")
			continue
		}
		fits, err := rules.Fits(pool, qty*closeValue)
		if err != nil {
			return report, err
		}
		if !fits {
			report.Skipped = append(report.Skipped, symbol+": does not fit the budget")
			continue
		}
		// One call creates the position, the fill and the cash movement together.
		note := "seeded at the last close"
		if err := trades.Record(pool, id, trades.SideBuy, qty, closeValue, &note, nil, models.ModeLong); err != nil {
			return report, err
		}
		report.PositionsAdded++
	}

	added, err := insertInstrument(pool, "AXISELSS", models.ClassMutualFund, "Axis ELSS Tax Saver")
	if err != nil {
		return report, err
	}
	report.InstrumentsAdded += added
	if withPositions {
		id, err := instrumentID(pool, "AXISELSS")
		if err != nil {
			return report, err
		}
		has, err := hasPosition(pool, id)
		if err != nil {
			return report, err
		}
		if !has {
			text, err := market.FetchNAVAll(client)
			if err == nil {
				var nav market.NavRow
				nav, err = market.ParseNAV(text, MFSScheme)
				if err == nil {
					qty := rules.UnitsFloor(spendPerFund / nav.NAV)
					fits, ferr := rules.Fits(pool, qty*nav.NAV)
					if ferr != nil {
						return report, ferr
					}
					if qty > 0 && fits {
						note := "seeded at AMFI NAV"
						if rerr := trades.Record(pool, id, trades.SideBuy, qty, nav.NAV, &note, nil, models.ModeLong); rerr != nil {
							return report, rerr
						}
						report.PositionsAdded++
					}
				}
			}
			if err != nil {
				report.Skipped = append(report.Skipped, fmt.Sprintf("AXISELSS: AMFI unavailable (%v)", err))
			}
		}
	}
	return report, nil
}

func insertInstrument(pool *sql.DB, symbol string, class models.AssetClass, name string) (int, error) {
	res, err := pool.Exec(
		"INSERT OR IGNORE INTO instruments (symbol, class, name, currency) VALUES (?, ?, ?, 'INR')",
		symbol, string(class), name)
	if err != nil {
		return 0, err
	}
	n, _ := res.RowsAffected()
	return int(n), nil
}

func instrumentID(pool *sql.DB, symbol string) (int64, error) {
	var id int64
	err := pool.QueryRow("SELECT id FROM instruments WHERE symbol = ?", symbol).Scan(&id)
	return id, err
}

func hasPosition(pool *sql.DB, instrumentID int64) (bool, error) {
	var n int
	err := pool.QueryRow(
		"SELECT COUNT(*) FROM positions WHERE instrument_id = ? AND state = 'open'", instrumentID,
	).Scan(&n)
	return n > 0, err
}

var _ = time.Now
