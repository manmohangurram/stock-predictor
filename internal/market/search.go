package market

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strconv"
	"strings"

	"stock-predictor/internal/models"
)

// One search hit, from either source. Code is what the user adds.
type Hit struct {
	Code  string
	Name  string
	Class models.AssetClass
	Venue string
	Price *float64
}

// Yahoo covers exchange-listed instruments, and Indian mutual funds too — under opaque codes
// like 0P00012ALS.BO, which are priceable and addable. AMFI is the other source, by scheme code.
func ParseYahooSearch(body string) ([]Hit, error) {
	var q struct {
		Quotes []struct {
			Symbol    string  `json:"symbol"`
			ShortName *string `json:"shortname"`
			LongName  *string `json:"longname"`
			Venue     *string `json:"exchDisp"`
			QuoteType *string `json:"quoteType"`
		} `json:"quotes"`
	}
	if err := json.Unmarshal([]byte(body), &q); err != nil {
		return nil, err
	}
	out := []Hit{}
	for _, x := range q.Quotes {
		var class models.AssetClass
		if x.QuoteType != nil {
			switch *x.QuoteType {
			case "EQUITY", "ETF", "INDEX":
				class = models.ClassStock
			case "MUTUALFUND":
				class = models.ClassMutualFund
			default:
				continue
			}
		} else {
			continue
		}
		// For a fund the short name is the code, so the readable one is longname.
		name := ""
		if class == models.ClassMutualFund {
			if x.LongName != nil {
				name = *x.LongName
			} else if x.ShortName != nil {
				name = *x.ShortName
			}
		} else {
			if x.ShortName != nil {
				name = *x.ShortName
			} else if x.LongName != nil {
				name = *x.LongName
			}
		}
		if name == "" {
			name = x.Symbol
		}
		venue := ""
		if x.Venue != nil {
			venue = *x.Venue
		}
		out = append(out, Hit{Code: x.Symbol, Name: name, Class: class, Venue: venue})
	}
	return out, nil
}

// AMFI's file is the only reliable source for Indian mutual funds.
func SearchAMFI(navall, query string, limit int) []Hit {
	needle := strings.ToLower(strings.TrimSpace(query))
	if len(needle) < 3 {
		return []Hit{}
	}
	out := []Hit{}
	for _, raw := range strings.Split(navall, "\n") {
		line := strings.TrimSuffix(raw, "\r")
		fields := strings.Split(line, ";")
		for i := range fields {
			fields[i] = strings.TrimSpace(fields[i])
		}
		if len(fields) < 8 || strings.EqualFold(fields[0], "scheme code") {
			continue
		}
		name := fields[3]
		if !strings.Contains(strings.ToLower(line), needle) {
			continue
		}
		if strings.EqualFold(fields[5], "idcw-re-investment") {
			continue
		}
		var price *float64
		if v, err := strconv.ParseFloat(fields[6], 64); err == nil {
			price = &v
		}
		out = append(out, Hit{
			Code: fields[0], Name: strings.TrimSpace(name + " — " + fields[4] + " " + fields[5]),
			Class: models.ClassMutualFund, Venue: "AMFI", Price: price,
		})
		if len(out) >= limit {
			break
		}
	}
	return out
}

// Scheme code lookup used when adding a fund by code.
func AMFIByCode(navall, code string) *Hit {
	row, err := ParseNAV(navall, code)
	if err != nil {
		return nil
	}
	price := row.NAV
	return &Hit{Code: row.SchemeCode, Name: row.Name, Class: models.ClassMutualFund, Venue: "AMFI", Price: &price}
}

// Search across both sources. Yahoo for exchange-listed instruments, AMFI for Indian funds.
func Search(client *http.Client, query string, amfiText *string) ([]Hit, error) {
	q := url.Values{}
	q.Set("q", query)
	q.Set("quotesCount", "10")
	q.Set("newsCount", "0")
	resp, err := getYahoo(client, searchURL+"?"+q.Encode())
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode > 299 {
		return nil, fmt.Errorf("search: http status %d", resp.StatusCode)
	}
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, err
	}
	hits, err := ParseYahooSearch(string(body))
	if err != nil {
		hits = nil
	}
	if amfiText != nil {
		hits = append(hits, SearchAMFI(*amfiText, query, 5)...)
	}
	return hits, nil
}
