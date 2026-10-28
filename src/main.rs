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
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);

    let cfg = Config::from_env()?;
    let pool = db::connect_and_migrate(&cfg.db_path).await?;
    settings::seed(&pool).await?;

    if flag("--seed") {
        let report = stock_predicter::seed::run(&pool, !flag("--no-positions")).await?;
        println!("seed: {report:?}");
    }

    // Prove the push channel end to end before relying on it.
    if flag("--test-notify") {
        let Some(notifier) = Ntfy::from_settings(&pool).await? else {
            anyhow::bail!("no ntfy topic configured — set one in Settings, or NTFY_TOPIC");
        };
        let n = stock_predicter::notify::Notification {
            title: "stock-predicter · test".into(),
            body: "If this arrived on your phone, the push channel works.".into(),
            priority: stock_predicter::notify::Priority::Default,
            dedupe_key: "test-notify".into(),
            signal_id: None,
            symbol: String::new(),
        };
        match notifier.send_test(&n).await {
            Ok(()) => println!("test notification sent"),
            Err(e) => println!("failed: {e}"),
        }
        return Ok(());
    }

    if let Some(i) = args.iter().position(|a| a == "--ask") {
        let symbol = args.get(i + 1).cloned().unwrap_or_default();
        let row: Option<(i64, String, String, String)> =
            sqlx::query_as("SELECT id, symbol, class, name FROM instruments WHERE symbol = ?")
                .bind(&symbol)
                .fetch_optional(&pool)
                .await?;
        let (id, symbol, class, name) =
            row.ok_or_else(|| anyhow::anyhow!("no instrument {symbol} — add it first"))?;
        let market = YahooSource::new()?;
        let transport = ReqwestTransport::new(settings::model_config(&pool).await?.api_key)?;
        let notifier = match Ntfy::from_settings(&pool).await? {
            Some(n) => n,
            None => Ntfy::new("https://ntfy.sh", "", "", "")?,
        };
        let instrument = stock_predicter::models::Instrument {
            id,
            symbol,
            class: stock_predicter::models::AssetClass::parse(&class)?,
            name,
            currency: "INR".into(),
            active: true,
        };
        let deps = EngineDeps {
            pool: &pool,
            cfg: &cfg,
            market: &market,
            transport: &transport,
            notifier: &notifier,
            rules: RuleConfig::default(),
            force: true,
        };
        match stock_predicter::engine::advise(&deps, &instrument, Utc::now()).await? {
            Some(line) => println!("ADVICE {line}"),
            None => println!("ADVICE none"),
        }
        return Ok(());
    }

    if flag("--poll-once") || flag("--predict") {
        let market = YahooSource::new()?;
        let transport = ReqwestTransport::new(settings::model_config(&pool).await?.api_key)?;
        let notifier = match Ntfy::from_settings(&pool).await? {
            Some(n) => n,
            None => Ntfy::new("https://ntfy.sh", "", "", "")?,
        };
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

    let state = api::AppState::new(pool.clone(), cfg.clone());
    // Client-side routes must not 404 on a direct load or a refresh.
    let app = api::router(state).fallback_service(
        ServeDir::new("web/dist").fallback(ServeFile::new("web/dist/index.html")),
    );

    let poll_cfg = cfg.clone();
    let poll_pool = pool.clone();
    tokio::spawn(async move {
        loop {
            let now = Utc::now();
            // The key can be changed from the UI at runtime, so it is read per tick.
            let key = settings::model_config(&poll_pool)
                .await
                .map(|m| m.api_key)
                .unwrap_or_default();
            let (market, transport) = match (YahooSource::new(), ReqwestTransport::new(key)) {
                (Ok(m), Ok(t)) => (m, t),
                _ => {
                    tracing::error!("clients unavailable, skipping tick");
                    continue;
                }
            };
            // The notifier is rebuilt each tick so a topic edited in the UI takes effect.
            let notifier = match Ntfy::from_settings(&poll_pool).await {
                Ok(Some(n)) => n,
                Ok(None) => match Ntfy::new("https://ntfy.sh", "", "", "") {
                    Ok(n) => n,
                    Err(e) => {
                        tracing::error!(error = %e, "notifier unavailable");
                        continue;
                    }
                },
                Err(e) => {
                    tracing::error!(error = %e, "notifier settings unreadable");
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

            // 15 minutes while the market is open, two hours otherwise.
            let secs = if schedule::in_stock_window(Utc::now()) {
                poll_cfg.poll_secs
            } else {
                poll_cfg.off_hours_secs
            };
            tokio::time::sleep(Duration::from_secs(secs.max(60))).await;
        }
    });

    let listener = TcpListener::bind(&cfg.bind).await?;
    tracing::info!(bind = %cfg.bind, "listening on http://{}", cfg.bind);
    axum::serve(listener, app).await?;
    Ok(())
}
