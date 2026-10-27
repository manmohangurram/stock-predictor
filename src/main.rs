use stock_predicter::{config, db};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cfg = config::Config::from_env()?;
    let pool = db::connect_and_migrate(&cfg.db_path).await?;
    tracing::info!(db = %cfg.db_path, "database ready");
    let _ = pool;
    Ok(())
}
