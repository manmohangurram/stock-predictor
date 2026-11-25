package rules

import (
	"math"
	"testing"

	"stock-predictor/internal/models"
)

// A folded series must give the same numbers as the dense one it replaces. The sizes are chosen so
// both long windows end on a bucket boundary: 200-70 = 10 x 13 and 252-70 = 14 x 13.
func TestAFoldedSeriesGivesTheSameIndicators(t *testing.T) {
	dense := seriesOf(300)
	daily := dense[230:]               // the newest 70 sessions, in full
	buckets := fold(dense[48:230], 13) // 182 sessions = 14 buckets, oldest first

	want, err := ComputeIndicators(dense)
	if err != nil {
		t.Fatal(err)
	}
	got, err := ComputeIndicatorsFrom(daily, buckets)
	if err != nil {
		t.Fatal(err)
	}
	if got.SMA50 != want.SMA50 {
		t.Fatalf("SMA50 comes from the daily rows and must be identical: %.9f vs %.9f", got.SMA50, want.SMA50)
	}
	// RSI and ATR are Wilder-smoothed from the start of the series, so a shorter past leaves a
	// residue: after 70 sessions the seed's weight is (13/14)^70, under 0.6%. The folded series is
	// never allowed to drift further than that.
	if relDiff(got.RSI14, want.RSI14) > 0.01 || relDiff(got.ATR14, want.ATR14) > 0.01 {
		t.Fatalf("RSI14/ATR14 drifted: %.4f/%.4f vs %.4f/%.4f",
			got.RSI14, got.ATR14, want.RSI14, want.ATR14)
	}
	if math.Abs(got.SMA200-want.SMA200) > 1e-9 {
		t.Fatalf("200-day mean: folded %.9f, dense %.9f", got.SMA200, want.SMA200)
	}
	if got.High52 != want.High52 || got.Low52 != want.Low52 {
		t.Fatalf("52-week range: folded %.2f/%.2f, dense %.2f/%.2f",
			got.High52, got.Low52, want.High52, want.Low52)
	}
}

func TestAFoldedSeriesTooShortSaysSoRatherThanGuessing(t *testing.T) {
	dense := seriesOf(300)
	daily := dense[230:]
	if _, err := ComputeIndicatorsFrom(daily, nil); err == nil {
		t.Fatal("70 daily sessions and no buckets cannot carry a 200-day mean")
	}
	if _, err := ComputeIndicatorsFrom(dense[290:], nil); err == nil {
		t.Fatal("ten daily rows are not a series")
	}
}

func relDiff(a, b float64) float64 {
	if b == 0 {
		return math.Abs(a)
	}
	return math.Abs(a-b) / math.Abs(b)
}

// A deterministic series with a rising close, a two-point range and volume.
func seriesOf(n int) []models.Bar {
	out := make([]models.Bar, n)
	for i := 0; i < n; i++ {
		close := 100 + float64(i)*0.37 + 3*math.Sin(float64(i)/4)
		out[i] = models.Bar{
			Ts:    "2026-01-01T00:00:00+00:00",
			Open:  close,
			High:  close + 1.5,
			Low:   close - 1.5,
			Close: close,
		}
	}
	return out
}

// fold collapses consecutive runs of sessions into buckets, oldest first.
func fold(bars []models.Bar, size int) []models.Bucket {
	out := []models.Bucket{}
	for start := 0; start < len(bars); start += size {
		end := start + size
		if end > len(bars) {
			end = len(bars)
		}
		run := bars[start:end]
		mean := 0.0
		high, low := math.Inf(-1), math.Inf(1)
		for _, b := range run {
			mean += b.Close
			if b.High > high {
				high = b.High
			}
			if b.Low < low {
				low = b.Low
			}
		}
		out = append(out, models.Bucket{
			EndsOn: run[len(run)-1].Ts, Sessions: len(run),
			Mean: mean / float64(len(run)), High: high, Low: low,
		})
	}
	return out
}
