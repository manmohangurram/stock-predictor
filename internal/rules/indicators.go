package rules

import (
	"fmt"
	"math"

	"stock-predictor/internal/models"
)

// Half-width of the validator's allowed price band, in ATRs. Defined once and used by both
// ATRBands and the LLM proposal validator so the two can never drift apart.
const BandMult = 1.0

const MinBars = 200

// The series that is kept in full, and the fold that carries everything older.
//
// 200-70 = 10 x 13 and 252-70 = 14 x 13: with these two numbers both long windows end exactly on a
// bucket boundary, so a folded series gives the same 200-day mean and the same 52-week range as the
// dense one, instead of a mean that includes part of a bucket it cannot see inside.
const (
	DailyFloor     = 70
	BucketSessions = 13
	BucketKeep     = 20
)

type Indicators struct {
	SMA50  float64 `json:"sma50"`
	SMA200 float64 `json:"sma200"`
	RSI14  float64 `json:"rsi14"`
	ATR14  float64 `json:"atr14"`
	High52 float64 `json:"high52"`
	Low52  float64 `json:"low52"`
}

type Bands struct {
	Min float64
	Max float64
}

type IndicatorError struct {
	Msg string
}

func (e IndicatorError) Error() string { return e.Msg }

// Every output is finite or the call errors. A NaN signal is worse than no signal.
func ComputeIndicators(bars []models.Bar) (Indicators, error) {
	if len(bars) < MinBars {
		return Indicators{}, IndicatorError{Msg: fmt.Sprintf("insufficient bars: need %d, got %d", MinBars, len(bars))}
	}
	ind := Indicators{
		SMA50:  meanCloses(bars, len(bars)-50, len(bars)),
		SMA200: meanCloses(bars, len(bars)-200, len(bars)),
		RSI14:  rsi(bars, 14),
		ATR14:  atr(bars, 14),
	}
	ind.High52 = math.Inf(-1)
	ind.Low52 = math.Inf(1)
	for i := max(0, len(bars)-252); i < len(bars); i++ {
		if bars[i].High > ind.High52 {
			ind.High52 = bars[i].High
		}
		if bars[i].Low < ind.Low52 {
			ind.Low52 = bars[i].Low
		}
	}

	if !isFinite(ind.SMA50) || !isFinite(ind.SMA200) || !isFinite(ind.RSI14) ||
		!isFinite(ind.ATR14) || !isFinite(ind.High52) || !isFinite(ind.Low52) {
		return Indicators{}, IndicatorError{Msg: "non-finite indicator value — bad input series"}
	}
	return ind, nil
}

// ComputeIndicatorsFrom is ComputeIndicators over the folded series: the newest daily rows, then
// buckets for everything older. The short indicators can only come from daily rows — RSI and ATR
// need individual sessions, not means — while the 200-day mean and the 52-week range walk both.
func ComputeIndicatorsFrom(daily []models.Bar, buckets []models.Bucket) (Indicators, error) {
	if len(daily) < 50 {
		return Indicators{}, IndicatorError{Msg: fmt.Sprintf("insufficient daily bars: need 50, got %d", len(daily))}
	}
	ind := Indicators{
		SMA50: meanCloses(daily, len(daily)-50, len(daily)),
		RSI14: rsi(daily, 14),
		ATR14: atr(daily, 14),
	}
	sma200, ok := windowMean(daily, buckets, 200)
	if !ok {
		return Indicators{}, IndicatorError{Msg: "insufficient history for the 200-day mean"}
	}
	ind.SMA200 = sma200
	high, low, ok := windowRange(daily, buckets, 252)
	if !ok {
		return Indicators{}, IndicatorError{Msg: "insufficient history for the 52-week range"}
	}
	ind.High52, ind.Low52 = high, low
	if !isFinite(ind.SMA50) || !isFinite(ind.SMA200) || !isFinite(ind.RSI14) ||
		!isFinite(ind.ATR14) || !isFinite(ind.High52) || !isFinite(ind.Low52) {
		return Indicators{}, IndicatorError{Msg: "non-finite indicator value — bad input series"}
	}
	return ind, nil
}

// The newest sessions, newest first: daily rows one at a time, then the buckets, oldest bucket last.
type sessionRun struct {
	sessions int
	mean     float64
	high     float64
	low      float64
}

func newestFirst(daily []models.Bar, buckets []models.Bucket) []sessionRun {
	out := make([]sessionRun, 0, len(daily)+len(buckets))
	for i := len(daily) - 1; i >= 0; i-- {
		out = append(out, sessionRun{1, daily[i].Close, daily[i].High, daily[i].Low})
	}
	for i := len(buckets) - 1; i >= 0; i-- {
		out = append(out, sessionRun{buckets[i].Sessions, buckets[i].Mean, buckets[i].High, buckets[i].Low})
	}
	return out
}

// The mean of the newest window sessions. A window that ends inside a bucket uses the bucket's mean
// for the sessions it takes — the fold cannot see inside — which is why the sizes are chosen so that
// a full series never lands there.
func windowMean(daily []models.Bar, buckets []models.Bucket, window int) (float64, bool) {
	sum, count := 0.0, 0
	for _, r := range newestFirst(daily, buckets) {
		if count >= window {
			break
		}
		take := r.sessions
		if count+take > window {
			take = window - count
		}
		sum += r.mean * float64(take)
		count += take
	}
	if count < window {
		return 0, false
	}
	return sum / float64(count), true
}

// The highest high and lowest low over the newest window sessions. A window that starts inside a
// bucket stops at the last whole run rather than counting a high from a session outside the window,
// so the range can be short but never overstated.
func windowRange(daily []models.Bar, buckets []models.Bucket, window int) (float64, float64, bool) {
	high, low, count := math.Inf(-1), math.Inf(1), 0
	for _, r := range newestFirst(daily, buckets) {
		if count >= window {
			break
		}
		if r.sessions > window-count {
			break
		}
		if r.high > high {
			high = r.high
		}
		if r.low < low {
			low = r.low
		}
		count += r.sessions
	}
	if count == 0 {
		return 0, 0, false
	}
	return high, low, true
}

func ATRBands(lastClose, atr14 float64) Bands {
	span := atr14 * BandMult
	return Bands{Min: lastClose - span, Max: lastClose + span}
}

func isFinite(v float64) bool {
	return !math.IsNaN(v) && !math.IsInf(v, 0)
}

func mean(xs []float64) float64 {
	if len(xs) == 0 {
		return math.NaN()
	}
	s := 0.0
	for _, x := range xs {
		s += x
	}
	return s / float64(len(xs))
}

func meanCloses(bars []models.Bar, from, to int) float64 {
	if to <= from {
		return math.NaN()
	}
	s := 0.0
	for i := from; i < to; i++ {
		s += bars[i].Close
	}
	return s / float64(to-from)
}

func rsi(bars []models.Bar, period int) float64 {
	if len(bars) <= period {
		return math.NaN()
	}
	gains, losses := 0.0, 0.0
	for i := 1; i <= period; i++ {
		d := bars[i].Close - bars[i-1].Close
		if d >= 0.0 {
			gains += d
		} else {
			losses -= d
		}
	}
	ag, al := gains/float64(period), losses/float64(period)

	for i := period + 1; i < len(bars); i++ {
		d := bars[i].Close - bars[i-1].Close
		g, l := 0.0, 0.0
		if d >= 0.0 {
			g = d
		} else {
			l = -d
		}
		ag = (ag*float64(period-1) + g) / float64(period)
		al = (al*float64(period-1) + l) / float64(period)
	}
	if al == 0.0 {
		return 100.0
	}
	rs := ag / al
	return 100.0 - 100.0/(1.0+rs)
}

func atr(bars []models.Bar, period int) float64 {
	if len(bars) <= period {
		return math.NaN()
	}
	fp := float64(period)
	fpm1 := fp - 1.0
	sum := 0.0
	for i := 1; i <= period; i++ {
		b := &bars[i]
		prev := bars[i-1].Close
		hl := b.High - b.Low
		hc := b.High - prev
		if hc < 0 {
			hc = -hc
		}
		lc := b.Low - prev
		if lc < 0 {
			lc = -lc
		}
		tr := hl
		if hc > tr {
			tr = hc
		}
		if lc > tr {
			tr = lc
		}
		sum += tr
	}
	a := sum / fp
	for i := period + 1; i < len(bars); i++ {
		b := &bars[i]
		prev := bars[i-1].Close
		hl := b.High - b.Low
		hc := b.High - prev
		if hc < 0 {
			hc = -hc
		}
		lc := b.Low - prev
		if lc < 0 {
			lc = -lc
		}
		tr := hl
		if hc > tr {
			tr = hc
		}
		if lc > tr {
			tr = lc
		}
		a = (a*fpm1 + tr) / fp
	}
	return a
}
