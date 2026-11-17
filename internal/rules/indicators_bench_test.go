package rules

import (
	"testing"

	"stock-predictor/internal/models"
)

func BenchmarkIndicators250(b *testing.B) {
	bars := make([]models.Bar, 250)
	for i := range bars {
		c := 100.0 + float64(i)*0.37
		bars[i] = models.Bar{
			Ts: "2026-11-14T00:00:00Z", Open: c - 0.1, High: c + 0.6, Low: c - 0.6, Close: c, Volume: 1_000_000,
		}
	}
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		if _, err := ComputeIndicators(bars); err != nil {
			b.Fatal(err)
		}
	}
}
