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
