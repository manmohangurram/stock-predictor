use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use stock_predicter::api;
use stock_predicter::config::Config;
use stock_predicter::db;
use stock_predicter::engine::{self, EngineDeps, YahooSource};
use stock_predicter::llm::deepseek::ReqwestTransport;
use stock_predicter::notify::Ntfy;
use stock_predicter::rules::evaluate::RuleConfig;
use stock_predicter::rules::settings;
use stock_predicter::schedule;
use tokio::net::TcpListener;
use tower_http::services::ServeDir;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cfg = Config::from_env()?;
    let pool = db::connect_and_migrate(&cfg.db_path).await?;
    settings::seed(&pool, cfg.seed_budget, &cfg.seed_currency).await?;

    let state = api::AppState {
        pool: pool.clone(),
        cfg: cfg.clone(),
    };
    let app = api::router(state).fallback_service(ServeDir::new("web/dist"));

    // The poll loop owns its own clients; rebuild them per tick-free lifetime.
    let poll_cfg = cfg.clone();
    let poll_pool = pool.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(poll_cfg.poll_secs.max(30)));
        loop {
            tick.tick().await;
            let now = Utc::now();
            if !(schedule::in_stock_window(now) || schedule::is_daily_pass(now)) {
                continue;
            }
            let market = match YahooSource::new() {
                Ok(m) => m,
                Err(e) => {
                    tracing::error!(error = %e, "market source unavailable");
                    continue;
                }
            };
            let transport = match ReqwestTransport::new(poll_cfg.deepseek_key.clone()) {
                Ok(t) => t,
                Err(e) => {
                    tracing::error!(error = %e, "deepseek client unavailable");
                    continue;
                }
            };
            let notifier = match Ntfy::new(&poll_cfg) {
                Ok(n) => n,
                Err(e) => {
                    tracing::error!(error = %e, "notifier unavailable");
                    continue;
                }
            };
            let deps = EngineDeps {
                pool: &poll_pool,
                cfg: &poll_cfg,
                market: &market,
                transport: &transport,
                notifier: &notifier,
                rules: RuleConfig::default(),
            };
            match engine::poll_once(&deps, now).await {
                Ok(r) => tracing::info!(?r, "poll complete"),
                Err(e) => tracing::error!(error = %e, "poll failed"),
            }
        }
    });

    let listener = TcpListener::bind(&cfg.bind).await?;
    tracing::info!(bind = %cfg.bind, "listening");
    axum::serve(listener, app).await?;
    Ok(())
}
