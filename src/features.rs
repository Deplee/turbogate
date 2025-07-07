use std::sync::Arc;
use anyhow::Result;
use tracing::{info, debug, warn};

use crate::config::Config;
use crate::rate_limit::RateLimiter;
use crate::ddos_protection::DdosProtection;
use crate::hot_reload::HotReload;
use crate::compression::Compressor;

pub struct FeaturesManager {
    pub rate_limiter: Option<RateLimiter>,
    pub ddos_protection: Option<DdosProtection>,
    pub hot_reload: Option<HotReload>,
    pub compressor: Option<Compressor>,
    pub config: Arc<Config>,
}

impl FeaturesManager {
    pub fn new(config: Arc<Config>) -> Result<Self> {
        let mut features = Self {
            rate_limiter: None,
            ddos_protection: None,
            hot_reload: None,
            compressor: None,
            config,
        };

        features.initialize_features()?;
        Ok(features)
    }

    fn initialize_features(&mut self) -> Result<()> {
        info!("Initializing Turbogate features...");

        self.initialize_rate_limiting()?;
        self.initialize_ddos_protection()?;
        self.initialize_hot_reload()?;
        self.initialize_compression()?;

        info!("All features initialized successfully");
        Ok(())
    }

    fn initialize_rate_limiting(&mut self) -> Result<()> {
        if let Some(rate_limit_config) = &self.config.rate_limit {
            let rate_limiter = RateLimiter::new(crate::rate_limit::RateLimitConfig {
                requests_per_second: rate_limit_config.requests_per_second,
                burst_size: rate_limit_config.burst_size,
                window_size: std::time::Duration::from_secs(rate_limit_config.window_size),
            });
            self.rate_limiter = Some(rate_limiter);
        }
        Ok(())
    }

    fn initialize_ddos_protection(&mut self) -> Result<()> {
        if let Some(ddos_config) = &self.config.ddos_protection {
            info!("DDoS features: reset_interval_seconds = {}", ddos_config.reset_interval_seconds);
            if let Some(max_requests) = ddos_config.max_requests_per_minute {
                info!("DDoS features: max_requests_per_minute = {}", max_requests);
            }
            if let Some(max_connections) = ddos_config.max_connections_per_ip {
                info!("DDoS features: max_connections_per_ip = {}", max_connections);
            }
            if !ddos_config.suspicious_patterns.is_empty() {
                info!("DDoS features: suspicious_patterns = {:?}", ddos_config.suspicious_patterns);
            }
            if !ddos_config.whitelist.is_empty() {
                info!("DDoS features: whitelist = {:?}", ddos_config.whitelist);
            }
            if !ddos_config.blacklist.is_empty() {
                info!("DDoS features: blacklist = {:?}", ddos_config.blacklist);
            }
            
            info!("Initializing DDoS protection...");
            
            let mut whitelist = Vec::new();
            for ip_str in &ddos_config.whitelist {
                if let Ok(ip) = ip_str.parse::<std::net::IpAddr>() {
                    whitelist.push(ip);
                } else {
                    warn!("Invalid whitelist IP: {}", ip_str);
                }
            }
            
            let mut blacklist = Vec::new();
            for ip_str in &ddos_config.blacklist {
                if let Ok(ip) = ip_str.parse::<std::net::IpAddr>() {
                    blacklist.push(ip);
                } else {
                    warn!("Invalid blacklist IP: {}", ip_str);
                }
            }
            
            let ddos_protection = DdosProtection::new(crate::ddos_protection::DdosConfig {
                reset_interval_seconds: ddos_config.reset_interval_seconds,
                max_requests_per_minute: ddos_config.max_requests_per_minute,
                max_connections_per_ip: ddos_config.max_connections_per_ip,
                suspicious_patterns: ddos_config.suspicious_patterns.clone(),
                whitelist,
                blacklist,
            });
            self.ddos_protection = Some(ddos_protection);
        }
        Ok(())
    }

    fn initialize_hot_reload(&mut self) -> Result<()> {
        if let Some(hot_reload_config) = &self.config.hot_reload {
            if hot_reload_config.enabled {
                info!("Initializing hot reload...");
                let hot_reload = HotReload::new("test.cfg".to_string())?;
                hot_reload.start_watching()?;
                self.hot_reload = Some(hot_reload);
                debug!("Hot reload enabled with interval: {}s", hot_reload_config.watch_interval);
            }
        }
        Ok(())
    }

    fn initialize_compression(&mut self) -> Result<()> {
        if let Some(compression_config) = &self.config.compression {
            info!("Initializing compression...");
            let compressor = Compressor::new(crate::compression::CompressionConfig {
                gzip_enabled: compression_config.gzip_enabled,
                brotli_enabled: compression_config.brotli_enabled,
                deflate_enabled: compression_config.deflate_enabled,
                min_size: compression_config.min_size,
                max_size: compression_config.max_size,
                compression_level: compression_config.compression_level,
                content_types: compression_config.content_types.clone(),
            });
            self.compressor = Some(compressor);
            debug!("Compression configured: gzip={}, brotli={}, deflate={}", 
                compression_config.gzip_enabled, compression_config.brotli_enabled, 
                compression_config.deflate_enabled);
        }
        Ok(())
    }
}
