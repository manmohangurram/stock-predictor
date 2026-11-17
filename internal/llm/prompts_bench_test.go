package llm

import (
	"testing"
	"time"

	"stock-predictor/internal/models"
	"stock-predictor/internal/rules"
)

func BenchmarkPromptRender(b *testing.B) {
	ind := &rules.Indicators{SMA50: 1500, SMA200: 1450, RSI14: 57, ATR14: 22, High52: 1700, Low52: 1200}
	headlines := make([]models.Headline, 8)
	for i := range headlines {
		headlines[i] = models.Headline{
			ID: int64(i + 1), Title: "Infosys raises guidance after a strong quarter",
			Publisher: "Reuters", PublishedAt: "2026-11-16T10:00:00.000000000+00:00",
			URL: "https://example.in/article",
		}
	}
	ctx := &PromptContext{
		Instrument: &models.Instrument{Symbol: "INFY.NS", Class: models.ClassStock, Currency: "INR"},
		Ind:        ind, LastClose: 1600, CashAvailable: 50_000,
		Limits: "trade size 500 to 5000 per fill", Headlines: headlines,
		WindowHours: 4, PriceWindow: "(tape)", DailyDays: 10, DailySummary: "2026-11-16 1600.00",
		Fundamentals: "FY2026: revenue 2.0T INR", ShortContext: "Session: 120 min", Positions: "(book)",
		RecentTrades: "none", LastDecision: "none", Now: time.Now().UTC(),
	}
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		if _, err := Render("decide_v1", ctx); err != nil {
			b.Fatal(err)
		}
	}
}
