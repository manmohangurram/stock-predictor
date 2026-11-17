package market

import (
	"encoding/json"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"sort"
	"strconv"
	"strings"
)

// NSE's own list of what moved today.
//
// One endpoint, no cookies: live-analysis-variations?index=gainers returns the day's variation
// lists grouped by index. allSec is every security, already ranked by percentage change — which
// is exactly the "what is hot in India right now" question, answered by the exchange.

const gainersURL = "https://www.nseindia.com/api/live-analysis-variations?index=gainers"
const nseUserAgent = "Mozilla/5.0 (X11; Linux x86_64) stock-predictor/0.4"

// One row of the variation list, trimmed to what the app uses.
type Gainer struct {
	Symbol        string
	LTP           float64
	PreviousClose float64
	PerChange     float64
	Turnover      *float64
}

// The symbol as Yahoo knows it. NSE quotes the bare ticker; Yahoo wants the exchange suffix.
func (g Gainer) YahooSymbol() string {
	return g.Symbol + ".NS"
}

// Both lists from one request: the whole market's top gainers (allSec), and the Nifty 50's own
// (NIFTY, which the exchange already ranks).
func FetchToday(client *http.Client, top int) ([]Gainer, []Gainer, error) {
	req, err := http.NewRequest(http.MethodGet, gainersURL, nil)
	if err != nil {
		return nil, nil, err
	}
	req.Header.Set("User-Agent", nseUserAgent)
	req.Header.Set("Accept", "application/json")
	resp, err := client.Do(req)
	if err != nil {
		return nil, nil, fmt.Errorf("nse: request: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode > 299 {
		return nil, nil, fmt.Errorf("nse: status %d", resp.StatusCode)
	}
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, nil, fmt.Errorf("nse: body: %w", err)
	}
	gainers, err := ParseGainers(string(body), top)
	if err != nil {
		return nil, nil, err
	}
	movers, err := ParseBenchmarkMovers(string(body), top)
	if err != nil {
		return nil, nil, err
	}
	slog.Debug("nse movers", "gainers", len(gainers), "nifty", len(movers))
	return gainers, movers, nil
}

type nseEnvelope struct {
	AllSec *nseGroup `json:"allSec"`
	Nifty  *nseGroup `json:"NIFTY"`
}

type nseGroup struct {
	Data []nseRow `json:"data"`
}

type nseRow struct {
	Symbol    string          `json:"symbol"`
	LTP       json.RawMessage `json:"ltp"`
	PrevPrice json.RawMessage `json:"prev_price"`
	PerChange json.RawMessage `json:"perChange"`
	Turnover  json.RawMessage `json:"turnover"`
}

// Pure, so the shape can be tested against a fixture.
func ParseGainers(body string, top int) ([]Gainer, error) {
	var env nseEnvelope
	if err := json.Unmarshal([]byte(body), &env); err != nil {
		return nil, fmt.Errorf("nse: payload shape: %w", err)
	}
	if env.AllSec == nil {
		return nil, fmt.Errorf("nse: no allSec group in the payload")
	}
	return rankGainers(env.AllSec.Data, top), nil
}

// The Nifty 50's own top movers: the same payload's NIFTY group, already ranked by the exchange.
// equity-stockIndices — the obvious endpoint — refuses non-browser clients, so this group is the
// honest way in.
func ParseBenchmarkMovers(body string, top int) ([]Gainer, error) {
	var env nseEnvelope
	if err := json.Unmarshal([]byte(body), &env); err != nil {
		return nil, fmt.Errorf("nse: payload shape: %w", err)
	}
	if env.Nifty == nil {
		return nil, fmt.Errorf("nse: no NIFTY group in the payload")
	}
	return rankGainers(env.Nifty.Data, top), nil
}

// One mapper for both groups: a row is a row, whichever list it came in.
func rankGainers(rows []nseRow, top int) []Gainer {
	out := []Gainer{}
	for _, r := range rows {
		ltp, ok := numberOrText(r.LTP)
		if !ok || ltp <= 0 {
			continue
		}
		prev, _ := numberOrText(r.PrevPrice)
		if prev < 0 {
			prev = 0
		}
		pct, ok := numberOrText(r.PerChange)
		if !ok {
			continue
		}
		turnover, hasTurnover := numberOrText(r.Turnover)
		var turnoverPtr *float64
		if hasTurnover {
			turnoverPtr = &turnover
		}
		symbol := strings.TrimSpace(r.Symbol)
		if symbol == "" {
			continue
		}
		out = append(out, Gainer{
			Symbol: symbol, LTP: ltp, PreviousClose: prev, PerChange: pct, Turnover: turnoverPtr,
		})
	}
	sort.SliceStable(out, func(i, j int) bool { return out[i].PerChange > out[j].PerChange })
	if len(out) > top {
		out = out[:top]
	}
	return out
}

// NSE is not consistent about types: the same field is 22.86 on one row and "22.86 Cr" on the
// next, so one formatted value used to reject the entire payload with "invalid type: string".
// Read a number, a number in text, or a text with an Indian magnitude suffix.
func numberOrText(raw json.RawMessage) (float64, bool) {
	if len(raw) == 0 {
		return 0, false
	}
	var f float64
	if err := json.Unmarshal(raw, &f); err == nil {
		return f, true
	}
	var s string
	if err := json.Unmarshal(raw, &s); err == nil {
		v, ok := FromText(s)
		return v, ok
	}
	return 0, false
}

// "22.86 Cr" → 22.86 crore; "1.5 L" → 1.5 lakh; "14.18%" → 14.18; "-" → false.
func FromText(raw string) (float64, bool) {
	text := strings.ReplaceAll(strings.TrimSpace(raw), ",", "")
	if text == "" || text == "-" || strings.EqualFold(text, "null") {
		return 0, false
	}
	text = strings.TrimSpace(strings.TrimSuffix(text, "%"))
	lower := strings.ToLower(text)
	body := lower
	scale := 1.0
	switch {
	case strings.HasSuffix(lower, "cr"):
		body = strings.TrimSpace(strings.TrimSuffix(lower, "cr"))
		scale = 1e7
	case strings.HasSuffix(lower, "lakh"):
		body = strings.TrimSpace(strings.TrimSuffix(lower, "lakh"))
		scale = 1e5
	case strings.HasSuffix(lower, "lac"):
		body = strings.TrimSpace(strings.TrimSuffix(lower, "lac"))
		scale = 1e5
	case strings.HasSuffix(lower, "l"):
		body = strings.TrimSpace(strings.TrimSuffix(lower, "l"))
		scale = 1e5
	case strings.HasSuffix(lower, "k"):
		body = strings.TrimSpace(strings.TrimSuffix(lower, "k"))
		scale = 1e3
	}
	n, err := strconv.ParseFloat(strings.TrimSpace(body), 64)
	if err != nil {
		return 0, false
	}
	return n * scale, true
}
