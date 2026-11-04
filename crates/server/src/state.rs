use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use chrono::{DateTime, NaiveDate, Utc};

use domain::{LcrResult, ServerEvent};
use sqlx::PgPool;
use tokio::sync::broadcast;

use crate::auth::limiter::RateLimiter;
use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    /// Deal change notifications from Postgres, fanned out to event streams.
    pub events: broadcast::Sender<ServerEvent>,
    /// Session token hashes revoked by logout, so open event streams close at once.
    pub revoked: broadcast::Sender<Vec<u8>>,
    pub login_limiter: Arc<RateLimiter>,
    pub config: Arc<Config>,
    /// Official ALM results (saved parameters, no overrides) per snapshot (date and generation
    /// time) and parameter version. All three are immutable once written, so an entry never goes
    /// stale; a regenerated snapshot simply gets a new key.
    pub alm: Arc<AlmCache>,
}

#[derive(Default)]
pub struct AlmCache {
    pub lcr: RwLock<HashMap<(NaiveDate, DateTime<Utc>, i32), Arc<LcrResult>>>,
}

impl AppState {
    pub fn new(pool: PgPool, config: Config) -> Self {
        let (events, _) = broadcast::channel(1024);
        let (revoked, _) = broadcast::channel(256);
        Self {
            pool,
            events,
            revoked,
            login_limiter: Arc::new(RateLimiter::new(config.login_ip_limit)),
            config: Arc::new(config),
            alm: Arc::new(AlmCache::default()),
        }
    }
}
