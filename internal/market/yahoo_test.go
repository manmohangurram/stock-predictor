package market

import (
	"strconv"
	"strings"
	"testing"
	"time"
)

const chartBody = `{"chart":{"result":[{
  "timestamp":[1763001600,1763088000],
  "indicators":{"quote":[{
    "open":[100.0,101.0],"high":[102.0,103.0],"low":[99.0,100.0],
    "close":[101.0,102.0],"volume":[1000,2000]
  }]}
}]}}`

func TestAChartBecomesBars(t *testing.T) {
	bars, err := ParseChart(chartBody)
	if err != nil {
		t.Fatal(err)
	}
	if len(bars) != 2 {
		t.Fatalf("%d", len(bars))
	}
	if bars[0].Close != 101 || bars[1].Volume != 2000 {
		t.Fatalf("%+v", bars)
	}
	if !strings.HasPrefix(bars[0].Ts, "2025-11-13") {
		t.Fatalf("%q", bars[0].Ts)
	}
}

func TestAHaltedCandleIsSkippedNotFabricated(t *testing.T) {
	body := `{"chart":{"result":[{"timestamp":[1763001600,1763088000],
      "indicators":{"quote":[{"open":[100.0,null],"high":[102.0,null],"low":[99.0,null],
      "close":[101.0,null],"volume":[1,2]}]}}]}}`
	bars, err := ParseChart(body)
	if err != nil {
		t.Fatal(err)
	}
	if len(bars) != 1 {
		t.Fatalf("%+v", bars)
	}
}

func TestAMissingResultIsAnError(t *testing.T) {
	if _, err := ParseChart(`{"chart":{"result":[]}}`); err != nil {
		t.Fatalf("an empty result is an empty series: %v", err)
	}
	if _, err := ParseChart(`{"chart":{}}`); err == nil {
		t.Fatal("a missing result passed")
	}
}

func TestTheChartWindowIsTheRangeAsEpochSeconds(t *testing.T) {
	now := time.Date(2026, 11, 20, 10, 0, 0, 0, time.UTC)
	q := chartQuery("1m", "5d", now)
	if q.Get("interval") != "1m" || q.Get("period2") != strconv.FormatInt(now.Unix(), 10) {
		t.Fatalf("%v", q)
	}
	start, _ := strconv.ParseInt(q.Get("period1"), 10, 64)
	if now.Unix()-start != int64((5 * 24 * time.Hour).Seconds()) {
		t.Fatalf("period1 is %d seconds back", now.Unix()-start)
	}
	for key, want := range map[string]string{
		"includePrePost": "true", "events": "div|split|earn",
		"usePreMarketChartPreviousClose": "true", "lang": "en-US", "region": "US",
		"source": "cosaic",
	} {
		if q.Get(key) != want {
			t.Fatalf("%s = %q, want %q", key, q.Get(key), want)
		}
	}
}

func TestBothYahooHostsAreUsed(t *testing.T) {
	seen := map[string]bool{}
	for i := 0; i < 200; i++ {
		seen[yahooHost()] = true
	}
	if len(seen) != len(yahooHosts) {
		t.Fatalf("%d of %d hosts used", len(seen), len(yahooHosts))
	}
}

const newsBody = `{"news":[
  {"title":"Infosys raises guidance","publisher":"Reuters","link":"https://n/1","providerPublishTime":1763001600},
  {"title":"Infosys raises guidance","publisher":"Reuters","link":"https://n/1","providerPublishTime":1763001600},
  {"title":"Infosys wins a deal","publisher":"Mint","link":"https://n/2","providerPublishTime":1763088000}
]}`

func TestHeadlinesAreDedupedByUrl(t *testing.T) {
	news, err := ParseYahooNews(newsBody)
	if err != nil {
		t.Fatal(err)
	}
	if len(news) != 2 || news[1].Publisher != "Mint" {
		t.Fatalf("%+v", news)
	}
}

func TestTheNewsQueryDropsTheExchangeSuffix(t *testing.T) {
	if NewsQuery("INFY.NS") != "INFY" || NewsQuery("INFY.BO") != "INFY" || NewsQuery("ABC") != "ABC" {
		t.Fatal("news query")
	}
}

const navAll = `Scheme Code;ISIN Div Payout;ISIN Div Reinvestment;Scheme Name;Plan;Option;Net Asset Value;Date
120503;INF846K01EW2;-;Axis ELSS Tax Saver Fund;Growth;-;74.62;13-Nov-2026
118955;-;-;HDFC Flexi Cap Fund;Growth;-;1,234.5;13-Nov-2026
`

func TestTheHeaderIsTrustedForColumnPositions(t *testing.T) {
	row, err := ParseNAV(navAll, "120503")
	if err != nil {
		t.Fatal(err)
	}
	if row.NAV != 74.62 || row.Date != "2026-11-13" {
		t.Fatalf("%+v", row)
	}
	if !strings.Contains(row.Name, "Axis ELSS") || !strings.Contains(row.Name, "Growth") {
		t.Fatalf("%q", row.Name)
	}
}

func TestAMissingSchemeIsAnError(t *testing.T) {
	if _, err := ParseNAV(navAll, "999999"); err == nil {
		t.Fatal("missing scheme passed")
	}
}

func TestAMFISearchMatchesTheSchemeName(t *testing.T) {
	hits := SearchAMFI(navAll, "axis elss", 5)
	if len(hits) != 1 || hits[0].Code != "120503" || hits[0].Class != "mutual_fund" {
		t.Fatalf("%+v", hits)
	}
	if len(SearchAMFI(navAll, "ax", 5)) != 0 {
		t.Fatal("a two-letter query must not search")
	}
}

func TestYahooSearchKeepsOnlyTradableTypes(t *testing.T) {
	body := `{"quotes":[
      {"symbol":"INFY.NS","shortname":"Infosys","exchDisp":"NSE","quoteType":"EQUITY"},
      {"symbol":"0P00012ALS.BO","longname":"Axis ELSS","quoteType":"MUTUALFUND"},
      {"symbol":"^NSEI","shortname":"Nifty","quoteType":"CURRENCY"}
    ]}`
	hits, err := ParseYahooSearch(body)
	if err != nil {
		t.Fatal(err)
	}
	if len(hits) != 2 || hits[0].Class != "stock" || hits[1].Name != "Axis ELSS" {
		t.Fatalf("%+v", hits)
	}
}
