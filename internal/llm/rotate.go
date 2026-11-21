package llm

import (
	"encoding/json"
	"fmt"
	"strings"

	"stock-predictor/internal/models"
	"stock-predictor/internal/rules"
)

// Portfolio rotation: sell something you hold to fund something better.
//
// One decision with two legs. The buy is only valid if the sell happens, so both are stored
// linked and approved together — a half-applied swap would leave the book in a state neither
// leg intended.

// One line of the holdings table the model is given.
type HoldingLine struct {
	Symbol  string
	Name    string
	Qty     float64
	AvgCost float64
	LTP     *float64
	Stop    *float64
	Target  *float64
}

func (h HoldingLine) ProfitPct() (float64, bool) {
	if h.LTP == nil || h.AvgCost <= 0 {
		return 0, false
	}
	return (*h.LTP - h.AvgCost) / h.AvgCost * 100.0, true
}

type Leg struct {
	Symbol     string   `json:"symbol"`
	Qty        float64  `json:"qty"`
	Price      float64  `json:"price"`
	Why        *string  `json:"why"`
	Rationale  *string  `json:"rationale"`
	Stop       *float64 `json:"stop"`
	Target     *float64 `json:"target"`
	Confidence *string  `json:"confidence"`
}

type Swap struct {
	Sell Leg
	Buy  Leg
}

type Rotation struct {
	Swap *Swap
	Pass bool
	Why  string
}

// The table the model reads. Compact on purpose: one line per holding.
func HoldingsTable(holdings []HoldingLine, cash float64, currency string) string {
	out := "symbol | name | qty | avg cost | last | P&L % | stop | target\n"
	for _, h := range holdings {
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
		out += fmt.Sprintf("%s | %s | %v | %.2f | %s | %s | %s | %s\n",
			h.Symbol, h.Name, h.Qty, h.AvgCost, ltp, pct, stop, target)
	}
	out += fmt.Sprintf("\ncash available: %.2f %s\n", cash, currency)
	return out
}

// Tolerates a fence or a stray sentence around the object.
func extractJSON(content string) (map[string]any, bool) {
	start := strings.Index(content, "{")
	end := strings.LastIndex(content, "}")
	if start < 0 || end < start {
		return nil, false
	}
	var body map[string]any
	if err := json.Unmarshal([]byte(content[start:end+1]), &body); err != nil {
		return nil, false
	}
	return body, true
}

func asFloat(v any) (float64, bool) {
	f, ok := v.(float64)
	return f, ok
}

// What the model answered, before it is trusted.
func ParseRotation(content string) (Rotation, error) {
	body, ok := extractJSON(content)
	if !ok {
		return Rotation{}, fmt.Errorf("rotation: no JSON in the reply")
	}
	action, _ := body["action"].(string)
	switch action {
	case "rotate", "decide":
		sellRaw, ok := body["sell"]
		if !ok {
			return Rotation{}, fmt.Errorf("rotation: no sell leg")
		}
		buyRaw, ok := body["buy"]
		if !ok {
			return Rotation{}, fmt.Errorf("rotation: no buy leg")
		}
		sellBytes, _ := json.Marshal(sellRaw)
		buyBytes, _ := json.Marshal(buyRaw)
		var sell, buy Leg
		if err := json.Unmarshal(sellBytes, &sell); err != nil {
			return Rotation{}, err
		}
		if err := json.Unmarshal(buyBytes, &buy); err != nil {
			return Rotation{}, err
		}
		return Rotation{Swap: &Swap{Sell: sell, Buy: buy}}, nil
	case "pass", "none":
		why, _ := body["why"].(string)
		if why == "" {
			why = "nothing worth selling"
		}
		return Rotation{Pass: true, Why: why}, nil
	default:
		return Rotation{}, fmt.Errorf("rotation: unexpected action %q", action)
	}
}

// Why a swap is not allowed. Every message is user-facing.
func ValidateRotation(r Rotation, holdings []HoldingLine, cash float64, limits rules.Limits, mode models.Mode) error {
	if r.Swap == nil {
		return nil
	}
	sell, buy := r.Swap.Sell, r.Swap.Buy

	var held *HoldingLine
	for i := range holdings {
		if strings.EqualFold(holdings[i].Symbol, sell.Symbol) {
			held = &holdings[i]
			break
		}
	}
	if held == nil {
		return fmt.Errorf("rotation: %s is not held", sell.Symbol)
	}
	if !finite(sell.Qty) || sell.Qty <= 0.0 || sell.Qty > held.Qty+1e-9 {
		return fmt.Errorf("rotation: cannot sell %v of %s, only %v held", sell.Qty, sell.Symbol, held.Qty)
	}
	if !finite(sell.Price) || sell.Price <= 0.0 {
		return fmt.Errorf("rotation: the sell price is not a price")
	}
	// Selling a loss to buy something else is not a rotation, it is a way to lock in damage.
	pct, ok := held.ProfitPct()
	if !ok {
		return fmt.Errorf("rotation: %s has no live price", held.Symbol)
	}
	if pct < limits.MinProfitPct {
		return fmt.Errorf(
			"rotation: %s is only %.1f%% up, below the %.1f%% the pot expects — not a profit to take",
			held.Symbol, pct, limits.MinProfitPct)
	}

	if !finite(buy.Qty) || buy.Qty <= 0.0 || !finite(buy.Price) || buy.Price <= 0.0 {
		return fmt.Errorf("rotation: the buy leg is not a trade")
	}
	proceeds := sell.Qty * sell.Price
	cost := buy.Qty * buy.Price
	if cost > proceeds+cash+1e-9 {
		return fmt.Errorf("rotation: cost %.2f exceeds proceeds %.2f plus cash %.2f", cost, proceeds, cash)
	}
	if cost > limits.MaxTradeAmount+1e-9 {
		return fmt.Errorf("rotation: %.2f is above the pot's largest trade %.2f", cost, limits.MaxTradeAmount)
	}
	if buy.Stop != nil && buy.Target != nil {
		if !finite(*buy.Stop) || *buy.Stop >= buy.Price {
			return fmt.Errorf("rotation: stop must sit below the buy price")
		}
		if !finite(*buy.Target) || *buy.Target <= buy.Price {
			return fmt.Errorf("rotation: target must sit above the buy price")
		}
		floor := buy.Price * (1.0 + limits.MinProfitPct/100.0)
		if *buy.Target+1e-9 < floor {
			return fmt.Errorf("rotation: the target is below the pot's minimum profit of %.1f%%", limits.MinProfitPct)
		}
	} else {
		return fmt.Errorf("rotation: the buy needs both a stop and a target")
	}

	if strings.EqualFold(buy.Symbol, sell.Symbol) {
		return fmt.Errorf("rotation: selling and buying the same symbol")
	}
	return nil
}
