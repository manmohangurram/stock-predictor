use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::header::{ETAG, IF_NONE_MATCH};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::config::Config;
use crate::models::AssetClass;
use crate::models::Bar;
use crate::rules::budget::{self, CashKind, NewCashEntry};
use crate::rules::evaluate::{self, CandidateInput, RuleConfig};
use crate::rules::indicators;
use crate::rules::settings::{self, SettingsError, SettingsPatch};

type AmfiCache = Arc<tokio::sync::Mutex<Option<(chrono::DateTime<chrono::Utc>, String)>>>;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub cfg: Config,
    /// AMFI's scheme file is ~3 MB; it is fetched at most every few hours, not per keystroke.
    amfi: AmfiCache,
}

impl AppState {
    pub fn new(pool: SqlitePool, cfg: Config) -> Self {
        Self {
            pool,
            cfg,
            amfi: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    async fn amfi_text(&self) -> Option<String> {
        let mut guard = self.amfi.lock().await;
        if let Some((at, text)) = guard.as_ref() {
            if (chrono::Utc::now() - *at) < chrono::Duration::hours(6) {
                return Some(text.clone());
            }
        }
        let rt = crate::rules::settings::runtime(&self.pool).await.ok()?;
        let client = crate::market::yahoo::client(&rt).ok()?;
        match crate::market::amfi::fetch_nav_all(&client).await {
            Ok(text) => {
                *guard = Some((chrono::Utc::now(), text.clone()));
                Some(text)
            }
            Err(e) => {
                tracing::warn!(error = %e, "AMFI unavailable");
                None
            }
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/dashboard", get(dashboard))
        .route("/api/settings", get(get_settings).put(put_settings))
        .route("/api/settings/proxy/test", post(test_proxy))
        .route("/api/instruments", post(add_instrument))
        .route("/api/instruments/{symbol}", get(instrument_detail))
        .route("/api/trades", get(all_trades))
        .route("/api/archived", get(archived_instruments))
        .route("/api/calls/{id}", get(call_detail))
        .route(
            "/api/instruments/{symbol}/trades",
            get(instrument_trades).post(record_trade),
        )
        .route("/api/search", get(search_instruments))
        .route(
            "/api/instruments/{symbol}/refresh",
            post(refresh_instrument),
        )
        .route(
            "/api/instruments/{symbol}/target-profit",
            put(set_target_profit),
        )
        .route(
            "/api/instruments/{symbol}",
            axum::routing::delete(archive_instrument),
        )
        .route("/api/notify/test", post(test_notify))
        .route("/api/instruments/{symbol}/analysis", get(analysis))
        .route("/api/signals/{id}/approve", post(approve_signal))
        .route("/api/signals/{id}/reject", post(reject_signal))
        .route("/api/calls", get(calls))
        .route("/api/positions", post(post_position))
        .route("/api/positions/{id}", patch(patch_position))
        .route("/api/ipo", post(post_ipo))
        .with_state(state)
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("currency is fixed once the ledger has entries")]
    CurrencyLocked,
    #[error("internal error")]
    Internal,
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        tracing::error!(error = %e, "api error");
        ApiError::Internal
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        tracing::error!(error = %e, "sqlx error");
        ApiError::Internal
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = match &self {
            ApiError::NotFound(m) => (StatusCode::NOT_FOUND, serde_json::json!({"error": m})),
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, serde_json::json!({"error": m})),
            ApiError::CurrencyLocked => (
                StatusCode::CONFLICT,
                serde_json::json!({"error": "currency_locked"}),
            ),
            ApiError::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({"error": "internal"}),
            ),
        };
        (status, Json(body)).into_response()
    }
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"ok": true}))
}

/// Mirrors the joined query one-to-one, so a column order mistake is a compile error rather
/// than a TEXT-decoded-as-REAL at runtime. Two such bugs already shipped from positional tuples.
#[derive(sqlx::FromRow)]
struct SignalDbRow {
    id: i64,
    symbol: String,
    name: String,
    class: String,
    action: String,
    status: String,
    status_reason: Option<String>,
    filled_qty: Option<f64>,
    filled_price: Option<f64>,
    qty: Option<f64>,
    confidence: Option<String>,
    entry: Option<f64>,
    stop: Option<f64>,
    target: Option<f64>,
    valid_until: Option<String>,
    reason_json: Option<String>,
    created_at: String,
    plan_id: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct PositionDbRow {
    id: i64,
    symbol: String,
    name: String,
    class: String,
    qty: f64,
    avg_cost: f64,
    ltp: Option<f64>,
    stop: Option<f64>,
    target: Option<f64>,
    state: String,
}

#[derive(Debug, Serialize)]
pub struct SignalRow {
    pub id: i64,
    pub symbol: String,
    /// The readable name — a plan card says "Infosys", not "INFY.NS".
    pub name: String,
    pub class: String,
    pub action: String,
    pub qty: Option<f64>,
    pub confidence: Option<String>,
    pub entry: Option<f64>,
    pub stop: Option<f64>,
    pub target: Option<f64>,
    pub valid_until: Option<String>,
    pub timing_note: String,
    pub reasons: Vec<String>,
    pub rationale: Option<String>,
    pub created_at: String,
    /// pending | approved | rejected — the ledger shows pending only.
    pub status: String,
    pub status_reason: Option<String>,
    /// What you actually got, when the numbers differ from the proposal.
    pub filled_qty: Option<f64>,
    pub filled_price: Option<f64>,
    /// Set when this row is one leg of a plan; every leg shares the first leg's id.
    pub plan_id: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct PositionRow {
    pub id: i64,
    pub symbol: String,
    /// The readable name. A ticker tells you nothing about what you own.
    pub name: String,
    pub class: String,
    pub qty: f64,
    pub avg_cost: f64,
    pub ltp: Option<f64>,
    pub value: Option<f64>,
    pub stop: Option<f64>,
    pub target: Option<f64>,
    pub state: String,
    pub timing_note: String,
}

#[derive(Debug, Serialize)]
pub struct Dashboard {
    pub currency: String,
    pub budget: f64,
    pub committed: f64,
    pub available: f64,
    pub realised: f64,
    pub mode: String,
    pub currency_locked: bool,
    pub positions: Vec<PositionRow>,
    pub signals: Vec<SignalRow>,
    pub blocked: Vec<SignalRow>,
    /// The rail's recent fills: one request per page instead of two.
    pub recent_trades: Vec<crate::trades::Trade>,
}

/// Cheap version tag for the dashboard: max signal id, trade id, position count, settings
/// count. A minute-by-minute poll that finds nothing new costs a 304 with no body.
async fn dashboard_etag(pool: &SqlitePool, mode: crate::models::Mode) -> Result<String, ApiError> {
    let row: (i64, i64, i64, i64, i64) = sqlx::query_as(
        // Approving or rejecting changes no id, so the pending count and the last status change
        // are part of the tag — otherwise the page answers 304 and the list never updates.
        "SELECT (SELECT COALESCE(MAX(id),0) FROM signals WHERE mode = ?),
                (SELECT COALESCE(MAX(id),0) FROM trades WHERE mode = ?),
                (SELECT COUNT(*) FROM positions WHERE mode = ?),
                (SELECT COUNT(*) FROM settings),
                (SELECT COUNT(*) FROM signals WHERE mode = ? AND status = 'pending')",
    )
    .bind(mode.as_str())
    .bind(mode.as_str())
    .bind(mode.as_str())
    .bind(mode.as_str())
    .fetch_one(pool)
    .await?;
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(
        format!(
            "{}-{}-{}-{}-{}-{}",
            row.0,
            row.1,
            row.2,
            row.3,
            row.4,
            mode.as_str()
        )
        .as_bytes(),
    );
    Ok(format!("\"{}\"", crate::hex(&h.finalize())))
}

async fn dashboard(
    State(st): State<AppState>,
    Query(q): Query<ModeQuery>,
    headers: HeaderMap,
) -> Result<axum::response::Response, ApiError> {
    let mode = mode_of(&q.mode)?;
    let s = settings::get(&st.pool).await?;
    let budget = budget::budget_total_in(&st.pool, mode).await?;
    let committed = budget::committed_in(&st.pool, mode).await?;
    let positions = load_positions(&st.pool, mode).await?;
    let status = q.decisions.as_deref().unwrap_or("pending");
    let signals = load_signals(&st.pool, false, mode, status).await?;
    let blocked = load_signals(&st.pool, true, mode, "pending").await?;
    let etag = dashboard_etag(&st.pool, mode).await?;
    if headers
        .get(IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == etag)
    {
        return Ok((StatusCode::NOT_MODIFIED, [(ETAG, etag)]).into_response());
    }

    let body = Json(Dashboard {
        currency: s.currency,
        mode: mode.as_str().to_string(),
        budget,
        committed,
        realised: crate::trades::realised_in(&st.pool, mode).await?,
        available: budget - committed,
        currency_locked: s.currency_locked,
        positions,
        signals,
        blocked,
        recent_trades: crate::trades::history(&st.pool, None, 20).await?,
    });
    Ok(([(ETAG, etag)], body).into_response())
}

async fn get_settings(State(st): State<AppState>) -> Result<Json<settings::Settings>, ApiError> {
    Ok(Json(settings::get(&st.pool).await?))
}

#[derive(Debug, Deserialize)]
pub struct PutSettings {
    pub budget: Option<f64>,
    pub currency: Option<String>,
    pub deepseek_key: Option<String>,
    pub trading_budget: Option<f64>,
    pub mf_budget: Option<f64>,
    pub ntfy_url: Option<String>,
    pub ntfy_topic: Option<String>,
    pub ntfy_token: Option<String>,
    pub llm_provider: Option<String>,
    pub llm_model: Option<String>,
    pub llm_base_url: Option<String>,
    pub llm_key: Option<String>,
    pub limits_long: Option<crate::rules::limits::Limits>,
    pub limits_trade: Option<crate::rules::limits::Limits>,
    pub poll_secs: Option<u64>,
    pub off_hours_secs: Option<u64>,
    pub prompt_window_hours: Option<u32>,
    pub min_price_move: Option<f64>,
    pub max_silence_secs: Option<i64>,
    pub daily_cap: Option<u32>,
    pub proxy: Option<String>,
    pub proxy_bypass: Option<String>,
    pub proxy_user: Option<String>,
    /// Empty or whitespace keeps the stored password.
    pub proxy_pass: Option<String>,
}

async fn put_settings(
    State(st): State<AppState>,
    Json(body): Json<PutSettings>,
) -> Result<Json<settings::Settings>, ApiError> {
    settings::set(
        &st.pool,
        SettingsPatch {
            budget: body.budget,
            currency: body.currency,
            deepseek_key: body.deepseek_key,
            trading_budget: body.trading_budget,
            mf_budget: body.mf_budget,
            ntfy_url: body.ntfy_url,
            ntfy_topic: body.ntfy_topic,
            ntfy_token: body.ntfy_token,
            llm_provider: body.llm_provider,
            llm_model: body.llm_model,
            llm_base_url: body.llm_base_url,
            llm_key: body.llm_key,
            limits_long: body.limits_long,
            limits_trade: body.limits_trade,
            poll_secs: body.poll_secs,
            off_hours_secs: body.off_hours_secs,
            prompt_window_hours: body.prompt_window_hours,
            min_price_move: body.min_price_move,
            max_silence_secs: body.max_silence_secs,
            daily_cap: body.daily_cap,
            proxy: body.proxy,
            proxy_bypass: body.proxy_bypass,
            proxy_user: body.proxy_user,
            proxy_pass: body.proxy_pass,
        },
    )
    .await
    .map_err(|e| match e {
        SettingsError::CurrencyLocked => ApiError::CurrencyLocked,
        SettingsError::InvalidAmount => ApiError::BadRequest("invalid amount".into()),
        SettingsError::BadRequest(message) => ApiError::BadRequest(message),
    })?;
    Ok(Json(settings::get(&st.pool).await?))
}

/// One real request through the stored proxy. A diagnostic, not a health check: it reports the
/// failure instead of failing the call, because a wrong proxy otherwise shows up as silence.
async fn test_proxy(State(st): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let rt = settings::runtime(&st.pool).await?;
    let started = std::time::Instant::now();
    let result = match crate::http::client(&rt, 5) {
        Ok(client) => client
            .get("https://api.deepseek.com/")
            .send()
            .await
            .map(|r| r.status().as_u16()),
        Err(e) => {
            return Ok(Json(serde_json::json!({
                "ok": false,
                "status": null,
                "millis": started.elapsed().as_millis() as u64,
                "error": e.to_string(),
            })))
        }
    };
    let millis = started.elapsed().as_millis() as u64;
    Ok(Json(match result {
        Ok(status) => {
            serde_json::json!({ "ok": true, "status": status, "millis": millis, "error": null })
        }
        Err(e) => serde_json::json!({
            "ok": false,
            "status": null,
            "millis": millis,
            "error": e.to_string(),
        }),
    }))
}

#[derive(Debug, Serialize)]
pub struct InstrumentDetail {
    pub instrument: InstrumentRow,
    pub headlines: Vec<HeadlineRow>,
    pub signals: Vec<SignalRow>,
}

/// Remove an instrument, or hide it when it has history.
/// Archive an instrument: it leaves the working book but keeps its record.
///
/// There is no delete. Archiving clears the *temporary* data — bars, headlines, memos — because that
/// is re-fetchable and re-dated, but keeps the *history* that makes the ledger true: fills, decisions
/// and closed positions. Anything still held is refused until it is sold. Archived rows stay
/// reachable from a dedicated list, not from the working ledger.
async fn archive_instrument(
    State(st): State<AppState>,
    axum::extract::Path(symbol): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let row: Option<(i64, i64)> =
        sqlx::query_as("SELECT id, active FROM instruments WHERE symbol = ?")
            .bind(&symbol)
            .fetch_optional(&st.pool)
            .await?;
    let Some((id, active)) = row else {
        return Err(ApiError::NotFound(format!("no instrument {symbol}")));
    };
    if active == 0 {
        return Err(ApiError::BadRequest(format!(
            "{symbol} is already archived"
        )));
    }

    let held: (f64,) = sqlx::query_as(
        "SELECT CAST(COALESCE(SUM(qty), 0) AS REAL) FROM positions
         WHERE instrument_id = ? AND state <> 'closed'",
    )
    .bind(id)
    .fetch_one(&st.pool)
    .await?;
    if held.0 > 0.0 {
        return Err(ApiError::BadRequest(format!(
            "cannot archive {symbol} while {} units are held — sell it first",
            held.0
        )));
    }

    let mut tx = st.pool.begin().await?;
    sqlx::query("UPDATE instruments SET active = 0 WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    // Temporary data goes: it is re-fetchable, and keeping it would re-date it the next time the
    // symbol is opened. History stays.
    for sql in [
        "DELETE FROM bars WHERE instrument_id = ?",
        "DELETE FROM news WHERE instrument_id = ?",
        "DELETE FROM llm_memos WHERE instrument_id = ?",
    ] {
        sqlx::query(sql).bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(Json(serde_json::json!({
        "symbol": symbol,
        "archived": true,
        "deleted": false,
    })))
}

#[derive(Debug, Serialize)]
pub struct InstrumentRow {
    pub id: i64,
    pub symbol: String,
    pub class: String,
    pub name: String,
    pub currency: String,
    pub timing_note: String,
    /// The owner's expected profit, in percent. Decisions must reach it.
    pub target_profit_pct: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct HeadlineRow {
    pub id: i64,
    pub title: String,
    pub publisher: Option<String>,
    pub published_at: String,
    pub url: String,
}

async fn instrument_detail(
    State(st): State<AppState>,
    Path(symbol): Path<String>,
) -> Result<Json<InstrumentDetail>, ApiError> {
    let row: Option<(i64, String, String, String, String, Option<f64>)> = sqlx::query_as(
        "SELECT id, symbol, class, name, currency, target_profit_pct
         FROM instruments WHERE symbol = ?",
    )
    .bind(&symbol)
    .fetch_optional(&st.pool)
    .await?;
    let (id, symbol, class, name, currency, target_profit_pct) =
        row.ok_or_else(|| ApiError::NotFound(format!("no instrument {symbol}")))?;
    let class = AssetClass::parse(&class).map_err(|_| ApiError::Internal)?;

    let headlines: Vec<HeadlineRow> =
        sqlx::query_as::<_, (i64, String, Option<String>, String, String)>(
            "SELECT id, title, publisher, published_at, url FROM news
         WHERE instrument_id = ? ORDER BY published_at DESC LIMIT 20",
        )
        .bind(id)
        .fetch_all(&st.pool)
        .await?
        .into_iter()
        .map(|r| HeadlineRow {
            id: r.0,
            title: r.1,
            publisher: r.2,
            published_at: r.3,
            url: r.4,
        })
        .collect();

    let signals: Vec<SignalRow> =
        load_signals(&st.pool, false, crate::models::Mode::Long, "everything")
            .await?
            .into_iter()
            .filter(|s| s.symbol == symbol)
            .collect();

    Ok(Json(InstrumentDetail {
        instrument: InstrumentRow {
            id,
            symbol,
            class: class.as_str().to_string(),
            name,
            currency,
            timing_note: class.timing_note().to_string(),
            target_profit_pct,
        },
        headlines,
        signals,
    }))
}

#[derive(Debug, Serialize)]
pub struct BarPoint {
    pub ts: String,
    pub close: f64,
}

#[derive(Debug, Serialize)]
pub struct CheckRow {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct Analysis {
    pub instrument: InstrumentRow,
    pub bars: Vec<BarPoint>,
    pub indicators: Option<serde_json::Value>,
    pub checks: Vec<CheckRow>,
    pub position: Option<PositionRow>,
    pub headlines: Vec<HeadlineRow>,
    /// Everything below saves two extra round trips on the instrument page.
    pub signals: Vec<SignalRow>,
    pub trades: Vec<crate::trades::Trade>,
}

/// "Why is there no signal?" answered with the same predicates the engine used.
async fn analysis(
    State(st): State<AppState>,
    Path(symbol): Path<String>,
) -> Result<Json<Analysis>, ApiError> {
    let row: Option<(i64, String, String, String, String, Option<f64>)> = sqlx::query_as(
        "SELECT id, symbol, class, name, currency, target_profit_pct
         FROM instruments WHERE symbol = ?",
    )
    .bind(&symbol)
    .fetch_optional(&st.pool)
    .await?;
    let (id, symbol, class_raw, name, currency, target_profit_pct) =
        row.ok_or_else(|| ApiError::NotFound(format!("no instrument {symbol}")))?;
    let class = AssetClass::parse(&class_raw).map_err(|_| ApiError::Internal)?;

    let rows: Vec<(String, f64, f64, f64, f64, i64)> = sqlx::query_as(
        "SELECT ts, COALESCE(open, close), COALESCE(high, close), COALESCE(low, close), close,
                COALESCE(volume, 0)
         FROM bars WHERE instrument_id = ? ORDER BY ts DESC LIMIT 400",
    )
    .bind(id)
    .fetch_all(&st.pool)
    .await?;
    let mut bars: Vec<Bar> = rows
        .into_iter()
        .map(|(ts, open, high, low, close, volume)| Bar {
            ts,
            open,
            high,
            low,
            close,
            volume,
        })
        .collect();
    bars.reverse();
    bars.dedup_by(|a, b| a.ts == b.ts);

    let position = load_positions(&st.pool, crate::models::Mode::Long)
        .await?
        .into_iter()
        .find(|p| p.symbol == symbol && p.state == "open");
    let available = budget::available(&st.pool).await?;
    let instrument = crate::models::Instrument {
        id,
        symbol: symbol.clone(),
        class,
        name: name.clone(),
        currency: currency.clone(),
        active: true,
    };

    let ind = indicators::compute(&bars).ok();
    let checks = match ind {
        Some(ref ind) => {
            let model = position.as_ref().map(|p| crate::models::Position {
                id: p.id,
                instrument_id: id,
                qty: p.qty,
                avg_cost: p.avg_cost,
                stop: p.stop,
                target: p.target,
                state: p.state.clone(),
            });
            evaluate::diagnose(&CandidateInput {
                instrument: &instrument,
                bars: &bars,
                ind,
                position: model.as_ref(),
                available_cash: available,
                cfg: RuleConfig::default(),
            })
            .into_iter()
            .map(|c| CheckRow {
                name: c.name.to_string(),
                passed: c.passed,
                detail: c.detail,
            })
            .collect()
        }
        None => vec![CheckRow {
            name: "price data".into(),
            passed: false,
            detail: format!("{} stored bars — the rules need 200", bars.len()),
        }],
    };

    let headlines: Vec<HeadlineRow> =
        sqlx::query_as::<_, (i64, String, Option<String>, String, String)>(
            "SELECT id, title, publisher, published_at, url FROM news
             WHERE instrument_id = ? ORDER BY published_at DESC LIMIT 10",
        )
        .bind(id)
        .fetch_all(&st.pool)
        .await?
        .into_iter()
        .map(|r| HeadlineRow {
            id: r.0,
            title: r.1,
            publisher: r.2,
            published_at: r.3,
            url: r.4,
        })
        .collect();

    let signals: Vec<SignalRow> =
        load_signals(&st.pool, false, crate::models::Mode::Long, "everything")
            .await?
            .into_iter()
            .filter(|s| s.symbol == symbol)
            .collect();
    let trades = crate::trades::history(&st.pool, Some(id), 200).await?;

    Ok(Json(Analysis {
        instrument: InstrumentRow {
            id,
            symbol,
            class: class.as_str().to_string(),
            name,
            currency,
            timing_note: class.timing_note().to_string(),
            target_profit_pct,
        },
        bars: bars
            .iter()
            .rev()
            .take(120)
            .rev()
            .map(|b| BarPoint {
                ts: b.ts.clone(),
                close: b.close,
            })
            .collect(),
        indicators: ind.map(|i| {
            serde_json::json!({
                "sma50": i.sma50, "sma200": i.sma200, "rsi14": i.rsi14,
                "atr14": i.atr14, "high52": i.high52, "low52": i.low52
            })
        }),
        checks,
        position,
        headlines,
        signals,
        trades,
    }))
}

#[derive(Debug, Serialize)]
pub struct CallRow {
    pub id: i64,
    pub ts: String,
    pub endpoint: String,
    /// "BLOCKED" when the proposal could not be funded, so the review column can say so.
    pub decision_action: Option<String>,
    /// Why it was refused, when it was.
    pub decision_reason: Option<String>,
    /// The holding's name, which is what a person reads; the symbol is secondary.
    pub instrument_name: Option<String>,
    pub instrument_symbol: Option<String>,
    /// Why the model was asked: "price moved …", "new headline …", "rotation …".
    pub trigger: Option<String>,
    /// The decision this call produced, and what you did with it.
    pub decision_id: Option<i64>,
    pub decision_status: Option<String>,
    pub purpose: Option<String>,
    pub round: Option<i64>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub validator_verdict: Option<String>,
    pub error: Option<String>,
}

#[derive(sqlx::FromRow)]
struct CallDbRow {
    id: i64,
    ts: String,
    endpoint: String,
    decision_action: Option<String>,
    decision_reason: Option<String>,
    instrument_name: Option<String>,
    instrument_symbol: Option<String>,
    trigger: Option<String>,
    decision_id: Option<i64>,
    decision_status: Option<String>,
    purpose: Option<String>,
    round: Option<i64>,
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    validator_verdict: Option<String>,
    error: Option<String>,
}

#[derive(sqlx::FromRow)]
struct CallDetailRow {
    id: i64,
    ts: String,
    endpoint: String,
    purpose: Option<String>,
    round: Option<i64>,
    /// Recorded when the poll decided a call was warranted — the sentence the audit table used
    /// to print inline, now part of the expanded view.
    trigger: Option<String>,
    /// Set when the proposal was refused for want of money; the expanded view says why.
    decision_reason: Option<String>,
    instrument_name: Option<String>,
    instrument_symbol: Option<String>,
    prompt_template: Option<String>,
    request_json: Option<String>,
    response_json: Option<String>,
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    validator_verdict: Option<String>,
    error: Option<String>,
}

/// One call, with the prompt and the raw reply. Fetched on demand: the list must stay small.
async fn call_detail(
    State(st): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let row: Option<CallDetailRow> = sqlx::query_as(
        "SELECT a.id, a.ts, a.endpoint, a.purpose, a.round, a.trigger, a.prompt_template,
                a.request_json, a.response_json, a.prompt_tokens, a.completion_tokens,
                a.validator_verdict, a.error,
                d.reason_json AS decision_reason,
                i.name AS instrument_name, i.symbol AS instrument_symbol
         FROM api_calls a
         LEFT JOIN signals d ON d.id = (
             SELECT s.id FROM signals s WHERE s.conversation_id = a.conversation_id
             ORDER BY s.id DESC LIMIT 1)
         LEFT JOIN instruments i ON i.id = d.instrument_id
         WHERE a.id = ?",
    )
    .bind(id)
    .fetch_optional(&st.pool)
    .await?;
    let Some(r) = row else {
        return Err(ApiError::NotFound(format!("no call {id}")));
    };
    Ok(Json(serde_json::json!({
        "id": r.id,
        "ts": r.ts,
        "endpoint": r.endpoint,
        "purpose": r.purpose,
        "round": r.round,
        "trigger": r.trigger,
        "decision_reason": blocked_reason(r.decision_reason.as_deref()),
        "instrument_name": r.instrument_name,
        "instrument_symbol": r.instrument_symbol,
        "prompt_template": r.prompt_template,
        "request_json": r.request_json,
        "response_json": r.response_json,
        "prompt_tokens": r.prompt_tokens,
        "completion_tokens": r.completion_tokens,
        "validator_verdict": r.validator_verdict,
        "error": r.error,
    })))
}

async fn calls(State(st): State<AppState>) -> Result<Json<Vec<CallRow>>, ApiError> {
    let rows: Vec<CallDbRow> = sqlx::query_as(
        "SELECT a.id AS id, a.ts AS ts, a.endpoint AS endpoint, a.purpose AS purpose,
                a.round AS round, a.prompt_tokens AS prompt_tokens,
                a.completion_tokens AS completion_tokens,
                a.validator_verdict AS validator_verdict, a.error AS error,
                a.trigger AS trigger, d.id AS decision_id, d.status AS decision_status,
                d.action AS decision_action, d.reason_json AS decision_reason,
                i.name AS instrument_name, i.symbol AS instrument_symbol
         FROM api_calls a
         LEFT JOIN signals d ON d.id = (
             SELECT s.id FROM signals s WHERE s.conversation_id = a.conversation_id
             ORDER BY s.id DESC LIMIT 1)
         LEFT JOIN instruments i ON i.id = d.instrument_id
         ORDER BY a.id DESC LIMIT 100",
    )
    .fetch_all(&st.pool)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| CallRow {
                id: r.id,
                ts: r.ts,
                endpoint: r.endpoint,
                decision_action: r.decision_action,
                decision_reason: blocked_reason(r.decision_reason.as_deref()),
                instrument_name: r.instrument_name,
                instrument_symbol: r.instrument_symbol,
                trigger: r.trigger,
                decision_id: r.decision_id,
                decision_status: r.decision_status,
                purpose: r.purpose,
                round: r.round,
                prompt_tokens: r.prompt_tokens,
                completion_tokens: r.completion_tokens,
                validator_verdict: r.validator_verdict,
                error: r.error,
            })
            .collect(),
    ))
}

/// The sentence a blocked row already carries, pulled out of its JSON for the audit table.
fn blocked_reason(raw: Option<&str>) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(raw?).ok()?;
    let kind = v.get("blocked")?.as_str()?;
    let cost = v.get("cost").and_then(|c| c.as_f64());
    let available = v.get("available").and_then(|c| c.as_f64());
    Some(match (cost, available) {
        (Some(c), Some(a)) => format!("{kind}: cost {c:.2}, available {a:.2}"),
        _ => kind.to_string(),
    })
}

#[derive(Debug, Deserialize)]
pub struct NewPosition {
    pub symbol: String,
    pub qty: f64,
    pub avg_cost: f64,
    pub stop: Option<f64>,
    pub target: Option<f64>,
}

async fn post_position(
    State(st): State<AppState>,
    Json(body): Json<NewPosition>,
) -> Result<Json<PositionRow>, ApiError> {
    if !body.qty.is_finite()
        || body.qty <= 0.0
        || !body.avg_cost.is_finite()
        || body.avg_cost <= 0.0
    {
        return Err(ApiError::BadRequest(
            "qty and avg_cost must be positive".into(),
        ));
    }
    let id = instrument_id_by_symbol(&st.pool, &body.symbol).await?;
    let cost = budget::cost(body.qty, body.avg_cost, AssetClass::Stock);
    if !budget::fits(&st.pool, cost).await? {
        return Err(ApiError::BadRequest("does not fit the budget".into()));
    }

    sqlx::query(
        "INSERT INTO positions (instrument_id, qty, avg_cost, opened_at, stop, target, state)
         VALUES (?, ?, ?, ?, ?, ?, 'open')",
    )
    .bind(id)
    .bind(body.qty)
    .bind(body.avg_cost)
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(body.stop)
    .bind(body.target)
    .execute(&st.pool)
    .await?;
    budget::record(
        &st.pool,
        NewCashEntry {
            kind: CashKind::Buy,
            amount: -cost,
            reference: format!("buy {} @ {}", body.symbol, body.avg_cost),
            bucket: None,
        },
    )
    .await?;
    let row = load_positions(&st.pool, crate::models::Mode::Long)
        .await?
        .into_iter()
        .find(|p| p.symbol == body.symbol && p.state == "open")
        .ok_or(ApiError::Internal)?;
    Ok(Json(row))
}

#[derive(Debug, Deserialize)]
pub struct PatchPosition {
    pub qty: Option<f64>,
    pub stop: Option<f64>,
    pub target: Option<f64>,
    pub state: Option<String>,
}

async fn patch_position(
    State(st): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<PatchPosition>,
) -> Result<Json<PositionRow>, ApiError> {
    let existing: Option<(f64, f64)> =
        sqlx::query_as("SELECT qty, avg_cost FROM positions WHERE id = ?")
            .bind(id)
            .fetch_optional(&st.pool)
            .await?;
    let (old_qty, avg_cost) = existing.ok_or_else(|| ApiError::NotFound("no position".into()))?;

    if let Some(qty) = body.qty {
        sqlx::query("UPDATE positions SET qty = ? WHERE id = ?")
            .bind(qty)
            .bind(id)
            .execute(&st.pool)
            .await?;
        let sold = old_qty - qty;
        if sold > 0.0 {
            budget::record(
                &st.pool,
                NewCashEntry {
                    kind: CashKind::Sell,
                    amount: sold * avg_cost,
                    reference: format!("sell position {id}"),
                    bucket: None,
                },
            )
            .await?;
        }
    }
    if let Some(stop) = body.stop {
        sqlx::query("UPDATE positions SET stop = ? WHERE id = ?")
            .bind(stop)
            .bind(id)
            .execute(&st.pool)
            .await?;
    }
    if let Some(target) = body.target {
        sqlx::query("UPDATE positions SET target = ? WHERE id = ?")
            .bind(target)
            .bind(id)
            .execute(&st.pool)
            .await?;
    }
    if let Some(state) = body.state {
        sqlx::query("UPDATE positions SET state = ? WHERE id = ?")
            .bind(&state)
            .bind(id)
            .execute(&st.pool)
            .await?;
    }

    load_positions(&st.pool, crate::models::Mode::Long)
        .await?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or(ApiError::Internal)
        .map(Json)
}

#[derive(Debug, Deserialize)]
pub struct NewIpo {
    pub symbol: String,
    pub qty: f64,
    pub price: f64,
}

async fn post_ipo(
    State(st): State<AppState>,
    Json(body): Json<NewIpo>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !body.qty.is_finite() || body.qty <= 0.0 || !body.price.is_finite() || body.price <= 0.0 {
        return Err(ApiError::BadRequest(
            "qty and price must be positive".into(),
        ));
    }
    let id = instrument_id_by_symbol(&st.pool, &body.symbol).await?;
    let cost = budget::cost(body.qty, body.price, AssetClass::Ipo);
    if !budget::fits(&st.pool, cost).await? {
        return Err(ApiError::BadRequest("does not fit the budget".into()));
    }
    budget::reserve_ipo(&st.pool, id, body.qty, body.price).await?;
    Ok(Json(serde_json::json!({
        "reserved": cost,
        "available": budget::available(&st.pool).await?,
    })))
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: String,
}

#[derive(Debug, Deserialize)]
pub struct ModeQuery {
    pub mode: Option<String>,
    /// pending (default) | approved | rejected | all
    pub decisions: Option<String>,
}

fn mode_of(q: &Option<String>) -> Result<crate::models::Mode, ApiError> {
    // Tolerate a hand-written URL: `?mode=long?decisions=rejected` puts the rest of the query into
    // the value. Take the part that is actually the mode, and ignore the leftovers.
    let raw = q
        .as_deref()
        .unwrap_or("")
        .split(['?', '&'])
        .next()
        .unwrap_or("")
        .trim();
    crate::models::Mode::parse(raw).map_err(|e| ApiError::BadRequest(e.to_string()))
}

#[derive(Debug, Serialize)]
pub struct HitRow {
    pub code: String,
    pub name: String,
    pub class: String,
    pub venue: String,
    pub price: Option<f64>,
}

async fn search_instruments(
    State(st): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<Vec<HitRow>>, ApiError> {
    let query = q.q.trim();
    if query.len() < 2 {
        return Ok(Json(Vec::new()));
    }
    let rt = crate::rules::settings::runtime(&st.pool).await?;
    let client = crate::market::yahoo::client(&rt)?;
    let amfi = st.amfi_text().await;
    let hits = crate::market::search(&client, query, amfi.as_deref())
        .await
        .unwrap_or_default();
    Ok(Json(
        hits.into_iter()
            .map(|h| HitRow {
                code: h.code,
                name: h.name,
                class: h.class.as_str().to_string(),
                venue: h.venue,
                price: h.price,
            })
            .collect(),
    ))
}

#[derive(Debug, Deserialize)]
pub struct AddInstrument {
    pub code: String,
    pub name: Option<String>,
    /// "stock" (default) or "mutual_fund".
    pub class: Option<String>,
}

async fn add_instrument(
    State(st): State<AppState>,
    Json(body): Json<AddInstrument>,
) -> Result<Json<InstrumentRow>, ApiError> {
    let code = body.code.trim();
    if code.is_empty() {
        return Err(ApiError::BadRequest("code is required".into()));
    }

    let (class, name) = match body.class.as_deref() {
        Some("mutual_fund") => {
            // Two kinds of fund code exist: AMFI's (135762, with NAV in the scheme list) and
            // Yahoo's (0P00012ALS.BO). Accept either; the AMFI list is not always reachable.
            let amfi_hit = match st.amfi_text().await {
                Some(text) => crate::market::search::amfi_by_code(&text, code),
                None => None,
            };
            match amfi_hit {
                Some(hit) => (crate::models::AssetClass::MutualFund, hit.name),
                None => {
                    let rt = crate::rules::settings::runtime(&st.pool).await?;
                    let client = crate::market::yahoo::client(&rt)?;
                    let known = crate::market::search(&client, code, None)
                        .await
                        .unwrap_or_default()
                        .into_iter()
                        .find(|h| h.code.eq_ignore_ascii_case(code))
                        .ok_or_else(|| {
                            ApiError::BadRequest(format!(
                                "{code} is not in the AMFI scheme list and is not a Yahoo symbol —                                  search for the fund by name and pick a suggestion"
                            ))
                        })?;
                    (crate::models::AssetClass::MutualFund, known.name)
                }
            }
        }
        _ => (
            crate::models::AssetClass::Stock,
            body.name.unwrap_or_else(|| code.to_string()),
        ),
    };

    sqlx::query(
        "INSERT OR IGNORE INTO instruments (symbol, class, name, currency) VALUES (?, ?, ?, 'INR')",
    )
    .bind(code)
    .bind(class.as_str())
    .bind(&name)
    .execute(&st.pool)
    .await?;

    let id: (i64,) = sqlx::query_as("SELECT id FROM instruments WHERE symbol = ?")
        .bind(code)
        .fetch_one(&st.pool)
        .await?;

    // A newly added symbol should have a chart and headlines on first open, not tomorrow
    // morning. AMFI scheme codes have no Yahoo series; a Yahoo-coded fund (0P…BO) does.
    if class == crate::models::AssetClass::Stock || code.contains('.') {
        if let Ok(market) =
            crate::market::YahooSource::new(&crate::rules::settings::runtime(&st.pool).await?)
        {
            let fetch = crate::store::refresh(&st.pool, id.0, code, &market);
            match tokio::time::timeout(std::time::Duration::from_secs(20), fetch).await {
                Ok(Ok((bars, news))) => {
                    tracing::info!(symbol = code, bars, news, "prices and headlines stored")
                }
                Ok(Err(e)) => tracing::warn!(symbol = code, error = %e, "refresh failed"),
                Err(_) => tracing::warn!(symbol = code, "refresh timed out"),
            }
        }
    }

    Ok(Json(InstrumentRow {
        id: id.0,
        symbol: code.to_string(),
        class: class.as_str().to_string(),
        name,
        currency: "INR".into(),
        timing_note: class.timing_note().to_string(),
        target_profit_pct: None,
    }))
}

#[derive(Debug, Deserialize)]
pub struct SetTargetProfit {
    pub target_profit_pct: Option<f64>,
}

/// Set or clear the profit expectation for an instrument.
async fn set_target_profit(
    State(st): State<AppState>,
    Path(symbol): Path<String>,
    Json(body): Json<SetTargetProfit>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if let Some(p) = body.target_profit_pct {
        if !p.is_finite() || p <= 0.0 || p > 1000.0 {
            return Err(ApiError::BadRequest(
                "target_profit_pct must be between 0 and 1000".into(),
            ));
        }
        // The global band bounds what a single stock may ask for. Checked against the tighter
        // of the two pots so the stored value is legal whichever book the stock is held in.
        let long = settings::limits(&st.pool, crate::models::Mode::Long).await?;
        let trade = settings::limits(&st.pool, crate::models::Mode::Trade).await?;
        let floor = long.min_profit_pct.max(trade.min_profit_pct);
        let ceiling = long.max_profit_pct.min(trade.max_profit_pct);
        if p < floor - 1e-9 || p > ceiling + 1e-9 {
            return Err(ApiError::BadRequest(format!(
                "profit expectation must be between {floor:.1}% and {ceiling:.1}% — the global band"
            )));
        }
    }
    let id = instrument_id_by_symbol(&st.pool, &symbol).await?;
    sqlx::query("UPDATE instruments SET target_profit_pct = ? WHERE id = ?")
        .bind(body.target_profit_pct)
        .bind(id)
        .execute(&st.pool)
        .await?;
    Ok(Json(serde_json::json!({
        "symbol": symbol,
        "target_profit_pct": body.target_profit_pct,
    })))
}

#[derive(Debug, Deserialize)]
pub struct TradeBody {
    pub side: String,
    pub qty: f64,
    pub price: Option<f64>,
    pub note: Option<String>,
    /// `YYYY-MM-DD` or RFC3339. Omitted means "now".
    pub at: Option<String>,
    /// "long" (default) or "trade" — which pot this fill belongs to.
    pub mode: Option<String>,
}

async fn record_trade(
    State(st): State<AppState>,
    Path(symbol): Path<String>,
    Json(body): Json<TradeBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = instrument_id_by_symbol(&st.pool, &symbol).await?;
    let side =
        crate::trades::Side::parse(&body.side).map_err(|e| ApiError::BadRequest(e.to_string()))?;

    // A price is required; there is no "current price" for a fill that already happened.
    let price = body
        .price
        .filter(|p| p.is_finite() && *p > 0.0)
        .ok_or_else(|| ApiError::BadRequest("price is required".into()))?;

    if !body.qty.is_finite() || body.qty <= 0.0 {
        return Err(ApiError::BadRequest("qty must be positive".into()));
    }

    let when = crate::trades::parse_when(body.at.as_deref().unwrap_or(""), chrono::Utc::now())
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let mode = crate::models::Mode::parse(body.mode.as_deref().unwrap_or(""))
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    crate::trades::record(
        &st.pool,
        id,
        side,
        body.qty,
        price,
        body.note.as_deref(),
        Some(when),
        mode,
    )
    .await
    .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let cash = budget::available_in(&st.pool, mode).await?;
    Ok(Json(serde_json::json!({
        "ok": true,
        "cash": cash,
        "realised": crate::trades::realised_in(&st.pool, mode).await?,
    })))
}

/// Fetch prices and headlines for one instrument on demand — for instruments added before the
/// add-time fetch existed, or simply to pull the latest without waiting for a poll tick.
async fn refresh_instrument(
    State(st): State<AppState>,
    Path(symbol): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let row: Option<(i64, String)> =
        sqlx::query_as("SELECT id, class FROM instruments WHERE symbol = ?")
            .bind(&symbol)
            .fetch_optional(&st.pool)
            .await?;
    let (id, class) = row.ok_or_else(|| ApiError::NotFound(format!("no instrument {symbol}")))?;

    let market =
        crate::market::YahooSource::new(&crate::rules::settings::runtime(&st.pool).await?)?;
    let (bars, news) = crate::store::refresh(&st.pool, id, &symbol, &market)
        .await
        .map_err(|e| ApiError::BadRequest(format!("{symbol} ({class}): {e}")))?;
    Ok(Json(serde_json::json!({ "bars": bars, "headlines": news })))
}

async fn instrument_trades(
    State(st): State<AppState>,
    Path(symbol): Path<String>,
) -> Result<Json<Vec<crate::trades::Trade>>, ApiError> {
    let id = instrument_id_by_symbol(&st.pool, &symbol).await?;
    Ok(Json(crate::trades::history(&st.pool, Some(id), 200).await?))
}

async fn all_trades(
    State(st): State<AppState>,
) -> Result<Json<Vec<crate::trades::Trade>>, ApiError> {
    Ok(Json(crate::trades::history(&st.pool, None, 200).await?))
}

/// Sends one notification through the same path a real signal uses.
async fn test_notify(State(st): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let Some(notifier) = crate::notify::Ntfy::from_settings(&st.pool, &st.cfg.public_url)
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?
    else {
        return Err(ApiError::BadRequest(
            "no topic configured — add one below first".into(),
        ));
    };
    let n = crate::notify::Notification {
        title: "stock-predictor · test".into(),
        body: "If this arrived on your phone, the channel works.".into(),
        priority: crate::notify::Priority::Default,
        dedupe_key: format!("test-{}", chrono::Utc::now().timestamp()),
        signal_id: None,
        symbol: String::new(),
    };
    notifier
        .send_test(&n)
        .await
        .map_err(|e| ApiError::BadRequest(format!("ntfy: {e}")))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Archived instruments with their record, not their history: what they were, and the decisions and
/// fills that remain. Bars and headlines were cleared when they left the book.
#[derive(Debug, Serialize)]
pub struct ArchivedRow {
    pub id: i64,
    pub symbol: String,
    pub name: String,
    pub class: String,
    pub fills: i64,
    pub decisions: i64,
    pub last_fill: Option<String>,
}

async fn archived_instruments(
    State(st): State<AppState>,
) -> Result<Json<Vec<ArchivedRow>>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct ArchivedDbRow {
        id: i64,
        symbol: String,
        name: String,
        class: String,
        fills: i64,
        decisions: i64,
        last_fill: Option<String>,
    }
    let rows: Vec<ArchivedDbRow> = sqlx::query_as(
        "SELECT i.id, i.symbol, i.name, i.class,
                (SELECT COUNT(*) FROM trades t WHERE t.instrument_id = i.id) AS fills,
                (SELECT COUNT(*) FROM signals s WHERE s.instrument_id = i.id) AS decisions,
                (SELECT MAX(t.traded_at) FROM trades t WHERE t.instrument_id = i.id) AS last_fill
         FROM instruments i
         WHERE i.active = 0
         ORDER BY last_fill DESC",
    )
    .fetch_all(&st.pool)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| ArchivedRow {
                id: r.id,
                symbol: r.symbol,
                name: r.name,
                class: r.class,
                fills: r.fills,
                decisions: r.decisions,
                last_fill: r.last_fill,
            })
            .collect(),
    ))
}

async fn instrument_id_by_symbol(pool: &SqlitePool, symbol: &str) -> Result<i64, ApiError> {
    let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM instruments WHERE symbol = ?")
        .bind(symbol)
        .fetch_optional(pool)
        .await?;
    row.map(|r| r.0)
        .ok_or_else(|| ApiError::NotFound(format!("no instrument {symbol}")))
}

async fn load_positions(
    pool: &SqlitePool,
    mode: crate::models::Mode,
) -> Result<Vec<PositionRow>, ApiError> {
    let rows: Vec<PositionDbRow> = sqlx::query_as(
        "SELECT p.id AS id, i.symbol AS symbol, i.name AS name, i.class AS class, p.qty AS qty,
                p.avg_cost AS avg_cost, p.stop AS stop, p.target AS target, p.state AS state,
                (SELECT b.close FROM bars b WHERE b.instrument_id = p.instrument_id
                 ORDER BY b.ts DESC LIMIT 1) AS ltp
         FROM positions p JOIN instruments i ON i.id = p.instrument_id
         WHERE p.mode = ? AND i.active = 1 ORDER BY p.id DESC",
    )
    .bind(mode.as_str())
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let class = AssetClass::parse(&r.class).unwrap_or(AssetClass::Stock);
            PositionRow {
                id: r.id,
                symbol: r.symbol,
                name: r.name,
                class: class.as_str().to_string(),
                qty: r.qty,
                avg_cost: r.avg_cost,
                ltp: r.ltp,
                value: r.ltp.map(|price| price * r.qty),
                stop: r.stop,
                target: r.target,
                state: r.state,
                timing_note: class.timing_note().to_string(),
            }
        })
        .collect())
}

#[derive(Debug, Deserialize)]
pub struct ApproveBody {
    /// Defaults to what the decision proposed; edit it to what you actually got.
    pub qty: Option<f64>,
    pub price: Option<f64>,
    pub at: Option<String>,
}

/// Approve a decision by recording the fill you actually got.
///
/// The trade is written with a note naming the decision, so approving the same decision twice is
/// detected rather than doubling your position: the second call only closes the status.
async fn approve_signal(
    State(st): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
    Json(body): Json<ApproveBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct PendingDecision {
        instrument_id: i64,
        action: String,
        qty: Option<f64>,
        entry: Option<f64>,
        mode: String,
        status: String,
        kind: Option<String>,
        pair_id: Option<i64>,
    }
    let row: Option<PendingDecision> = sqlx::query_as(
        "SELECT instrument_id, action, qty, entry, mode, status, kind, pair_id
         FROM signals WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&st.pool)
    .await?;
    let Some(PendingDecision {
        instrument_id,
        action,
        qty,
        entry,
        mode: mode_raw,
        status,
        kind,
        pair_id,
    }) = row
    else {
        return Err(ApiError::NotFound(format!("no decision {id}")));
    };
    if status != "pending" {
        return Err(ApiError::BadRequest(format!(
            "decision {id} is already {status}"
        )));
    }
    let side = match action.as_str() {
        "BUY" => crate::trades::Side::Buy,
        "SELL" => crate::trades::Side::Sell,
        other => {
            return Err(ApiError::BadRequest(format!(
                "a {other} decision has nothing to record"
            )))
        }
    };
    let qty = body.qty.or(qty).filter(|q| *q > 0.0).ok_or_else(|| {
        ApiError::BadRequest("quantity is required — the decision did not propose one".into())
    })?;
    let price = body
        .price
        .or(entry)
        .filter(|p| p.is_finite() && *p > 0.0)
        .ok_or_else(|| ApiError::BadRequest("price is required".into()))?;
    let mode =
        crate::models::Mode::parse(&mode_raw).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let when = crate::trades::parse_when(body.at.as_deref().unwrap_or(""), chrono::Utc::now())
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    // A rotation is approved as a pair, and only if both legs can actually post: the buy is paid
    // for by the sell, so the affordability check has to happen before any money moves.
    let pair = if kind.as_deref() == Some("rotation") {
        let primary = pair_id.unwrap_or(id);
        let sell: Option<(i64, i64, f64, f64, String)> = sqlx::query_as(
            "SELECT id, instrument_id, qty, entry, mode FROM signals
             WHERE id = ? AND action = 'SELL'",
        )
        .bind(primary)
        .fetch_optional(&st.pool)
        .await?;
        let buy: Option<(i64, i64, f64, f64)> = sqlx::query_as(
            "SELECT id, instrument_id, qty, entry FROM signals
             WHERE pair_id = ? AND action = 'BUY' AND status = 'pending'",
        )
        .bind(primary)
        .fetch_optional(&st.pool)
        .await?;
        match (sell, buy) {
            (Some(s), Some(b)) => Some((s, b)),
            _ => {
                return Err(ApiError::BadRequest(
                    "this rotation is missing a leg; it cannot be approved".into(),
                ))
            }
        }
    } else {
        None
    };

    if let Some(((_, _, sell_qty, sell_price, _), (_, _, buy_qty, buy_price))) = pair {
        let proceeds = sell_qty * sell_price;
        let cash = crate::rules::budget::available_in(&st.pool, mode).await?;
        let cost = buy_qty * buy_price;
        if cost > proceeds + cash + 1e-9 {
            return Err(ApiError::BadRequest(format!(
                "the swap does not balance: cost {cost:.2} exceeds proceeds {proceeds:.2} plus cash {cash:.2}"
            )));
        }
    }

    let note = format!("decision {id}");
    let already: Option<(i64,)> =
        sqlx::query_as("SELECT id FROM trades WHERE note = ? AND instrument_id = ?")
            .bind(&note)
            .bind(instrument_id)
            .fetch_optional(&st.pool)
            .await?;

    if already.is_none() {
        crate::trades::record(
            &st.pool,
            instrument_id,
            side,
            qty,
            price,
            Some(&note),
            Some(when),
            mode,
        )
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    }

    sqlx::query(
        "UPDATE signals SET status = 'approved', status_at = ?, filled_qty = ?, filled_price = ?
         WHERE id = ?",
    )
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(qty)
    .bind(price)
    .bind(id)
    .execute(&st.pool)
    .await?;

    if let Some(((sell_id, _, _, _, sell_mode), (buy_id, buy_instrument, buy_qty, buy_price))) =
        pair
    {
        let buy_note = format!("decision {buy_id}");
        let already: Option<(i64,)> = sqlx::query_as("SELECT id FROM trades WHERE note = ?")
            .bind(&buy_note)
            .fetch_optional(&st.pool)
            .await?;
        if already.is_none() {
            let pair_mode = crate::models::Mode::parse(&sell_mode)
                .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            crate::trades::record(
                &st.pool,
                buy_instrument,
                crate::trades::Side::Buy,
                buy_qty,
                buy_price,
                Some(&buy_note),
                Some(when),
                pair_mode,
            )
            .await
            .map_err(|e| ApiError::BadRequest(e.to_string()))?;
        }
        sqlx::query(
            "UPDATE signals SET status = 'approved', status_at = ?, filled_qty = ?, filled_price = ?
             WHERE id = ?",
        )
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(buy_qty)
        .bind(buy_price)
        .bind(buy_id)
        .execute(&st.pool)
        .await?;
        let _ = sell_id;
    }

    Ok(Json(serde_json::json!({
        "id": id,
        "status": "approved",
        "filled_qty": qty,
        "filled_price": price,
        "recorded": already.is_none(),
    })))
}

#[derive(Debug, Deserialize, Default)]
pub struct RejectBody {
    pub reason: Option<String>,
}

async fn reject_signal(
    State(st): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
    Json(body): Json<RejectBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let changed = sqlx::query(
        "UPDATE signals SET status = 'rejected', status_at = ?, status_reason = ?
         WHERE id = ? AND status = 'pending'",
    )
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(
        body.reason
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty()),
    )
    .bind(id)
    .execute(&st.pool)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(ApiError::NotFound(format!("no pending decision {id}")));
    }
    Ok(Json(serde_json::json!({ "id": id, "status": "rejected" })))
}

async fn load_signals(
    pool: &SqlitePool,
    blocked: bool,
    mode: crate::models::Mode,
    status: &str,
) -> Result<Vec<SignalRow>, ApiError> {
    let predicate = if blocked {
        "s.action = 'BLOCKED'"
    } else if status == "everything" {
        // The instrument page's whole record, blocked proposals included: they are decisions too.
        "1 = 1"
    } else {
        // The ledger is a to-do list: pending only, unless a filter asks for history.
        match status {
            "all" => "s.action <> 'BLOCKED'",
            "approved" => "s.action <> 'BLOCKED' AND s.status = 'approved'",
            "rejected" => "s.action <> 'BLOCKED' AND s.status = 'rejected'",
            _ => "s.action <> 'BLOCKED' AND s.status = 'pending'",
        }
    };
    let rows: Vec<SignalDbRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT s.id AS id, i.symbol AS symbol, i.name AS name, i.class AS class,
                s.action AS action,
                s.qty AS qty, s.confidence AS confidence, s.entry AS entry, s.stop AS stop,
                s.target AS target, s.valid_until AS valid_until,
                s.reason_json AS reason_json, s.created_at AS created_at,
                s.status AS status, s.status_reason AS status_reason,
                s.filled_qty AS filled_qty, s.filled_price AS filled_price,
                s.plan_id AS plan_id
         FROM signals s JOIN instruments i ON i.id = s.instrument_id
         WHERE {predicate} AND s.mode = ? ORDER BY s.id DESC LIMIT 50"
    )))
    .bind(mode.as_str())
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let class = AssetClass::parse(&r.class).unwrap_or(AssetClass::Stock);
            let reason: serde_json::Value = r
                .reason_json
                .as_deref()
                .and_then(|j| serde_json::from_str(j).ok())
                .unwrap_or(serde_json::json!({}));
            let mut reasons: Vec<String> = reason
                .get("reasons")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            if let (Some(blocked), Some(cost), Some(available)) = (
                reason.get("blocked").and_then(|v| v.as_str()),
                reason.get("cost").and_then(|v| v.as_f64()),
                reason.get("available").and_then(|v| v.as_f64()),
            ) {
                reasons.push(format!(
                    "{blocked}: cost {cost:.2}, available {available:.2} — short by {:.2}",
                    (cost - available).max(0.0)
                ));
            }
            SignalRow {
                id: r.id,
                symbol: r.symbol,
                name: r.name,
                class: class.as_str().to_string(),
                action: r.action,
                qty: r.qty,
                confidence: r.confidence,
                entry: r.entry,
                stop: r.stop,
                target: r.target,
                valid_until: r.valid_until,
                timing_note: class.timing_note().to_string(),
                reasons,
                status: r.status,
                status_reason: r.status_reason,
                filled_qty: r.filled_qty,
                filled_price: r.filled_price,
                rationale: reason
                    .get("rationale")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                created_at: r.created_at,
                plan_id: r.plan_id,
            }
        })
        .collect())
}
