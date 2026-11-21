package market

import (
	"encoding/json"
	"fmt"
	"io"
	"log/slog"
	"math/rand/v2"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"stock-predictor/internal/httpx"
	"stock-predictor/internal/models"
	"stock-predictor/internal/rules"
	"stock-predictor/internal/schedule"
	"stock-predictor/internal/tstamp"
)

const (
	chartPath  = "/v8/finance/chart"
	searchPath = "/v1/finance/search"
	sparkPath  = "/v8/finance/spark"
	yahooUA    = "Mozilla/5.0 (X11; Linux x86_64) stock-predictor/0.1"
)

// Both edges serve the same data; one host taking every poll is what earns the 429.
var yahooHosts = [...]string{"https://query1.finance.yahoo.com", "https://query2.finance.yahoo.com"}

func yahooHost() string { return yahooHosts[rand.IntN(len(yahooHosts))] }

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

// A missing chart is normal for mutual funds, which Yahoo does not carry for India.
func FetchBars(client *http.Client, symbol string) ([]models.Bar, error) {
	return fetchChart(client, symbol, "1d", "1y")
}

// Intraday tape. Yahoo's intervals that actually exist: 1m, 5m, 15m, 30m, 60m, 90m, 1h.
func FetchIntraday(client *http.Client, symbol, interval, rangeStr string) ([]models.Bar, error) {
	return fetchChart(client, symbol, interval, rangeStr)
}

// Yahoo's limit is burst-shaped, not volume-shaped: a hundred charts in one tick is a 429, the same
// hundred spread over a day is not. One request every 200 ms is below anything it has refused.
var yahooGap = 200 * time.Millisecond

var yahooNext atomic.Int64

// pace holds the next request until the gap has passed, and claims the slot after it.
func pace() {
	for {
		now := time.Now().UnixNano()
		next := yahooNext.Load()
		if next > now {
			time.Sleep(time.Duration(next - now))
			continue
		}
		if yahooNext.CompareAndSwap(next, now+int64(yahooGap)) {
			return
		}
	}
}

// A 429 costs thirty seconds, then a minute, then two. The flat five-minute block this replaces
// turned one throttled sweep into an afternoon of blank screens.
var (
	throttleFirst = 30 * time.Second
	throttleMax   = 10 * time.Minute
	throttleCalm  = 10 * time.Minute
)

var (
	throttleMu      sync.Mutex
	throttleUntil   = map[string]time.Time{}
	throttleAt      = map[string]time.Time{}
	throttleStrikes int
)

// dailyCache is one year of daily bars per symbol, keyed by the IST day. A daily candle changes
// once a session; the live price comes from the intraday tape that gather already fetches, so a
// poll no longer re-downloads a year of history for every instrument.
var (
	dailyMu    sync.Mutex
	dailyCache = map[string]dailyBars{}
)

type dailyBars struct {
	day  string
	bars []models.Bar
}

// CachedBars is Bars with a day-long memory. The returned slice is a copy: callers may patch the
// live candle without poisoning the cache.
func (y *YahooSource) CachedBars(symbol string) ([]models.Bar, error) {
	day := schedule.ISTDay(time.Now().UTC())
	dailyMu.Lock()
	entry, ok := dailyCache[symbol]
	dailyMu.Unlock()
	if ok && entry.day == day {
		out := make([]models.Bar, len(entry.bars))
		copy(out, entry.bars)
		return out, nil
	}
	bars, err := y.Bars(symbol)
	if err != nil {
		return nil, err
	}
	dailyMu.Lock()
	dailyCache[symbol] = dailyBars{day: day, bars: bars}
	dailyMu.Unlock()
	slog.Debug("daily bars fetched", "symbol", symbol, "bars", len(bars), "day", day)
	return bars, nil
}

// DropDailyCache forgets one symbol, for a forced refresh.
func DropDailyCache(symbol string) {
	dailyMu.Lock()
	delete(dailyCache, symbol)
	dailyMu.Unlock()
}

// noteThrottle remembers a refusal per edge: query1 saying no must not stop the ask query2 would
// have answered, which is what made one 429 look like an outage of the whole poll.
func noteThrottle(host string, resp *http.Response) {
	throttleMu.Lock()
	defer throttleMu.Unlock()
	throttleStrikes++
	wait := throttleFirst << (throttleStrikes - 1)
	if wait <= 0 || wait > throttleMax {
		wait = throttleMax
	}
	if raw := resp.Header.Get("Retry-After"); raw != "" {
		if secs, err := strconv.Atoi(strings.TrimSpace(raw)); err == nil && secs > 0 && secs <= 900 {
			if h := time.Duration(secs) * time.Second; h > wait {
				wait = h
			}
		}
	}
	now := time.Now()
	throttleAt[host] = now
	throttleUntil[host] = now.Add(wait)
	slog.Warn("yahoo throttled this edge; backing off", "host", host, "for", wait.Round(time.Second),
		"strikes", throttleStrikes)
}

func hostBlocked(host string) bool {
	throttleMu.Lock()
	defer throttleMu.Unlock()
	return time.Now().Before(throttleUntil[host])
}

// throttleFreeIn is how long until the first edge takes a request again — zero when one is free.
func throttleFreeIn() time.Duration {
	throttleMu.Lock()
	defer throttleMu.Unlock()
	now := time.Now()
	free := time.Duration(-1)
	for _, host := range yahooHosts {
		left := throttleUntil[host].Sub(now)
		if left <= 0 {
			return 0
		}
		if free < 0 || left < free {
			free = left
		}
	}
	return free
}

// A quiet window is what resets the strike count: a single good reply between two bursts must not
// make the next 429 cheap again.
func throttleEnded() {
	throttleMu.Lock()
	defer throttleMu.Unlock()
	last := time.Time{}
	for _, at := range throttleAt {
		if at.After(last) {
			last = at
		}
	}
	if time.Since(last) > throttleCalm {
		throttleStrikes = 0
	}
}

// waitOutThrottle sleeps through a block short enough to be worth waiting for, and reports whether
// it did. The daily sweep can afford this; a poll tick cannot.
func waitOutThrottle(max time.Duration) bool {
	left := throttleFreeIn()
	if left <= 0 || left > max {
		return false
	}
	time.Sleep(left)
	return true
}

// doRetry absorbs the connection failures and 5xx a second attempt fixes. A 429 is not one of them:
// asking a host that just refused is what turned one refusal into a minute of silence.
func doRetry(client *http.Client, url string, attempts int) (*http.Response, error) {
	req, err := http.NewRequest(http.MethodGet, url, nil)
	if err != nil {
		return nil, err
	}
	for i := 0; ; i++ {
		pace()
		resp, err := client.Do(req)
		if err != nil {
			if i >= attempts-1 {
				return nil, err
			}
			time.Sleep(time.Duration(300*(i+1)) * time.Millisecond)
			continue
		}
		if resp.StatusCode < 500 {
			throttleEnded()
			return resp, nil
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

// getYahoo takes a path, not a URL: a refusal from one edge has to be able to hand the same ask to
// the other before the caller loses its answer.
func getYahoo(client *http.Client, path string) (*http.Response, error) {
	first := yahooHost()
	second := yahooHosts[0]
	if second == first {
		second = yahooHosts[1]
	}
	for _, host := range []string{first, second} {
		if hostBlocked(host) {
			continue
		}
		resp, err := doRetry(client, host+path, 2)
		if err != nil {
			return nil, err
		}
		if resp.StatusCode != http.StatusTooManyRequests {
			return resp, nil
		}
		noteThrottle(host, resp)
		resp.Body.Close()
	}
	return nil, fmt.Errorf("yahoo is throttling this client; backing off")
}

// Yahoo wants the window as epoch seconds; every caller speaks ranges, so one maps to the other.
func rangeSpan(rangeStr string) time.Duration {
	switch rangeStr {
	case "1d":
		return 24 * time.Hour
	case "5d":
		return 5 * 24 * time.Hour
	case "7d":
		return 7 * 24 * time.Hour
	case "1mo":
		return 31 * 24 * time.Hour
	case "3mo":
		return 92 * 24 * time.Hour
	case "6mo":
		return 183 * 24 * time.Hour
	case "2y":
		return 730 * 24 * time.Hour
	}
	return 366 * 24 * time.Hour
}

func chartQuery(interval, rangeStr string, now time.Time) url.Values {
	q := url.Values{}
	q.Set("period1", strconv.FormatInt(now.Add(-rangeSpan(rangeStr)).Unix(), 10))
	q.Set("period2", strconv.FormatInt(now.Unix(), 10))
	q.Set("interval", interval)
	q.Set("includePrePost", "true")
	q.Set("events", "div|split|earn")
	q.Set("usePreMarketChartPreviousClose", "true")
	q.Set("lang", "en-US")
	q.Set("region", "US")
	q.Set("source", "cosaic")
	return q
}

func fetchChart(client *http.Client, symbol, interval, rangeStr string) ([]models.Bar, error) {
	q := chartQuery(interval, rangeStr, time.Now())
	resp, err := getYahoo(client, chartPath+"/"+symbol+"?"+q.Encode())
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
	resp, err := getYahoo(client, searchPath+"?"+q.Encode())
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

// Yahoo's spark endpoint takes at most twenty symbols per request — twenty-one is a 400 — and the
// per-symbol chart endpoint throttles a 100-name sweep outright.
const sparkChunk = 20

// How long the daily sweep will sit out a throttle before giving up on the rest of the day.
var sparkWaitMax = 2 * time.Minute

func sparkRequest(client *http.Client, symbols []string) (map[string][]float64, error) {
	q := url.Values{}
	q.Set("symbols", strings.Join(symbols, ","))
	q.Set("range", "1mo")
	q.Set("interval", "1d")
	resp, err := getYahoo(client, sparkPath+"?"+q.Encode())
	if err != nil {
		return nil, fmt.Errorf("spark: request: %w", err)
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, fmt.Errorf("spark: read body: %w", err)
	}
	if resp.StatusCode < 200 || resp.StatusCode > 299 {
		return nil, fmt.Errorf("spark: http status %d", resp.StatusCode)
	}
	var payload map[string]struct {
		Close []float64 `json:"close"`
	}
	if err := json.Unmarshal(body, &payload); err != nil {
		return nil, fmt.Errorf("spark: payload: %w", err)
	}
	out := make(map[string][]float64, len(payload))
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
	return out, nil
}

// FetchClosesBatch returns daily closes for a hundred symbols in five requests, using Yahoo's spark
// endpoint. A chunk that is throttled is waited out and retried once; a chunk that still fails is
// dropped, not fatal, so one 429 costs twenty names rather than the whole day's list.
func FetchClosesBatch(client *http.Client, symbols []string) (map[string][]float64, error) {
	out := map[string][]float64{}
	var firstErr error
	for start := 0; start < len(symbols); start += sparkChunk {
		end := start + sparkChunk
		if end > len(symbols) {
			end = len(symbols)
		}
		batch, err := sparkRequest(client, symbols[start:end])
		if err != nil {
			firstErr = err
			if waitOutThrottle(sparkWaitMax) {
				batch, err = sparkRequest(client, symbols[start:end])
			}
		}
		if err != nil {
			slog.Warn("spark chunk dropped", "symbols", end-start, "error", err)
			continue
		}
		for symbol, closes := range batch {
			out[symbol] = closes
		}
		slog.Debug("spark chunk", "requested", end-start, "returned", len(batch))
	}
	if len(out) == 0 && firstErr != nil {
		return out, firstErr
	}
	if firstErr != nil {
		slog.Warn("spark sweep is partial", "symbols", len(symbols), "measured", len(out))
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
