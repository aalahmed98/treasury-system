use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, bail};

#[derive(Debug, Clone)]
pub struct TlsPaths {
    pub cert: PathBuf,
    pub key: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub app_database_url: String,
    pub bind_addr: SocketAddr,
    pub tls: Option<TlsPaths>,
    /// Absolute session lifetime.
    pub session_ttl: chrono::Duration,
    /// A session with no authenticated request for this long is dead.
    pub session_idle: chrono::Duration,
    /// Consecutive failures on one username before it is locked.
    pub login_max_failures: i32,
    pub login_lock: chrono::Duration,
    /// Login attempts allowed per source IP per minute.
    pub login_ip_limit: u32,
    /// How often an open event stream re-checks its session and entitlements.
    pub stream_check: std::time::Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            app_database_url: String::new(),
            bind_addr: "127.0.0.1:8787".parse().expect("valid literal"),
            tls: None,
            session_ttl: chrono::Duration::hours(12),
            session_idle: chrono::Duration::minutes(30),
            login_max_failures: 5,
            login_lock: chrono::Duration::minutes(15),
            login_ip_limit: 20,
            stream_check: std::time::Duration::from_secs(10),
        }
    }
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let d = Self::default();
        let app_database_url = std::env::var("APP_DATABASE_URL").context("APP_DATABASE_URL is required")?;
        let bind_addr: SocketAddr = std::env::var("BIND_ADDR")
            .unwrap_or_else(|_| d.bind_addr.to_string())
            .parse()
            .context("BIND_ADDR must be an ip:port such as 127.0.0.1:8787")?;

        let tls = match (std::env::var("TLS_CERT"), std::env::var("TLS_KEY")) {
            (Ok(cert), Ok(key)) => Some(TlsPaths { cert: cert.into(), key: key.into() }),
            (Err(_), Err(_)) => None,
            _ => bail!("TLS_CERT and TLS_KEY must be set together"),
        };

        // Credentials and tokens travel in clear text without TLS, so plaintext is loopback-only.
        let insecure_ok = std::env::var("ALLOW_INSECURE_BIND").map(|v| v == "1").unwrap_or(false);
        if !bind_addr.ip().is_loopback() && tls.is_none() && !insecure_ok {
            bail!(
                "refusing to serve plaintext HTTP on {bind_addr}: set TLS_CERT and TLS_KEY, bind to 127.0.0.1, \
                 or set ALLOW_INSECURE_BIND=1 on a throwaway development box"
            );
        }

        Ok(Self {
            app_database_url,
            bind_addr,
            tls,
            session_ttl: chrono::Duration::hours(env_i64("SESSION_TTL_HOURS", d.session_ttl.num_hours())?),
            session_idle: chrono::Duration::minutes(env_i64("SESSION_IDLE_MINUTES", d.session_idle.num_minutes())?),
            login_max_failures: env_i64("LOGIN_MAX_FAILURES", d.login_max_failures as i64)? as i32,
            login_lock: chrono::Duration::minutes(env_i64("LOGIN_LOCK_MINUTES", d.login_lock.num_minutes())?),
            login_ip_limit: env_i64("LOGIN_IP_LIMIT_PER_MINUTE", d.login_ip_limit as i64)? as u32,
            stream_check: d.stream_check,
        })
    }
}

fn env_i64(name: &str, default: i64) -> anyhow::Result<i64> {
    match std::env::var(name) {
        Ok(v) => v.parse::<i64>().with_context(|| format!("{name} must be an integer")).and_then(|n| {
            if n <= 0 { bail!("{name} must be positive") } else { Ok(n) }
        }),
        Err(_) => Ok(default),
    }
}
