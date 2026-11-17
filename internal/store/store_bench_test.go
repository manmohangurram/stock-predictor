package store

import (
	"database/sql"
	"path/filepath"
	"testing"
	"time"

	"stock-predictor/internal/db"
	"stock-predictor/internal/models"
)

func benchPool(b *testing.B) (*sql.DB, int64) {
	b.Helper()
	pool, err := db.ConnectAndMigrate(b.Context(), filepath.Join(b.TempDir(), "bench.db"))
	if err != nil {
		b.Fatal(err)
	}
	res, err := pool.Exec(
		"INSERT INTO instruments (symbol, class, name, currency, active) VALUES ('B.NS', 'stock', 'B', 'INR', 1)")
	if err != nil {
		b.Fatal(err)
	}
	id, _ := res.LastInsertId()
	b.Cleanup(func() { pool.Close() })
	return pool, id
}

func benchBars(n int) []models.Bar {
	bars := make([]models.Bar, n)
	start := time.Date(2025, 11, 17, 0, 0, 0, 0, time.UTC)
	for i := range bars {
		c := 100.0 + float64(i)*0.1
		bars[i] = models.Bar{
			Ts:   start.AddDate(0, 0, i).Format("2006-01-02") + "T00:00:00.000000000+00:00",
			Open: c, High: c + 1, Low: c - 1, Close: c, Volume: 1000,
		}
	}
	return bars
}

func BenchmarkBarUpsertCold(b *testing.B) {
	pool, id := benchPool(b)
	bars := benchBars(250)
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		b.StopTimer()
		if _, err := pool.Exec("DELETE FROM bars WHERE instrument_id = ?", id); err != nil {
			b.Fatal(err)
		}
		b.StartTimer()
		if err := Bars(pool, id, bars); err != nil {
			b.Fatal(err)
		}
	}
}

func BenchmarkBarUpsertTail(b *testing.B) {
	pool, id := benchPool(b)
	if err := Bars(pool, id, benchBars(250)); err != nil {
		b.Fatal(err)
	}
	tail := benchBars(251)
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		if err := Bars(pool, id, tail); err != nil {
			b.Fatal(err)
		}
	}
}
