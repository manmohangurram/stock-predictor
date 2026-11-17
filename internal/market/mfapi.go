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

// AMFI's NAV history through the mfapi.in mirror: one scheme code, its whole NAV series.
//
// Yahoo's Indian fund coverage is patchy and opaque, but AMFI scheme codes are stable and
// complete. This is the source for "what has this fund actually returned".

const mfapiURL = "https://api.mfapi.in/mf"

// One fund's history, oldest first, with its name and the trailing returns.
type FundHistory struct {
	Code      string
	Name      string
	FundHouse string
	Category  string
	Navs      [][2]any // [date string, nav float64]
}

// Percentage return over the trailing years, from the oldest point within that window.
func (h FundHistory) TrailingReturn(years float64) (float64, bool) {
	if len(h.Navs) == 0 {
		return 0, false
	}
	lastDate, _ := h.Navs[len(h.Navs)-1][0].(string)
	lastNav, _ := h.Navs[len(h.Navs)-1][1].(float64)
	cutoff := monthsAgo(lastDate, int64(years*12))
	var startDate string
	var startNav float64
	found := false
	for i := len(h.Navs) - 1; i >= 0; i-- {
		d, _ := h.Navs[i][0].(string)
		if d <= cutoff {
			startDate = d
			startNav, _ = h.Navs[i][1].(float64)
			found = true
			break
		}
	}
	if !found || startNav <= 0 || startDate > cutoff {
		return 0, false
	}
	return (lastNav - startNav) / startNav * 100, true
}

// DD-MM-YYYY -> YYYY-MM-DD, so comparisons and arithmetic work on a sortable string.
func sortableDate(date string) string {
	parts := strings.Split(date, "-")
	if len(parts) == 3 && len(parts[2]) == 4 {
		return parts[2] + "-" + parts[1] + "-" + parts[0]
	}
	return date
}

// YYYY-MM-DD shifted back months months. Approximation is fine: the exact anniversary day is not
// what decides a ranking.
func monthsAgo(date string, months int64) string {
	y, m, d := 2026, 1, 1
	parts := strings.Split(date, "-")
	if len(parts) == 3 {
		y, _ = strconv.Atoi(parts[0])
		m, _ = strconv.Atoi(parts[1])
		d, _ = strconv.Atoi(parts[2])
	}
	total := int64(y)*12 + int64(m-1) - months
	ny := floorDiv(total, 12)
	nm := total - ny*12 + 1
	return fmt.Sprintf("%04d-%02d-%02d", ny, nm, d)
}

func floorDiv(a, b int64) int64 {
	q := a / b
	if (a%b != 0) && ((a < 0) != (b < 0)) {
		q--
	}
	return q
}

func FetchFundHistory(client *http.Client, code string) (FundHistory, error) {
	req, err := http.NewRequest(http.MethodGet, mfapiURL+"/"+code, nil)
	if err != nil {
		return FundHistory{}, err
	}
	req.Header.Set("User-Agent", "Mozilla/5.0 stock-predictor")
	resp, err := client.Do(req)
	if err != nil {
		return FundHistory{}, fmt.Errorf("mfapi: request: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode > 299 {
		return FundHistory{}, fmt.Errorf("mfapi: status %d", resp.StatusCode)
	}
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return FundHistory{}, fmt.Errorf("mfapi: body: %w", err)
	}
	history, err := ParseFundHistory(code, string(body))
	if err == nil {
		slog.Debug("mfapi history", "code", code, "name", history.Name, "navs", len(history.Navs))
	}
	return history, err
}

func ParseFundHistory(code, body string) (FundHistory, error) {
	var env struct {
		Meta struct {
			SchemeName     string `json:"scheme_name"`
			FundHouse      string `json:"fund_house"`
			SchemeCategory string `json:"scheme_category"`
		} `json:"meta"`
		Data []struct {
			Date string          `json:"date"`
			Nav  json.RawMessage `json:"nav"`
		} `json:"data"`
	}
	if err := json.Unmarshal([]byte(body), &env); err != nil {
		return FundHistory{}, fmt.Errorf("mfapi: payload shape: %w", err)
	}
	navs := [][2]any{}
	for _, p := range env.Data {
		nav, ok := parseNav(p.Nav)
		if !ok || nav <= 0 {
			continue
		}
		navs = append(navs, [2]any{sortableDate(p.Date), nav})
	}
	sort.SliceStable(navs, func(i, j int) bool {
		a, _ := navs[i][0].(string)
		b, _ := navs[j][0].(string)
		return a < b
	})
	return FundHistory{
		Code: code, Name: env.Meta.SchemeName, FundHouse: env.Meta.FundHouse,
		Category: env.Meta.SchemeCategory, Navs: navs,
	}, nil
}
func parseNav(raw json.RawMessage) (float64, bool) {
	var f float64
	if err := json.Unmarshal(raw, &f); err == nil {
		return f, true
	}
	var s string
	if err := json.Unmarshal(raw, &s); err == nil {
		if v, err := strconv.ParseFloat(strings.TrimSpace(s), 64); err == nil {
			return v, true
		}
	}
	return 0, false
}
