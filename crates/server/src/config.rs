use anyhow::Context;

#[derive(Debug, Clone)]
pub struct Config {
    pub app_database_url: String,
    pub bind_addr: String,
    pub session_ttl: chrono::Duration,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let app_database_url = std::env::var("APP_DATABASE_URL").context("APP_DATABASE_URL is required")?;
        let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".into());
        let hours: i64 = std::env::var("SESSION_TTL_HOURS").ok().and_then(|v| v.parse().ok()).unwrap_or(12);
        Ok(Self { app_database_url, bind_addr, session_ttl: chrono::Duration::hours(hours) })
    }
}
