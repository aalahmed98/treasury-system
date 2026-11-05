use std::net::SocketAddr;
use std::path::Path;

use anyhow::{Context, bail};
use axum_server::tls_rustls::RustlsConfig;
use sqlx::PgPool;
use tracing::info;
use tracing_subscriber::EnvFilter;

use server::{AppState, config::Config, db, devcert, housekeeping, listener, seed};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    if rustls::crypto::ring::default_provider().install_default().is_err() {
        tracing::warn!("a TLS crypto provider was already installed");
    }

    match std::env::args().nth(1).as_deref() {
        Some("migrate") => migrate().await,
        Some("seed") => {
            let cfg = Config::from_env()?;
            let owner = PgPool::connect(&owner_url()?).await.context("connecting as owner")?;
            let app = db::connect(&cfg.app_database_url).await.context("connecting as app role")?;
            seed::run(&owner, &app).await
        }
        Some("gen-dev-cert") => {
            let (cert, key) = devcert::generate(Path::new("certs"))?;
            info!(cert = %cert.display(), key = %key.display(), "development certificate written");
            println!("TLS_CERT={}\nTLS_KEY={}", cert.display(), key.display());
            Ok(())
        }
        Some("ftp-import") => ftp_import().await,
        Some("serve") | None => serve().await,
        Some(other) => bail!("unknown command `{other}`; expected migrate | seed | serve | gen-dev-cert | ftp-import"),
    }
}

/// `ftp-import <workbook.xlsx> [--as-of YYYY-MM-DD] [--user alm1] --note "..."`: publishes ALM's
/// FTP workbook from a file, with the same checks and audit as a drop in the browser.
async fn ftp_import() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(2).collect();
    let mut file = None;
    let mut as_of = None;
    let mut user = "alm1".to_string();
    let mut note = String::new();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--as-of" => as_of = Some(it.next().context("--as-of needs a date")?.parse::<chrono::NaiveDate>().context("--as-of must be YYYY-MM-DD")?),
            "--user" => user = it.next().context("--user needs a username")?,
            "--note" => note = it.next().context("--note needs text")?,
            f if !f.starts_with("--") && file.is_none() => file = Some(std::path::PathBuf::from(f)),
            other => bail!("unexpected argument `{other}`"),
        }
    }
    let file = file.context("usage: ftp-import <workbook.xlsx> [--as-of YYYY-MM-DD] [--user alm1] --note \"...\"")?;
    let cfg = Config::from_env()?;
    let owner = PgPool::connect(&owner_url()?).await.context("connecting as owner")?;
    let app = db::connect(&cfg.app_database_url).await.context("connecting as app role")?;
    let (id, date, ccys) = server::routes::ftp::import_file(&owner, &app, &user, &file, as_of, &note).await?;
    info!(publication = id, as_of = %date, currencies = %ccys.join(","), "FTP curves published");
    Ok(())
}

fn owner_url() -> anyhow::Result<String> {
    std::env::var("DATABASE_URL").context("DATABASE_URL (owner role) is required")
}

async fn migrate() -> anyhow::Result<()> {
    let pool = PgPool::connect(&owner_url()?).await.context("connecting as owner")?;
    sqlx::migrate!("../../db/migrations").run(&pool).await?;
    info!("migrations applied");
    Ok(())
}

async fn serve() -> anyhow::Result<()> {
    let cfg = Config::from_env()?;
    let pool = db::connect(&cfg.app_database_url).await.context("connecting as app role")?;
    let state = AppState::new(pool.clone(), cfg.clone());
    tokio::spawn(listener::run(pool.clone(), state.events.clone()));
    tokio::spawn(housekeeping::run(pool, cfg.session_idle));

    let app = server::app(state).into_make_service_with_connect_info::<SocketAddr>();
    match &cfg.tls {
        Some(tls) => {
            let rustls = RustlsConfig::from_pem_file(&tls.cert, &tls.key)
                .await
                .with_context(|| format!("loading TLS material from {} and {}", tls.cert.display(), tls.key.display()))?;
            info!(addr = %cfg.bind_addr, "treasury server listening (https)");
            axum_server::bind_rustls(cfg.bind_addr, rustls).serve(app).await?;
        }
        None => {
            info!(addr = %cfg.bind_addr, "treasury server listening (plain http, loopback only)");
            axum_server::bind(cfg.bind_addr).serve(app).await?;
        }
    }
    Ok(())
}
