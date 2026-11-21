package rules

import (
	"fmt"
	"math"

	"stock-predictor/internal/models"
)

type RuleConfig struct {
	RsiBuyCeiling float64
	RsiSellFloor  float64
	StopATR       float64
	TargetATR     float64
}

func DefaultRuleConfig() RuleConfig {
	return RuleConfig{
		RsiBuyCeiling: 70.0,
		RsiSellFloor:  75.0,
		StopATR:       2.0,
		TargetATR:     3.0,
	}
}

type CandidateInput struct {
	Instrument    *models.Instrument
	Bars          []models.Bar
	Ind           *Indicators
	Position      *models.Position
	AvailableCash float64
	Cfg           RuleConfig
	PriceOnly     bool
}

// The cheap gate. Returns nil when no rule fires, which is what keeps the LLM off the
// wire on an ordinary poll.
func Candidate(input CandidateInput) *models.Candidate {
	if len(input.Bars) == 0 {
		return nil
	}
	close := input.Bars[len(input.Bars)-1].Close
	if math.IsNaN(close) || math.IsInf(close, 0) || close <= 0.0 {
		return nil
	}
	ind := input.Ind
	class := input.Instrument.Class
	reasons := []string{}
	if input.Position != nil {
		pos := input.Position
		belowStop := pos.Stop != nil && close < *pos.Stop
		// The other half of a trade: the target is a sell order that already exists, not a
		// wish.
		atTarget := pos.Target != nil && close >= *pos.Target
		trendBreak := !input.PriceOnly && ind != nil && ind.RSI14 > input.Cfg.RsiSellFloor && close < ind.SMA50
		if belowStop {
			reasons = append(reasons, fmt.Sprintf(
				"close %.2f below recorded stop %.2f", close, *pos.Stop))
		}
		if atTarget {
			reasons = append(reasons, fmt.Sprintf(
				"close %.2f at or above recorded target %.2f — take the profit", close, *pos.Target))
		}
		if trendBreak {
			reasons = append(reasons, fmt.Sprintf(
				"RSI %.1f overbought with close below SMA50 %.2f", ind.RSI14, ind.SMA50))
		}
		if belowStop || trendBreak || atTarget {
			return &models.Candidate{
				InstrumentID: input.Instrument.ID,
				Action:       models.ActionSell,
				Reasons:      reasons,
				Entry:        close,
				Stop:         nil,
				Target:       pos.Target,
				Qty:          pos.Qty,
				TimingNote:   class.TimingNote(),
				Urgent:       belowStop || atTarget,
			}
		}
	}

	if input.PriceOnly || ind == nil {
		return nil
	}

	uptrend := close > ind.SMA50 && ind.SMA50 > ind.SMA200
	notOverbought := ind.RSI14 < input.Cfg.RsiBuyCeiling
	if !(uptrend && notOverbought) {
		return nil
	}

	var qty float64
	if class == models.ClassMutualFund {
		qty = UnitsFloor(input.AvailableCash / close)
	} else {
		qty = math.Floor(input.AvailableCash / close)
	}
	if qty < 1.0 && !(class == models.ClassMutualFund && qty > 0.0) {
		return nil
	}

	reasons = append(reasons, fmt.Sprintf(
		"close %.2f above SMA50 %.2f and SMA200 %.2f", close, ind.SMA50, ind.SMA200))
	reasons = append(reasons, fmt.Sprintf(
		"RSI %.1f below buy ceiling %.1f", ind.RSI14, input.Cfg.RsiBuyCeiling))

	var stop, target *float64
	if class == models.ClassStock {
		stop = fptr(close - input.Cfg.StopATR*ind.ATR14)
		target = fptr(close + input.Cfg.TargetATR*ind.ATR14)
		// MF and IPO have no intraday stop: their execution is NAV-of-the-day or allotment.
	}

	return &models.Candidate{
		InstrumentID: input.Instrument.ID,
		Action:       models.ActionBuy,
		Reasons:      reasons,
		Entry:        close,
		Stop:         stop,
		Target:       target,
		Qty:          qty,
		TimingNote:   class.TimingNote(),
		Urgent:       false,
	}
}

// One named rule and whether it passed, with the numbers that decided it.
type Check struct {
	Name   string `json:"name"`
	Passed bool   `json:"passed"`
	Detail string `json:"detail"`
}

// The same predicates Candidate applies, but reported instead of collapsed into nil. This
// is what lets the UI answer "why is the ledger empty?".
func Diagnose(input CandidateInput) []Check {
	checks := []Check{}
	if len(input.Bars) == 0 {
		return []Check{{Name: "price data", Passed: false, Detail: "no bars stored"}}
	}
	close := input.Bars[len(input.Bars)-1].Close
	ind := input.Ind

	checks = append(checks, Check{
		Name: "price data", Passed: true,
		Detail: fmt.Sprintf("%d bars, last close %.2f", len(input.Bars), close),
	})
	checks = append(checks, Check{
		Name:   "trend",
		Passed: close > ind.SMA50 && ind.SMA50 > ind.SMA200,
		Detail: fmt.Sprintf("close %.2f vs SMA50 %.2f vs SMA200 %.2f", close, ind.SMA50, ind.SMA200),
	})
	checks = append(checks, Check{
		Name:   "not overbought",
		Passed: ind.RSI14 < input.Cfg.RsiBuyCeiling,
		Detail: fmt.Sprintf("RSI %.1f must be below %.0f", ind.RSI14, input.Cfg.RsiBuyCeiling),
	})
	var qty float64
	if input.Instrument.Class == models.ClassMutualFund {
		qty = UnitsFloor(input.AvailableCash / math.Max(close, 1e-9))
	} else {
		qty = math.Floor(input.AvailableCash / math.Max(close, 1e-9))
	}
	checks = append(checks, Check{
		Name:   "affordable",
		Passed: qty >= 1.0 && input.Instrument.Class != models.ClassMutualFund || qty > 0.0,
		Detail: fmt.Sprintf("%v units at %.2f against %.2f available", qty, close, input.AvailableCash),
	})
	if input.Position != nil {
		pos := input.Position
		stopHit := pos.Stop != nil && close < *pos.Stop
		detail := "no stop recorded"
		if pos.Stop != nil {
			detail = fmt.Sprintf("close %.2f vs stop %.2f", close, *pos.Stop)
		}
		checks = append(checks, Check{
			Name: "stop not breached", Passed: !stopHit, Detail: detail,
		})
	}

	return checks
}

// Why a holding has left its band, if it has. "" means the rules already cover it or it is
// inside the band.
func BandReason(stop *float64, avgCost, close, maxProfitPct float64) string {
	if avgCost > 0.0 && close >= avgCost*(1.0+maxProfitPct/100.0) {
		return "ceiling"
	}
	if stop != nil && close <= *stop {
		return "broken"
	}
	return ""
}

// Where a stop should sit given the price now, for a position already held.
//
// Two ATRs under the close, never under what was paid once the price is above it, never
// below the stop already recorded. Raise only.
func TrailedStop(current *float64, avgCost, close, atr float64) *float64 {
	if close <= 0.0 || avgCost <= 0.0 || math.IsNaN(atr) || math.IsInf(atr, 0) || atr <= 0.0 {
		return current
	}
	// Below what was paid there is nothing to protect: the stop is the owner's own.
	if close <= avgCost {
		return current
	}
	next := math.Max(close-2.0*atr, avgCost)
	if current != nil {
		next = math.Max(next, *current)
	}
	return fptr(next)
}
