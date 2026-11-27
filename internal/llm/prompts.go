// Package llm ports the model-client layer: prompts, parsing, validation, audit, and the
// DeepSeek transport.
package llm

import (
	"crypto/sha256"
	"embed"
	"fmt"
	"strings"
	"time"

	"stock-predictor/internal/models"
	"stock-predictor/internal/rules"
)

//go:embed prompts/*.md
var promptFS embed.FS

var templates = map[string]string{
	"decide_v1":     mustPrompt("decide_v1.md"),
	"escalate_v1":   mustPrompt("escalate_v1.md"),
	"rotate_v1":     mustPrompt("rotate_v1.md"),
	"trending_v1":   mustPrompt("trending_v1.md"),
	"plan_stock_v1": mustPrompt("plan_stock_v1.md"),
	"plan_mf_v1":    mustPrompt("plan_mf_v1.md"),
	"plan_trade_v1": mustPrompt("plan_trade_v1.md"),
}

func mustPrompt(name string) string {
	b, err := promptFS.ReadFile("prompts/" + name)
	if err != nil {
		panic(err)
	}
	return string(b)
}

type Scope string

const (
	ScopeStock Scope = "stock"
	ScopeFund  Scope = "fund"
	ScopeTrade Scope = "trade"
)

// The old name plan_v1 still resolves to the stock prompt, so an audit row written before
// the split still renders.
func (s Scope) Template() string {
	switch s {
	case ScopeFund:
		return "plan_mf_v1"
	case ScopeTrade:
		return "plan_trade_v1"
	}
	return "plan_stock_v1"
}

func (s Scope) IsFund() bool  { return s == ScopeFund }
func (s Scope) IsTrade() bool { return s == ScopeTrade }

// The word used in a refusal, so the message names the purse the reader thinks in.
func (s Scope) PurseName() string {
	switch s {
	case ScopeFund:
		return "fund"
	case ScopeTrade:
		return "trade"
	}
	return "stock"
}

type RenderedPrompt struct {
	Text     string `json:"text"`
	Hash     string `json:"hash"`
	Template string `json:"template"`
}

// Everything the model is allowed to see, assembled by the caller. Nothing reaches the
// prompt that is not in here — which is what keeps article bodies off the wire until asked.
type PromptContext struct {
	Instrument      *models.Instrument
	Ind             *rules.Indicators
	LastClose       float64
	CashAvailable   float64
	TargetProfitPct *float64
	Limits          string
	Headlines       []models.Headline
	WindowHours     uint32
	PriceWindow     string
	DailyDays       uint32
	DailySummary    string
	Fundamentals    string
	ShortContext    string
	Positions       string
	RecentTrades    string
	LastDecision    string
	Now             time.Time
	Extracts        []Extract
	PriceHistory    string
}

type Extract struct {
	HeadlineID int64
	Text       string
}

// The one sentence that stops an unbuyable proposal. Percentages of a pot and a per-trade
// ceiling are fences; this is the fact.
func CapLine(available float64, currency string) string {
	return fmt.Sprintf("You may spend at most %.2f %s in this purse.\nA buy costing more than that is refused, not shrunk.", available, currency)
}

func Template(name string) (string, error) {
	if name == "plan_v1" {
		name = "plan_stock_v1"
	}
	t, ok := templates[name]
	if !ok {
		return "", fmt.Errorf("unknown prompt template %q", name)
	}
	return t, nil
}

// SHA-256 of the template file — not of the rendered text — so a signal can be traced to the
// prompt version that produced it.
func TemplateHash(name string) (string, error) {
	t, err := Template(name)
	if err != nil {
		return "", err
	}
	sum := sha256.Sum256([]byte(t))
	return hex(sum[:]), nil
}

func hex(b []byte) string {
	const digits = "0123456789abcdef"
	out := make([]byte, 0, len(b)*2)
	for _, v := range b {
		out = append(out, digits[v>>4], digits[v&0xf])
	}
	return string(out)
}

// The book call: the whole book, the cash, today's candidates, and room for up to three
// moves.
func RenderPlan(scope Scope, book, cap, limits, candidates, fundamentals string, cash float64, currency string) (RenderedPrompt, error) {
	name := scope.Template()
	t, err := Template(name)
	if err != nil {
		return RenderedPrompt{}, err
	}
	text := strings.NewReplacer(
		"{{book}}", book,
		"{{cap}}", cap,
		"{{cash}}", fmt.Sprintf("%.2f", cash),
		"{{currency}}", currency,
		"{{limits}}", limits,
		"{{candidates}}", candidates,
		"{{fundamentals}}", fundamentals,
	).Replace(t)
	hash, err := TemplateHash(name)
	if err != nil {
		return RenderedPrompt{}, err
	}
	return RenderedPrompt{Text: text, Hash: hash, Template: name}, nil
}

// The rotation prompt: a holdings table and one candidate, no market context.
func RenderRotation(holdings, candidate string) (RenderedPrompt, error) {
	t, err := Template("rotate_v1")
	if err != nil {
		return RenderedPrompt{}, err
	}
	hash, _ := TemplateHash("rotate_v1")
	return RenderedPrompt{
		Text:     strings.NewReplacer("{{holdings}}", holdings, "{{candidate}}", candidate).Replace(t),
		Hash:     hash,
		Template: "rotate_v1",
	}, nil
}

// Numbered web hits for the prompt.
//
// Only a dated, titled result is evidence: an undated page is a quote page or a filings
// index, and the model may not treat it as today's news. Undated hits are dropped rather
// than shown with a blank date, because a blank date reads as "current".
func RenderWebHits(hits []SearchHit) string {
	const limit = 5
	kept := []SearchHit{}
	for _, h := range hits {
		if strings.TrimSpace(h.Title) != "" {
			kept = append(kept, h)
			if len(kept) == limit {
				break
			}
		}
	}
	if len(kept) == 0 {
		return "nothing dated in the last three days — judge on the tape and the stored headlines alone, and treat the move as having no known catalyst"
	}
	lines := make([]string, 0, len(kept))
	for i, h := range kept {
		day := "undated"
		if h.Date != nil && *h.Date != "" {
			d := *h.Date
			if len(d) > 10 {
				d = d[:10]
			}
			day = d
		}
		when := ""
		for _, part := range []string{day, h.Site} {
			if part != "" {
				when += " · " + part
			}
		}
		gist := strings.TrimSpace(h.Snippet)
		if gist == "" {
			gist = strings.TrimSpace(h.Summary)
		}
		lines = append(lines, fmt.Sprintf("[%d] %s%s\n    %s", i+1, strings.TrimSpace(h.Title), when, gist))
	}
	return strings.Join(lines, "\n")
}

// Every prompt goes through here: the template, then one substitution per field the prompt
// names.
func Render(name string, ctx *PromptContext) (RenderedPrompt, error) {
	var headlineBlock string
	if len(ctx.Headlines) == 0 {
		headlineBlock = "(no headlines in the last 7 days)"
	} else {
		lines := make([]string, len(ctx.Headlines))
		for i, h := range ctx.Headlines {
			publisher := "unknown"
			if h.Publisher != "" {
				publisher = h.Publisher
			}
			day := h.PublishedAt
			if len(day) > 10 {
				day = day[:10]
			}
			lines[i] = fmt.Sprintf("[%d] %s — %s, %s", h.ID, h.Title, publisher, day)
		}
		headlineBlock = strings.Join(lines, "\n")
	}

	extractBlock := "(none requested)"
	if len(ctx.Extracts) > 0 {
		parts := make([]string, len(ctx.Extracts))
		for i, e := range ctx.Extracts {
			parts[i] = fmt.Sprintf("### headline [%d]\n%s", e.HeadlineID, e.Text)
		}
		extractBlock = strings.Join(parts, "\n\n")
	}

	historyBlock := "(not requested)"
	if strings.TrimSpace(ctx.PriceHistory) != "" {
		historyBlock = ctx.PriceHistory
	}

	targetProfit := "no minimum set — choose what the setup justifies"
	if ctx.TargetProfitPct != nil {
		p := *ctx.TargetProfitPct
		targetProfit = fmt.Sprintf("at least %.1f%% (a take-profit below %.2f is rejected)",
			p, ctx.LastClose*(1.0+p/100.0))
	}

	t, err := Template(name)
	if err != nil {
		return RenderedPrompt{}, err
	}
	text := strings.NewReplacer(
		"{{symbol}}", ctx.Instrument.Symbol,
		"{{class}}", string(ctx.Instrument.Class),
		"{{currency}}", ctx.Instrument.Currency,
		"{{last_close}}", fmt.Sprintf("%.2f", ctx.LastClose),
		"{{now}}", ctx.Now.UTC().Format("2006-01-02T15:04:05Z"),
		"{{indicators}}", fmt.Sprintf(
			"sma50=%.2f  sma200=%.2f  rsi14=%.1f  atr14=%.2f  52w low=%.2f  high=%.2f",
			ctx.Ind.SMA50, ctx.Ind.SMA200, ctx.Ind.RSI14, ctx.Ind.ATR14, ctx.Ind.Low52, ctx.Ind.High52),
		"{{window_hours}}", fmt.Sprint(ctx.WindowHours),
		"{{price_window}}", ctx.PriceWindow,
		"{{daily_days}}", fmt.Sprint(ctx.DailyDays),
		"{{daily_summary}}", ctx.DailySummary,
		"{{fundamentals}}", ctx.Fundamentals,
		"{{short_context}}", ctx.ShortContext,
		"{{positions}}", ctx.Positions,
		"{{recent_trades}}", ctx.RecentTrades,
		"{{last_decision}}", ctx.LastDecision,
		"{{cash}}", fmt.Sprintf("%.2f", ctx.CashAvailable),
		"{{cap}}", CapLine(ctx.CashAvailable, ctx.Instrument.Currency),
		"{{limits}}", ctx.Limits,
		"{{target_profit}}", targetProfit,
		"{{class_rules}}", ctx.Instrument.Class.TimingNote(),
		"{{liquidity_rules}}", ctx.Instrument.Class.LiquidityNote(),
		"{{headlines}}", headlineBlock,
		"{{extracts}}", extractBlock,
		"{{price_history}}", historyBlock,
	).Replace(t)

	hash, err := TemplateHash(name)
	if err != nil {
		return RenderedPrompt{}, err
	}
	return RenderedPrompt{Text: text, Hash: hash, Template: name}, nil
}
