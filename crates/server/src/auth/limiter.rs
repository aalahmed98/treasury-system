//! Fixed-window per-IP limiter for the login endpoint. In-memory and per process, which is
//! enough for one server; a shared store is needed if the server is ever scaled out.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(60);
const PRUNE_ABOVE: usize = 10_000;

struct Window {
    start: Instant,
    count: u32,
}

pub struct RateLimiter {
    per_window: u32,
    inner: Mutex<HashMap<IpAddr, Window>>,
}

impl RateLimiter {
    pub fn new(per_window: u32) -> Self {
        Self { per_window, inner: Mutex::new(HashMap::new()) }
    }

    /// Ok if the attempt may proceed; Err(seconds until the window resets) otherwise.
    pub fn check(&self, ip: IpAddr) -> Result<(), u64> {
        let now = Instant::now();
        let mut map = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if map.len() > PRUNE_ABOVE {
            map.retain(|_, w| now.duration_since(w.start) < WINDOW);
        }
        let w = map.entry(ip).or_insert(Window { start: now, count: 0 });
        if now.duration_since(w.start) >= WINDOW {
            *w = Window { start: now, count: 0 };
        }
        if w.count >= self.per_window {
            let left = WINDOW.saturating_sub(now.duration_since(w.start)).as_secs();
            return Err(left.max(1));
        }
        w.count += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_after_limit_per_ip() {
        let l = RateLimiter::new(3);
        let a: IpAddr = "10.0.0.1".parse().unwrap();
        let b: IpAddr = "10.0.0.2".parse().unwrap();
        assert!(l.check(a).is_ok());
        assert!(l.check(a).is_ok());
        assert!(l.check(a).is_ok());
        assert!(l.check(a).is_err(), "4th attempt from the same ip is refused");
        assert!(l.check(b).is_ok(), "another ip is unaffected");
    }
}
