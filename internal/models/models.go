// Package models holds the value types shared by every layer.
package models

import (
	"fmt"
	"strings"
)

// Which pot a row belongs to. Long-term and trading never share capital.
type Mode string

const (
	ModeLong  Mode = "long"
	ModeTrade Mode = "trade"
)

func ParseMode(s string) (Mode, error) {
	switch strings.ToLower(strings.TrimSpace(s)) {
	case "long", "long_term", "":
		return ModeLong, nil
	case "trade", "trading":
		return ModeTrade, nil
	}
	return "", fmt.Errorf("mode must be long or trade, got %q", s)
}

type AssetClass string

const (
	ClassStock      AssetClass = "stock"
	ClassMutualFund AssetClass = "mutual_fund"
	ClassIpo        AssetClass = "ipo"
)

func ParseAssetClass(s string) (AssetClass, error) {
	switch s {
	case "stock":
		return ClassStock, nil
	case "mutual_fund":
		return ClassMutualFund, nil
	case "ipo":
		return ClassIpo, nil
	}
	return "", fmt.Errorf("unknown asset class %q", s)
}

// Which pot this class may be held in. A mutual fund is bought at NAV and held: there is
// no intraday exit, so it belongs in the long-term book only.
func (c AssetClass) PotAllowed(mode Mode) bool {
	return mode == ModeLong || c == ClassStock
}

// The money's journey: how long before cash is out and how long before it is back.
func (c AssetClass) LiquidityNote() string {
	switch c {
	case ClassStock:
		return "cash is out the day you fill, and back the day you sell"
	case ClassMutualFund:
		return "a buy settles in ~1 day; selling returns the cash after ~4 days — patience is the trade"
	case ClassIpo:
		return "cash is locked from application until listing"
	}
	return ""
}

// The execution semantics that actually exist for this class.
func (c AssetClass) TimingNote() string {
	switch c {
	case ClassStock:
		return "limit order, intraday or next open"
	case ClassMutualFund:
		return "units at NAV of the 15:00 cutoff; buy settles in ~1 day, selling returns cash after ~4 days"
	case ClassIpo:
		return "cash locked until allotment; not sellable until listing"
	}
	return ""
}

type Bar struct {
	Ts     string  `json:"ts"`
	Open   float64 `json:"open"`
	High   float64 `json:"high"`
	Low    float64 `json:"low"`
	Close  float64 `json:"close"`
	Volume int64   `json:"volume"`
}

type Headline struct {
	ID          int64  `json:"id"`
	Title       string `json:"title"`
	Publisher   string `json:"publisher"`
	PublishedAt string `json:"published_at"`
	URL         string `json:"url"`
}

type Instrument struct {
	ID       string     `json:"id"`
	Symbol   string     `json:"symbol"`
	Class    AssetClass `json:"class"`
	Name     string     `json:"name"`
	Currency string     `json:"currency"`
	Active   bool       `json:"active"`
}

// One spelling per symbol: the database holds one row per symbol whatever case it was typed in.
func NormalizeSymbol(symbol string) string {
	return strings.ToUpper(strings.TrimSpace(symbol))
}

type Position struct {
	ID           string   `json:"id"`
	InstrumentID string   `json:"instrument_id"`
	Qty          float64  `json:"qty"`
	AvgCost      float64  `json:"avg_cost"`
	Stop         *float64 `json:"stop"`
	Target       *float64 `json:"target"`
	State        string   `json:"state"`
}

type Action string

const (
	ActionBuy  Action = "BUY"
	ActionSell Action = "SELL"
	ActionHold Action = "HOLD"
	ActionNone Action = "NONE"
)

func ParseAction(s string) (Action, error) {
	switch strings.ToUpper(strings.TrimSpace(s)) {
	case "BUY":
		return ActionBuy, nil
	case "SELL":
		return ActionSell, nil
	case "HOLD":
		return ActionHold, nil
	case "NONE", "PASS":
		return ActionNone, nil
	}
	return "", fmt.Errorf("unknown action %q", s)
}

// What the cheap rules layer proposes. Stop/Target are nil for classes that have no
// intraday price (mutual funds, IPOs) — a 0.0 sentinel would read as a real price.
type Candidate struct {
	InstrumentID string   `json:"instrument_id"`
	Action       Action   `json:"action"`
	Reasons      []string `json:"reasons"`
	Entry        float64  `json:"entry"`
	Stop         *float64 `json:"stop"`
	Target       *float64 `json:"target"`
	Qty          float64  `json:"qty"`
	TimingNote   string   `json:"timing_note"`
	Urgent       bool     `json:"urgent"`
}
