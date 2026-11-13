use anyhow::Result;
use serde::Serialize;
use sqlx::SqlitePool;

/// One row per outbound DeepSeek call, success or failure.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ApiCallRow {
    pub conversation_id: Option<String>,
    pub round: Option<i64>,
    pub endpoint: String,
    /// Why this call happened: "price moved 1,150.20 → 1,171.60", "new headline: …".
    pub trigger: Option<String>,
    pub purpose: Option<String>,
    pub prompt_template: Option<String>,
    pub prompt_hash: Option<String>,
    pub request_json: Option<String>,
    pub response_json: Option<String>,
    pub http_status: Option<i64>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub latency_ms: Option<i64>,
    pub validator_verdict: Option<String>,
    pub error: Option<String>,
}

pub async fn record(pool: &SqlitePool, row: ApiCallRow) -> Result<()> {
    sqlx::query(
        "INSERT INTO api_calls (ts, conversation_id, round, endpoint, purpose, prompt_template,
            prompt_hash, request_json, response_json, http_status, prompt_tokens,
            completion_tokens, latency_ms, validator_verdict, error, trigger)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(row.conversation_id)
    .bind(row.round)
    .bind(row.endpoint)
    .bind(row.purpose)
    .bind(row.prompt_template)
    .bind(row.prompt_hash)
    .bind(row.request_json)
    .bind(row.response_json)
    .bind(row.http_status)
    .bind(row.prompt_tokens)
    .bind(row.completion_tokens)
    .bind(row.latency_ms)
    .bind(row.validator_verdict)
    .bind(row.error)
    .bind(row.trigger)
    .execute(pool)
    .await?;
    Ok(())
}

/// Delete calls older than `days`. The audit keeps three months; older rows are the bulk of the
/// file and nothing reads them.
pub async fn prune(pool: &SqlitePool, days: i64) -> Result<u64> {
    let cutoff = (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339();
    let done = sqlx::query("DELETE FROM api_calls WHERE ts < ?")
        .bind(cutoff)
        .execute(pool)
        .await?;
    Ok(done.rows_affected())
}
