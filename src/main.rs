use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use stock_predictor::engine::{self, EngineDeps, YahooSource};
use stock_predictor::llm::deepseek::ReqwestTransport;
use stock_predictor::notify::Ntfy;
use stock_predictor::rules::evaluate::RuleConfig;
use stock_predictor::rules::settings;
use stock_predictor::{api, config::Config, db, schedule};
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
    for name in Config::retired_warnings() {
        tracing::warn!(
            variable = name,
            "no longer read — set it in Settings instead (Cadence or Network)"
        );
    }
    // Renaming the app changed the default database file. A database written under the old
    // spelling is moved to the new one on boot — checkpointed first, so no WAL commit is lost.
    let db_path = cfg.db_path.clone();
    if std::env::var("DB_PATH").is_err() {
        if let Some(moved) =
            db::migrate_legacy_path("data/stock-predicter.db", &db_path).await?
        {
            tracing::info!(to = %moved, "moved the database from the old name");
        }
    }
    tracing::info!(db = %db_path, "database");
    let pool = db::connect_and_migrate(&db_path).await?;

    // Secrets at rest. `SECRET_KEY` wins; without it a key file sits beside the database, which
    // protects a copied .db but not the whole data directory.
    let key = match cfg.secret_key.clone() {
        Some(k) => k,
        None => {
            tracing::warn!(
                "SECRET_KEY is not set — secrets are encrypted with {}. Set SECRET_KEY to keep \
                 the key out of the data directory.",
                stock_predictor::secrets::key_path(&cfg.db_path).display()
            );
            stock_predictor::secrets::load_or_create_key(&cfg.db_path)?
        }
    };
    stock_predictor::secrets::init(key);
    let encrypted = stock_predictor::secrets::migrate(&pool).await?;
    if encrypted > 0 {
        tracing::info!(encrypted, "encrypted stored secrets");
    }
    settings::seed(&pool).await?;

    if flag("--seed") {
        let report = stock_predictor::seed::run(&pool, !flag("--no-positions")).await?;
        println!("seed: {report:?}");
    }

    // Prove the push channel end to end before relying on it.
    if flag("--test-notify") {
        let Some(notifier) = Ntfy::from_settings(&pool, &cfg.public_url).await? else {
            anyhow::bail!("no ntfy topic configured — set one in Settings, or NTFY_TOPIC");
        };
        let n = stock_predictor::notify::Notification {
            title: "stock-predictor · test".into(),
            body: "If this arrived on your phone, the push channel works.".into(),
            priority: stock_predictor::notify::Priority::Default,
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
        let rt = settings::runtime(&pool).await?;
        let market = YahooSource::new(&rt)?;
        let transport = ReqwestTransport::new(settings::model_config(&pool).await?.api_key, &rt)?;
        let notifier = match Ntfy::from_settings(&pool, &cfg.public_url).await? {
            Some(n) => n,
            None => Ntfy::new("https://ntfy.sh", "", "", "", &rt)?,
        };
        let instrument = stock_predictor::models::Instrument {
            id,
            symbol,
            class: stock_predictor::models::AssetClass::parse(&class)?,
            name,
            currency: "INR".into(),
            active: true,
        };
        let deps = EngineDeps {
            pool: &pool,
            runtime: settings::runtime(&pool).await?,
            market: &market,
            transport: &transport,
            notifier: &notifier,
            rules: RuleConfig::default(),
            force: true,
        };
        match stock_predictor::engine::advise(&deps, &instrument, Utc::now()).await? {
            Some(line) => println!("ADVICE {line}"),
            None => println!("ADVICE none"),
        }
        return Ok(());
    }

    if flag("--poll-once") || flag("--predict") {
        let rt = settings::runtime(&pool).await?;
        let market = YahooSource::new(&rt)?;
        let transport = ReqwestTransport::new(settings::model_config(&pool).await?.api_key, &rt)?;
        let notifier = match Ntfy::from_settings(&pool, &cfg.public_url).await? {
            Some(n) => n,
            None => Ntfy::new("https://ntfy.sh", "", "", "", &rt)?,
        };
        let deps = EngineDeps {
            pool: &pool,
            runtime: settings::runtime(&pool).await?,
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

    let poll_pool = pool.clone();
    tokio::spawn(async move {
        loop {
            let now = Utc::now();
            // The key can be changed from the UI at runtime, so it is read per tick.
            let key = settings::model_config(&poll_pool)
                .await
                .map(|m| m.api_key)
                .unwrap_or_default();
            // One runtime per tick: cadence, gate and proxy all come from the same read.
            let runtime = match settings::runtime(&poll_pool).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!(error = %e, "runtime settings unreadable");
                    continue;
                }
            };
            let (market, transport) = match (
                YahooSource::new(&runtime),
                ReqwestTransport::new(key, &runtime),
            ) {
                (Ok(m), Ok(t)) => (m, t),
                _ => {
                    tracing::error!("clients unavailable, skipping tick");
                    continue;
                }
            };
            // The notifier is rebuilt each tick so a topic edited in the UI takes effect.
            let notifier = match Ntfy::from_settings(&poll_pool, &cfg.public_url).await {
                Ok(Some(n)) => n,
                Ok(None) => match Ntfy::new("https://ntfy.sh", "", "", "", &runtime) {
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
                runtime: runtime.clone(),
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
                runtime.poll_secs
            } else {
                runtime.off_hours_secs
            };
            tokio::time::sleep(Duration::from_secs(secs.max(60))).await;
        }
    });

    let listener = TcpListener::bind(&cfg.bind).await?;
    tracing::info!(bind = %cfg.bind, "listening on http://{}", cfg.bind);

    // Shut down cleanly so the WAL is folded back into the database. A killed process leaves
    // recent commits in stock-predictor.db-wal, and copying only the .db file then looks like
    // data loss.
    let shutdown_db = pool.clone();
    let shutdown = async move {
        let ctrl_c = tokio::signal::ctrl_c();
        #[cfg(unix)]
        {
            let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
            tokio::select! {
                _ = ctrl_c => {}
                _ = term.recv() => {}
            }
        }
        #[cfg(not(unix))]
        ctrl_c.await.ok();
        tracing::info!("shutting down, checkpointing the database");
        let _ = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&shutdown_db)
            .await;
        shutdown_db.close().await;
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
}
