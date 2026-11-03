//! Every time-dependent decision (SLA deadlines, grace windows, freshness) reads one clock so
//! that accelerated replays and tests are exact.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use chrono::{DateTime, Duration, Utc};

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Runs `speed` times faster than wall time from a chosen starting instant, until paused.
pub struct SimClock {
    start_sim: DateTime<Utc>,
    start_wall: Instant,
    speed: f64,
    paused_at: Mutex<Option<DateTime<Utc>>>,
}

impl SimClock {
    pub fn new(start_sim: DateTime<Utc>, speed: f64) -> Self {
        Self { start_sim, start_wall: Instant::now(), speed: speed.max(0.0), paused_at: Mutex::new(None) }
    }

    /// Stop the clock at `at` (or at its current reading when `at` is None).
    pub fn pause(&self, at: Option<DateTime<Utc>>) {
        let t = at.unwrap_or_else(|| self.now());
        *self.paused_at.lock().unwrap() = Some(t);
    }
}

impl Clock for SimClock {
    fn now(&self) -> DateTime<Utc> {
        if let Some(t) = *self.paused_at.lock().unwrap() {
            return t;
        }
        let elapsed = self.start_wall.elapsed().as_secs_f64() * self.speed;
        self.start_sim + Duration::milliseconds((elapsed * 1000.0) as i64)
    }
}

/// A clock that only moves when told to. For tests and for catch-up replays.
#[derive(Clone)]
pub struct ManualClock(Arc<Mutex<DateTime<Utc>>>);

impl ManualClock {
    pub fn at(t: DateTime<Utc>) -> Self {
        Self(Arc::new(Mutex::new(t)))
    }

    pub fn set(&self, t: DateTime<Utc>) {
        *self.0.lock().unwrap() = t;
    }

    pub fn advance(&self, by: Duration) {
        *self.0.lock().unwrap() += by;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}
