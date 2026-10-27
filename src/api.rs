use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
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

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub cfg: Config,
    /// AMFI's scheme file is ~3 MB; it is fetched at most every few hours, not per keystroke.
    amfi: Arc<tokio::sync::Mutex<Option<(chrono::DateTime<chrono::Utc>, String)>>>,
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
        let client = crate::market::yahoo::client().ok()?;
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
        .route("/api/instruments", post(add_instrument))
        .route("/api/instruments/{symbol}", get(instrument_detail))
        .route("/api/trades", get(all_trades))
        .route(
            "/api/instruments/{symbol}/trades",
            get(instrument_trades).post(record_trade),
        )
        .route("/api/search", get(search_instruments))
        .route("/api/instruments/{symbol}/analysis", get(analysis))
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
    #[error("committed is {committed}")]
    BelowCommitted { committed: f64 },
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
            ApiError::BelowCommitted { committed } => (
                StatusCode::CONFLICT,
                serde_json::json!({"error": "below_committed", "committed": committed}),
            ),
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

#[derive(Debug, Serialize)]
pub struct SignalRow {
    pub id: i64,
    pub symbol: String,
    pub class: String,
    pub action: String,
    pub qty: Option<f64>,
    pub entry: Option<f64>,
    pub stop: Option<f64>,
    pub target: Option<f64>,
    pub valid_until: Option<String>,
    pub timing_note: String,
    pub reasons: Vec<String>,
    pub rationale: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct PositionRow {
    pub id: i64,
    pub symbol: String,
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
    pub currency_locked: bool,
    pub positions: Vec<PositionRow>,
    pub signals: Vec<SignalRow>,
    pub blocked: Vec<SignalRow>,
}

async fn dashboard(State(st): State<AppState>) -> Result<Json<Dashboard>, ApiError> {
    let s = settings::get(&st.pool).await?;
    let available = s.available;
    let positions = load_positions(&st.pool).await?;
    let signals = load_signals(&st.pool, false).await?;
    let blocked = load_signals(&st.pool, true).await?;
    Ok(Json(Dashboard {
        currency: s.currency,
        budget: s.budget,
        committed: s.committed,
        realised: crate::trades::realised_total(&st.pool).await?,
        available,
        currency_locked: s.currency_locked,
        positions,
        signals,
        blocked,
    }))
}

async fn get_settings(State(st): State<AppState>) -> Result<Json<settings::Settings>, ApiError> {
    Ok(Json(settings::get(&st.pool).await?))
}

#[derive(Debug, Deserialize)]
pub struct PutSettings {
    pub budget: Option<f64>,
    pub currency: Option<String>,
    pub deepseek_key: Option<String>,
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
        },
    )
    .await
    .map_err(|e| match e {
        SettingsError::BelowCommitted { committed } => ApiError::BelowCommitted { committed },
        SettingsError::CurrencyLocked => ApiError::CurrencyLocked,
        SettingsError::InvalidAmount => ApiError::BadRequest("invalid amount".into()),
    })?;
    Ok(Json(settings::get(&st.pool).await?))
}

#[derive(Debug, Serialize)]
pub struct InstrumentDetail {
    pub instrument: InstrumentRow,
    pub headlines: Vec<HeadlineRow>,
    pub signals: Vec<SignalRow>,
}

#[derive(Debug, Serialize)]
pub struct InstrumentRow {
    pub id: i64,
    pub symbol: String,
    pub class: String,
    pub name: String,
    pub currency: String,
    pub timing_note: String,
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
    let row: Option<(i64, String, String, String, String)> = sqlx::query_as(
        "SELECT id, symbol, class, name, currency FROM instruments WHERE symbol = ?",
    )
    .bind(&symbol)
    .fetch_optional(&st.pool)
    .await?;
    let (id, symbol, class, name, currency) =
        row.ok_or_else(|| ApiError::NotFound(format!("no instrument {symbol}")))?;
    let class = AssetClass::parse(&class).map_err(|_| ApiError::Internal)?;

    let headlines: Vec<HeadlineRow> = sqlx::query_as::<_, (i64, String, Option<String>, String, String)>(
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

    let signals: Vec<SignalRow> = load_signals(&st.pool, false)
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
}

/// "Why is there no signal?" answered with the same predicates the engine used.
async fn analysis(
    State(st): State<AppState>,
    Path(symbol): Path<String>,
) -> Result<Json<Analysis>, ApiError> {
    let row: Option<(i64, String, String, String, String)> = sqlx::query_as(
        "SELECT id, symbol, class, name, currency FROM instruments WHERE symbol = ?",
    )
    .bind(&symbol)
    .fetch_optional(&st.pool)
    .await?;
    let (id, symbol, class_raw, name, currency) =
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

    let position = load_positions(&st.pool)
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

    Ok(Json(Analysis {
        instrument: InstrumentRow {
            id,
            symbol,
            class: class.as_str().to_string(),
            name,
            currency,
            timing_note: class.timing_note().to_string(),
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
    }))
}

#[derive(Debug, Serialize)]
pub struct CallRow {
    pub id: i64,
    pub ts: String,
    pub endpoint: String,
    pub purpose: Option<String>,
    pub round: Option<i64>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub validator_verdict: Option<String>,
    pub error: Option<String>,
}

async fn calls(State(st): State<AppState>) -> Result<Json<Vec<CallRow>>, ApiError> {
    let rows: Vec<(i64, String, String, Option<String>, Option<i64>, Option<i64>, Option<i64>, Option<String>, Option<String>)> =
        sqlx::query_as(
            "SELECT id, ts, endpoint, purpose, round, prompt_tokens, completion_tokens,
                    validator_verdict, error
             FROM api_calls ORDER BY id DESC LIMIT 100",
        )
        .fetch_all(&st.pool)
        .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| CallRow {
                id: r.0,
                ts: r.1,
                endpoint: r.2,
                purpose: r.3,
                round: r.4,
                prompt_tokens: r.5,
                completion_tokens: r.6,
                validator_verdict: r.7,
                error: r.8,
            })
            .collect(),
    ))
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
    if !body.qty.is_finite() || body.qty <= 0.0 || !body.avg_cost.is_finite() || body.avg_cost <= 0.0
    {
        return Err(ApiError::BadRequest("qty and avg_cost must be positive".into()));
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
        },
    )
    .await?;
    let row = load_positions(&st.pool)
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

    load_positions(&st.pool)
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
        return Err(ApiError::BadRequest("qty and price must be positive".into()));
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
    let client = crate::market::yahoo::client()?;
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
            let text = st
                .amfi_text()
                .await
                .ok_or_else(|| ApiError::BadRequest("AMFI scheme list unavailable".into()))?;
            let hit = crate::market::search::amfi_by_code(&text, code)
                .ok_or_else(|| ApiError::NotFound(format!("no scheme {code}")))?;
            (crate::models::AssetClass::MutualFund, hit.name)
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
    Ok(Json(InstrumentRow {
        id: id.0,
        symbol: code.to_string(),
        class: class.as_str().to_string(),
        name,
        currency: "INR".into(),
        timing_note: class.timing_note().to_string(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct TradeBody {
    pub side: String,
    pub qty: f64,
    pub price: Option<f64>,
    pub note: Option<String>,
    /// `YYYY-MM-DD` or RFC3339. Omitted means "now".
    pub at: Option<String>,
}

async fn record_trade(
    State(st): State<AppState>,
    Path(symbol): Path<String>,
    Json(body): Json<TradeBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = instrument_id_by_symbol(&st.pool, &symbol).await?;
    let side = crate::trades::Side::parse(&body.side)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

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

    crate::trades::record(
        &st.pool,
        id,
        side,
        body.qty,
        price,
        body.note.as_deref(),
        Some(when),
    )
    .await
    .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let s = settings::get(&st.pool).await?;
    Ok(Json(serde_json::json!({
        "ok": true,
        "cash": s.available,
        "realised": crate::trades::realised_total(&st.pool).await?,
    })))
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

async fn instrument_id_by_symbol(pool: &SqlitePool, symbol: &str) -> Result<i64, ApiError> {
    let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM instruments WHERE symbol = ?")
        .bind(symbol)
        .fetch_optional(pool)
        .await?;
    row.map(|r| r.0)
        .ok_or_else(|| ApiError::NotFound(format!("no instrument {symbol}")))
}

async fn load_positions(pool: &SqlitePool) -> Result<Vec<PositionRow>, ApiError> {
    let rows: Vec<(i64, String, String, f64, f64, Option<f64>, Option<f64>, String, Option<f64>)> =
        sqlx::query_as(
        "SELECT p.id, i.symbol, i.class, p.qty, p.avg_cost, p.stop, p.target, p.state,
                (SELECT b.close FROM bars b WHERE b.instrument_id = p.instrument_id
                 ORDER BY b.ts DESC LIMIT 1)
         FROM positions p JOIN instruments i ON i.id = p.instrument_id ORDER BY p.id DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let class = AssetClass::parse(&r.2).unwrap_or(AssetClass::Stock);
            PositionRow {
                id: r.0,
                symbol: r.1,
                class: class.as_str().to_string(),
                qty: r.3,
                avg_cost: r.4,
                ltp: r.8,
                value: r.8.map(|p| p * r.3),
                stop: r.5,
                target: r.6,
                state: r.7,
                timing_note: class.timing_note().to_string(),
            }
        })
        .collect())
}

async fn load_signals(pool: &SqlitePool, blocked: bool) -> Result<Vec<SignalRow>, ApiError> {
    let predicate = if blocked {
        "s.action = 'BLOCKED'"
    } else {
        "s.action <> 'BLOCKED'"
    };
    let rows: Vec<(i64, String, String, String, Option<f64>, Option<f64>, Option<f64>, Option<f64>, Option<String>, Option<String>, String)> =
        sqlx::query_as(&format!(
            "SELECT s.id, i.symbol, i.class, s.action, s.qty, s.entry, s.stop, s.target,
                    s.valid_until, s.reason_json, s.created_at
             FROM signals s JOIN instruments i ON i.id = s.instrument_id
             WHERE {predicate} ORDER BY s.id DESC LIMIT 50"
        ))
        .fetch_all(pool)
        .await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let class = AssetClass::parse(&r.2).unwrap_or(AssetClass::Stock);
            let reason: serde_json::Value = r
                .9
                .as_deref()
                .and_then(|j| serde_json::from_str(j).ok())
                .unwrap_or(serde_json::json!({}));
            let reasons = reason
                .get("reasons")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            SignalRow {
                id: r.0,
                symbol: r.1,
                class: class.as_str().to_string(),
                action: r.3,
                qty: r.4,
                entry: r.5,
                stop: r.6,
                target: r.7,
                valid_until: r.8,
                timing_note: class.timing_note().to_string(),
                reasons,
                rationale: reason
                    .get("rationale")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                created_at: r.10,
            }
        })
        .collect())
}
