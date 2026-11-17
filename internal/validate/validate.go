// Package validate is the leaf error vocabulary shared by the rules layer and the llm
// layer, so neither imports the other. The messages are the API's words; do not reword.
package validate

import "fmt"

// A stop may sit further from the close than the entry band; anything beyond this many
// ATRs is a typo or a hallucination, not a stop.
const (
	MaxStopATRs   = 5.0
	MaxTargetATRs = 10.0
)

type Kind string

const (
	KindStopOnWrongSide   Kind = "stop_on_wrong_side"
	KindPriceOutOfBand    Kind = "price_out_of_band"
	KindDistanceTooFar    Kind = "distance_too_far"
	KindInsufficientCash  Kind = "insufficient_cash"
	KindNotHeld           Kind = "not_held"
	KindBadValidity       Kind = "bad_validity"
	KindUnknownHeadlineID Kind = "unknown_headline_id"
	KindMissingCitation   Kind = "missing_citation"
	KindConfidenceTooLow  Kind = "confidence_too_low"
	KindMissingExit       Kind = "missing_exit"
	KindBadQuantity       Kind = "bad_quantity"
	KindTooManyUnits      Kind = "too_many_units"
	KindTargetBelowReq    Kind = "target_below_requirement"
	KindTargetAboveLimit  Kind = "target_above_limit"
	KindTradeTooSmall     Kind = "trade_too_small"
	KindTradeTooLarge     Kind = "trade_too_large"
	KindInstrumentLimit   Kind = "instrument_limit"
)

type Error struct {
	Kind        Kind
	RequiredPct float64 // TargetBelowRequirement
	MaxPct      float64 // TargetAboveLimit
	Min         float64 // TradeTooSmall
	Max         float64 // TradeTooLarge
	Cap         float64 // InstrumentLimit
	Already     float64 // InstrumentLimit
}

func (e Error) Error() string {
	switch e.Kind {
	case KindStopOnWrongSide:
		return "stop on the wrong side of entry"
	case KindPriceOutOfBand:
		return "entry is outside the ATR band around the last close"
	case KindDistanceTooFar:
		return "stop or target is implausibly far from the entry"
	case KindInsufficientCash:
		return "the trade costs more than the cash available"
	case KindNotHeld:
		return "sell proposed for an instrument that is not held"
	case KindBadValidity:
		return "valid_until is not a future session"
	case KindUnknownHeadlineID:
		return "cited headline id was never sent to the model"
	case KindMissingCitation:
		return "a decision must cite at least one headline"
	case KindConfidenceTooLow:
		return "a short-term buy must be high confidence — medium or low is refused"
	case KindMissingExit:
		return "a buy must carry both an exit stop and a take-profit target"
	case KindBadQuantity:
		return "quantity must be a positive, finite number"
	case KindTooManyUnits:
		return "cannot sell more units than are held"
	case KindTargetBelowReq:
		return fmt.Sprintf("take-profit does not reach the required %v%%", e.RequiredPct)
	case KindTargetAboveLimit:
		return fmt.Sprintf("take-profit exceeds the %v%% limit — do not chase fantasy targets", e.MaxPct)
	case KindTradeTooSmall:
		return fmt.Sprintf("trade is below the minimum of %v", e.Min)
	case KindTradeTooLarge:
		return fmt.Sprintf("trade is above the maximum of %v", e.Max)
	case KindInstrumentLimit:
		return fmt.Sprintf("this would put %v + the trade over the %v cap for one instrument", e.Already, e.Cap)
	}
	return string(e.Kind)
}
