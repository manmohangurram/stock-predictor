use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use stock_predictor::engine::{self, EngineDeps, YahooSource};
use stock_predictor::llm::deepseek::ReqwestTransport;
use stock_predictor::notify::Ntfy;
use stock_predictor::rules::evaluate::RuleConfig;
use stock_predictor::rules::settings;

/// The audit of outbound model calls is kept this long; older rows are the bulk of the file.
const AUDIT_KEEP_DAYS: i64 = 90;
use stock_predictor::{api, config::Config, db, schedule};
use tokio::net::TcpListener;
use tower_http::services::ServeDir;

/// The app shell: never cached without revalidation. A rebuild renames the hashed bundle, so a
/// stale copy of this file points at a script that no longer exists — the "hard refresh needed"
/// report. The bundle itself is content-addressed and can be cached forever.
async fn spa_index() -> impl axum::response::IntoResponse {
    let html = tokio::fs::read_to_string("web/dist/index.html")
        .await
        .unwrap_or_else(|_| "<h1>Build the front end: cd web && npm run build</h1>".to_string());
    (
        [(
            axum::http::header::CACHE_CONTROL,
            "no-cache, must-revalidate",
        )],
        axum::response::Html(html),
    )
}

/// One line that tells two polls apart: how many instruments were asked, and what came of it.
fn poll_summary(r: &stock_predictor::engine::PollReport) -> String {
    let total = r.polled;
    let mut line = format!(
        "{} of {} instruments asked · {} decisions · {} plans",
        r.asked, total, r.signals, r.plans
    );
    if r.blocked > 0 {
        line.push_str(&format!(" · {} blocked", r.blocked));
    }
    if !r.not_asked.is_empty() {
        line.push_str(&format!(
            " · left alone: {}",
            r.not_asked.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    line
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);

    for name in stock_predictor::config::load_dotenv() {
        tracing::debug!(%name, "loaded from .env");
    }
    let cfg = Config::from_env()?;
    for name in Config::retired_warnings() {
        tracing::warn!(
            variable = name,
            "no longer read — set it in Settings instead (Cadence or Network)"
        );
    }
    let db_path = cfg.db_path.clone();
    let legacy = db::legacy_sibling(&db_path);
    if let Some(old) = legacy {
        if let Some(moved) = db::migrate_legacy_path(&old, &db_path).await? {
            tracing::info!(from = %old, to = %moved, "moved the database from the old name");
        }
    }
    tracing::info!(db = %db_path, "database");
    let pool = db::connect_and_migrate(&db_path).await?;

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

    if flag("--fundamentals") {
        let symbol = args
            .iter()
            .position(|a| a == "--fundamentals")
            .and_then(|i| args.get(i + 1))
            .cloned()
            .unwrap_or_else(|| "INFY.NS".into());
        let rt = settings::runtime(&pool).await?;
        let client = stock_predictor::http::client_with(&rt, 20, Some("Mozilla/5.0"))?;
        use chrono::Datelike;
        let f = stock_predictor::market::fundamentals::fetch(
            &client,
            &symbol,
            chrono::Utc::now().year(),
        )
        .await?;
        println!("{f:#?}");
        println!("---\n{}", f.summary());
        return Ok(());
    }

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

        if let Ok(r) = stock_predictor::fundscan::run(&pool, &rt, Utc::now()).await {
            if r.examined > 0 {
                println!("fund scan: {r:#?}");
            }
        }
        let once_deps = EngineDeps {
            pool: &pool,
            runtime: rt.clone(),
            market: &market,
            transport: &transport,
            notifier: &notifier,
            rules: RuleConfig::default(),
            force: true,
        };
        match engine::plan_trade(&once_deps, Utc::now(), None).await {
            Ok(true) => println!("trade plan: stored"),
            Ok(false) => println!("trade plan: nothing worth doing"),
            Err(e) => println!("trade plan failed: {e:#}"),
        }
        if !flag("--serve") {
            return Ok(());
        }
    }

    let (manual_tx, mut manual_rx) =
        tokio::sync::mpsc::channel::<(i64, stock_predictor::runs::Manual)>(8);
    let mut state = api::AppState::new(pool.clone(), cfg.clone());
    state.manual = Some(manual_tx);
    let app = api::router(state)
        .fallback_service(
            ServeDir::new("web/dist")
                .append_index_html_on_directories(false)
                .fallback(axum::routing::get(spa_index)),
        );

    let poll_pool = pool.clone();
    tokio::spawn(async move {
        // A run left "running" by a process that died is not running any more.
        if let Err(e) = stock_predictor::runs::close_orphans(&poll_pool, Utc::now()).await {
            tracing::warn!(error = %e, "could not close orphaned runs");
        }
        let mut pending: Option<(i64, stock_predictor::runs::Manual)> = None;
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
            let mut handled_manual = false;
            if let Some((run_id, command)) = pending.take() {
                handled_manual = true;
                use stock_predictor::runs::Manual;
                let manual_deps = EngineDeps {
                    pool: &poll_pool,
                    runtime: runtime.clone(),
                    market: &market,
                    transport: &transport,
                    notifier: &notifier,
                    rules: RuleConfig::default(),
                    force: true, // a human asked for this answer now
                };
                let outcome = match command {
                    Manual::Poll => match engine::poll_once(&manual_deps, Utc::now()).await {
                        Ok(r) => Ok(poll_summary(&r)),
                        Err(e) => Err(format!("{e:#}")),
                    },
                    Manual::PlanStock => {
                        match engine::plan_stock(&manual_deps, Utc::now(), Some(run_id)).await {
                            Ok(true) => Ok("a stock plan is in the ledger".to_string()),
                            Ok(false) => Ok("nothing worth doing — no plan".to_string()),
                            Err(e) => Err(format!("{e:#}")),
                        }
                    }
                    Manual::PlanTrade => {
                        match engine::plan_trade(&manual_deps, Utc::now(), Some(run_id)).await {
                            Ok(true) => Ok("a trade plan is in the ledger".to_string()),
                            Ok(false) => Ok("nothing worth doing — no trade plan".to_string()),
                            Err(e) => Err(format!("{e:#}")),
                        }
                    }
                    Manual::PlanFund => {
                        match engine::plan_funds(&manual_deps, Utc::now(), Some(run_id)).await {
                            Ok(true) => Ok("a fund plan is in the ledger".to_string()),
                            Ok(false) => Ok("nothing worth doing — no plan".to_string()),
                            Err(e) => Err(format!("{e:#}")),
                        }
                    }
                    Manual::Decide { symbol } => {
                        match engine::decide_one(&manual_deps, &symbol, Utc::now()).await {
                            Ok(Some(line)) => Ok(line),
                            Ok(None) => Ok("no decision — nothing to act on".to_string()),
                            Err(e) => Err(format!("{e:#}")),
                        }
                    }
                };
                match outcome {
                    Ok(summary) => {
                        let _ =
                            stock_predictor::runs::finish(&poll_pool, run_id, &summary, None).await;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "manual run failed");
                        let _ = stock_predictor::runs::fail(&poll_pool, run_id, &error).await;
                    }
                }
            }

            if !handled_manual {
                // The month boundary, once per purse: stocks, mutual funds, short-term.
                let purses = [
                    (stock_predictor::models::Mode::Long, None),
                    (stock_predictor::models::Mode::Long, Some("mf")),
                    (stock_predictor::models::Mode::Trade, None),
                ];
                for (mode, bucket) in purses {
                    let allowance = match bucket {
                        Some("mf") => settings::mf_allowance(&poll_pool).await,
                        _ => settings::allowance(&poll_pool, mode).await,
                    };
                    match allowance {
                        Ok(allowance) => {
                            match stock_predictor::rules::pot::ensure_month(
                                &poll_pool,
                                mode,
                                bucket,
                                allowance,
                                Utc::now(),
                            )
                            .await
                            {
                                Ok(r) if r.granted > 0.0 || r.expired > 0.0 => tracing::info!(
                                    month = %r.month,
                                    mode = mode.as_str(),
                                    purse = bucket.unwrap_or("main"),
                                    expired = r.expired,
                                    granted = r.granted,
                                    "month rolled over"
                                ),
                                Ok(_) => {}
                                Err(e) => tracing::error!(error = %e, "month reset failed"),
                            }
                        }
                        Err(e) => tracing::error!(error = %e, "allowance unreadable"),
                    }
                }

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

            // The audit keeps three months; the sweep is idempotent, so it runs with the poll.
            match stock_predictor::llm::audit::prune(&poll_pool, AUDIT_KEEP_DAYS).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(deleted = n, "audit rows older than {AUDIT_KEEP_DAYS} days"),
                Err(e) => tracing::warn!(error = %e, "audit prune failed"),
            }

            // Best-fund discovery: once a day, rank the curated list by trailing return.
                match stock_predictor::fundscan::run(&poll_pool, &runtime, Utc::now()).await {
                    Ok(r) if r.examined > 0 => tracing::info!(?r, "fund scan"),
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = %e, "fund scan failed"),
                }

                let fund_deps = stock_predictor::engine::EngineDeps {
                    pool: &poll_pool,
                    runtime: runtime.clone(),
                    market: &market,
                    transport: &transport,
                    notifier: &notifier,
                    rules: RuleConfig::default(),
                    force: false,
                };
                match stock_predictor::engine::plan_funds(&fund_deps, Utc::now(), None).await {
                    Ok(true) => tracing::info!("fund plan stored"),
                    Ok(false) => {}
                    Err(e) => tracing::warn!(error = %e, "fund plan failed"),
                }

                match stock_predictor::engine::plan_trade(&deps, Utc::now(), None).await {
                    Ok(true) => tracing::info!("trade plan stored"),
                    Ok(false) => {}
                    Err(e) => tracing::warn!(error = %e, "trade plan failed"),
                }
            }

            let secs = if schedule::in_stock_window(Utc::now()) {
                runtime.poll_secs
            } else {
                runtime.off_hours_secs
            };
            tokio::select! {
                Some(req) = manual_rx.recv() => pending = Some(req),
                _ = tokio::time::sleep(Duration::from_secs(secs.max(60))) => {}
            }
        }
    });

    let listener = TcpListener::bind(&cfg.bind).await?;
    tracing::info!(bind = %cfg.bind, "listening on http://{}", cfg.bind);

    let shutdown_db = pool.clone();
    let shutdown = async move {
        let ctrl_c = tokio::signal::ctrl_c();
        #[cfg(unix)]
        {
            let mut term =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
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
