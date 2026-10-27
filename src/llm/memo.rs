use anyhow::Result;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::models::Headline;

/// Order-independent fingerprint of what the model would actually read: title + url.
/// A re-published title under a new url is new information; the same url is not.
pub fn fingerprint(headlines: &[Headline]) -> String {
    let mut parts: Vec<String> = headlines
        .iter()
        .map(|h| format!("{}\u{1f}{}", h.title.trim(), h.url.trim()))
        .collect();
    parts.sort();
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update(b"\n");
    }
    format!("{:x}", h.finalize())
}

/// True when the model has already answered for exactly this headline set.
pub async fn unchanged(pool: &SqlitePool, instrument_id: i64, fp: &str) -> Result<bool> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT headline_fp FROM llm_memos WHERE instrument_id = ?")
            .bind(instrument_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.is_some_and(|r| r.0 == fp))
}

/// How the per-instrument call gate decides.
#[derive(Debug, Clone, Copy)]
pub struct CallGate {
    /// Below this price move (in the instrument's currency), a call is not worth making.
    pub min_move: f64,
    /// ...unless this long has passed since the last one, so nothing sits unwatched.
    pub max_interval_secs: i64,
}

impl Default for CallGate {
    fn default() -> Self {
        Self {
            min_move: 2.0,
            max_interval_secs: 3600,
        }
    }
}

/// True when the model should be consulted for this instrument at this price and time.
pub async fn should_call(
    pool: &SqlitePool,
    instrument_id: i64,
    price: f64,
    now: chrono::DateTime<chrono::Utc>,
    gate: CallGate,
    headlines_changed: bool,
) -> Result<bool> {
    // New information always wins: a fresh headline is exactly when the model is worth asking,
    // even at an unchanged price.
    if headlines_changed {
        return Ok(true);
    }
    let row: Option<(Option<f64>, Option<String>)> = sqlx::query_as(
        "SELECT last_price, last_call_at FROM llm_memos WHERE instrument_id = ?",
    )
    .bind(instrument_id)
    .fetch_optional(pool)
    .await?;

    let Some((last_price, last_call_at)) = row else {
        return Ok(true); // never asked: ask
    };
    let moved = match last_price {
        Some(p) => (price - p).abs() >= gate.min_move,
        None => true,
    };
    if moved {
        return Ok(true);
    }
    let stale = last_call_at
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
        .map(|d| (now - d.with_timezone(&chrono::Utc)).num_seconds() >= gate.max_interval_secs)
        .unwrap_or(true);
    Ok(stale)
}

/// Recorded after every attempt, success or failure: a failing call must not loop every tick.
pub async fn record_call(
    pool: &SqlitePool,
    instrument_id: i64,
    price: f64,
    now: chrono::DateTime<chrono::Utc>,
    headline_fp: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO llm_memos (instrument_id, headline_fp, decided_at, last_price, last_call_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(instrument_id) DO UPDATE SET
             headline_fp = excluded.headline_fp,
             last_price = excluded.last_price, last_call_at = excluded.last_call_at",
    )
    .bind(instrument_id)
    .bind(headline_fp)
    .bind(now.to_rfc3339())
    .bind(price)
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn remember(pool: &SqlitePool, instrument_id: i64, fp: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO llm_memos (instrument_id, headline_fp, decided_at) VALUES (?, ?, ?)
         ON CONFLICT(instrument_id) DO UPDATE SET headline_fp = excluded.headline_fp,
             decided_at = excluded.decided_at",
    )
    .bind(instrument_id)
    .bind(fp)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}
