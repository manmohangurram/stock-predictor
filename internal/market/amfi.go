package market

import (
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"strconv"
	"strings"
	"time"
)

// AMFI moved this file to the portal host; the old www path redirects.
const navAllURL = "https://portal.amfiindia.com/spages/NAVAll.txt"

type NavRow struct {
	SchemeCode string
	Name       string
	NAV        float64
	Date       string
}

// The live file has changed shape twice. Column positions are therefore read from the header
// when one is present, falling back to the historical six-column layout.
func ParseNAV(text, schemeCode string) (NavRow, error) {
	var columns []string
	for _, raw := range strings.Split(text, "\n") {
		line := strings.TrimSpace(strings.TrimSuffix(raw, "\r"))
		if line == "" {
			continue
		}
		fields := strings.Split(line, ";")
		for i := range fields {
			fields[i] = strings.TrimSpace(fields[i])
		}
		if strings.EqualFold(fields[0], "scheme code") {
			columns = make([]string, len(fields))
			for i, c := range fields {
				columns[i] = strings.ToLower(c)
			}
			continue
		}
		if fields[0] != schemeCode {
			continue
		}
		navIndex, dateIndex, nameIndex := 4, 5, 1
		planIndex, optionIndex := -1, -1
		if columns != nil {
			index := func(want string) (int, error) {
				for i, h := range columns {
					if h == want {
						return i, nil
					}
				}
				return -1, fmt.Errorf("amfi: header has no %q column — file shape changed", want)
			}
			var err error
			if navIndex, err = index("net asset value"); err != nil {
				return NavRow{}, err
			}
			if dateIndex, err = index("date"); err != nil {
				return NavRow{}, err
			}
			if nameIndex, err = index("scheme name"); err != nil {
				return NavRow{}, err
			}
			for i, h := range columns {
				if h == "plan" {
					planIndex = i
				}
				if h == "option" {
					optionIndex = i
				}
			}
		}
		if navIndex >= len(fields) {
			return NavRow{}, fmt.Errorf("amfi: row too short for nav column %d", navIndex)
		}
		nav, err := strconv.ParseFloat(fields[navIndex], 64)
		if err != nil {
			return NavRow{}, fmt.Errorf("amfi: bad nav %q for %s", fields[navIndex], schemeCode)
		}
		if !(nav > 0) {
			return NavRow{}, fmt.Errorf("amfi: non-positive nav %v for %s", nav, schemeCode)
		}
		if dateIndex >= len(fields) {
			return NavRow{}, fmt.Errorf("amfi: row too short for date column")
		}
		name := ""
		if nameIndex < len(fields) {
			name = fields[nameIndex]
		}
		for _, i := range []int{planIndex, optionIndex} {
			if i >= 0 && i < len(fields) && fields[i] != "" && fields[i] != "-" {
				name += " — " + fields[i]
			}
		}
		iso, err := isoDate(fields[dateIndex])
		if err != nil {
			return NavRow{}, err
		}
		return NavRow{SchemeCode: fields[0], Name: name, NAV: nav, Date: iso}, nil
	}
	return NavRow{}, fmt.Errorf("amfi: scheme %s not found", schemeCode)
}

func isoDate(d string) (string, error) {
	parsed, err := time.Parse("02-Jan-2006", d)
	if err != nil {
		return "", fmt.Errorf("amfi: bad date %q", d)
	}
	return parsed.Format("2006-01-02"), nil
}

func FetchNAVAll(client *http.Client) (string, error) {
	resp, err := client.Get(navAllURL)
	if err != nil {
		return "", fmt.Errorf("amfi: request: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode > 299 {
		return "", fmt.Errorf("amfi: http status %d", resp.StatusCode)
	}
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return "", fmt.Errorf("amfi: read body: %w", err)
	}
	slog.Debug("amfi navall fetched", "bytes", len(body))
	return string(body), nil
}
