use anyhow::Result;
use sqlx::SqlitePool;

use crate::market::MarketSource;
use crate::models::{Bar, Headline};

/// Daily bars, upserted. Shared by the poll loop and by "add instrument", so a freshly added
/// symbol has a chart immediately instead of waiting for the next tick.
pub async fn bars(pool: &SqlitePool, instrument_id: i64, bars: &[Bar]) -> Result<()> {
    for b in bars {
        sqlx::query(
            "INSERT INTO bars (instrument_id, ts, open, high, low, close, volume)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(instrument_id, ts) DO UPDATE SET
               open = excluded.open, high = excluded.high, low = excluded.low,
               close = excluded.close, volume = excluded.volume",
        )
        .bind(instrument_id)
        .bind(&b.ts)
        .bind(b.open)
        .bind(b.high)
        .bind(b.low)
        .bind(b.close)
        .bind(b.volume)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Headlines are stored once per url. The body is never fetched here — only on escalation.
pub async fn headlines(pool: &SqlitePool, instrument_id: i64, news: &[Headline]) -> Result<()> {
    for h in news {
        sqlx::query(
            "INSERT OR IGNORE INTO news (instrument_id, published_at, title, publisher, url)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(instrument_id)
        .bind(&h.published_at)
        .bind(&h.title)
        .bind(&h.publisher)
        .bind(&h.url)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// One round trip for a newly added symbol: daily prices and headlines, both persisted.
/// A failure here must not fail the add — the instrument exists either way.
pub async fn refresh(
    pool: &SqlitePool,
    instrument_id: i64,
    symbol: &str,
    market: &dyn MarketSource,
) -> Result<(usize, usize)> {
    let daily = market.bars(symbol).await?;
    bars(pool, instrument_id, &daily).await?;
    let news = market.news(symbol).await.unwrap_or_default();
    headlines(pool, instrument_id, &news).await?;
    Ok((daily.len(), news.len()))
}
