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
	return tx.Commit()
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
