package market

import (
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"net/url"
	"strings"
	"time"

	"stock-predictor/internal/models"
	"stock-predictor/internal/tstamp"
)

// Company news from Google News' own RSS feed. Public, no key, English and the India edition,
// with a real publisher and a date on every item.

const googleNewsURL = "https://news.google.com/rss/search"

// The last days of coverage for company. when: is Google News' own operator, so the window is
// applied by the index, not by us after the fact.
func FetchGoogleNews(client *http.Client, company string, days int) ([]models.Headline, error) {
	// Aimed at what re-rates a stock, in the market it trades in. A bare name answers with prize
	// ceremonies and interviews.
	query := fmt.Sprintf(
		`"%s" (stock OR shares OR results OR earnings OR guidance OR order OR profit OR downgrade OR upgrade) India when:%dd`,
		company, days)
	q := url.Values{}
	q.Set("q", query)
	q.Set("hl", "en-IN")
	q.Set("gl", "IN")
	q.Set("ceid", "IN:en")
	resp, err := client.Get(googleNewsURL + "?" + q.Encode())
	if err != nil {
		return nil, fmt.Errorf("google news: request: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode > 299 {
		return nil, fmt.Errorf("google news: http status %d", resp.StatusCode)
	}
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, fmt.Errorf("google news: read body: %w", err)
	}
	items := ParseGoogleNews(string(body))
	slog.Debug("google news", "company", company, "days", days, "items", len(items))
	return items, nil
}

// Publishers that report markets rather than reprint a press release. Preferred, not required:
// an empty list is worse than a weaker source.
var trustedPublishers = []string{
	"reuters", "business standard", "the economic times", "economic times", "mint", "livemint",
	"moneycontrol", "cnbc", "the hindu businessline", "businessline", "financial express",
	"bloomberg", "ndtv profit", "the times of india", "barron", "marketwatch", "forbes india",
}

func isTrusted(publisher string) bool {
	p := strings.ToLower(publisher)
	for _, t := range trustedPublishers {
		if strings.Contains(p, t) {
			return true
		}
	}
	return false
}

// Keep the market publishers when the feed has any, and everything when it has none.
func PreferMarketSources(items []models.Headline) []models.Headline {
	hasTrusted := false
	for _, h := range items {
		if isTrusted(h.Publisher) {
			hasTrusted = true
			break
		}
	}
	if !hasTrusted {
		return items
	}
	out := []models.Headline{}
	for _, h := range items {
		if isTrusted(h.Publisher) {
			out = append(out, h)
		}
	}
	return out
}

// The items of a Google News RSS body. Hand-parsed: the shape is fixed and a dependency for one
// feed is not worth it.
func ParseGoogleNews(body string) []models.Headline {
	out := []models.Headline{}
	rest := body
	for {
		start := strings.Index(rest, "<item>")
		if start < 0 {
			break
		}
		after := rest[start+len("<item>"):]
		end := strings.Index(after, "</item>")
		if end < 0 {
			break
		}
		item := after[:end]
		rest = after[end+len("</item>"):]

		link, ok := rssTag(item, "link")
		if !ok {
			continue
		}
		rawTitle, ok := rssTag(item, "title")
		if !ok {
			continue
		}
		publisher, _ := rssTag(item, "source")
		// Google formats the title as "Headline - Publisher"; the publisher has its own tag.
		title := rawTitle
		if idx := strings.LastIndex(rawTitle, " - "); idx >= 0 && publisher != "" &&
			rawTitle[idx+3:] == publisher {
			title = rawTitle[:idx]
		}
		publishedAt := ""
		if rawDate, ok := rssTag(item, "pubDate"); ok {
			if d, err := time.Parse(time.RFC1123Z, rawDate); err == nil {
				publishedAt = tstamp.From(d)
			} else if d, err := time.Parse(time.RFC1123, rawDate); err == nil {
				publishedAt = tstamp.From(d)
			}
		}
		if publishedAt == "" {
			continue
		}
		out = append(out, models.Headline{
			Title: title, Publisher: publisher, PublishedAt: publishedAt, URL: link,
		})
	}
	return out
}

// One tag's text, with the CDATA and entity forms RSS feeds actually use.
func rssTag(item, name string) (string, bool) {
	open := "<" + name
	start := strings.Index(item, open)
	if start < 0 {
		return "", false
	}
	after := item[start:]
	inline := strings.Index(after, ">")
	if inline < 0 {
		return "", false
	}
	if strings.HasSuffix(after[:inline], "/") {
		return "", false
	}
	body := after[inline+1:]
	closeIdx := strings.Index(body, "</"+name+">")
	if closeIdx < 0 {
		return "", false
	}
	raw := strings.TrimSpace(body[:closeIdx])
	text := raw
	if strings.HasPrefix(text, "<![CDATA[") && strings.HasSuffix(text, "]]>") {
		text = text[len("<![CDATA[") : len(text)-len("]]>")]
	}
	return unescapeXML(text), true
}

func unescapeXML(s string) string {
	r := strings.NewReplacer(
		"&lt;", "<", "&gt;", ">", "&quot;", `"`, "&#39;", "'", "&apos;", "'", "&amp;", "&",
	)
	return r.Replace(s)
}
