// Package market is everything the engine needs from the outside market. Tests implement
// this interface with fakes, so no test reaches the network.
package market

import (
	"stock-predictor/internal/models"
)

// (yahoo symbol, name) — one row of an index.
type Constituent struct {
	Symbol string
	Name   string
}

type MarketSource interface {
	Bars(symbol string) ([]models.Bar, error)

	// Intraday tape. Interval/range follow Yahoo's vocabulary (15m/5d, 60m/7d).
	Intraday(symbol, interval, rangeStr string) ([]models.Bar, error)

	News(symbol string) ([]models.Headline, error)

	// The constituents of the long-term benchmark, as (yahoo symbol, name). Behind the
	// interface so a test can answer with a list instead of reaching NSE.
	Constituents() ([]Constituent, error)
}
