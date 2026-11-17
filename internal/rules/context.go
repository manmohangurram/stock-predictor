package rules

import (
	"fmt"
	"math"
	"time"

	"stock-predictor/internal/models"
)

// What a short-term decision needs that a long-term one does not.
//
// A trade that must finish inside a week lives or dies on the next few sessions: where
// today sits against yesterday's range, whether the move has volume behind it, how much
// of the day is left, and whether the market itself is risk-on. Computed from the bars we
// already store, so the prompt carries about twenty numbers instead of a thousand bars.

const istOffsetMinutes = 330
const (
	openMin  = 9*60 + 15
	closeMin = 15*60 + 30
)

type ShortContext struct {
	// Minutes since the open (0 before it) and until the close (0 after it).
	MinutesFromOpen uint32 `json:"minutes_from_open"`
	MinutesToClose  uint32 `json:"minutes_to_close"`
	// Today's move so far, and the gap from yesterday's close, in percent.
	DayChangePct *float64 `json:"day_change_pct"`
	GapPct       *float64 `json:"gap_pct"`
	// Yesterday's high/low and where the last price sits between them, in percent.
	PrevHighPct *float64 `json:"prev_high_pct"`
	PrevLowPct  *float64 `json:"prev_low_pct"`
	// Relative volume: today's pace against the 20-day average, 1.0 = average.
	Rvol   *float64 `json:"rvol"`
	ATR14  *float64 `json:"atr14"`
	AtrPct *float64 `json:"atr_pct"`
	// Close-to-close returns over 3, 5 and 10 sessions.
	Ret3D  *float64 `json:"ret_3d"`
	Ret5D  *float64 `json:"ret_5d"`
	Ret10D *float64 `json:"ret_10d"`
	// Distance from the 52-week extremes, in percent.
	From52wHighPct *float64 `json:"from_52w_high_pct"`
	From52wLowPct  *float64 `json:"from_52w_low_pct"`
}

func pctStr(v *float64) string {
	if v == nil {
		return "—"
	}
	return fmt.Sprintf("%+.2f%%", *v)
}

func numStr(v *float64) string {
	if v == nil {
		return "—"
	}
	return fmt.Sprintf("%.2f", *v)
}

// A compact block a model can read in a few seconds. Every line is a number it would
// otherwise have to compute, and an uncomputable line is omitted rather than filled with
// a guess.
func (c ShortContext) Render() string {
	neg := func(v *float64) *float64 {
		if v == nil {
			return nil
		}
		x := -*v
		return &x
	}
	out := ""
	out += fmt.Sprintf("Session: %d min since the open, %d min to the close\n",
		c.MinutesFromOpen, c.MinutesToClose)
	out += fmt.Sprintf("Today: change %s, gap %s, RVOL %s (20-day average = 1.0)\n",
		pctStr(c.DayChangePct), pctStr(c.GapPct), numStr(c.Rvol))
	out += fmt.Sprintf("Yesterday's range: last price is %s from yesterday's high, %s from its low\n",
		pctStr(c.PrevHighPct), pctStr(c.PrevLowPct))
	out += fmt.Sprintf("Volatility: ATR14 %s, that is %s of price\n",
		numStr(c.ATR14), pctStr(c.AtrPct))
	out += fmt.Sprintf("Momentum: 3-day %s, 5-day %s, 10-day %s\n",
		pctStr(c.Ret3D), pctStr(c.Ret5D), pctStr(c.Ret10D))
	out += fmt.Sprintf("Position in the year: %s below the 52-week high, %s above the low\n",
		pctStr(neg(c.From52wHighPct)), pctStr(c.From52wLowPct))
	return out
}

// Today's session position, in IST minutes.
func session(now time.Time) (uint32, uint32) {
	ist := now.Add(istOffsetMinutes * time.Minute)
	mins := ist.Hour()*60 + ist.Minute()
	fromOpen := uint32(0)
	if mins > openMin {
		fromOpen = uint32(mins - openMin)
	}
	toClose := uint32(0)
	if closeMin > mins {
		toClose = uint32(closeMin - mins)
	}
	if max := uint32(closeMin - openMin); fromOpen > max {
		fromOpen = max
	}
	return fromOpen, toClose
}

func fptr(v float64) *float64 { return &v }

// bars are daily closes, oldest first; intraday is today's tape (any interval), oldest first.
func ComputeShortContext(bars, intraday []models.Bar, now time.Time) ShortContext {
	c := ShortContext{}
	fromOpen, toClose := session(now)
	c.MinutesFromOpen = fromOpen
	c.MinutesToClose = toClose

	if len(bars) == 0 {
		return c
	}
	last := bars[len(bars)-1]
	var prev *models.Bar
	if len(bars) >= 2 {
		prev = &bars[len(bars)-2]
	}

	if prev != nil {
		if prev.Close > 0.0 {
			c.DayChangePct = fptr((last.Close - prev.Close) / prev.Close * 100.0)
			c.PrevHighPct = fptr((last.Close - prev.High) / prev.Close * 100.0)
			c.PrevLowPct = fptr((last.Close - prev.Low) / prev.Close * 100.0)
		}
		if len(intraday) > 0 {
			if prev.Close > 0.0 && intraday[0].Open > 0.0 {
				c.GapPct = fptr((intraday[0].Open - prev.Close) / prev.Close * 100.0)
			}
		}
	}

	// ATR14 from the daily series: simple true range average, which is what the rules use.
	if len(bars) >= 15 {
		tail := bars[len(bars)-14:]
		sum := 0.0
		for i := range tail {
			prevClose := tail[0].Open
			if i != 0 {
				prevClose = tail[i-1].Close
			}
			tr := tail[i].High - tail[i].Low
			tr = mathMax(tr, mathAbs(tail[i].High-prevClose))
			tr = mathMax(tr, mathAbs(tail[i].Low-prevClose))
			sum += tr
		}
		a := sum / float64(len(tail))
		c.ATR14 = fptr(a)
		if last.Close > 0.0 {
			c.AtrPct = fptr(a / last.Close * 100.0)
		}
	}

	// Relative volume: today's traded quantity against the 20-day average.
	if len(bars) >= 21 {
		sum := 0.0
		for _, b := range bars[len(bars)-21 : len(bars)-1] {
			sum += float64(b.Volume)
		}
		avg := sum / 20.0
		if avg > 0.0 {
			today := 0.0
			for _, b := range intraday {
				today += float64(b.Volume)
			}
			if today > 0.0 {
				c.Rvol = fptr(today / avg)
			}
		}
	}

	ret := func(n int) *float64 {
		if len(bars) <= n {
			return nil
		}
		a := bars[len(bars)-1-n].Close
		if a <= 0.0 {
			return nil
		}
		return fptr((last.Close - a) / a * 100.0)
	}
	c.Ret3D = ret(3)
	c.Ret5D = ret(5)
	c.Ret10D = ret(10)

	hi := math.Inf(-1)
	lo := math.Inf(1)
	for _, b := range bars {
		hi = math.Max(hi, b.High)
		lo = math.Min(lo, b.Low)
	}
	if last.Close > 0.0 && hi > 0.0 && lo > 0.0 {
		c.From52wHighPct = fptr((hi - last.Close) / hi * 100.0)
		c.From52wLowPct = fptr((last.Close - lo) / lo * 100.0)
	}
	return c
}

func mathMax(a, b float64) float64 {
	if a > b {
		return a
	}
	return b
}

func mathMin(a, b float64) float64 {
	if a < b {
		return a
	}
	return b
}

func mathAbs(v float64) float64 {
	if v < 0 {
		return -v
	}
	return v
}
