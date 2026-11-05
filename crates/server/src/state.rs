use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use chrono::{DateTime, NaiveDate, Utc};

use domain::ServerEvent;
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
    /// The bond portfolio and its market as last loaded, keyed by the business date and a stamp
    /// of the data behind it; reloaded when either changes.
    pub pmu: Arc<std::sync::Mutex<Option<PmuSlot>>>,
}

/// The cache key and the data loaded under it.
pub type PmuSlot = (String, Arc<PmuData>);

pub struct PmuData {
    pub portfolio: pmu::Portfolio,
    pub market: pmu::Market,
}

/// Key: tool, snapshot date, snapshot generation time, parameter version, extra tag (scenario).
pub type AlmKey = (&'static str, NaiveDate, DateTime<Utc>, i32, String);

#[derive(Default)]
pub struct AlmCache {
    pub results: RwLock<HashMap<AlmKey, Arc<serde_json::Value>>>,
}

impl AlmCache {
    pub fn get<T: serde::de::DeserializeOwned>(&self, key: &AlmKey) -> Option<T> {
        let hit = self.results.read().ok()?.get(key).cloned()?;
        serde_json::from_value((*hit).clone()).ok()
    }

    pub fn put<T: serde::Serialize>(&self, key: AlmKey, value: &T) {
        if let (Ok(v), Ok(mut w)) = (serde_json::to_value(value), self.results.write()) {
            w.insert(key, Arc::new(v));
        }
    }
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
            pmu: Arc::new(std::sync::Mutex::new(None)),
        }
    }
}
