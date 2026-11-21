package rules

import (
	"database/sql"
	"fmt"
	"log/slog"
	"math"
	"net/url"
	"strconv"
	"strings"

	"stock-predictor/internal/proxyurl"
	"stock-predictor/internal/secrets"
)

// Knobs the app reads while running, as opposed to the deployment facts in Config.
//
// These live in the settings table so they can be edited in the UI and take effect on the
// next poll without a restart. Reading is deliberately forgiving: a corrupt row falls back
// to its default, because a bad number must never stop the app from waking up.

// What the API is allowed to say about the proxy: that it exists, and where to.
type View struct {
	PollSecs            uint64  `json:"poll_secs"`
	OffHoursSecs        uint64  `json:"off_hours_secs"`
	PromptWindowHours   uint32  `json:"prompt_window_hours"`
	MinPriceMove        float64 `json:"min_price_move"`
	DailyCap            uint32  `json:"daily_cap"`
	ScanMinPrice        float64 `json:"scan_min_price"`
	ScanMinTurnover     float64 `json:"scan_min_turnover"`
	ScanLockedPct       float64 `json:"scan_locked_pct"`
	ProxyConfigured     bool    `json:"proxy_configured"`
	ProxyHost           string  `json:"proxy_host"`
	ProxyUser           *string `json:"proxy_user"`
	ProxyPassConfigured bool    `json:"proxy_pass_configured"`
	ProxyBypass         *string `json:"proxy_bypass"`
}

type Runtime struct {
	PollSecs          uint64
	OffHoursSecs      uint64
	PromptWindowHours uint32
	MinPriceMove      float64
	DailyCap          uint32
	ScanMinPrice      float64
	ScanMinTurnover   float64
	ScanLockedPct     float64
	Proxy             *string
	ProxyUser         *string
	ProxyPass         *string
	ProxyBypass       *string
}

func DefaultRuntime() Runtime {
	return Runtime{
		PollSecs:          900,
		OffHoursSecs:      7_200,
		PromptWindowHours: 4,
		MinPriceMove:      2.0,
		DailyCap:          200,
		ScanMinPrice:      10.0,
		ScanMinTurnover:   10_000_000.0,
		ScanLockedPct:     19.5,
	}
}

// (key, default, lowest, highest) — the table the migration and the UI both speak.
var Knobs = []struct {
	Key     string
	Default float64
	Lo      float64
	Hi      float64
}{
	{"poll_secs", 900.0, 60.0, 86_400.0},
	{"off_hours_secs", 7_200.0, 60.0, 86_400.0},
	{"prompt_window_hours", 4.0, 1.0, 24.0},
	{"min_price_move", 2.0, 0.0, 1_000_000.0},
	{"daily_cap", 200.0, 0.0, 10_000.0},
	// The short-term scan's three filters, in rupees and percent.
	{"scan_min_price", 10.0, 0.0, 1_000_000.0},
	{"scan_min_turnover", 10_000_000.0, 0.0, 1e13},
	{"scan_locked_pct", 19.5, 0.0, 100.0},
}

func knob(key string) *struct {
	Key     string
	Default float64
	Lo      float64
	Hi      float64
} {
	for i := range Knobs {
		if Knobs[i].Key == key {
			return &Knobs[i]
		}
	}
	return nil
}

func RuntimeFromMap(m map[string]string) Runtime {
	d := DefaultRuntime()
	num := func(key string, def float64) float64 {
		k := knob(key)
		if k == nil {
			return def
		}
		v, ok := m[key]
		if !ok || strings.TrimSpace(v) == "" {
			return def
		}
		n, err := strconv.ParseFloat(strings.TrimSpace(v), 64)
		if err != nil || math.IsNaN(n) || math.IsInf(n, 0) {
			slog.Warn("unreadable setting, using the default", "key", key, "value", v)
			return def
		}
		return math.Min(math.Max(n, k.Lo), k.Hi)
	}
	text := func(key string) *string {
		if v, ok := m[key]; ok {
			t := strings.TrimSpace(v)
			if t != "" {
				return &t
			}
		}
		return nil
	}
	rt := Runtime{
		PollSecs:          uint64(num("poll_secs", float64(d.PollSecs))),
		OffHoursSecs:      uint64(num("off_hours_secs", float64(d.OffHoursSecs))),
		PromptWindowHours: uint32(num("prompt_window_hours", float64(d.PromptWindowHours))),
		MinPriceMove:      num("min_price_move", d.MinPriceMove),
		DailyCap:          uint32(num("daily_cap", float64(d.DailyCap))),
		ScanMinPrice:      num("scan_min_price", d.ScanMinPrice),
		ScanMinTurnover:   num("scan_min_turnover", d.ScanMinTurnover),
		ScanLockedPct:     num("scan_locked_pct", d.ScanLockedPct),
	}
	if proxy := text("proxy"); proxy != nil {
		clean, user, _ := SplitLegacyProxy(*proxy)
		rt.Proxy = &clean
		if user != nil && rt.ProxyUser == nil {
			rt.ProxyUser = user
		}
	}
	if u := text("proxy_user"); u != nil {
		rt.ProxyUser = u
	}
	rt.ProxyBypass = text("proxy_bypass")
	return rt
}

func (r Runtime) View() View {
	ProxyHost := ""
	if r.Proxy != nil {
		ProxyHost = proxyurl.ProxyHost(*r.Proxy)
	}
	return View{
		PollSecs:            r.PollSecs,
		OffHoursSecs:        r.OffHoursSecs,
		PromptWindowHours:   r.PromptWindowHours,
		MinPriceMove:        r.MinPriceMove,
		DailyCap:            r.DailyCap,
		ScanMinPrice:        r.ScanMinPrice,
		ScanMinTurnover:     r.ScanMinTurnover,
		ScanLockedPct:       r.ScanLockedPct,
		ProxyConfigured:     r.Proxy != nil,
		ProxyHost:           ProxyHost,
		ProxyUser:           r.ProxyUser,
		ProxyPassConfigured: r.ProxyPass != nil,
		ProxyBypass:         r.ProxyBypass,
	}
}

// Range check used when a human writes a value: reject, never clamp. Clamping is for reads.
func RuntimeCheck(key string, value float64) error {
	k := knob(key)
	if k == nil {
		return nil
	}
	if math.IsNaN(value) || math.IsInf(value, 0) || value < k.Lo || value > k.Hi {
		return fmt.Errorf("%s must be between %v and %v", key, k.Lo, k.Hi)
	}
	return nil
}

// An older database stored http://user:pass@host in one row. Split it so both shapes work.
func SplitLegacyProxy(raw string) (string, *string, *string) {
	u, err := url.Parse(raw)
	if err != nil || u.User == nil || u.User.Username() == "" {
		return raw, nil, nil
	}
	user := u.User.Username()
	clean := *u
	clean.User = nil
	var pass *string
	if p, ok := u.User.Password(); ok {
		pass = &p
	}
	return clean.String(), &user, pass
}

// One query per poll. Missing keys take their defaults, so an empty table is a working app.
func LoadRuntime(pool *sql.DB) (Runtime, error) {
	rows, err := pool.Query("SELECT key, value FROM settings")
	if err != nil {
		return Runtime{}, err
	}
	defer rows.Close()
	m := map[string]string{}
	for rows.Next() {
		var k, v string
		if err := rows.Scan(&k, &v); err != nil {
			return Runtime{}, err
		}
		m[k] = v
	}
	if err := rows.Err(); err != nil {
		return Runtime{}, err
	}
	rt := RuntimeFromMap(m)
	// The password is encrypted at rest, so it does not come back through RuntimeFromMap.
	pass, err := secrets.GetSecret(pool, "proxy_pass")
	if err != nil {
		return Runtime{}, err
	}
	if pass != "" {
		rt.ProxyPass = &pass
	}
	return rt, nil
}
