use anyhow::{Context, bail};
use sqlx::PgPool;
use tokio::sync::broadcast;
use tracing::info;
use tracing_subscriber::EnvFilter;

use server::{AppState, config::Config, db, listener, seed};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    match std::env::args().nth(1).as_deref() {
        Some("migrate") => migrate().await,
        Some("seed") => {
            let cfg = Config::from_env()?;
            let pool = db::connect(&cfg.app_database_url).await?;
            seed::run(&pool).await
        }
        Some("serve") | None => serve().await,
        Some(other) => bail!("unknown command `{other}`; expected migrate | seed | serve"),
    }
}

async fn migrate() -> anyhow::Result<()> {
    let url = std::env::var("DATABASE_URL").context("DATABASE_URL (owner role) is required for migrate")?;
    let pool = PgPool::connect(&url).await.context("connecting as owner")?;
    sqlx::migrate!("../../db/migrations").run(&pool).await?;
    info!("migrations applied");
    Ok(())
}

async fn serve() -> anyhow::Result<()> {
    let cfg = Config::from_env()?;
    let pool = db::connect(&cfg.app_database_url).await.context("connecting as app role")?;
    let (events, _) = broadcast::channel(1024);
    tokio::spawn(listener::run(pool.clone(), events.clone()));

    let state = AppState { pool, events, session_ttl: cfg.session_ttl };
    let app = server::app(state);
    let listener = tokio::net::TcpListener::bind(&cfg.bind_addr).await?;
    info!(addr = %cfg.bind_addr, "treasury server listening");
    axum::serve(listener, app).await?;
    Ok(())
}
