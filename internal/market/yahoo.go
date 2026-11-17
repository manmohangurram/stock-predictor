package market

import (
	"encoding/json"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"sync/atomic"
	"time"

	"stock-predictor/internal/httpx"
	"stock-predictor/internal/models"
	"stock-predictor/internal/rules"
	"stock-predictor/internal/tstamp"
)

const (
	chartURL  = "https://query2.finance.yahoo.com/v8/finance/chart"
	searchURL = "https://query2.finance.yahoo.com/v1/finance/search"
	yahooUA   = "Mozilla/5.0 (X11; Linux x86_64) stock-predictor/0.1"
)

// All Yahoo access goes through this module. Endpoint drift touches one file; the client itself
// comes from httpx so the proxy is decided in one place.
func YahooClient(rt *rules.Runtime) (*http.Client, error) {
	return httpx.ClientWith(rt, 20, yahooUA)
}

func rfc3339(secs int64) string {
	return tstamp.From(time.Unix(secs, 0).UTC())
}

func chartNum(raw json.RawMessage) (float64, bool) {
	if len(raw) == 0 || string(raw) == "null" {
		return 0, false
	}
	var f float64
	if err := json.Unmarshal(raw, &f); err == nil {
		return f, true
	}
	return 0, false
}

func ParseChart(body string) ([]models.Bar, error) {
	var v map[string]any
	if err := json.Unmarshal([]byte(body), &v); err != nil {
		return nil, fmt.Errorf("chart: invalid json: %w", err)
	}
	chart, _ := v["chart"].(map[string]any)
	if chart == nil || chart["result"] == nil {
		return nil, fmt.Errorf("chart: missing chart.result")
	}
	result, _ := chart["result"].([]any)
	if len(result) == 0 {
		return []models.Bar{}, nil
	}
	first, _ := result[0].(map[string]any)
	timestamps, ok := first["timestamp"].([]any)
	if !ok {
		return nil, fmt.Errorf("chart: missing timestamp")
	}
	indicators, _ := first["indicators"].(map[string]any)
	quotes, _ := indicators["quote"].([]any)
	if len(quotes) == 0 {
		return nil, fmt.Errorf("chart: missing indicators.quote")
	}
	quote, _ := quotes[0].(map[string]any)
	arr := func(name string) []any {
		a, _ := quote[name].([]any)
		return a
	}
	open, high, low, closeArr, vol := arr("open"), arr("high"), arr("low"), arr("close"), arr("volume")
	at := func(list []any, i int) (float64, bool) {
		if i >= len(list) {
			return 0, false
		}
		b, err := json.Marshal(list[i])
		if err != nil {
			return 0, false
		}
		return chartNum(b)
	}

	bars := make([]models.Bar, 0, len(timestamps))
	for i, tAny := range timestamps {
		closeValue, ok := at(closeArr, i)
		if !ok {
			continue // halted / missing candle: skip, never fabricate
		}
		secs, _ := tAny.(float64)
		openValue, ok := at(open, i)
		if !ok {
			openValue = closeValue
		}
		highValue, ok := at(high, i)
		if !ok {
			highValue = closeValue
		}
		lowValue, ok := at(low, i)
		if !ok {
			lowValue = closeValue
		}
		volume, _ := at(vol, i)
		bars = append(bars, models.Bar{
			Ts: rfc3339(int64(secs)), Open: openValue, High: highValue, Low: lowValue,
			Close: closeValue, Volume: int64(volume),
		})
	}
	return bars, nil
}

// Headlines only. The chosen endpoint carries title/publisher/date/link and no body at all,
// which is exactly what the token budget wants.
func ParseYahooNews(body string) ([]models.Headline, error) {
	var v map[string]any
	if err := json.Unmarshal([]byte(body), &v); err != nil {
		return nil, fmt.Errorf("news: invalid json: %w", err)
	}
	arr, ok := v["news"].([]any)
	if !ok {
		return nil, fmt.Errorf("news: missing news[] — endpoint shape changed")
	}
	seen := map[string]bool{}
	out := []models.Headline{}
	for _, itemAny := range arr {
		item, _ := itemAny.(map[string]any)
		link, _ := item["link"].(string)
		title, _ := item["title"].(string)
		if link == "" || title == "" || seen[link] {
			continue
		}
		seen[link] = true
		publisher, _ := item["publisher"].(string)
		published, _ := item["providerPublishTime"].(float64)
		out = append(out, models.Headline{
			Title: title, Publisher: publisher, PublishedAt: rfc3339(int64(published)), URL: link,
		})
	}
	return out, nil
}

func ChartURL(symbol string) string { return chartURL + "/" + symbol }

// A missing chart is normal for mutual funds, which Yahoo does not carry for India.
func FetchBars(client *http.Client, symbol string) ([]models.Bar, error) {
	return fetchChart(client, symbol, "1d", "1y")
}

// Intraday tape. Yahoo's intervals that actually exist: 1m, 5m, 15m, 30m, 60m, 90m, 1h.
func FetchIntraday(client *http.Client, symbol, interval, rangeStr string) ([]models.Bar, error) {
	return fetchChart(client, symbol, interval, rangeStr)
}

var yahooBlockedUntil atomic.Int64

// noteThrottle remembers a 429 so the next requests fail fast instead of extending the block.
// Yahoo's limit is per client and recovers only if the client stops asking.
func noteThrottle(resp *http.Response) {
	if yahooBlocked() {
		return // the block is already in force; one line is enough
	}
	wait := 5 * time.Minute
	if raw := resp.Header.Get("Retry-After"); raw != "" {
		if secs, err := strconv.Atoi(strings.TrimSpace(raw)); err == nil && secs > 0 && secs <= 900 {
			wait = time.Duration(secs) * time.Second
		}
	}
	yahooBlockedUntil.Store(time.Now().Add(wait).UnixNano())
	slog.Warn("yahoo throttled the client; backing off", "for", wait.Round(time.Second))
}

func yahooBlocked() bool {
	until := yahooBlockedUntil.Load()
	return until > 0 && time.Now().UnixNano() < until
}

// doRetry runs a GET, waiting out 429/5xx instead of treating a throttle as "no data". Yahoo
// rate-limits bursts (a 100-symbol sweep is exactly that), and a skipped chart silently becomes
// "this name did not rise".
func doRetry(client *http.Client, req *http.Request, attempts int) (*http.Response, error) {
	if yahooBlocked() {
		return nil, fmt.Errorf("yahoo is throttling this client; backing off")
	}
	for i := 0; ; i++ {
		resp, err := client.Do(req)
		if err != nil {
			if i >= attempts-1 {
				return nil, err
			}
			time.Sleep(time.Duration(300*(i+1)) * time.Millisecond)
			continue
		}
		if resp.StatusCode != http.StatusTooManyRequests && resp.StatusCode < 500 {
			return resp, nil
		}
		if resp.StatusCode == http.StatusTooManyRequests {
			noteThrottle(resp)
		}
		if i >= attempts-1 {
			return resp, nil
		}
		wait := time.Duration(500*(i+1)) * time.Millisecond
		if raw := resp.Header.Get("Retry-After"); raw != "" {
			if secs, err := strconv.Atoi(strings.TrimSpace(raw)); err == nil && secs > 0 && secs <= 10 {
				wait = time.Duration(secs) * time.Second
			}
		}
		resp.Body.Close()
		time.Sleep(wait)
	}
}

func getYahoo(client *http.Client, url string) (*http.Response, error) {
	req, err := http.NewRequest(http.MethodGet, url, nil)
	if err != nil {
		return nil, err
	}
	return doRetry(client, req, 3)
}

func fetchChart(client *http.Client, symbol, interval, rangeStr string) ([]models.Bar, error) {
	q := url.Values{}
	q.Set("range", rangeStr)
	q.Set("interval", interval)
	resp, err := getYahoo(client, ChartURL(symbol)+"?"+q.Encode())
	if err != nil {
		return nil, fmt.Errorf("chart: request: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode > 299 {
		return nil, fmt.Errorf("chart: http status %d", resp.StatusCode)
	}
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, fmt.Errorf("chart: read body: %w", err)
	}
	bars, err := ParseChart(string(body))
	if err != nil {
		return nil, fmt.Errorf("chart: parse %s: %w", symbol, err)
	}
	slog.Debug("chart fetched", "symbol", symbol, "interval", interval, "range", rangeStr,
		"bars", len(bars))
	return bars, nil
}

// Yahoo's search endpoint returns nothing for an exchange-qualified ticker (INFY.NS → 0 results),
// but finds the company by bare symbol (INFY → 5+).
func NewsQuery(symbol string) string {
	if idx := strings.LastIndex(symbol, "."); idx >= 0 {
		suffix := symbol[idx+1:]
		if suffix == "NS" || suffix == "BO" {
			return symbol[:idx]
		}
	}
	return symbol
}

func FetchYahooNews(client *http.Client, symbol string) ([]models.Headline, error) {
	q := url.Values{}
	q.Set("q", NewsQuery(symbol))
	q.Set("newsCount", "20")
	q.Set("quotesCount", "0")
	resp, err := getYahoo(client, searchURL+"?"+q.Encode())
	if err != nil {
		return nil, fmt.Errorf("news: request: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode > 299 {
		return nil, fmt.Errorf("news: http status %d", resp.StatusCode)
	}
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, fmt.Errorf("news: read body: %w", err)
	}
	news, err := ParseYahooNews(string(body))
	if err != nil {
		return nil, fmt.Errorf("news: parse %s: %w", symbol, err)
	}
	return news, nil
}

// FetchClosesBatch returns daily closes for up to a few dozen symbols in one request, using
// Yahoo's spark endpoint. The per-symbol chart endpoint throttles a 100-name sweep; this does
// the same job in five requests.
func FetchClosesBatch(client *http.Client, symbols []string) (map[string][]float64, error) {
	out := map[string][]float64{}
	const chunk = 20
	for start := 0; start < len(symbols); start += chunk {
		end := start + chunk
		if end > len(symbols) {
			end = len(symbols)
		}
		q := url.Values{}
		q.Set("symbols", strings.Join(symbols[start:end], ","))
		q.Set("range", "1mo")
		q.Set("interval", "1d")
		resp, err := getYahoo(client, "https://query1.finance.yahoo.com/v8/finance/spark?"+q.Encode())
		if err != nil {
			return out, fmt.Errorf("spark: request: %w", err)
		}
		body, err := io.ReadAll(resp.Body)
		resp.Body.Close()
		if err != nil {
			return out, fmt.Errorf("spark: read body: %w", err)
		}
		if resp.StatusCode < 200 || resp.StatusCode > 299 {
			return out, fmt.Errorf("spark: http status %d", resp.StatusCode)
		}
		var payload map[string]struct {
			Close []float64 `json:"close"`
		}
		if err := json.Unmarshal(body, &payload); err != nil {
			return out, fmt.Errorf("spark: payload: %w", err)
		}
		for symbol, row := range payload {
			closes := make([]float64, 0, len(row.Close))
			for _, c := range row.Close {
				if c > 0 {
					closes = append(closes, c)
				}
			}
			if len(closes) > 0 {
				out[symbol] = closes
			}
		}
		slog.Debug("spark chunk", "requested", end-start, "returned", len(payload))
		if end < len(symbols) {
			time.Sleep(100 * time.Millisecond)
		}
	}
	return out, nil
}

// Closes is the batch view of the same daily series, for callers that need many symbols at once.
func (y *YahooSource) Closes(symbols []string) (map[string][]float64, error) {
	return FetchClosesBatch(y.client, symbols)
}

// YahooSource is the real MarketSource: the trait the engine and tests share.
type YahooSource struct {
	client *http.Client
}

func NewYahooSource(rt *rules.Runtime) (*YahooSource, error) {
	client, err := YahooClient(rt)
	if err != nil {
		return nil, err
	}
	return &YahooSource{client: client}, nil
}

func (y *YahooSource) Bars(symbol string) ([]models.Bar, error) {
	return FetchBars(y.client, symbol)
}

func (y *YahooSource) Intraday(symbol, interval, rangeStr string) ([]models.Bar, error) {
	return FetchIntraday(y.client, symbol, interval, rangeStr)
}

func (y *YahooSource) News(symbol string) ([]models.Headline, error) {
	return FetchYahooNews(y.client, symbol)
}

func (y *YahooSource) Constituents() ([]Constituent, error) {
	req, err := http.NewRequest(http.MethodGet,
		"https://nsearchives.nseindia.com/content/indices/ind_nifty100list.csv", nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("User-Agent", "Mozilla/5.0 stock-predictor")
	resp, err := y.client.Do(req)
	if err != nil {
		return nil, fmt.Errorf("nifty 100 list: %w", err)
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, fmt.Errorf("nifty 100 list body: %w", err)
	}
	return ParseConstituentsCSV(string(body)), nil
}

// ParseConstituentsCSV lives here to avoid a market→benchmark import; benchmark parses with the
// same columns.
func ParseConstituentsCSV(csv string) []Constituent {
	out := []Constituent{}
	lines := strings.Split(csv, "\n")
	for i, line := range lines {
		if i == 0 {
			continue
		}
		cells := strings.Split(line, ",")
		if len(cells) < 3 {
			continue
		}
		name := strings.TrimSpace(cells[0])
		symbol := strings.TrimSpace(cells[2])
		if symbol == "" || strings.EqualFold(symbol, "symbol") {
			continue
		}
		out = append(out, Constituent{Symbol: symbol + ".NS", Name: name})
	}
	return out
}

var _ = strconv.Itoa
