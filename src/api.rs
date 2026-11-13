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
    /// The loop that owns the market clients listens here, so "Run now" runs exactly what the
    /// scheduler runs instead of a second, differently-wired copy of it.
    pub manual: Option<tokio::sync::mpsc::Sender<(i64, crate::runs::Manual)>>,
    /// AMFI's scheme file is ~3 MB; it is fetched at most every few hours, not per keystroke.
    amfi: AmfiCache,
}

impl AppState {
    pub fn new(pool: SqlitePool, cfg: Config) -> Self {
        Self {
            pool,
            cfg,
            manual: None,
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

/// API responses may be cached for reuse only after revalidation — so the ETag still saves the
/// body, but a changed row is never served from a stale copy.
async fn no_store(mut res: axum::response::Response) -> axum::response::Response {
    res.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-cache, must-revalidate"),
    );
    res
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
        .route("/api/trades/{id}", patch(edit_trade).delete(delete_trade))
        .route("/api/archived", get(archived_instruments))
        .route(
            "/api/instruments/{symbol}/restore",
            post(restore_instrument),
        )
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
        .route("/api/calls/facets", get(call_facets))
        .route("/api/schedule", get(schedule_view))
        .route("/api/market", get(market_state))
        .route("/api/runs", get(list_runs).post(start_run))
        .route("/api/positions", post(post_position))
        .route("/api/positions/{id}", patch(patch_position))
        .route("/api/ipo", post(post_ipo))
        // Every answer must be revalidated: a stale 200 in the browser's cache is exactly the
        // "nothing updates until I hard refresh" report.
        .layer(axum::middleware::map_response(no_store))
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
    target_profit_pct: Option<f64>,
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
    /// The sentence that explains the whole plan, carried on every leg.
    pub plan_rationale: Option<String>,
    /// "stock" or "fund" — the two plans are different arguments, and the card says which.
    pub plan_scope: Option<String>,
    pub amount: Option<f64>,
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
    /// The owner's profit expectation for this instrument. A fund has no price exit, so this is
    /// the only exit it can have: sell at NAV once the return is reached.
    pub target_profit_pct: Option<f64>,
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

    /// The rail's recent fills: one request per page instead of two.
    pub recent_trades: Vec<crate::trades::Trade>,
}

/// Cheap version tag for the dashboard: max signal id, trade id, position count, settings
/// count. A minute-by-minute poll that finds nothing new costs a 304 with no body.
/// A fingerprint of everything the dashboard renders.
///
/// Counting rows was not enough: approving changes no id, editing a fill changes values in place,
/// and archiving flips a flag. Each of those left the tag unchanged, so the page answered 304 and
/// the screen went stale until a hard refresh. The tag now hashes the *data*, not just its size.
async fn dashboard_etag(pool: &SqlitePool, mode: crate::models::Mode) -> Result<String, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Fingerprint {
        signals: i64,
        pending: i64,
        trades: i64,
        ledger: i64,
        positions: String,
        instruments: i64,
        settings: String,
    }
    let row: Fingerprint = sqlx::query_as(
        "SELECT (SELECT COALESCE(MAX(id),0) FROM signals WHERE mode = ?) AS signals,
                (SELECT COUNT(*) FROM signals WHERE mode = ? AND status = 'pending') AS pending,
                (SELECT COALESCE(MAX(id),0) FROM trades WHERE mode = ?) AS trades,
                (SELECT COALESCE(MAX(id),0) FROM cash_ledger WHERE mode = ?) AS ledger,
                (SELECT COALESCE(group_concat(id || ':' || qty || ':' || avg_cost || ':'
                         || COALESCE(stop, '') || ':' || COALESCE(target, '') || ':' || state), '')
                   FROM positions WHERE mode = ?) AS positions,
                (SELECT COUNT(*) FROM instruments WHERE active = 1) AS instruments,
                (SELECT COALESCE(group_concat(key || '=' || value), '') FROM settings) AS settings",
    )
    .bind(mode.as_str())
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
            "{}-{}-{}-{}-{}-{}-{}-{}",
            row.signals,
            row.pending,
            row.trades,
            row.ledger,
            row.positions,
            row.instruments,
            row.settings,
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
    let signals = load_signals(&st.pool, mode, status).await?;

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
    pub daily_cap: Option<u32>,
    pub scan_min_price: Option<f64>,
    pub scan_min_turnover: Option<f64>,
    pub scan_locked_pct: Option<f64>,
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
            daily_cap: body.daily_cap,
            scan_min_price: body.scan_min_price,
            scan_min_turnover: body.scan_min_turnover,
            scan_locked_pct: body.scan_locked_pct,
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
    let archived: Option<(Option<String>,)> =
        sqlx::query_as("SELECT archived_at FROM instruments WHERE id = ?")
            .bind(id)
            .fetch_optional(&st.pool)
            .await?;
    if archived.and_then(|r| r.0).is_some() {
        return Err(ApiError::BadRequest(format!(
            "{symbol} is already archived"
        )));
    }
    let _ = active;

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
    // `archived_at` is the record of a decision; `active = 0` alone is also what discovery writes.
    sqlx::query("UPDATE instruments SET active = 0, archived_at = ? WHERE id = ?")
        .bind(chrono::Utc::now().to_rfc3339())
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

/// The instrument row both the detail page and the analysis payload need, in one place so the two
/// cannot drift.
#[derive(sqlx::FromRow)]
struct InstrumentRecord {
    id: i64,
    symbol: String,
    class: String,
    name: String,
    currency: String,
    target_profit_pct: Option<f64>,
    active: i64,
    /// Null unless a person archived it: discovery writes `active = 0` too.
    archived_at: Option<String>,
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
    /// False when the instrument has been archived: the page then offers a way back, and says
    /// that the prices it is showing are the last ones before it left.
    pub active: bool,
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
    let row: Option<InstrumentRecord> = sqlx::query_as(
        "SELECT id, symbol, class, name, currency, target_profit_pct, active, archived_at
         FROM instruments WHERE symbol = ?",
    )
    .bind(&symbol)
    .fetch_optional(&st.pool)
    .await?;
    let record = row.ok_or_else(|| ApiError::NotFound(format!("no instrument {symbol}")))?;
    let InstrumentRecord {
        id,
        symbol,
        name,
        currency,
        target_profit_pct,
        active,
        archived_at,
        ..
    } = record;
    let _ = active;
    let archived = archived_at.is_some();
    let class = AssetClass::parse(&record.class).map_err(|_| ApiError::Internal)?;

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
        load_signals(&st.pool, crate::models::Mode::Long, "everything")
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
            // The page asks "is there a way back?", which only an archive decision answers.
            active: !archived,
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
    let row: Option<InstrumentRecord> = sqlx::query_as(
        "SELECT id, symbol, class, name, currency, target_profit_pct, active, archived_at
         FROM instruments WHERE symbol = ?",
    )
    .bind(&symbol)
    .fetch_optional(&st.pool)
    .await?;
    let record = row.ok_or_else(|| ApiError::NotFound(format!("no instrument {symbol}")))?;
    let InstrumentRecord {
        id,
        symbol,
        name,
        currency,
        target_profit_pct,
        active,
        archived_at,
        ..
    } = record;
    let _ = active;
    let archived = archived_at.is_some();
    let class = AssetClass::parse(&record.class).map_err(|_| ApiError::Internal)?;

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
        active: active != 0,
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
        load_signals(&st.pool, crate::models::Mode::Long, "everything")
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
            // The page asks "is there a way back?", which only an archive decision answers.
            active: !archived,
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
    /// How many moves a book call produced. Zero for every other kind of call.
    pub plan_legs: i64,
    /// The prompt template that was sent — decide_v1, plan_stock_v1, short_v1.
    pub model: Option<String>,
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
    plan_legs: i64,
    prompt_template: Option<String>,
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
                COALESCE(i.name, pi.name) AS instrument_name,
                COALESCE(i.symbol, pi.symbol) AS instrument_symbol,
                (SELECT COUNT(*) FROM signals p
                  WHERE p.conversation_id = a.conversation_id AND p.kind = 'plan') AS plan_legs
         FROM api_calls a
         LEFT JOIN signals d ON d.id = (
             SELECT s.id FROM signals s WHERE s.conversation_id = a.conversation_id
             ORDER BY s.id DESC LIMIT 1)
         LEFT JOIN instruments i ON i.id = d.instrument_id
         -- A call with no decision still names a holding in its purpose, after the verb.
         LEFT JOIN instruments pi ON pi.symbol = trim(substr(a.purpose, instr(a.purpose, ' ') + 1))
         WHERE a.id = ?",
    )
    .bind(id)
    .fetch_optional(&st.pool)
    .await?;
    let Some(r) = row else {
        return Err(ApiError::NotFound(format!("no call {id}")));
    };
    // A reply names its moves by symbol; the scheme code says nothing.
    let named: Vec<(String, String)> = sqlx::query_as(
        "SELECT i.symbol, i.name FROM signals s JOIN instruments i ON i.id = s.instrument_id
         WHERE s.conversation_id = (SELECT conversation_id FROM api_calls WHERE id = ?)
           AND s.kind = 'plan'",
    )
    .bind(id)
    .fetch_all(&st.pool)
    .await?;
    let names: std::collections::HashMap<String, String> = named.into_iter().collect();
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
        "plan_names": names,
    })))
}

/// One page of the audit list. The total travels in a header, so the body stays a plain array
/// and the page size is the only thing a client has to know.
#[derive(Debug, Deserialize)]
pub struct CallsQuery {
    /// Offset/limit is the primitive; page/per_page is the same thing in friendlier clothes.
    pub offset: Option<u32>,
    pub limit: Option<u32>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
    /// Filters. Server-side, because the list is paged: filtering a page of 25 in the browser
    /// would filter the page, not the record.
    /// The prompt verb: decide, plan, rotate, escalate.
    pub prompt: Option<String>,
    /// The exchange channel: chat, chat+search.
    pub call: Option<String>,
    /// The prompt template that was sent: decide_v1, plan_stock_v1…
    pub model: Option<String>,
    /// pending | approved | rejected | blocked | none.
    pub review: Option<String>,
}

/// The reading, with the clock and the data passed in so it can be tested on any day.
pub fn market_state_for(
    weekday: bool,
    minutes: u32,
    priced_today: bool,
) -> (&'static str, &'static str) {
    if !weekday {
        return ("closed", "Weekend");
    }
    if !(9 * 60 + 15..=15 * 60 + 30).contains(&minutes) {
        return (
            "closed",
            if minutes < 9 * 60 + 15 {
                "Before the open"
            } else {
                "After the close"
            },
        );
    }
    if priced_today {
        ("open", "Trading")
    } else {
        // In session, a weekday, and nothing has priced today.
        (
            "holiday",
            "No prices recorded today — the exchange is probably closed for a holiday",
        )
    }
}

/// Is the exchange trading right now?
///
/// There is no holiday calendar in this app, so "holiday" is a *reading of the data*: a weekday
/// inside session hours with no prices recorded today. That is worth saying out loud — the
/// alternative is to claim "open" while every poll returns yesterday's close.
async fn market_state(State(st): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    use chrono::{Datelike, Timelike};
    let now = chrono::Utc::now();
    let ist = now + chrono::Duration::minutes(330);
    let weekday = !matches!(ist.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun);
    let minutes = ist.hour() * 60 + ist.minute();
    // The exchange trades 09:15–15:30 IST; the poll window is the wider 09:00–16:00.

    // The freshest bar anyone has. If it is not from today, no trades were recorded today.
    let latest: Option<String> = sqlx::query_scalar("SELECT MAX(ts) FROM bars")
        .fetch_one(&st.pool)
        .await?;
    let latest_day = latest
        .as_ref()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|d| (d + chrono::Duration::minutes(330)).date_naive());
    let today = ist.date_naive();
    let priced_today = latest_day == Some(today);

    let (state, reason) = market_state_for(weekday, minutes, priced_today);
    // When it next opens, and when it closes if it is open now. A holiday is not a calendar this
    // app has, so "next open" is the next weekday's 09:15 — the tooltip says as much.
    let to_utc = |d: chrono::NaiveDate, hour: u32, minute: u32| -> Option<chrono::DateTime<chrono::Utc>> {
        let naive = d.and_hms_opt(hour, minute, 0)?;
        Some(
            chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(naive, chrono::Utc)
                - chrono::Duration::minutes(330),
        )
    };
    let next_open_at = {
        let mut day = ist.date_naive();
        let mut found = None;
        for _ in 0..9 {
            let weekday = !matches!(chrono::Datelike::weekday(&day), chrono::Weekday::Sat | chrono::Weekday::Sun);
            if weekday {
                if let Some(at) = to_utc(day, 9, 15) {
                    if at > now {
                        found = Some(at);
                        break;
                    }
                }
            }
            day = day.succ_opt().unwrap_or(day);
        }
        found.map(|d| d.to_rfc3339())
    };
    let closes_at = if state == "open" {
        to_utc(ist.date_naive(), 15, 30).map(|d| d.to_rfc3339())
    } else {
        None
    };

    Ok(Json(serde_json::json!({
        "state": state,
        "reason": reason,
        "as_of": now.to_rfc3339(),
        "session": "09:15–15:30 IST, Mon–Fri",
        "last_bar_at": latest,
        "next_open_at": next_open_at,
        "closes_at": closes_at,
        "holiday_calendar": false,
    })))
}

/// The schedule, computed where the rules are: the daily markers, the NSE window and the poll
/// cadence all live on this side, so the page does not re-implement three rules and drift.
async fn schedule_view(State(st): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let runs = crate::runs::latest_by_prompt(&st.pool).await?;
    let latest = |prompt: &str| runs.iter().find(|r| r.prompt == prompt);
    let now = chrono::Utc::now();
    let today = ist_day(now);
    let marker = |key: &'static str| {
        let pool = st.pool.clone();
        async move {
            let row: Option<(String,)> =
                sqlx::query_as("SELECT value FROM settings WHERE key = ?")
                    .bind(key)
                    .fetch_optional(&pool)
                    .await
                    .ok()
                    .flatten();
            row.map(|r| r.0)
        }
    };

    // A daily prompt is due at the first tick after IST midnight; if it has not run today it is
    // due now. A session prompt is due on the next tick inside 09:00–16:00 IST, Mon–Fri.
    let daily_next = |ran_today: bool| -> Option<String> {
        let ist = now + chrono::Duration::minutes(330);
        let next_midnight = (ist.date_naive() + chrono::Days::new(1))
            .and_hms_opt(0, 5, 0)
            .map(|d| d.and_utc() - chrono::Duration::minutes(330));
        if ran_today {
            next_midnight.map(|d| d.to_rfc3339())
        } else {
            Some(now.to_rfc3339()) // due now
        }
    };

    let stock_ran = marker("stock_plan_last").await.as_deref() == Some(today.as_str());
    let fund_ran = marker("mf_plan_last").await.as_deref() == Some(today.as_str());

    let entry = |prompt: &'static str, label: &'static str, rule: &'static str, next: Option<String>| {
        let last = latest(prompt);
        serde_json::json!({
            "prompt": prompt,
            "label": label,
            "rule": rule,
            "next_due_at": next,
            "last": last.map(|r| serde_json::json!({
                "status": r.status,
                "trigger": r.trigger,
                "at": r.started_at,
                "finished_at": r.finished_at,
                "summary": r.summary,
                "error": r.error,
            })),
        })
    };

    Ok(Json(serde_json::json!({
        "now": now.to_rfc3339(),
        "prompts": [
            entry(
                "plan_stock",
                "Stock plan",
                "once a day, first tick after midnight IST",
                daily_next(stock_ran),
            ),
            entry(
                "plan_fund",
                "Fund plan",
                "once a day, after the fund scan",
                daily_next(fund_ran),
            ),
            entry(
                "plan_trade",
                "Trade plan",
                "every two hours in session — 09:15, 11:15, 13:15 IST",
                trade_next(now),
            ),
        ],
    })))
}

/// The next trade slot, from the same function the scheduler uses, so the card cannot promise a
/// cadence the loop does not keep.
fn trade_next(now: chrono::DateTime<chrono::Utc>) -> Option<String> {
    // Walk forward in minutes until a slot appears: at most a day and a half of ticks.
    for step in 0..(60 * 40) {
        let at = now + chrono::Duration::minutes(step);
        if crate::schedule::trade_slot(at).is_some() && step > 0 {
            return Some(at.to_rfc3339());
        }
    }
    None
}

/// `YYYY-MM-DD` of the current IST day — the unit the daily markers are written in.
fn ist_day(now: chrono::DateTime<chrono::Utc>) -> String {
    use chrono::Datelike;
    let ist = now + chrono::Duration::minutes(330);
    format!("{:04}-{:02}-{:02}", ist.year(), ist.month(), ist.day())
}

/// The distinct values the filters offer, so the page never hard-codes a list that drifts.
async fn call_facets(State(st): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let distinct = |sql: &'static str| {
        let pool = st.pool.clone();
        async move {
            let rows: Vec<(Option<String>,)> = sqlx::query_as(sql).fetch_all(&pool).await?;
            let out: Vec<String> = rows.into_iter().filter_map(|r| r.0).collect();
            Ok::<_, sqlx::Error>(out)
        }
    };
    Ok(Json(serde_json::json!({
        "prompts": distinct(
            "SELECT DISTINCT trim(substr(purpose, 1, instr(purpose, ' ') - 1)) AS v
             FROM api_calls WHERE purpose IS NOT NULL AND instr(purpose, ' ') > 0 ORDER BY v",
        )
        .await?,
        "calls": distinct("SELECT DISTINCT endpoint FROM api_calls ORDER BY endpoint").await?,
        "models": distinct(
            "SELECT DISTINCT prompt_template FROM api_calls
             WHERE prompt_template IS NOT NULL ORDER BY prompt_template",
        )
        .await?,
    })))
}

pub const CALLS_PER_PAGE: u32 = 25;

#[derive(Debug, Deserialize)]
pub struct RunQuery {
    pub limit: Option<i64>,
    /// One run per prompt — what the audit strip shows, so a prompt that last ran yesterday is
    /// not hidden behind a day of polls.
    pub latest: Option<bool>,
}

/// The runs the scheduler (or you) started, newest first. `running` tells the page whether it is
/// worth refreshing quickly.
async fn list_runs(
    State(st): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<RunQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let rows = if q.latest.unwrap_or(false) {
        crate::runs::latest_by_prompt(&st.pool).await?
    } else {
        crate::runs::recent(&st.pool, limit).await?
    };
    let running = crate::runs::running(&st.pool).await?;
    Ok(Json(
        serde_json::json!({ "runs": rows, "running": running }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct StartRunBody {
    pub prompt: String,
    /// For `decide`: the one instrument to ask about.
    pub symbol: Option<String>,
}

/// Run something now. The row is opened here, before the work, so the page can show it running
/// even if the loop is asleep; the loop closes it when the work ends.
async fn start_run(
    State(st): State<AppState>,
    Json(body): Json<StartRunBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let prompt = match body.prompt.as_str() {
        "poll" => crate::runs::Prompt::Poll,
        "plan_stock" => crate::runs::Prompt::PlanStock,
        "plan_fund" => crate::runs::Prompt::PlanFund,
        "plan_trade" => crate::runs::Prompt::PlanTrade,
        "decide" => crate::runs::Prompt::Decide,
        other => {
            return Err(ApiError::BadRequest(format!(
                "\"{other}\" is not something that can be run"
            )))
        }
    };
    let symbol = body
        .symbol
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if prompt == crate::runs::Prompt::Decide && symbol.is_none() {
        return Err(ApiError::BadRequest(
            "decide needs a symbol — it asks about one instrument".into(),
        ));
    }
    if let Some(sym) = symbol {
        // Fail before writing a run row for an instrument that does not exist.
        instrument_id_by_symbol(&st.pool, sym).await?;
    }
    let Some(tx) = &st.manual else {
        return Err(ApiError::BadRequest(
            "this server is not running its poll loop, so nothing can be started".into(),
        ));
    };
    let id = crate::runs::start(&st.pool, prompt, crate::runs::Trigger::Manual, symbol).await?;
    let manual = match prompt {
        crate::runs::Prompt::Poll => crate::runs::Manual::Poll,
        crate::runs::Prompt::PlanStock => crate::runs::Manual::PlanStock,
        crate::runs::Prompt::PlanTrade => crate::runs::Manual::PlanTrade,
        crate::runs::Prompt::PlanFund => crate::runs::Manual::PlanFund,
        _ => crate::runs::Manual::Decide {
            symbol: symbol.unwrap_or_default().to_string(),
        },
    };
    tx.send((id, manual))
        .await
        .map_err(|_| ApiError::BadRequest("the poll loop is not listening".into()))?;
    Ok(Json(serde_json::json!({ "id": id, "status": "running" })))
}

async fn calls(
    State(st): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<CallsQuery>,
) -> Result<axum::response::Response, ApiError> {
    let limit = q
        .limit
        .or(q.per_page)
        .unwrap_or(CALLS_PER_PAGE)
        .clamp(1, 200);
    let offset = q
        .offset
        .unwrap_or_else(|| (q.page.unwrap_or(1).max(1) - 1).saturating_mul(limit));

    // Filters are bound parameters and the clause text is fixed, so a request can narrow the
    // record but never rewrite the statement.
    let given = |v: &Option<String>| -> Option<String> {
        v.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    // Each filter is a clause plus, sometimes, one bound value. QueryBuilder writes the
    // placeholder itself, so the SQL text never contains a `?`.
    let mut filters: Vec<(&'static str, Option<String>)> = Vec::new();
    if let Some(v) = given(&q.prompt) {
        filters.push((
            " AND (CASE WHEN instr(a.purpose, ' ') > 0
                   THEN substr(a.purpose, 1, instr(a.purpose, ' ') - 1)
                   ELSE a.purpose END) = ",
            Some(v),
        ));
    }
    if let Some(v) = given(&q.call) {
        filters.push((" AND a.endpoint = ", Some(v)));
    }
    if let Some(v) = given(&q.model) {
        filters.push((" AND a.prompt_template = ", Some(v)));
    }
    match given(&q.review).as_deref() {
        Some("none") => filters.push((" AND d.id IS NULL", None)),
        Some("blocked") => filters.push((" AND d.action = 'BLOCKED'", None)),
        Some(status) => filters.push((" AND d.status = ", Some(status.to_string()))),
        None => {}
    }

    // The decision join is shared by the count and the page, so "total" always matches the filter.
    const JOINS: &str = " FROM api_calls a
         LEFT JOIN signals d ON d.id = (
             SELECT s.id FROM signals s WHERE s.conversation_id = a.conversation_id
             ORDER BY s.id DESC LIMIT 1)";

    // The `WHERE 1 = 1` anchor matters: without it the first filter is appended to the JOIN's ON
    // clause, where it silently stops filtering anything.
    let mut count_q =
        sqlx::QueryBuilder::<sqlx::Sqlite>::new(format!("SELECT COUNT(*){JOINS} WHERE 1 = 1"));
    for (sql, bind) in &filters {
        count_q.push(sql);
        if let Some(value) = bind {
            count_q.push_bind(value.clone());
        }
    }
    let total: i64 = count_q.build_query_scalar().fetch_one(&st.pool).await?;

    let mut page_q = sqlx::QueryBuilder::<sqlx::Sqlite>::new(format!(
        "SELECT a.id AS id, a.ts AS ts, a.endpoint AS endpoint, a.purpose AS purpose,
                a.round AS round, a.prompt_tokens AS prompt_tokens,
                a.completion_tokens AS completion_tokens,
                a.validator_verdict AS validator_verdict, a.error AS error,
                a.trigger AS trigger, a.prompt_template AS prompt_template,
                d.id AS decision_id, d.status AS decision_status,
                d.action AS decision_action, d.reason_json AS decision_reason,
                COALESCE(i.name, pi.name) AS instrument_name,
                COALESCE(i.symbol, pi.symbol) AS instrument_symbol,
                (SELECT COUNT(*) FROM signals p
                  WHERE p.conversation_id = a.conversation_id AND p.kind = 'plan') AS plan_legs
         {JOINS}
         LEFT JOIN instruments i ON i.id = d.instrument_id
         -- A call with no decision still names a holding in its purpose, after the verb.
         LEFT JOIN instruments pi ON pi.symbol = trim(substr(a.purpose, instr(a.purpose, ' ') + 1))
         WHERE 1 = 1"
    ));
    for (sql, bind) in &filters {
        page_q.push(sql);
        if let Some(value) = bind {
            page_q.push_bind(value.clone());
        }
    }
    page_q.push(" ORDER BY a.id DESC LIMIT ");
    page_q.push_bind(limit as i64);
    page_q.push(" OFFSET ");
    page_q.push_bind(offset as i64);
    let rows: Vec<CallDbRow> = page_q.build_query_as().fetch_all(&st.pool).await?;

    let body: Json<Vec<CallRow>> = Json(
        rows.into_iter()
            .map(|r| CallRow {
                id: r.id,
                ts: r.ts,
                endpoint: r.endpoint,
                decision_action: if r.plan_legs > 0 && r.decision_action.as_deref() != Some("BLOCKED")
                {
                    Some("PLAN".to_string())
                } else {
                    r.decision_action.clone()
                },
                decision_reason: blocked_reason(r.decision_reason.as_deref()),
                instrument_name: if r.plan_legs > 0 {
                    None
                } else {
                    r.instrument_name.clone()
                },
                instrument_symbol: if r.plan_legs > 0 {
                    None
                } else {
                    r.instrument_symbol.clone()
                },
                plan_legs: r.plan_legs,
                model: r.prompt_template,
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
    );
    // The count travels beside the body: the client needs it for the pager, not for the rows.
    Ok((
        [
            ("x-total-count", total.to_string()),
            ("x-offset", offset.to_string()),
            ("x-limit", limit.to_string()),
        ],
        body,
    )
        .into_response())
}

/// The sentence a blocked row already carries, pulled out of its JSON for the audit table.
fn blocked_reason(raw: Option<&str>) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(raw?).ok()?;
    let kind = v.get("blocked")?.as_str()?;
    let cost = v.get("cost").and_then(|c| c.as_f64());
    let available = v.get("available").and_then(|c| c.as_f64());
    // A refusal names what was wrong; a shortfall names the money it needed.
    if let Some(detail) = v.get("detail").and_then(|d| d.as_str()) {
        return Some(format!("{kind}: {detail}"));
    }
    if let Some(first) = v
        .get("reasons")
        .and_then(|r| r.as_array())
        .and_then(|a| a.first())
        .and_then(|s| s.as_str())
    {
        return Some(first.to_string());
    }
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

/// Adding is an act of intent, so it makes the row active — including a row the short-term scan
/// already created as a candidate. A genuinely archived instrument is not revived by it: the way
/// back is its own page, which says so.
pub async fn add_or_promote(
    pool: &SqlitePool,
    code: &str,
    class: &str,
    name: &str,
) -> Result<i64, ApiError> {
    let before: Option<(Option<String>,)> =
        sqlx::query_as("SELECT archived_at FROM instruments WHERE symbol = ?")
            .bind(code)
            .fetch_optional(pool)
            .await?;
    if let Some((Some(_),)) = before {
        return Err(ApiError::BadRequest(format!(
            "{code} is archived — restore it from its own page"
        )));
    }
    sqlx::query(
        "INSERT INTO instruments (symbol, class, name, currency, active)
         VALUES (?, ?, ?, 'INR', 1)
         ON CONFLICT(symbol) DO UPDATE SET active = 1",
    )
    .bind(code)
    .bind(class)
    .bind(name)
    .execute(pool)
    .await?;
    let id: (i64,) = sqlx::query_as("SELECT id FROM instruments WHERE symbol = ?")
        .bind(code)
        .fetch_one(pool)
        .await?;
    Ok(id.0)
}

async fn add_instrument(
    State(st): State<AppState>,
    Json(body): Json<AddInstrument>,
) -> Result<Json<InstrumentRow>, ApiError> {
    let code = body.code.trim();
    if code.is_empty() {
        return Err(ApiError::BadRequest("code is required".into()));
    }

    // The book is one currency and one exchange's rules. A foreign listing would be priced in its
    // own money and read here as rupees, which is how a $2.17 ADR produced a +11,000% "return".
    let indian_listing = code.ends_with(".NS")
        || code.ends_with(".BO")
        || body.class.as_deref() == Some("mutual_fund");
    if !indian_listing {
        return Err(ApiError::BadRequest(format!(
            "{code} is not an Indian listing — add an NSE (.NS) or BSE (.BO) symbol, or a fund by its AMFI code"
        )));
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

    // `OR IGNORE` means an existing row wins — and the short-term scan creates rows for every name
    // it notices, with `active = 0`. Adding one of those must promote it, or the instrument is
    // created and immediately looks archived. A genuinely archived name is not revived silently.
    let id = add_or_promote(&st.pool, code, class.as_str(), &name).await?;

    // A newly added symbol should have a chart and headlines on first open, not tomorrow
    // morning. AMFI scheme codes have no Yahoo series; a Yahoo-coded fund (0P…BO) does.
    if class == crate::models::AssetClass::Stock || code.contains('.') {
        if let Ok(market) =
            crate::market::YahooSource::new(&crate::rules::settings::runtime(&st.pool).await?)
        {
            let fetch = crate::store::refresh(&st.pool, id, code, &market);
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
        id,
        symbol: code.to_string(),
        class: class.as_str().to_string(),
        name,
        currency: "INR".into(),
        timing_note: class.timing_note().to_string(),
        target_profit_pct: None,
        active: true,
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

/// Put an archived instrument back in the book. Its prices were cleared on archive, so the next
/// poll fetches them again — the fills and decisions were never touched.
async fn restore_instrument(
    State(st): State<AppState>,
    axum::extract::Path(symbol): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = instrument_id_by_symbol(&st.pool, &symbol).await?;
    let changed = sqlx::query(
        "UPDATE instruments SET active = 1, archived_at = NULL WHERE id = ? AND archived_at IS NOT NULL",
    )
        .bind(id)
        .execute(&st.pool)
        .await?
        .rows_affected();
    if changed == 0 {
        return Err(ApiError::BadRequest(format!(
            "{symbol} is not archived — there is nothing to restore"
        )));
    }
    Ok(Json(serde_json::json!({ "symbol": symbol, "restored": true })))
}

/// Remove a fill: the position and the ledger move back, and the decision it satisfied reopens.
async fn delete_trade(
    State(st): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    crate::trades::delete(&st.pool, id)
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    Ok(Json(serde_json::json!({ "id": id, "deleted": true })))
}

#[derive(Debug, Deserialize, Default)]
pub struct EditTradeBody {
    pub qty: Option<f64>,
    pub price: Option<f64>,
    pub at: Option<String>,
}

/// Correct a recorded fill: a typo in the quantity, a price you read wrong. The position and the
/// ledger move by the difference; history is not rewritten, a correction is posted.
async fn edit_trade(
    State(st): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
    Json(body): Json<EditTradeBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let when = match body.at.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => Some(
            crate::trades::parse_when(raw, chrono::Utc::now())
                .map_err(|e| ApiError::BadRequest(e.to_string()))?,
        ),
        None => None,
    };
    crate::trades::edit(&st.pool, id, body.qty, body.price, when)
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    Ok(Json(serde_json::json!({ "id": id, "edited": true })))
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
    /// What the round trip actually made, from the fills that were kept: sells carry a realised
    /// figure, buys do not. Archived instruments have no price, so this is the only P&L left.
    pub realised: f64,
    /// Money in minus money out: what the round trip still has tied up. Negative means the fills
    /// returned more than they took, which a realised figure alone would not show.
    pub net_invested: f64,
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
        realised: f64,
        net_invested: f64,
    }
    let rows: Vec<ArchivedDbRow> = sqlx::query_as(
        "SELECT i.id, i.symbol, i.name, i.class,
                (SELECT COUNT(*) FROM trades t WHERE t.instrument_id = i.id) AS fills,
                (SELECT COUNT(*) FROM signals s WHERE s.instrument_id = i.id) AS decisions,
                (SELECT MAX(t.traded_at) FROM trades t WHERE t.instrument_id = i.id) AS last_fill,
                -- CAST: SQLite stores 300.0 as INTEGER when it has no fraction, and sqlx will
                -- not decode an INTEGER into f64. The cast costs nothing and cannot lose a row.
                (SELECT CAST(COALESCE(SUM(t.realised), 0) AS REAL) FROM trades t
                  WHERE t.instrument_id = i.id) AS realised,
                (SELECT CAST(COALESCE(SUM(CASE t.side WHEN 'buy' THEN t.qty * t.price
                                                       ELSE -t.qty * t.price END), 0) AS REAL)
                   FROM trades t WHERE t.instrument_id = i.id) AS net_invested
         FROM instruments i
         WHERE i.archived_at IS NOT NULL
         ORDER BY COALESCE(last_fill, i.archived_at) DESC",
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
                realised: r.realised,
                net_invested: r.net_invested,
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
                p.avg_cost AS avg_cost, p.stop AS stop, p.target AS target,
                i.target_profit_pct AS target_profit_pct, p.state AS state,
                (SELECT b.close FROM bars b WHERE b.instrument_id = p.instrument_id
                 ORDER BY b.ts DESC LIMIT 1) AS ltp
         FROM positions p JOIN instruments i ON i.id = p.instrument_id
         WHERE p.mode = ? AND i.active = 1 AND p.state = 'open' ORDER BY p.id DESC",
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
                target_profit_pct: r.target_profit_pct,
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
    /// A plan is several fills: one entry per leg, each with its own quantity and price.
    #[serde(default)]
    pub fills: Vec<LegFill>,
}

#[derive(Debug, Deserialize, Default)]
pub struct LegFill {
    pub id: i64,
    pub qty: Option<f64>,
    pub price: Option<f64>,
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
    // A plan is approved as a whole, or not at all — the same bargain a rotation already makes.
    if let Some(plan) = plan_of(&st.pool, id).await? {
        return approve_plan(&st, id, plan, &body).await;
    }
    #[derive(sqlx::FromRow)]
    struct PendingDecision {
        instrument_id: i64,
        action: String,
        qty: Option<f64>,
        entry: Option<f64>,
        stop: Option<f64>,
        target: Option<f64>,
        mode: String,
        status: String,
        kind: Option<String>,
        pair_id: Option<i64>,
    }
    let row: Option<PendingDecision> = sqlx::query_as(
        "SELECT instrument_id, action, qty, entry, stop, target, mode, status, kind, pair_id
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
        stop: planned_stop,
        target: planned_target,
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
        // The decision's exit plan becomes the position's, or the holding has none.
        apply_exit_plan(&st.pool, instrument_id, mode, planned_stop, planned_target).await?;
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

/// A decision carries the exit plan; approving it must put that plan on the position, or the
/// ledger would show a holding with no stop and no target the moment it was bought.
async fn apply_exit_plan(
    pool: &SqlitePool,
    instrument_id: i64,
    mode: crate::models::Mode,
    stop: Option<f64>,
    target: Option<f64>,
) -> Result<(), ApiError> {
    let mut tx = pool.begin().await?;
    apply_exit_plan_on(&mut tx, instrument_id, mode, stop, target).await?;
    tx.commit().await?;
    Ok(())
}

async fn apply_exit_plan_on(
    conn: &mut sqlx::SqliteConnection,
    instrument_id: i64,
    mode: crate::models::Mode,
    stop: Option<f64>,
    target: Option<f64>,
) -> Result<(), ApiError> {
    if stop.is_none() && target.is_none() {
        return Ok(());
    }
    sqlx::query(
        "UPDATE positions SET stop = COALESCE(?, stop), target = COALESCE(?, target)
         WHERE id = (SELECT id FROM positions
                     WHERE instrument_id = ? AND mode = ? AND state = 'open'
                     ORDER BY id DESC LIMIT 1)",
    )
    .bind(stop)
    .bind(target)
    .bind(instrument_id)
    .bind(mode.as_str())
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// The plan this decision belongs to, when it is one of several moves.
async fn plan_of(pool: &SqlitePool, id: i64) -> Result<Option<i64>, ApiError> {
    let row: Option<(Option<i64>,)> = sqlx::query_as("SELECT plan_id FROM signals WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|r| r.0))
}

#[derive(sqlx::FromRow)]
struct PlanLeg {
    id: i64,
    instrument_id: i64,
    action: String,
    qty: Option<f64>,
    entry: Option<f64>,
    stop: Option<f64>,
    target: Option<f64>,
    mode: String,
}

/// Post every leg, sells first, or none at all. The check is done before any money moves: the
/// plan is only affordable if its own sells pay for its buys.
async fn approve_plan(
    st: &AppState,
    id: i64,
    plan: i64,
    body: &ApproveBody,
) -> Result<Json<serde_json::Value>, ApiError> {
    let legs: Vec<PlanLeg> = sqlx::query_as(
        "SELECT id, instrument_id, action, qty, entry, stop, target, mode FROM signals
         WHERE plan_id = ? AND status = 'pending'
         ORDER BY CASE action WHEN 'SELL' THEN 0 ELSE 1 END, id",
    )
    .bind(plan)
    .fetch_all(&st.pool)
    .await?;
    if legs.is_empty() {
        return Err(ApiError::BadRequest(format!(
            "plan {plan} has no pending moves left"
        )));
    }
    let mode = crate::models::Mode::parse(&legs[0].mode)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    // The fills you are about to post, before any of them is posted: the balance is checked on
    // what you actually got, not on what was proposed.
    let mut planned = Vec::new();
    for leg in &legs {
        let given = body.fills.iter().find(|f| f.id == leg.id);
        let qty = given
            .and_then(|f| f.qty)
            .or(leg.qty)
            .filter(|q| *q > 0.0)
            .ok_or_else(|| ApiError::BadRequest(format!("move {} has no quantity", leg.id)))?;
        let price = given
            .and_then(|f| f.price)
            .or(leg.entry)
            .filter(|p| p.is_finite() && *p > 0.0)
            .ok_or_else(|| ApiError::BadRequest(format!("move {} has no price", leg.id)))?;
        planned.push((leg, qty, price));
    }
    let money = |action: &str| -> f64 {
        planned
            .iter()
            .filter(|(l, _, _)| l.action == action)
            .map(|(_, q, p)| q * p)
            .sum()
    };
    let (proceeds, cost) = (money("SELL"), money("BUY"));
    // A plan draws on the purse its own class belongs to: a fund spends the fund purse.
    let class: Option<(String,)> = sqlx::query_as("SELECT class FROM instruments WHERE id = ?")
        .bind(legs[0].instrument_id)
        .fetch_optional(&st.pool)
        .await?;
    let bucket = class
        .and_then(|(c,)| AssetClass::parse(&c).ok())
        .and_then(crate::rules::budget::bucket_for);
    let cash = match bucket {
        Some(b) => crate::rules::budget::cash_in_bucket(&st.pool, mode, Some(b)).await?,
        None => crate::rules::budget::available_in(&st.pool, mode).await?,
    };
    if cost > proceeds + cash + 1e-9 {
        return Err(ApiError::BadRequest(format!(
            "the plan does not balance: buys cost {cost:.2}, its sells free {proceeds:.2}, cash is {cash:.2}"
        )));
    }

    let when = crate::trades::parse_when(body.at.as_deref().unwrap_or(""), chrono::Utc::now())
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    // One transaction for the whole plan: a leg that cannot post undoes the legs before it.
    let mut tx = st.pool.begin().await?;
    let mut posted = Vec::new();
    for (leg, qty, price) in planned {
        let side = match leg.action.as_str() {
            "BUY" => crate::trades::Side::Buy,
            "SELL" => crate::trades::Side::Sell,
            other => {
                return Err(ApiError::BadRequest(format!(
                    "a {other} move has nothing to record"
                )))
            }
        };
        let note = format!("decision {}", leg.id);
        let already: Option<(i64,)> =
            sqlx::query_as("SELECT id FROM trades WHERE note = ? AND instrument_id = ?")
                .bind(&note)
                .bind(leg.instrument_id)
                .fetch_optional(&mut *tx)
                .await?;
        if already.is_none() {
            crate::trades::record_on(
                &mut tx,
                leg.instrument_id,
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
        .bind(leg.id)
        .execute(&mut *tx)
        .await?;
        // Each leg carries its own exits; the position keeps them for the ledger.
        apply_exit_plan_on(&mut tx, leg.instrument_id, mode, leg.stop, leg.target).await?;
        posted.push(leg.id);
    }
    tx.commit().await?;

    Ok(Json(serde_json::json!({
        "id": id,
        "plan": plan,
        "status": "approved",
        "legs": posted,
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
    // Rejecting one leg of a plan rejects the plan: a plan with half its moves gone is not a plan.
    let plan = plan_of(&st.pool, id).await?;
    let changed = sqlx::query(
        "UPDATE signals SET status = 'rejected', status_at = ?, status_reason = ?
         WHERE status = 'pending' AND (id = ? OR (? IS NOT NULL AND plan_id = ?))",
    )
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(
        body.reason
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty()),
    )
    .bind(id)
    .bind(plan)
    .bind(plan)
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
    mode: crate::models::Mode,
    status: &str,
) -> Result<Vec<SignalRow>, ApiError> {
    let predicate = if status == "everything" {
        // The instrument page's whole record, blocked proposals included: they are decisions too.
        "1 = 1"
    } else {
        // The ledger is a to-do list: only proposals you can act on (BUY/SELL) belong in the box.
        // A blocked or refused proposal is a record, not a decision to review — it stays on the
        // instrument's page and in the audit.
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
            if let Some(detail) = reason.get("detail").and_then(|v| v.as_str()) {
                reasons.push(detail.to_string());
            }
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
                plan_rationale: reason
                    .get("plan_rationale")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                plan_scope: reason
                    .get("scope")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                amount: reason.get("amount").and_then(|v| v.as_f64()),
            }
        })
        .collect())
}
