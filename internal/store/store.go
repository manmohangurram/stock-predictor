// Package store persists bars and headlines shared by the poll loop and by "add instrument".
package store

import (
	"database/sql"
	"log/slog"
	"time"

	"stock-predictor/internal/ids"
	"stock-predictor/internal/market"
	"stock-predictor/internal/models"
)

const upsertBar = `INSERT INTO bars (instrument_id, ts, open, high, low, close, volume, resolution)
	 VALUES (?, ?, ?, ?, ?, ?, ?, ?)
	 ON CONFLICT(instrument_id, ts) DO UPDATE SET
	   open = excluded.open, high = excluded.high, low = excluded.low,
	   close = excluded.close, volume = excluded.volume, resolution = excluded.resolution`

// The folded series a stored fallback needs: the newest daily rows in full, and the buckets that
// carry everything older, oldest first.
//
// A bucket is a bars row with resolution 'bucket': close is the mean of the run, high and low are
// its extremes, and volume is how many sessions went in. One table, one shape per period.
func DailyWithBuckets(pool *sql.DB, instrumentID string, dailyLimit, bucketLimit int) ([]models.Bar, []models.Bucket, error) {
	daily, err := barsAt(pool, instrumentID, "daily", dailyLimit)
	if err != nil {
		return nil, nil, err
	}
	buckets, err := bucketsOf(pool, instrumentID, bucketLimit)
	if err != nil {
		return nil, nil, err
	}
	return daily, buckets, nil
}

func barsAt(pool *sql.DB, instrumentID, resolution string, limit int) ([]models.Bar, error) {
	rows, err := pool.Query(
		`SELECT ts, COALESCE(open, close), COALESCE(high, close), COALESCE(low, close), close,
                COALESCE(volume, 0)
         FROM bars WHERE instrument_id = ? AND resolution = ? ORDER BY ts DESC LIMIT ?`,
		instrumentID, resolution, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	bars := []models.Bar{}
	for rows.Next() {
		var b models.Bar
		if err := rows.Scan(&b.Ts, &b.Open, &b.High, &b.Low, &b.Close, &b.Volume); err != nil {
			return nil, err
		}
		bars = append(bars, b)
	}
	for i, j := 0, len(bars)-1; i < j; i, j = i+1, j-1 {
		bars[i], bars[j] = bars[j], bars[i]
	}
	return bars, rows.Err()
}

func bucketsOf(pool *sql.DB, instrumentID string, limit int) ([]models.Bucket, error) {
	rows, err := pool.Query(
		`SELECT ts, close, COALESCE(high, close), COALESCE(low, close), COALESCE(volume, 0)
         FROM bars WHERE instrument_id = ? AND resolution = 'bucket' ORDER BY ts ASC LIMIT ?`,
		instrumentID, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []models.Bucket{}
	for rows.Next() {
		var b models.Bucket
		var sessions float64
		if err := rows.Scan(&b.EndsOn, &b.Mean, &b.High, &b.Low, &sessions); err != nil {
			return nil, err
		}
		b.Sessions = int(sessions)
		out = append(out, b)
	}
	return out, rows.Err()
}

const insertHeadline = `INSERT OR IGNORE INTO news (id, instrument_id, published_at, title, publisher, url)
	 VALUES (?, ?, ?, ?, ?, ?)`

// Daily bars, upserted. Shared by the poll loop and by "add instrument", so a freshly
// added symbol has a chart immediately instead of waiting for the next tick.
//
// One transaction per batch, and only the tail is written: rows at or after the newest
// stored ts, which keeps a poll's write at a fraction of a millisecond.
func Bars(pool *sql.DB, instrumentID string, bars []models.Bar) error {
	return BarsAt(pool, instrumentID, "daily", bars)
}

// BarsAt is Bars with the resolution written explicitly, so a row says which series it belongs to
// without a second table.
func BarsAt(pool *sql.DB, instrumentID, resolution string, bars []models.Bar) error {
	var last string
	err := pool.QueryRow(
		"SELECT ts FROM bars WHERE instrument_id = ? AND resolution = ? ORDER BY ts DESC LIMIT 1",
		instrumentID, resolution,
	).Scan(&last)
	if err != nil && err != sql.ErrNoRows {
		return err
	}
	var fresh []models.Bar
	for _, b := range bars {
		if b.Ts >= last {
			fresh = append(fresh, b)
		}
	}
	if len(fresh) == 0 {
		slog.Debug("bars unchanged", "instrument", instrumentID, "received", len(bars))
		return nil
	}
	slog.Debug("bars writing", "instrument", instrumentID, "fresh", len(fresh), "received", len(bars))
	tx, err := pool.Begin()
	if err != nil {
		return err
	}
	defer tx.Rollback()
	for _, b := range fresh {
		if _, err := tx.Exec(upsertBar,
			instrumentID, b.Ts, b.Open, b.High, b.Low, b.Close, b.Volume, resolution,
		); err != nil {
			return err
		}
	}
	if err := tx.Commit(); err != nil {
		return err
	}
	// A daily write is the only moment the series grows, so it is the only place the fold has to
	// run: the dense tail is trimmed and everything older becomes a bucket.
	if resolution == "daily" {
		if _, _, err := FoldBars(pool, instrumentID); err != nil {
			return err
		}
	}
	return nil
}

// FoldBars keeps the newest models.DailyFloor daily rows and folds everything older into buckets of
// models.BucketSessions, so the table holds one shape per period. Idempotent: a second run finds
// nothing above the floor and writes nothing.
func FoldBars(pool *sql.DB, instrumentID string) (kept, folded int, err error) {
	all, err := barsAt(pool, instrumentID, "daily", 1_000_000)
	if err != nil {
		return 0, 0, err
	}
	if len(all) <= models.DailyFloor {
		return len(all), 0, nil
	}
	keepFrom := len(all) - models.DailyFloor
	older := all[:keepFrom]
	cut := all[keepFrom].Ts

	// Groups are taken from the newest end of the folded run, so the boundary next to the daily rows
	// is aligned; the remainder at the oldest end is one short bucket, far outside the windows.
	buckets := []models.Bucket{}
	for end := len(older); end > 0; {
		start := end - models.BucketSessions
		if start < 0 {
			start = 0
		}
		buckets = append(buckets, bucketOf(older[start:end]))
		end = start
	}

	tx, err := pool.Begin()
	if err != nil {
		return 0, 0, err
	}
	defer tx.Rollback()
	if _, err := tx.Exec(
		"DELETE FROM bars WHERE instrument_id = ? AND resolution = 'daily' AND ts < ?",
		instrumentID, cut,
	); err != nil {
		return 0, 0, err
	}
	for _, b := range buckets {
		if _, err := tx.Exec(upsertBar,
			instrumentID, b.EndsOn, b.Mean, b.High, b.Low, b.Mean, float64(b.Sessions), "bucket",
		); err != nil {
			return 0, 0, err
		}
	}
	if err := pruneBuckets(tx, instrumentID); err != nil {
		return 0, 0, err
	}
	if err := tx.Commit(); err != nil {
		return 0, 0, err
	}
	slog.Debug("bars folded", "instrument", instrumentID, "kept", models.DailyFloor,
		"buckets", len(buckets))
	return models.DailyFloor, len(older), nil
}

// FoldAll runs FoldBars over every instrument that has a series, so a database written before the
// tiers existed is converted in place: nothing is kept twice.
func FoldAll(pool *sql.DB) (int, error) {
	rows, err := pool.Query("SELECT DISTINCT instrument_id FROM bars WHERE resolution = 'daily'")
	if err != nil {
		return 0, err
	}
	ids := []string{}
	for rows.Next() {
		var id string
		if err := rows.Scan(&id); err != nil {
			rows.Close()
			return 0, err
		}
		ids = append(ids, id)
	}
	rows.Close()
	if err := rows.Err(); err != nil {
		return 0, err
	}
	folded := 0
	for _, id := range ids {
		if _, _, err := FoldBars(pool, id); err != nil {
			return folded, err
		}
		folded++
	}
	return folded, nil
}

func bucketOf(run []models.Bar) models.Bucket {
	mean, high, low := 0.0, 0.0, 0.0
	for i, b := range run {
		mean += b.Close
		if i == 0 || b.High > high {
			high = b.High
		}
		if i == 0 || b.Low < low {
			low = b.Low
		}
	}
	return models.Bucket{
		EndsOn: run[len(run)-1].Ts, Sessions: len(run),
		Mean: mean / float64(len(run)), High: high, Low: low,
	}
}

func pruneBuckets(tx *sql.Tx, instrumentID string) error {
	var cut string
	err := tx.QueryRow(
		`SELECT ts FROM bars WHERE instrument_id = ? AND resolution = 'bucket'
         ORDER BY ts DESC LIMIT 1 OFFSET ?`, instrumentID, models.BucketKeep-1,
	).Scan(&cut)
	if err == sql.ErrNoRows {
		return nil
	}
	if err != nil {
		return err
	}
	_, err = tx.Exec(
		"DELETE FROM bars WHERE instrument_id = ? AND resolution = 'bucket' AND ts < ?",
		instrumentID, cut,
	)
	return err
}

// Headlines older than days. The prompt reads a recent window; the rest is dead weight.
func PruneNews(pool *sql.DB, days int64) (int64, error) {
	cutoff := time.Now().UTC().AddDate(0, 0, -int(days)).Format("2006-01-02T15:04:05.000000000+00:00")
	res, err := pool.Exec("DELETE FROM news WHERE published_at < ?", cutoff)
	if err != nil {
		return 0, err
	}
	return res.RowsAffected()
}

// Headlines are stored once per url. The body is never fetched here — only on escalation.
func Headlines(pool *sql.DB, instrumentID string, news []models.Headline) error {
	tx, err := pool.Begin()
	if err != nil {
		return err
	}
	defer tx.Rollback()
	for _, h := range news {
		if _, err := tx.Exec(insertHeadline,
			ids.New(), instrumentID, h.PublishedAt, h.Title, h.Publisher, h.URL,
		); err != nil {
			return err
		}
	}
	return tx.Commit()
}

// One round trip for a newly added symbol: daily prices and headlines, both persisted.
// A failure here must not fail the add — the instrument exists either way.
func Refresh(pool *sql.DB, instrumentID string, symbol string, market market.MarketSource) (int, int, error) {
	daily, err := market.Bars(symbol)
	if err != nil {
		return 0, 0, err
	}
	if err := Bars(pool, instrumentID, daily); err != nil {
		return 0, 0, err
	}
	news, err := market.News(symbol)
	if err != nil {
		news = nil
	}
	if err := Headlines(pool, instrumentID, news); err != nil {
		return 0, 0, err
	}
	return len(daily), len(news), nil
}
