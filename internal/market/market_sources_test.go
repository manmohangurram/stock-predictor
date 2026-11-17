package market

import (
	"strings"
	"testing"
)

const feed = `<?xml version="1.0"?><rss><channel>
  <item>
    <title>Infosys raises FY27 guidance after strong Q2 - Reuters</title>
    <link>https://news.google.com/rss/articles/AAA</link>
    <pubDate>Fri, 13 Nov 2026 10:30:00 GMT</pubDate>
    <source url="https://www.reuters.com">Reuters</source>
  </item>
  <item>
    <title><![CDATA[Infosys wins a large deal &amp; lifts outlook - The Economic Times]]></title>
    <link>https://news.google.com/rss/articles/BBB</link>
    <pubDate>Fri, 13 Nov 2026 08:00:00 GMT</pubDate>
    <source url="https://economictimes.indiatimes.com">The Economic Times</source>
  </item>
  <item>
    <title>no date, dropped</title>
    <link>https://news.google.com/rss/articles/CCC</link>
  </item>
</channel></rss>`

func TestAFeedBecomesDatedHeadlinesFromRealPublishers(t *testing.T) {
	hits := ParseGoogleNews(feed)
	if len(hits) != 2 {
		t.Fatalf("an item without a date is not an article: %+v", hits)
	}
	if hits[0].Title != "Infosys raises FY27 guidance after strong Q2" {
		t.Fatalf("%q", hits[0].Title)
	}
	if hits[0].Publisher != "Reuters" {
		t.Fatalf("%q", hits[0].Publisher)
	}
	if !strings.HasPrefix(hits[0].PublishedAt, "2026-11-13") {
		t.Fatalf("%q", hits[0].PublishedAt)
	}
	if hits[1].Title != "Infosys wins a large deal & lifts outlook" {
		t.Fatalf("%q", hits[1].Title)
	}
}

func TestAMarketPublisherWinsWhenTheFeedHasOne(t *testing.T) {
	raw := `<rss><channel>
      <item><title>Infosys raises guidance - Prizes Weekly</title>
        <link>https://n/1</link><pubDate>Fri, 13 Nov 2026 10:30:00 GMT</pubDate>
        <source url="https://prizes.test">Prizes Weekly</source></item>
      <item><title>Infosys Q2 results beat estimates - Business Standard</title>
        <link>https://n/2</link><pubDate>Fri, 13 Nov 2026 09:00:00 GMT</pubDate>
        <source url="https://business-standard.com">Business Standard</source></item>
    </channel></rss>`
	kept := PreferMarketSources(ParseGoogleNews(raw))
	if len(kept) != 1 || kept[0].Publisher != "Business Standard" {
		t.Fatalf("%+v", kept)
	}
}

func TestABodyThatIsNotAFeedIsEmptyNotAnError(t *testing.T) {
	if len(ParseGoogleNews("<html>not a feed</html>")) != 0 {
		t.Fatal("html parsed as a feed")
	}
	if len(ParseGoogleNews("")) != 0 {
		t.Fatal("empty body parsed")
	}
}

const fundamentalsBody = `{"timeseries":{"result":[{
  "meta":{"symbol":["INFY.NS"]},
  "annualTotalRevenue":[
    {"asOfDate":"2023-03-31","reportedValue":{"raw":1821200000000.0},"currencyCode":"INR"},
    {"asOfDate":"2024-03-31","reportedValue":{"raw":1862000000000.0},"currencyCode":"INR"},
    {"asOfDate":"2025-03-31","reportedValue":{"raw":1930000000000.0},"currencyCode":"INR"},
    {"asOfDate":"2026-03-31","reportedValue":{"raw":2020000000000.0},"currencyCode":"INR"}],
  "annualNetIncome":[
    {"asOfDate":"2025-03-31","reportedValue":{"raw":267130000000.0},"currencyCode":"INR"},
    {"asOfDate":"2026-03-31","reportedValue":{"raw":331000000000.0},"currencyCode":"INR"}]
}]}}`

func TestGrowthAndMarginAreComputedFromTheSeries(t *testing.T) {
	f, err := ParseFundamentals(fundamentalsBody)
	if err != nil {
		t.Fatal(err)
	}
	if len(f.Years) != 4 {
		t.Fatalf("%d years", len(f.Years))
	}
	if g, ok := f.RevenueGrowthPct(); !ok || g < 4.6 || g > 4.7 {
		t.Fatalf("revenue growth %v", g)
	}
	if p, ok := f.ProfitGrowthPct(); !ok || p < 23.8 || p > 24.0 {
		t.Fatalf("profit growth %v", p)
	}
	if m, ok := f.NetMarginPct(); !ok || m < 16.3 || m > 16.5 {
		t.Fatalf("net margin %v", m)
	}
}

func TestASingleYearHasNoGrowthToClaim(t *testing.T) {
	one := `{"timeseries":{"result":[{"annualTotalRevenue":[{"asOfDate":"2026-03-31","reportedValue":{"raw":1000.0},"currencyCode":"INR"}]}]}}`
	f, err := ParseFundamentals(one)
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := f.RevenueGrowthPct(); ok {
		t.Fatal("one year cannot grow")
	}
	if _, ok := f.ProfitGrowthPct(); ok {
		t.Fatal("one year cannot grow")
	}
}

func TestTheSummaryStatesCurrencyAndDirection(t *testing.T) {
	f, _ := ParseFundamentals(fundamentalsBody)
	s := f.Summary()
	if !strings.Contains(s, "INR") || !strings.Contains(s, "FY2026-03-31") || !strings.Contains(s, "+4.7%") {
		t.Fatalf("%q", s)
	}
}

const mfapiBody = `{
  "status":"SUCCESS",
  "meta":{"scheme_name":"Axis ELSS - Direct - Growth",
           "fund_house":"Axis Mutual Fund",
           "scheme_category":"Equity Scheme - ELSS"},
  "data":[
    {"date":"01-01-2023","nav":"100.00"},
    {"date":"01-01-2024","nav":"112.00"},
    {"date":"01-01-2025","nav":"118.00"},
    {"date":"01-01-2026","nav":"130.00"}
  ]
}`

func TestTrailingReturnReadsAcrossTheYearBoundary(t *testing.T) {
	h, err := ParseFundHistory("120503", mfapiBody)
	if err != nil {
		t.Fatal(err)
	}
	if h.Name != "Axis ELSS - Direct - Growth" {
		t.Fatalf("%q", h.Name)
	}
	r, ok := h.TrailingReturn(3.0)
	if !ok || r < 29 || r > 31 {
		t.Fatalf("%v %v", r, ok)
	}
}

func TestASeriesTooShortIsNoneNotAGuess(t *testing.T) {
	h, err := ParseFundHistory("120503", `{"meta":{},"data":[{"date":"01-01-2026","nav":"100.0"}]}`)
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := h.TrailingReturn(1.0); ok {
		t.Fatal("a short series produced a return")
	}
}

func TestNAVIsSortedOldestFirst(t *testing.T) {
	h, _ := ParseFundHistory("120503", mfapiBody)
	first, _ := h.Navs[0][0].(string)
	last, _ := h.Navs[len(h.Navs)-1][0].(string)
	if first != "2023-01-01" || last != "2026-01-01" {
		t.Fatalf("%q %q", first, last)
	}
}
