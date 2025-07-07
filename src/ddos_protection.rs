use std::net::IpAddr;
use std::sync::Arc;
use dashmap::DashMap;
use tracing::debug;

#[derive(Debug, Clone)]
pub struct DdosConfig {
    pub reset_interval_seconds: u64,
    pub max_requests_per_minute: Option<u32>,
    pub max_connections_per_ip: Option<u32>,
    pub suspicious_patterns: Vec<String>,
    pub whitelist: Vec<IpAddr>,
    pub blacklist: Vec<IpAddr>,
}

impl Default for DdosConfig {
    fn default() -> Self {
        Self {
            reset_interval_seconds: 60,
            max_requests_per_minute: None,
            max_connections_per_ip: None,
            suspicious_patterns: Vec::new(),
            whitelist: Vec::new(),
            blacklist: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct IpActivity {
    pub request_count: u32,
    pub connection_count: u32,
    pub last_request_time: std::time::Instant,
}

impl Default for IpActivity {
    fn default() -> Self {
        Self {
            request_count: 0,
            connection_count: 0,
            last_request_time: std::time::Instant::now(),
        }
    }
}

pub struct DdosProtection {
    activity: Arc<DashMap<IpAddr, IpActivity>>,
    config: DdosConfig,
}

impl DdosProtection {
    pub fn new(config: DdosConfig) -> Self {
        Self {
            activity: Arc::new(DashMap::new()),
            config,
        }
    }

    pub fn check_rate_limit(&self, client_ip: IpAddr) -> bool {
        if self.config.whitelist.contains(&client_ip) {
            return true;
        }

        if self.config.blacklist.contains(&client_ip) {
            return false;
        }

        let mut activity = self.activity.entry(client_ip).or_insert_with(IpActivity::default);
        
        let now = std::time::Instant::now();
        let time_since_last = now.duration_since(activity.last_request_time);
        
        if let Some(max_requests) = self.config.max_requests_per_minute {
            if time_since_last.as_secs() >= 60 {
                activity.request_count = 0;
            }
            
            if activity.request_count >= max_requests {
                debug!("DDoS protection: IP {} exceeded max requests per minute", client_ip);
                return false;
            }
            
            activity.request_count += 1;
            activity.last_request_time = now;
        }

        true
    }

    pub fn check_connection_limit(&self, client_ip: IpAddr) -> bool {
        if self.config.whitelist.contains(&client_ip) {
            return true;
        }

        if self.config.blacklist.contains(&client_ip) {
            return false;
        }

        let mut activity = self.activity.entry(client_ip).or_insert_with(IpActivity::default);
        
        if let Some(max_connections) = self.config.max_connections_per_ip {
            if activity.connection_count >= max_connections {
                debug!("DDoS protection: IP {} exceeded max connections per IP", client_ip);
                return false;
            }
            
            activity.connection_count += 1;
        }

        true
    }

    pub fn connection_closed(&self, client_ip: IpAddr) {
        if let Some(mut activity) = self.activity.get_mut(&client_ip) {
            if activity.connection_count > 0 {
                activity.connection_count -= 1;
            }
        }
    }

    pub fn check_suspicious_pattern(&self, user_agent: Option<&str>) -> bool {
        if let Some(ua) = user_agent {
            for pattern in &self.config.suspicious_patterns {
                if ua.to_lowercase().contains(&pattern.to_lowercase()) {
                    debug!("DDoS protection: suspicious pattern '{}' detected in User-Agent", pattern);
                    return false;
                }
            }
        }
        true
    }

    pub fn reset_counters(&self) {
        for mut entry in self.activity.iter_mut() {
            entry.request_count = 0;
            entry.connection_count = 0;
        }
    }

    pub fn reset_interval_seconds(&self) -> u64 {
        self.config.reset_interval_seconds
    }
}
