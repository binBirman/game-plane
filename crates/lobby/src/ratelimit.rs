use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

/// Sliding-window rate limiter keyed by client IP. In-memory; not cluster-wide.
#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<HashMap<IpAddr, VecDeque<Instant>>>>,
    window: Duration,
    max_in_window: usize,
}

impl RateLimiter {
    pub fn new(window: Duration, max_in_window: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            window,
            max_in_window,
        }
    }

    /// Returns true if the request is allowed (and records it).
    pub async fn check(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let cutoff = now - self.window;
        let mut g = self.inner.lock().await;
        let q = g.entry(ip).or_default();
        while q.front().map(|t| *t < cutoff).unwrap_or(false) {
            q.pop_front();
        }
        if q.len() >= self.max_in_window {
            return false;
        }
        q.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[tokio::test]
    async fn allows_up_to_max_then_blocks() {
        let rl = RateLimiter::new(Duration::from_secs(60), 3);
        let ip = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));
        assert!(rl.check(ip).await);
        assert!(rl.check(ip).await);
        assert!(rl.check(ip).await);
        assert!(!rl.check(ip).await);
        assert!(!rl.check(ip).await);
    }

    #[tokio::test]
    async fn per_ip_isolation() {
        let rl = RateLimiter::new(Duration::from_secs(60), 1);
        let a = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));
        let b = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 5));
        assert!(rl.check(a).await);
        assert!(!rl.check(a).await);
        assert!(rl.check(b).await);
    }
}