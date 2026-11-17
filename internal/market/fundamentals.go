package market

import (
	"encoding/json"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"net/url"
	"sort"
	"strconv"
	"sync"
	"time"
)

// Revenue, profit and margins, straight from Yahoo's fundamentals time series. The endpoint
// needs no crumb (unlike quoteSummary, which rate-limits us), and it answers for Indian
// listings.

const fundamentalsURL = "https://query2.finance.yahoo.com/ws/fundamentals-timeseries/v1/finance/timeseries"

type Year struct {
	AsOf            string
	Revenue         *float64
	NetIncome       *float64
	OperatingIncome *float64
	GrossProfit     *float64
	Currency        string
	// The same endpoint serves valuation: PE and market cap, crumb-free.
	PE        *float64
	MarketCap *float64
}

type Fundamentals struct {
	// Oldest first.
	Years []Year
	// The latest quarter, when the endpoint has one.
	Quarter *struct {
		AsOf    string
		Revenue float64
	}
	// Trailing PE and market cap, as of the latest close.
	Valuation *struct {
		PE        float64
		MarketCap float64
	}
}

func (f Fundamentals) Latest() *Year {
	if len(f.Years) == 0 {
		return nil
	}
	return &f.Years[len(f.Years)-1]
}

// Year-over-year revenue growth, in percent, from the last two annual figures.
func (f Fundamentals) RevenueGrowthPct() (float64, bool) {
	n := len(f.Years)
	if n < 2 || f.Years[n-2].Revenue == nil || f.Years[n-1].Revenue == nil {
		return 0, false
	}
	a, b := *f.Years[n-2].Revenue, *f.Years[n-1].Revenue
	if a <= 0 {
		return 0, false
	}
	return (b - a) / a * 100, true
}

// Year-over-year net income growth, in percent.
func (f Fundamentals) ProfitGrowthPct() (float64, bool) {
	n := len(f.Years)
	if n < 2 || f.Years[n-2].NetIncome == nil || f.Years[n-1].NetIncome == nil {
		return 0, false
	}
	a, b := *f.Years[n-2].NetIncome, *f.Years[n-1].NetIncome
	if a == 0 {
		return 0, false
	}
	return (b - a) / a * 100, true
}

// Net margin in percent — the shape of the business, not its size.
func (f Fundamentals) NetMarginPct() (float64, bool) {
	y := f.Latest()
	if y == nil || y.Revenue == nil || y.NetIncome == nil || *y.Revenue <= 0 {
		return 0, false
	}
	return *y.NetIncome / *y.Revenue * 100, true
}

// A few lines a broker would say out loud. Empty when there is nothing honest to say.
func (f Fundamentals) Summary() string {
	y := f.Latest()
	if y == nil {
		return ""
	}
	money := func(v *float64) string {
		if v == nil {
			return "—"
		}
		n := *v
		if abs(n) >= 1e9 {
			return fmt.Sprintf("%.1fB %s", n/1e9, y.Currency)
		}
		if abs(n) >= 1e6 {
			return fmt.Sprintf("%.0fM %s", n/1e6, y.Currency)
		}
		return fmt.Sprintf("%.0f %s", n, y.Currency)
	}
	pct := func(v float64, ok bool) string {
		if !ok {
			return "—"
		}
		return fmt.Sprintf("%+.1f%%", v)
	}
	margin, marginOK := f.NetMarginPct()
	revGrowth, revOK := f.RevenueGrowthPct()
	profitGrowth, profitOK := f.ProfitGrowthPct()
	out := fmt.Sprintf("FY%s: revenue %s, net income %s, net margin %s, operating income %s\n",
		y.AsOf, money(y.Revenue), money(y.NetIncome), pct(margin, marginOK), money(y.OperatingIncome))
	out += fmt.Sprintf("Growth: revenue %s YoY, profit %s YoY\n",
		pct(revGrowth, revOK), pct(profitGrowth, profitOK))
	if f.Valuation != nil {
		capText := ""
		if f.Valuation.MarketCap > 0 {
			capText = fmt.Sprintf("%.0fB ", f.Valuation.MarketCap/1e9)
		}
		out += fmt.Sprintf("Valuation: trailing PE %.1f, market cap %s%s\n",
			f.Valuation.PE, capText, y.Currency)
	}
	if f.Quarter != nil {
		rev := f.Quarter.Revenue
		amount := fmt.Sprintf("%.0f %s", rev, y.Currency)
		if abs(rev) >= 1e9 {
			amount = fmt.Sprintf("%.1fB %s", rev/1e9, y.Currency)
		}
		out += fmt.Sprintf("Latest quarter (%s): revenue %s\n", f.Quarter.AsOf, amount)
	}
	return out
}

func FetchFundamentals(client *http.Client, symbol string, nowYear int) (Fundamentals, error) {
	yearStart := func(y int) int64 {
		return time.Date(y, 1, 1, 0, 0, 0, 0, time.UTC).Unix()
	}
	q := url.Values{}
	q.Set("symbol", symbol)
	q.Set("type", "annualTotalRevenue,annualNetIncome,annualOperatingIncome,annualGrossProfit,trailingPeRatio,trailingMarketCap")
	q.Set("period1", strconv.FormatInt(yearStart(nowYear-5), 10))
	q.Set("period2", strconv.FormatInt(yearStart(nowYear+1), 10))
	req, err := http.NewRequest(http.MethodGet, fundamentalsURL+"/"+symbol+"?"+q.Encode(), nil)
	if err != nil {
		return Fundamentals{}, err
	}
	req.Header.Set("User-Agent", "Mozilla/5.0 stock-predictor")
	resp, err := client.Do(req)
	if err != nil {
		return Fundamentals{}, fmt.Errorf("fundamentals: request: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode > 299 {
		return Fundamentals{}, fmt.Errorf("fundamentals: status %d", resp.StatusCode)
	}
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return Fundamentals{}, fmt.Errorf("fundamentals: body: %w", err)
	}
	f, err := ParseFundamentals(string(body))
	if err == nil {
		slog.Debug("fundamentals", "symbol", symbol, "years", len(f.Years),
			"has_valuation", f.Valuation != nil)
	}
	return f, err
}

func ParseFundamentals(body string) (Fundamentals, error) {
	var v map[string]any
	if err := json.Unmarshal([]byte(body), &v); err != nil {
		return Fundamentals{}, fmt.Errorf("fundamentals: payload shape: %w", err)
	}
	ts, _ := v["timeseries"].(map[string]any)
	results, _ := ts["result"].([]any)
	if len(results) == 0 {
		return Fundamentals{}, fmt.Errorf("fundamentals: no result")
	}

	series := func(key string) [][3]any {
		var row map[string]any
		for _, r := range results {
			if m, ok := r.(map[string]any); ok && m[key] != nil {
				row = m
				break
			}
		}
		if row == nil {
			row, _ = results[0].(map[string]any)
		}
		arr, _ := row[key].([]any)
		out := [][3]any{}
		for _, p := range arr {
			pm, ok := p.(map[string]any)
			if !ok {
				continue
			}
			asOf, _ := pm["asOfDate"].(string)
			rv, _ := pm["reportedValue"].(map[string]any)
			raw, ok := rv["raw"].(float64)
			if asOf == "" || !ok {
				continue
			}
			currency, _ := pm["currencyCode"].(string)
			out = append(out, [3]any{asOf, raw, currency})
		}
		return out
	}

	revenue := series("annualTotalRevenue")
	lookup := func(s [][3]any, date string) *float64 {
		for _, row := range s {
			if row[0].(string) == date {
				v := row[1].(float64)
				return &v
			}
		}
		return nil
	}
	years := []Year{}
	for _, row := range revenue {
		asOf := row[0].(string)
		rev := row[1].(float64)
		currency, _ := row[2].(string)
		if currency == "" {
			currency = "INR"
		}
		years = append(years, Year{
			AsOf: asOf, Revenue: &rev,
			NetIncome:       lookup(series("annualNetIncome"), asOf),
			OperatingIncome: lookup(series("annualOperatingIncome"), asOf),
			GrossProfit:     lookup(series("annualGrossProfit"), asOf),
			Currency:        currency,
		})
	}
	sort.SliceStable(years, func(i, j int) bool { return years[i].AsOf < years[j].AsOf })
	deduped := []Year{}
	for _, y := range years {
		if len(deduped) > 0 && deduped[len(deduped)-1].AsOf == y.AsOf {
			continue
		}
		deduped = append(deduped, y)
	}
	years = deduped

	lastOf := func(key string) (float64, bool) {
		s := series(key)
		if len(s) == 0 {
			return 0, false
		}
		return s[len(s)-1][1].(float64), true
	}
	var valuation *struct {
		PE        float64
		MarketCap float64
	}
	if pe, ok := lastOf("trailingPeRatio"); ok {
		capValue, _ := lastOf("trailingMarketCap")
		valuation = &struct {
			PE        float64
			MarketCap float64
		}{PE: pe, MarketCap: capValue}
		if len(years) > 0 {
			peCopy := pe
			years[len(years)-1].PE = &peCopy
			if capValue != 0 {
				capCopy := capValue
				years[len(years)-1].MarketCap = &capCopy
			}
		}
	}
	return Fundamentals{Years: years, Valuation: valuation}, nil
}

// fundamentalsCache keeps a symbol's financials for half a day: they change once a quarter, not
// once a poll.
var (
	fundMu    sync.Mutex
	fundCache = map[string]fundamentalsEntry{}
)

type fundamentalsEntry struct {
	at time.Time
	f  Fundamentals
}

// FetchFundamentalsCached is FetchFundamentals with a 12-hour memory.
func FetchFundamentalsCached(client *http.Client, symbol string, nowYear int) (Fundamentals, error) {
	fundMu.Lock()
	entry, ok := fundCache[symbol]
	fundMu.Unlock()
	if ok && time.Since(entry.at) < 12*time.Hour {
		return entry.f, nil
	}
	f, err := FetchFundamentals(client, symbol, nowYear)
	if err != nil {
		return Fundamentals{}, err
	}
	fundMu.Lock()
	fundCache[symbol] = fundamentalsEntry{at: time.Now(), f: f}
	fundMu.Unlock()
	return f, nil
}

func abs(v float64) float64 {
	if v < 0 {
		return -v
	}
	return v
}
