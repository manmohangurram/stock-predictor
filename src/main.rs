use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use stock_predicter::engine::{self, EngineDeps, YahooSource};
use stock_predicter::llm::deepseek::ReqwestTransport;
use stock_predicter::notify::Ntfy;
use stock_predicter::rules::evaluate::RuleConfig;
use stock_predicter::rules::settings;
use stock_predicter::{api, config::Config, db, schedule};
use tokio::net::TcpListener;
use tower_http::services::{ServeDir, ServeFile};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);

    let cfg = Config::from_env()?;
    let pool = db::connect_and_migrate(&cfg.db_path).await?;
    settings::seed(&pool, cfg.seed_budget, &cfg.seed_currency, &cfg.deepseek_key).await?;

    if flag("--seed") {
        let report = stock_predicter::seed::run(&pool, !flag("--no-positions")).await?;
        println!("seed: {report:?}");
    }

    if flag("--poll-once") || flag("--predict") {
        let market = YahooSource::new()?;
        let key = settings::deepseek_key(&pool).await?.unwrap_or_default();
        let transport = ReqwestTransport::new(key)?;
        let notifier = Ntfy::new(&cfg)?;
        let deps = EngineDeps {
            pool: &pool,
            cfg: &cfg,
            market: &market,
            transport: &transport,
            notifier: &notifier,
            rules: RuleConfig::default(),
            force: flag("--predict"),
        };
        let report = engine::poll_once(&deps, Utc::now()).await?;
        for note in &report.notes {
            println!("DECISION {note}");
        }
        println!("poll: {report:#?}");
        if !flag("--serve") {
            return Ok(());
        }
    }

    let state = api::AppState {
        pool: pool.clone(),
        cfg: cfg.clone(),
    };
    // Client-side routes must not 404 on a direct load or a refresh.
    let app = api::router(state).fallback_service(
        ServeDir::new("web/dist").fallback(ServeFile::new("web/dist/index.html")),
    );

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
            // The key can be changed from the UI at runtime, so it is read per tick.
            let key = settings::deepseek_key(&poll_pool)
                .await
                .ok()
                .flatten()
                .unwrap_or_default();
            let (market, transport, notifier) = match (
                YahooSource::new(),
                ReqwestTransport::new(key),
                Ntfy::new(&poll_cfg),
            ) {
                (Ok(m), Ok(t), Ok(n)) => (m, t, n),
                _ => {
                    tracing::error!("clients unavailable, skipping tick");
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
                force: false,
            };
            match engine::poll_once(&deps, now).await {
                Ok(r) => tracing::info!(?r, "poll complete"),
                Err(e) => tracing::error!(error = %e, "poll failed"),
            }
        }
    });

    let listener = TcpListener::bind(&cfg.bind).await?;
    tracing::info!(bind = %cfg.bind, "listening on http://{}", cfg.bind);
    axum::serve(listener, app).await?;
    Ok(())
}
