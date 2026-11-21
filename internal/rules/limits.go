// Package rules is the cheap layer that runs before any model call: settings, budgets,
// pots, limits and indicators.
package rules

import (
	"math"

	"stock-predictor/internal/models"
	"stock-predictor/internal/validate"
)

// Risk limits for one pot. Percentages are of that pot; amounts are in the account currency.
type Limits struct {
	MinProfitPct     float64 `json:"min_profit_pct"`
	MaxProfitPct     float64 `json:"max_profit_pct"`
	MinTradeAmount   float64 `json:"min_trade_amount"`
	MaxTradeAmount   float64 `json:"max_trade_amount"`
	MaxInstrumentPct float64 `json:"max_instrument_pct"`
}

func DefaultLimits() Limits {
	return Limits{
		MinProfitPct:     5.0,
		MaxProfitPct:     50.0,
		MinTradeAmount:   500.0,
		MaxTradeAmount:   50_000.0,
		MaxInstrumentPct: 25.0,
	}
}

func LimitsKeyPrefix(mode models.Mode) string {
	return "limits_" + string(mode)
}

func (l Limits) field(name string) (float64, bool) {
	switch name {
	case "min_profit_pct":
		return l.MinProfitPct, true
	case "max_profit_pct":
		return l.MaxProfitPct, true
	case "min_trade_amount":
		return l.MinTradeAmount, true
	case "max_trade_amount":
		return l.MaxTradeAmount, true
	case "max_instrument_pct":
		return l.MaxInstrumentPct, true
	}
	return 0, false
}

func (l Limits) With(name string, value float64) (Limits, bool) {
	switch name {
	case "min_profit_pct":
		l.MinProfitPct = value
	case "max_profit_pct":
		l.MaxProfitPct = value
	case "min_trade_amount":
		l.MinTradeAmount = value
	case "max_trade_amount":
		l.MaxTradeAmount = value
	case "max_instrument_pct":
		l.MaxInstrumentPct = value
	default:
		return l, false
	}
	return l, true
}

// The largest a single fill may be, also bounded by the room left on that instrument.
func (l Limits) MaxFill(pot, spentOnInstrument float64) float64 {
	room := math.Max(pot*l.MaxInstrumentPct/100.0-spentOnInstrument, 0.0)
	return math.Min(l.MaxTradeAmount, room)
}

func (l Limits) Check(amount float64) error {
	if amount < l.MinTradeAmount {
		return validate.Error{Kind: validate.KindTradeTooSmall, Min: l.MinTradeAmount}
	}
	if amount > l.MaxTradeAmount {
		return validate.Error{Kind: validate.KindTradeTooLarge, Max: l.MaxTradeAmount}
	}
	return nil
}

func (l Limits) CheckInstrument(pot, alreadySpent, amount float64) error {
	cap := pot * l.MaxInstrumentPct / 100.0
	if alreadySpent+amount > cap+1e-9 {
		return validate.Error{Kind: validate.KindInstrumentLimit, Cap: cap, Already: alreadySpent}
	}
	return nil
}

// floor is this stock's own expectation (which defaults to MinProfitPct); the ceiling is
// always the global one, so a stock cannot buy itself a bigger allowance.
func (l Limits) CheckProfit(pct, floor float64) error {
	floor = math.Max(floor, l.MinProfitPct)
	if pct+1e-9 < floor {
		return validate.Error{Kind: validate.KindTargetBelowReq, RequiredPct: floor}
	}
	if pct > l.MaxProfitPct+1e-9 {
		return validate.Error{Kind: validate.KindTargetAboveLimit, MaxPct: l.MaxProfitPct}
	}
	return nil
}

// A stock's expectation must sit inside the global band, or it can never be satisfied.
func (l Limits) AllowsTarget(target float64) bool {
	return target >= l.MinProfitPct-1e-9 && target <= l.MaxProfitPct+1e-9
}
