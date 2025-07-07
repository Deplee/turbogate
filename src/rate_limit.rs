use governor::{Quota, RateLimiter as GovRateLimiter, state::keyed::DefaultKeyedStateStore, clock::DefaultClock};
use std::net::IpAddr;
use std::num::NonZeroU32;
use std::sync::Arc;
use dashmap::DashMap;

use tracing::{debug, warn};

#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    pub requests_per_second: u32,
    pub burst_size: u32,
    pub window_size: std::time::Duration,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            requests_per_second: 100,
            burst_size: 10,
            window_size: std::time::Duration::from_secs(1),
        }
    }
}

pub struct RateLimiter {
    limiters: Arc<DashMap<IpAddr, Arc<GovRateLimiter<IpAddr, DefaultKeyedStateStore<IpAddr>, DefaultClock>>>>,
    config: RateLimitConfig,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            limiters: Arc::new(DashMap::new()),
            config,
        }
    }

    pub fn check_rate_limit(&self, ip: IpAddr) -> bool {
        let limiter = self.limiters
            .entry(ip)
            .or_insert_with(|| {
                let quota = Quota::per_second(NonZeroU32::new(self.config.requests_per_second).unwrap())
                    .allow_burst(NonZeroU32::new(self.config.burst_size).unwrap());
                Arc::new(GovRateLimiter::keyed(quota))
            })
            .clone();

        match limiter.check_key(&ip) {
            Ok(_) => {
                debug!("Rate limit check passed for IP: {}", ip);
                true
            }
            Err(_) => {
                warn!("Rate limit exceeded for IP: {}", ip);
                false
            }
        }
    }

    pub fn update_config(&mut self, config: RateLimitConfig) {
        self.config = config;
        self.limiters.clear();
    }
}
