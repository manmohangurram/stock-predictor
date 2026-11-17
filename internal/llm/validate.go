package llm

import (
	"time"

	"stock-predictor/internal/models"
	"stock-predictor/internal/rules"
	"stock-predictor/internal/validate"
)

type ValidateCtx struct {
	// True for the short-term book: a buy there must be high confidence, because the horizon
	// forces an exit whether the trade is working or not.
	RequiresHighConfidence bool
	// Entry must sit inside this band; stop and target are deliberately outside it.
	Bands           rules.Bands
	ATR             float64
	AvailableCash   float64
	Qty             float64
	HeldQty         float64
	SentHeadlineIDs []int64
	// True when this instrument can actually be exited at a price (stocks). Funds and IPOs
	// cannot, so requiring an exit from them would be nonsense.
	RequiresExit bool
	// The limits for the pot this trade belongs to.
	Limits rules.Limits
	// This stock's profit expectation, defaulted to the band's minimum.
	RequiredProfitPct float64
	// What this pot has already committed to this instrument.
	InstrumentExposure float64
	// The pot's ceiling, which the percentage caps are measured against.
	Pot float64
	Now time.Time
}

func finite(v float64) bool {
	return !isNaN(v) && !isInf(v)
}

func isNaN(v float64) bool { return v != v }

func isInf(v float64) bool { return v > 1.7976931348623157e308 || v < -1.7976931348623157e308 }

func abs(v float64) float64 {
	if v < 0 {
		return -v
	}
	return v
}

func CheckProposal(p *Proposal, ctx ValidateCtx) error {
	if !finite(p.Entry) || p.Entry <= 0.0 {
		return validate.Error{Kind: validate.KindPriceOutOfBand}
	}

	switch p.Action {
	case models.ActionBuy:
		// Half a case is no case when the trade must close inside a week.
		if ctx.RequiresHighConfidence && p.Confidence != ConfHigh {
			return validate.Error{Kind: validate.KindConfidenceTooLow}
		}
		if ctx.RequiresExit && (p.Stop == nil || p.Target == nil) {
			return validate.Error{Kind: validate.KindMissingExit}
		}
		if p.Stop != nil {
			// Written as a positive test so a NaN stop fails instead of slipping through.
			if !finite(*p.Stop) || *p.Stop >= p.Entry {
				return validate.Error{Kind: validate.KindStopOnWrongSide}
			}
		}
	case models.ActionSell:
		if ctx.HeldQty <= 0.0 {
			return validate.Error{Kind: validate.KindNotHeld}
		}
		if p.Stop != nil {
			if !finite(*p.Stop) || *p.Stop <= p.Entry {
				return validate.Error{Kind: validate.KindStopOnWrongSide}
			}
		}
	}

	if p.Entry < ctx.Bands.Min || p.Entry > ctx.Bands.Max {
		return validate.Error{Kind: validate.KindPriceOutOfBand}
	}

	atr := ctx.ATR
	if atr < 1e-9 {
		atr = 1e-9
	}
	if p.Stop != nil && abs(p.Entry-*p.Stop) > validate.MaxStopATRs*atr {
		return validate.Error{Kind: validate.KindDistanceTooFar}
	}
	if p.Target != nil && abs(*p.Target-p.Entry) > validate.MaxTargetATRs*atr {
		return validate.Error{Kind: validate.KindDistanceTooFar}
	}

	// The model may size the trade; if it does, its number is what gets checked.
	if p.Qty != nil {
		if !finite(*p.Qty) || *p.Qty <= 0.0 {
			return validate.Error{Kind: validate.KindBadQuantity}
		}
		if p.Action == models.ActionSell && *p.Qty > ctx.HeldQty+1e-9 {
			return validate.Error{Kind: validate.KindTooManyUnits}
		}
	}
	units := ctx.Qty
	if p.Qty != nil {
		units = *p.Qty
	}
	if p.Action == models.ActionBuy && units*p.Entry > ctx.AvailableCash+1e-9 {
		return validate.Error{Kind: validate.KindInsufficientCash}
	}

	// Size limits, applied to both directions: a sale is a transaction too.
	amount := units * p.Entry
	if err := ctx.Limits.Check(amount); err != nil {
		return err
	}
	if p.Action == models.ActionBuy {
		if err := ctx.Limits.CheckInstrument(ctx.Pot, ctx.InstrumentExposure, amount); err != nil {
			return err
		}
		if p.Target != nil {
			achieved := (*p.Target - p.Entry) / p.Entry * 100.0
			if err := ctx.Limits.CheckProfit(achieved, ctx.RequiredProfitPct); err != nil {
				return err
			}
		}
	}

	until, err := time.Parse(time.RFC3339, p.ValidUntil)
	if err != nil {
		return validate.Error{Kind: validate.KindBadValidity}
	}
	if !until.After(ctx.Now) {
		return validate.Error{Kind: validate.KindBadValidity}
	}

	if len(ctx.SentHeadlineIDs) > 0 && len(p.Cites) == 0 {
		return validate.Error{Kind: validate.KindMissingCitation}
	}
	for _, id := range p.Cites {
		if !containsID(ctx.SentHeadlineIDs, id) {
			return validate.Error{Kind: validate.KindUnknownHeadlineID}
		}
	}

	return nil
}

func containsID(ids []int64, id int64) bool {
	for _, v := range ids {
		if v == id {
			return true
		}
	}
	return false
}
