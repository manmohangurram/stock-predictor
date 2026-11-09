//! Runs: what the scheduler (or a person) started, and how it ended.
//!
//! A run is opened *before* the work and closed after it, so the audit page can answer "is it
//! running?" — something `api_calls` cannot, because a call row only exists once the reply lands.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::SqlitePool;

/// What was run. The string is stored, so a new kind needs no migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prompt {
    /// The per-instrument decisions, plus the stock plan when one fires.
    Poll,
    PlanStock,
    PlanFund,
    /// The short-term pass over NSE gainers.
    Short,
    /// One instrument, asked on its own.
    Decide,
    /// A buy that did not fit the cash, asking what to sell.
    Rotate,
}

impl Prompt {
    pub fn as_str(self) -> &'static str {
        match self {
            Prompt::Poll => "poll",
            Prompt::PlanStock => "plan_stock",
            Prompt::PlanFund => "plan_fund",
            Prompt::Short => "short",
            Prompt::Decide => "decide",
            Prompt::Rotate => "rotate",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "poll" => Prompt::Poll,
            "plan_stock" => Prompt::PlanStock,
            "plan_fund" => Prompt::PlanFund,
            "short" => Prompt::Short,
            "decide" => Prompt::Decide,
            "rotate" => Prompt::Rotate,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Schedule,
    Change,
    Manual,
}

impl Trigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Trigger::Schedule => "schedule",
            Trigger::Change => "change",
            Trigger::Manual => "manual",
        }
    }
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Run {
    pub id: i64,
    pub prompt: String,
    pub trigger: String,
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub summary: Option<String>,
    pub conversation_id: Option<String>,
    pub error: Option<String>,
}

/// Open a run. It exists while the work is still going, which is the whole point.
pub async fn start(
    pool: &SqlitePool,
    prompt: Prompt,
    trigger: Trigger,
    symbol: Option<&str>,
) -> Result<i64> {
    let summary = symbol.map(|s| format!("{s} only"));
    let id: (i64,) = sqlx::query_as(
        "INSERT INTO runs (prompt, trigger, status, started_at, summary)
         VALUES (?, ?, 'running', ?, ?) RETURNING id",
    )
    .bind(prompt.as_str())
    .bind(trigger.as_str())
    .bind(Utc::now().to_rfc3339())
    .bind(summary)
    .fetch_one(pool)
    .await?;
    Ok(id.0)
}

/// Close a run with one line a person can read.
pub async fn finish(
    pool: &SqlitePool,
    id: i64,
    summary: &str,
    conversation_id: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "UPDATE runs SET status = 'done', finished_at = ?, summary = ?,
            conversation_id = COALESCE(?, conversation_id)
         WHERE id = ?",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(summary)
    .bind(conversation_id)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Close a run that broke. The error is kept: a run that failed is still a thing that happened.
pub async fn fail(pool: &SqlitePool, id: i64, error: &str) -> Result<()> {
    sqlx::query("UPDATE runs SET status = 'failed', finished_at = ?, error = ? WHERE id = ?")
        .bind(Utc::now().to_rfc3339())
        .bind(error)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// A run still marked running when the process died is not running any more. Called at boot so
/// the page never shows a spinner that will not stop.
pub async fn close_orphans(pool: &SqlitePool, now: DateTime<Utc>) -> Result<u64> {
    let rows = sqlx::query(
        "UPDATE runs SET status = 'failed', finished_at = ?,
            error = COALESCE(error, 'the process stopped before this finished')
         WHERE status = 'running'",
    )
    .bind(now.to_rfc3339())
    .execute(pool)
    .await?
    .rows_affected();
    Ok(rows)
}

pub async fn recent(pool: &SqlitePool, limit: i64) -> Result<Vec<Run>> {
    let rows: Vec<Run> = sqlx::query_as(
        "SELECT id, prompt, trigger, status, started_at, finished_at, summary,
                conversation_id, error
         FROM runs ORDER BY id DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn running(pool: &SqlitePool) -> Result<i64> {
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM runs WHERE status = 'running'")
        .fetch_one(pool)
        .await?;
    Ok(count.0)
}

/// A request to run something now, sent from the API to the loop that owns the clients.
#[derive(Debug, Clone)]
pub enum Manual {
    Poll,
    PlanStock,
    PlanFund,
    Short,
    /// One instrument, asked on its own — the precise version of a poll.
    Decide {
        symbol: String,
    },
}
