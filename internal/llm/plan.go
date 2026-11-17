package llm

import (
	"encoding/json"
	"fmt"
	"strings"
	"time"

	"stock-predictor/internal/models"
	"stock-predictor/internal/rules"
)

// The book call: the model sees every holding and the cash, and may answer with a plan of up
// to three moves that stand or fall together.
//
// The parse is forgiving about the wrapper and strict about the content; the validation
// refuses the whole plan rather than repairing it, for the same reason every other validator
// here does — a half-executed swap is worse than none.

// More than this is not a plan, it is a wish list.
const MaxMoves = 3

// A trade plan is allowed at most this many moves: one exit and the one entry it funds.
const MaxTradeMoves = 2

// The furthest a trade idea may be said to last. Three sessions is a delivery trade; beyond
// that it is the long-term book's business.
const TradeMaxSessions = 3

// The close of the last session a trade idea may name. Weekends are skipped.
func SessionDeadline(now time.Time, sessions int) time.Time {
	day := now
	left := sessions
	if left < 0 {
		left = 0
	}
	for left > 0 {
		day = day.AddDate(0, 0, 1)
		if day.Weekday() != time.Saturday && day.Weekday() != time.Sunday {
			left--
		}
	}
	// The close of that session: 15:30 IST == 10:00 UTC, and NSE has no DST.
	return time.Date(day.Year(), day.Month(), day.Day(), 10, 0, 0, 0, time.UTC)
}

type Move struct {
	Action models.Action `json:"action"`
	Symbol string        `json:"symbol"`
	// Units for a stock plan; a fund plan sizes in rupees instead, because units arrive at
	// NAV.
	Qty   *float64 `json:"qty"`
	Price *float64 `json:"price"`
	// Rupees, for a fund plan. The purse is denominated in money, not units.
	Amount *float64 `json:"amount"`
	Stop   *float64 `json:"stop"`
	Target *float64 `json:"target"`
	Why    string   `json:"why"`
	// When the idea goes stale. Required by a trade plan, ignored by the others.
	ValidUntil *string `json:"valid_until"`
}

type Plan struct {
	Moves     []Move
	Rationale string
}

type Answer struct {
	// Exactly one of Plan / Pass is set.
	Plan *Plan
	Pass bool
	Why  string
}

// The model's reply, before it is trusted.
func ParsePlan(content string) (Answer, error) {
	body, ok := extractJSON(content)
	if !ok {
		return Answer{}, fmt.Errorf("plan: no JSON in the reply")
	}
	raw := []any{}
	if arr, ok := body["moves"].([]any); ok {
		raw = arr
	}
	if len(raw) == 0 {
		why, _ := body["pass_reason"].(string)
		if why == "" {
			why = "nothing worth doing today"
		}
		return Answer{Pass: true, Why: why}, nil
	}
	if len(raw) > MaxMoves {
		return Answer{}, fmt.Errorf("plan: %d moves is more than the %d allowed", len(raw), MaxMoves)
	}
	moves := make([]Move, 0, len(raw))
	for _, item := range raw {
		b, err := json.Marshal(item)
		if err != nil {
			return Answer{}, err
		}
		var m Move
		if err := json.Unmarshal(b, &m); err != nil {
			return Answer{}, err
		}
		if m.Action != models.ActionBuy && m.Action != models.ActionSell {
			return Answer{}, fmt.Errorf("a plan moves BUY or SELL, not %s", m.Action)
		}
		moves = append(moves, m)
	}
	rationale, _ := body["rationale"].(string)
	return Answer{Plan: &Plan{Moves: moves, Rationale: rationale}}, nil
}

// Cash free in each purse, before this plan's own sales.
type Cash struct {
	Stock float64
	Fund  float64
	Trade float64
}

func (c Cash) get(scope Scope) float64 {
	switch scope {
	case ScopeFund:
		return c.Fund
	case ScopeTrade:
		return c.Trade
	}
	return c.Stock
}

// Why a plan is not allowed. Every message is user-facing.
//
// Leg by leg first — a symbol that is not held, a stop on the wrong side, a quantity that is
// not a number — then the plan as a whole: buys may not cost more than cash plus what the
// plan's own sells free, and no symbol may appear twice.
func ValidatePlan(plan Plan, book []HoldingLine, cash Cash, limits rules.Limits, currency string, now time.Time, funds []string, scope Scope) error {
	// A plan has one purse. The others are reported as zero, so a mixed plan cannot balance.
	purse := cash.get(scope)
	freed := 0.0
	spent := 0.0
	seen := []string{}

	for _, m := range plan.Moves {
		isFund := false
		for _, f := range funds {
			if f == m.Symbol {
				isFund = true
				break
			}
		}
		switch {
		case scope == ScopeTrade && isFund:
			return fmt.Errorf("%s is a mutual fund: the trade plan may only move stocks", m.Symbol)
		case scope == ScopeStock && isFund:
			return fmt.Errorf("%s is a mutual fund: it belongs to the fund plan, not the stock plan", m.Symbol)
		case scope == ScopeFund && !isFund:
			return fmt.Errorf("%s is a stock: the fund plan may only move mutual funds", m.Symbol)
		}
		if containsStr(seen, m.Symbol) {
			return fmt.Errorf("plan moves %s twice", m.Symbol)
		}
		seen = append(seen, m.Symbol)

		if scope == ScopeTrade && m.Action == models.ActionBuy {
			// The short-term book enters once. A second entry on a name already held is how one
			// up-move becomes three buys at higher prices and targets that were never reachable.
			for _, h := range book {
				if h.Symbol == m.Symbol {
					return fmt.Errorf("%s is already held in the short-term book — a second entry at a higher price is not this book's trade", m.Symbol)
				}
			}
			if m.ValidUntil == nil || strings.TrimSpace(*m.ValidUntil) == "" {
				return fmt.Errorf("%s has no expiry — a trade move goes stale", m.Symbol)
			}
			until := strings.TrimSpace(*m.ValidUntil)
			parsed, err := time.Parse(time.RFC3339, until)
			if err != nil {
				return fmt.Errorf("%s: expiry %s is not a time", m.Symbol, until)
			}
			parsed = parsed.UTC()
			if !parsed.After(now) {
				return fmt.Errorf("%s: expiry %s is already past", m.Symbol, until)
			}
			if parsed.After(SessionDeadline(now, TradeMaxSessions)) {
				return fmt.Errorf("%s: expiry is more than %d sessions out — that is not this book's trade", m.Symbol, TradeMaxSessions)
			}
		}

		// Money is the unit of account: stock plans price it as qty × price, fund plans take
		// rupees.
		var value float64
		if scope == ScopeFund {
			if m.Amount == nil || !finite(*m.Amount) || *m.Amount <= 0 {
				return fmt.Errorf("%s has no amount — a fund move is sized in rupees", m.Symbol)
			}
			value = *m.Amount
		} else {
			if m.Qty == nil || !finite(*m.Qty) || *m.Qty <= 0 {
				return fmt.Errorf("%s has no usable quantity", m.Symbol)
			}
			if m.Price == nil || !finite(*m.Price) || *m.Price <= 0 {
				return fmt.Errorf("%s has no usable price", m.Symbol)
			}
			value = *m.Qty * *m.Price
		}

		switch m.Action {
		case models.ActionSell:
			var held *HoldingLine
			for i := range book {
				if book[i].Symbol == m.Symbol {
					held = &book[i]
					break
				}
			}
			if held == nil {
				return fmt.Errorf("%s is not held", m.Symbol)
			}
			if m.Qty != nil && *m.Qty > held.Qty+1e-9 {
				return fmt.Errorf("%s sells %.3f but only %.3f is held", m.Symbol, *m.Qty, held.Qty)
			}
			if scope == ScopeFund {
				ltp := held.AvgCost
				if held.LTP != nil {
					ltp = *held.LTP
				}
				heldValue := held.Qty * ltp
				if value > heldValue+1e-6 {
					return fmt.Errorf("%s sells %.2f but only %.2f is held", m.Symbol, value, heldValue)
				}
			}
			freed += value
		case models.ActionBuy:
			// A stock buy without an exit is a position with no way out.
			if scope != ScopeFund {
				if m.Stop == nil || m.Target == nil {
					return fmt.Errorf("%s needs a stop and a target", m.Symbol)
				}
				price := 0.0
				if m.Price != nil {
					price = *m.Price
				}
				if !finite(*m.Stop) || *m.Stop >= price {
					return fmt.Errorf("%s: stop on the wrong side of entry", m.Symbol)
				}
				if !finite(*m.Target) || *m.Target <= price {
					return fmt.Errorf("%s: target on the wrong side of entry", m.Symbol)
				}
			}
			spent += value
		}
	}

	if spent > freed+purse+1e-6 {
		return fmt.Errorf(
			"the %s purse does not balance: buys cost %.2f but the plan frees %.2f plus %.2f %s of %s cash",
			scope.PurseName(), spent, freed, purse, currency, scope.PurseName())
	}
	if spent > limits.MaxTradeAmount+freed+1e-6 {
		return fmt.Errorf("the plan spends %.2f, above the %.2f ceiling for one plan", spent, limits.MaxTradeAmount)
	}
	return nil
}

// The book as the prompt reads it: one line per holding, richest first.
func BookBlock(book []HoldingLine, currency string) string {
	out := "symbol | name | qty | avg cost | last | P&L % | stop | target\n"
	invested := 0.0
	for _, h := range book {
		pct := "—"
		if p, ok := h.ProfitPct(); ok {
			pct = fmt.Sprintf("%+.1f%%", p)
		}
		ltp := "—"
		if h.LTP != nil {
			ltp = fmt.Sprintf("%.2f", *h.LTP)
		}
		stop := "—"
		if h.Stop != nil {
			stop = fmt.Sprintf("%.2f", *h.Stop)
		}
		target := "—"
		if h.Target != nil {
			target = fmt.Sprintf("%.2f", *h.Target)
		}
		out += fmt.Sprintf("%s | %s | %.3f | %.2f | %s | %s | %s | %s\n",
			h.Symbol, h.Name, h.Qty, h.AvgCost, ltp, pct, stop, target)
		invested += h.Qty * h.AvgCost
	}
	out += fmt.Sprintf("\ntotal invested: %.2f %s\n", invested, currency)
	return out
}

func containsStr(list []string, s string) bool {
	for _, v := range list {
		if v == s {
			return true
		}
	}
	return false
}
