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

	Intraday(symbol, interval, rangeStr string) ([]models.Bar, error)

	News(symbol string) ([]models.Headline, error)

	Constituents() ([]Constituent, error)
}
