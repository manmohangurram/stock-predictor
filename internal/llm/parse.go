package llm

import (
	"encoding/json"
	"fmt"
	"strings"

	"stock-predictor/internal/models"
)

type Confidence string

const (
	ConfLow    Confidence = "low"
	ConfMedium Confidence = "medium"
	ConfHigh   Confidence = "high"
)

type HeadlineRequest struct {
	HeadlineID int64  `json:"headline_id"`
	Why        string `json:"why"`
}

func (h *HeadlineRequest) UnmarshalJSON(b []byte) error {
	var raw map[string]json.RawMessage
	if err := json.Unmarshal(b, &raw); err != nil {
		return err
	}
	for _, key := range []string{"headline_id", "id", "headlineId"} {
		if v, ok := raw[key]; ok {
			if err := json.Unmarshal(v, &h.HeadlineID); err != nil {
				return err
			}
			break
		}
	}
	if v, ok := raw["why"]; ok {
		json.Unmarshal(v, &h.Why)
	}
	return nil
}

// What the model asked for in its one escalation: specific article bodies, the coarser
// price history, or both.
type MoreRequest struct {
	Headlines    []HeadlineRequest
	PriceHistory bool
}

func (m MoreRequest) IsEmpty() bool {
	return len(m.Headlines) == 0 && !m.PriceHistory
}

type Proposal struct {
	Action     models.Action
	Qty        *float64
	Entry      float64
	Stop       *float64
	Target     *float64
	ValidUntil string
	Confidence Confidence
	Rationale  string
	Cites      []int64
}

type LlmVerdict struct {
	Decide   *Proposal
	NeedMore *MoreRequest
	Pass     bool
	Why      string
}

type envelope struct {
	Action       string            `json:"action"`
	Signal       *signalJSON       `json:"signal"`
	Headlines    []HeadlineRequest `json:"headlines"`
	PriceHistory json.RawMessage   `json:"price_history"`
	Why          *string           `json:"why"`
}

type signalJSON struct {
	Action     string     `json:"action"`
	Qty        *float64   `json:"qty"`
	Entry      float64    `json:"entry"`
	Stop       *float64   `json:"stop"`
	Target     *float64   `json:"target"`
	ValidUntil string     `json:"valid_until"`
	Confidence Confidence `json:"confidence"`
	Rationale  string     `json:"rationale"`
	Cites      []int64    `json:"cites"`
}

// Anything that fails the shape is an error: the caller degrades to the rules candidate
// rather than guessing what a truncated response meant.
func Verdict(content string) (LlmVerdict, error) {
	var env envelope
	// `headlines` is what the prompt asks for; `requests` is accepted for older transcripts.
	var raw map[string]json.RawMessage
	if err := json.Unmarshal([]byte(content), &raw); err != nil {
		return LlmVerdict{}, fmt.Errorf("llm response is not the agreed shape: %w", err)
	}
	if r, ok := raw["requests"]; ok {
		if _, has := raw["headlines"]; !has {
			raw["headlines"] = r
		}
	}
	merged, _ := json.Marshal(raw)
	if err := json.Unmarshal(merged, &env); err != nil {
		return LlmVerdict{}, fmt.Errorf("llm response is not the agreed shape: %w", err)
	}

	switch env.Action {
	case "decide":
		if env.Signal == nil {
			return LlmVerdict{}, fmt.Errorf("decide without signal")
		}
		s := env.Signal
		action, err := models.ParseAction(s.Action)
		if err != nil {
			return LlmVerdict{}, err
		}
		return LlmVerdict{Decide: &Proposal{
			Action: action, Qty: s.Qty, Entry: s.Entry, Stop: s.Stop, Target: s.Target,
			ValidUntil: s.ValidUntil, Confidence: s.Confidence, Rationale: s.Rationale, Cites: s.Cites,
		}}, nil
	case "need_more":
		more := MoreRequest{Headlines: env.Headlines}
		// Models like to send true, "7d" or 7 here; any truthy value means yes.
		var truthy any
		if len(env.PriceHistory) > 0 {
			json.Unmarshal(env.PriceHistory, &truthy)
		}
		switch v := truthy.(type) {
		case bool:
			more.PriceHistory = v
		case string:
			more.PriceHistory = v != "" && v != "false"
		case float64:
			more.PriceHistory = v > 0
		}
		if more.IsEmpty() {
			return LlmVerdict{}, fmt.Errorf("need_more with nothing requested")
		}
		return LlmVerdict{NeedMore: &more}, nil
	case "pass":
		why := "no reason given"
		if env.Why != nil {
			why = *env.Why
		}
		return LlmVerdict{Pass: true, Why: why}, nil
	default:
		return LlmVerdict{}, fmt.Errorf("unknown verdict action %q", strings.TrimSpace(env.Action))
	}
}
