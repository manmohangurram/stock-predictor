package store

import (
	"database/sql"
	"math"
	"path/filepath"
	"testing"
	"time"

	"stock-predictor/internal/db"
	"stock-predictor/internal/models"
	"stock-predictor/internal/rules"
)

// A database written before the tiers existed is converted in place: 70 daily rows stay, everything
// older becomes a bucket, and the indicators do not move beyond the smoothing residue.
func TestFoldingKeepsTheIndicatorsAndOneShape(t *testing.T) {
	pool, err := db.ConnectAndMigrate(t.Context(), filepath.Join(t.TempDir(), "t.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { pool.Close() })

	id := "01a1f000-0000-7000-8000-0000000000f9"
	if _, err := pool.Exec(
		`INSERT INTO instruments (id, symbol, class, name, currency, active)
         VALUES (?, 'TIER.NS', 'stock', 'Tier Test', 'INR', 1)`, id); err != nil {
		t.Fatal(err)
	}
	dense := 300
	base := time.Date(2025, 9, 1, 0, 0, 0, 0, time.UTC)
	for i := 0; i < dense; i++ {
		close := 100 + float64(i)*0.37 + 3*math.Sin(float64(i)/4)
		if _, err := pool.Exec(
			`INSERT INTO bars (instrument_id, ts, open, high, low, close, volume, resolution)
             VALUES (?, ?, ?, ?, ?, ?, 0, 'daily')`,
			id, base.AddDate(0, 0, i).Format("2006-01-02T15:04:05.000000000+00:00"),
			close, close+1.5, close-1.5, close); err != nil {
			t.Fatal(err)
		}
	}
	want, err := rules.ComputeIndicators(loadDaily(pool, t, id))
	if err != nil {
		t.Fatal(err)
	}

	kept, folded, err := FoldBars(pool, id)
	if err != nil {
		t.Fatal(err)
	}
	if kept != models.DailyFloor || folded != dense-models.DailyFloor {
		t.Fatalf("kept %d folded %d", kept, folded)
	}

	daily, buckets, err := DailyWithBuckets(pool, id, models.DailyFloor, models.BucketKeep)
	if err != nil {
		t.Fatal(err)
	}
	if len(daily) != models.DailyFloor {
		t.Fatalf("daily rows left: %d", len(daily))
	}
	if len(buckets) == 0 {
		t.Fatal("no buckets written")
	}
	if len(buckets) > models.BucketKeep {
		t.Fatalf("buckets kept: %d", len(buckets))
	}

	got, err := rules.ComputeIndicatorsFrom(daily, buckets)
	if err != nil {
		t.Fatal(err)
	}
	if math.Abs(got.SMA200-want.SMA200) > 1e-9 {
		t.Fatalf("200-day mean moved: %.9f vs %.9f", got.SMA200, want.SMA200)
	}
	if got.High52 != want.High52 || got.Low52 != want.Low52 {
		t.Fatalf("52-week range moved: %.2f/%.2f vs %.2f/%.2f",
			got.High52, got.Low52, want.High52, want.Low52)
	}

	// One shape per period, and a second pass writes nothing.
	var dailyRows, bucketRows int
	if err := pool.QueryRow(
		`SELECT (SELECT COUNT(*) FROM bars WHERE instrument_id = ? AND resolution = 'daily'),
                (SELECT COUNT(*) FROM bars WHERE instrument_id = ? AND resolution = 'bucket')`,
		id, id).Scan(&dailyRows, &bucketRows); err != nil {
		t.Fatal(err)
	}
	if dailyRows != models.DailyFloor {
		t.Fatalf("daily rows in the table: %d", dailyRows)
	}
	if again, foldedAgain, err := FoldBars(pool, id); err != nil || foldedAgain != 0 ||
		again != models.DailyFloor {
		t.Fatalf("a second fold must change nothing: kept %d folded %d err %v", again, foldedAgain, err)
	}
}

func loadDaily(pool *sql.DB, t *testing.T, id string) []models.Bar {
	t.Helper()
	rows, err := pool.Query(
		`SELECT ts, open, high, low, close, volume FROM bars
         WHERE instrument_id = ? AND resolution = 'daily' ORDER BY ts ASC`, id)
	if err != nil {
		t.Fatal(err)
	}
	defer rows.Close()
	out := []models.Bar{}
	for rows.Next() {
		var b models.Bar
		if err := rows.Scan(&b.Ts, &b.Open, &b.High, &b.Low, &b.Close, &b.Volume); err != nil {
			t.Fatal(err)
		}
		out = append(out, b)
	}
	return out
}
