package llm

import (
	"database/sql"
	"fmt"
	"log/slog"
	"time"

	"stock-predictor/internal/market"
	"stock-predictor/internal/models"
	"stock-predictor/internal/rules"
	"stock-predictor/internal/validate"
)

const (
	MaxHeadlinesPerEscalation = 3
	MaxCharsPerArticle        = 4_000
	MaxTotalFetchedChars      = 10_000
	// Hard ceiling on any historical window the model can trigger. Requests above this are
	// clamped, never honoured — 7 days is the most the prompt will ever carry.
	MaxHistoryDays = 7
)

func HistoryRange() string {
	return fmt.Sprintf("%dd", MaxHistoryDays)
}

type LlmDeps struct {
	Pool      *sql.DB
	Runtime   rules.Runtime
	Transport Transport
	Market    market.MarketSource
}

type RunInput struct {
	CallReason         *string
	Instrument         *models.Instrument
	Ind                *rules.Indicators
	Candidate          models.Candidate
	AvailableCash      float64
	HeldQty            float64
	Limits             rules.Limits
	RequiredProfitPct  float64
	InstrumentExposure float64
	Pot                float64
	WindowHours        uint32
	Headlines          []models.Headline
	PriceWindow        string
	DailySummary       string
	Fundamentals       string
	ShortContext       string
	Mode               models.Mode
	Positions          string
	RecentTrades       string
	LastDecision       string
	Now                time.Time
}

type OutcomeKind int

const (
	OutcomeLlm OutcomeKind = iota
	OutcomeRules
	// The model answered, but the proposal failed validation. The caller decides the
	// consequence: over-budget becomes BLOCKED, everything else falls back to rules.
	OutcomeRejected
	OutcomePass
)

type Outcome struct {
	Kind      OutcomeKind
	Proposal  *Proposal
	Candidate *models.Candidate
	Error     *validate.Error
	Why       string
}

type Decision struct {
	Outcome        Outcome
	ConversationID string
}

func Run(deps *LlmDeps, input RunInput) (Decision, error) {
	conversationID := fmt.Sprintf("%s-%d", input.Instrument.Symbol, input.Now.UnixMilli())

	calls, err := DailyChatCalls(deps.Pool)
	if err != nil {
		return Decision{}, err
	}
	if calls >= int64(deps.Runtime.DailyCap) {
		return Decision{Outcome: Outcome{Kind: OutcomeRules, Candidate: &input.Candidate}, ConversationID: conversationID}, nil
	}

	headlines := make([]models.Headline, len(input.Headlines))
	copy(headlines, input.Headlines)
	for i := range headlines {
		headlines[i].ID = int64(i) + 1
	}
	sentIDs := make([]int64, len(headlines))
	for i, h := range headlines {
		sentIDs[i] = h.ID
	}

	requiredProfit := input.RequiredProfitPct
	base := &PromptContext{
		Instrument:      input.Instrument,
		Ind:             input.Ind,
		LastClose:       input.Candidate.Entry,
		CashAvailable:   input.AvailableCash,
		TargetProfitPct: &requiredProfit,
		Limits:          LimitsBlock(input.Limits, input.Pot, input.InstrumentExposure),
		Headlines:       headlines,
		WindowHours:     input.WindowHours,
		PriceWindow:     input.PriceWindow,
		DailyDays:       10,
		DailySummary:    input.DailySummary,
		Fundamentals:    input.Fundamentals,
		ShortContext:    input.ShortContext,
		Positions:       input.Positions,
		RecentTrades:    input.RecentTrades,
		LastDecision:    input.LastDecision,
		Now:             input.Now,
	}

	rendered, err := Render("decide_v1", base)
	if err != nil {
		return Decision{}, err
	}
	model, err := rules.ModelConfigFor(deps.Pool)
	if err != nil {
		return Decision{}, err
	}
	reply, err := Chat(deps.Pool, &model, deps.Transport, ChatRequest{
		ConversationID: conversationID,
		Round:          1,
		Prompt:         rendered.Text,
		PromptTemplate: rendered.Template,
		PromptHash:     rendered.Hash,
		Trigger:        input.CallReason,
		Purpose:        "decide " + input.Instrument.Symbol,
	})
	if err != nil {
		slog.Warn("phase 1 failed, using rules numbers", "error", err)
		return rulesOnly(input, conversationID), nil
	}
	verdict, err := Verdict(reply.Content)
	if err != nil {
		slog.Warn("phase 1 unparseable, using rules numbers", "error", err)
		return rulesOnly(input, conversationID), nil
	}

	final := verdict
	if verdict.NeedMore != nil {
		more := *verdict.NeedMore
		wanted := []int64{}
		for _, req := range more.Headlines {
			if len(wanted) == MaxHeadlinesPerEscalation {
				break
			}
			if containsID(sentIDs, req.HeadlineID) {
				wanted = append(wanted, req.HeadlineID)
			}
		}

		extracts := []Extract{}
		canFetch := model.Provider.SupportsArticleFetch()
		budgetChars := MaxTotalFetchedChars
		for _, id := range wanted {
			var h *models.Headline
			for i := range headlines {
				if headlines[i].ID == id {
					h = &headlines[i]
					break
				}
			}
			if h == nil || !canFetch {
				continue
			}
			text, err := FetchPage(deps.Pool, model.BaseURL, deps.Transport, h.URL)
			if err != nil {
				slog.Warn("fetch failed", "error", err, "headline", id)
				continue
			}
			cap := MaxCharsPerArticle
			if budgetChars < cap {
				cap = budgetChars
			}
			if cap == 0 {
				break
			}
			truncated := truncateChars(text, cap)
			budgetChars -= len([]rune(truncated))
			extracts = append(extracts, Extract{HeadlineID: id, Text: truncated})
		}

		// The second tier of the data pyramid: hourly bars, never more than 7 days.
		history := ""
		if more.PriceHistory {
			bars, err := deps.Market.Intraday(input.Instrument.Symbol, "60m", HistoryRange())
			if err != nil {
				slog.Warn("price history unavailable", "error", err)
				history = "(unavailable)"
			} else {
				history = FormatHistory(bars)
			}
		}

		if len(extracts) == 0 && history == "" {
			final = LlmVerdict{Pass: true, Why: "asked for more, then received nothing usable"}
		} else {
			ctx2 := *base
			ctx2.Extracts = extracts
			ctx2.PriceHistory = history
			rendered2, err := Render("escalate_v1", &ctx2)
			if err != nil {
				return Decision{}, err
			}
			trigger := "the first answer asked for more data"
			reply2, err2 := Chat(deps.Pool, &model, deps.Transport, ChatRequest{
				ConversationID: conversationID,
				Round:          2,
				Prompt:         rendered2.Text,
				PromptTemplate: rendered2.Template,
				PromptHash:     rendered2.Hash,
				Trigger:        &trigger,
				Purpose:        "escalate " + input.Instrument.Symbol,
			})
			if err2 != nil {
				final = LlmVerdict{Pass: true, Why: "escalation did not resolve to a decision"}
			} else if v2, err3 := Verdict(reply2.Content); err3 != nil {
				final = LlmVerdict{Pass: true, Why: "escalation did not resolve to a decision"}
			} else if v2.NeedMore != nil {
				// One escalation only; a second request is treated as a pass.
				final = LlmVerdict{Pass: true, Why: "escalation did not resolve to a decision"}
			} else {
				final = v2
			}
		}
	}

	switch {
	case final.Decide != nil:
		ctx := ValidateCtx{
			// The trade book's horizon forces an exit, so it demands high confidence.
			RequiresHighConfidence: input.Mode == models.ModeTrade,
			Bands:                  rules.ATRBands(input.Candidate.Entry, input.Ind.ATR14),
			ATR:                    input.Ind.ATR14,
			AvailableCash:          input.AvailableCash,
			Qty:                    input.Candidate.Qty,
			HeldQty:                input.HeldQty,
			SentHeadlineIDs:        sentIDs,
			RequiresExit:           input.Instrument.Class == models.ClassStock,
			Limits:                 input.Limits,
			RequiredProfitPct:      input.RequiredProfitPct,
			InstrumentExposure:     input.InstrumentExposure,
			Pot:                    input.Pot,
			Now:                    input.Now,
		}
		if err := CheckProposal(final.Decide, ctx); err != nil {
			slog.Warn("llm proposal rejected", "error", err)
			var verr validate.Error
			switch e := err.(type) {
			case validate.Error:
				verr = e
			default:
				verr = validate.Error{Kind: validate.KindPriceOutOfBand}
			}
			return Decision{
				Outcome:        Outcome{Kind: OutcomeRejected, Proposal: final.Decide, Error: &verr},
				ConversationID: conversationID,
			}, nil
		}
		return Decision{Outcome: Outcome{Kind: OutcomeLlm, Proposal: final.Decide}, ConversationID: conversationID}, nil
	case final.Pass:
		return Decision{Outcome: Outcome{Kind: OutcomePass, Why: final.Why}, ConversationID: conversationID}, nil
	default:
		// A second need_more that survived here falls back to the rules numbers.
		return rulesOnly(input, conversationID), nil
	}
}

func rulesOnly(input RunInput, conversationID string) Decision {
	c := input.Candidate
	return Decision{Outcome: Outcome{Kind: OutcomeRules, Candidate: &c}, ConversationID: conversationID}
}

// Hourly bars rendered compactly: one line per bar, oldest first.
func FormatHistory(bars []models.Bar) string {
	ist := time.FixedZone("IST", 330*60)
	lines := make([]string, len(bars))
	for i, b := range bars {
		ts := b.Ts
		if parsed, err := time.Parse(time.RFC3339, b.Ts); err == nil {
			ts = parsed.In(ist).Format("01-02 15:04")
		}
		lines[i] = fmt.Sprintf("%s %.2f %.2f %.2f %.2f %d", ts, b.Open, b.High, b.Low, b.Close, b.Volume)
	}
	out := ""
	for i, line := range lines {
		if i > 0 {
			out += "\n"
		}
		out += line
	}
	return out
}

// The limits as one readable paragraph, with the room that is actually left.
func LimitsBlock(limits rules.Limits, pot, alreadySpent float64) string {
	cap := pot * limits.MaxInstrumentPct / 100.0
	room := cap - alreadySpent
	if room < 0 {
		room = 0
	}
	return fmt.Sprintf(
		"trade size %.2f to %.2f per fill; at most %.2f in this instrument (%.0f%% of the pot), of which %.2f is already committed, leaving %.2f of room; take-profit between %.1f%% and %.1f%% above entry",
		limits.MinTradeAmount, limits.MaxTradeAmount, cap, limits.MaxInstrumentPct,
		alreadySpent, room, limits.MinProfitPct, limits.MaxProfitPct)
}

func DailyChatCalls(db *sql.DB) (int64, error) {
	start := time.Now().UTC().Format("2006-01-02")
	var n int64
	err := db.QueryRow(
		"SELECT COUNT(*) FROM api_calls WHERE endpoint = 'chat' AND ts >= ?", start,
	).Scan(&n)
	return n, err
}

func truncateChars(s string, n int) string {
	runes := []rune(s)
	if len(runes) <= n {
		return s
	}
	return string(runes[:n])
}
