use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;
use anyhow::{Result, anyhow};
use tracing::{debug, warn, info};
use crate::options::Options;
use std::net::IpAddr;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub global: GlobalConfig,
    pub defaults: DefaultsConfig,
    pub frontends: Vec<FrontendConfig>,
    pub backends: Vec<BackendConfig>,
    pub metrics: MetricsConfig,
    pub rate_limit: Option<RateLimitConfig>,
    pub ddos_protection: Option<DdosProtectionConfig>,
    pub hot_reload: Option<HotReloadConfig>,
    pub compression: Option<CompressionConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalConfig {
    pub maxconn: Option<u32>,
    pub log: Option<String>,
    pub user: Option<String>,
    pub group: Option<String>,
    pub daemon: Option<bool>,
    pub pidfile: Option<String>,
    pub ssl_default_bind_ciphers: Option<String>,
    pub ssl_default_bind_options: Option<String>,
    pub option: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultsConfig {
    pub mode: Option<String>,
    pub log: Option<String>,
    pub option: Vec<String>,
    pub timeout: HashMap<String, String>,
    pub retries: Option<u32>,
    pub options: Option<Options>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontendConfig {
    pub name: String,
    pub bind: Vec<String>,
    pub mode: Option<String>,
    pub default_backend: Option<String>,
    pub acl: Vec<AclConfig>,
    pub use_backend: Vec<UseBackendConfig>,
    pub option: Vec<String>,
    pub timeout: HashMap<String, String>,
    pub options: Option<Options>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendConfig {
    pub name: String,
    pub mode: Option<String>,
    pub balance: Option<String>,
    pub server: Vec<ServerConfig>,
    pub option: Vec<String>,
    pub timeout: HashMap<String, String>,
    pub health_check: Option<HealthCheckConfig>,
    pub options: Option<Options>,
    pub retries: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub name: String,
    pub address: String,
    pub port: u16,
    pub weight: Option<u32>,
    pub maxconn: Option<u32>,
    pub check: Option<bool>,
    pub inter: Option<String>,
    pub rise: Option<u32>,
    pub fall: Option<u32>,
    pub backup: Option<bool>,
    pub disabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AclConfig {
    pub name: String,
    pub criterion: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UseBackendConfig {
    pub backend: String,
    pub condition: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckConfig {
    pub interval: String,
    pub timeout: String,
    pub rise: u32,
    pub fall: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsConfig {
    pub enabled: bool,
    pub bind: Option<String>,
    pub path: Option<String>,
}

impl Config {
    pub async fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(path).await?;
        Self::from_haproxy_config(&content)
    }

    pub fn from_haproxy_config(content: &str) -> Result<Self> {
        let mut config = Config {
            global: GlobalConfig::default(),
            defaults: DefaultsConfig::default(),
            frontends: Vec::new(),
            backends: Vec::new(),
            metrics: MetricsConfig::default(),
            rate_limit: None,
            ddos_protection: None,
            hot_reload: None,
            compression: None,
        };
        
        let mut stats_binds = Vec::new();

        let mut current_section = None;
        let mut current_frontend: Option<FrontendConfig> = None;
        let mut current_backend: Option<BackendConfig> = None;

        for (line_num, line) in content.lines().enumerate() {
            let line = line.trim();
            
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            debug!("Parsing line {}: '{}'", line_num + 1, line);

            match parse_line(line, line_num + 1)? {
                LineType::Section(section) => {
                    if let Some(mut frontend) = current_frontend.take() {
                        let mode = frontend.mode.as_deref().unwrap_or("tcp");
                        frontend.options = Some(Options::from_strings(&frontend.option, mode)?);
                        config.frontends.push(frontend);
                    }
                    if let Some(mut backend) = current_backend.take() {
                        let mode = backend.mode.as_deref().unwrap_or("tcp");
                        backend.options = Some(Options::from_strings(&backend.option, mode)?);
                        
                        backend.health_check = create_health_check_config(&backend);
                        
                        config.backends.push(backend);
                    }

                    current_section = Some(section.clone());
                    match section.as_str() {
                        "global" => {},
                        "defaults" => {},
                        _ if section.starts_with("frontend ") => {
                            let name = section.split_whitespace().nth(1)
                                .ok_or_else(|| anyhow!("Invalid frontend name at line {}", line_num + 1))?;
                            current_frontend = Some(FrontendConfig {
                                name: name.to_string(),
                                bind: Vec::new(),
                                mode: None,
                                default_backend: None,
                                acl: Vec::new(),
                                use_backend: Vec::new(),
                                option: Vec::new(),
                                timeout: HashMap::new(),
                                options: None,
                            });
                        },
                        _ if section.starts_with("backend ") => {
                            let name = section.split_whitespace().nth(1)
                                .ok_or_else(|| anyhow!("Invalid backend name at line {}", line_num + 1))?;
                            current_backend = Some(BackendConfig {
                                name: name.to_string(),
                                mode: None,
                                balance: None,
                                server: Vec::new(),
                                option: Vec::new(),
                                timeout: HashMap::new(),
                                health_check: None,
                                options: None,
                                retries: None,
                            });
                        },
                        _ => {
                            warn!("Unknown section: {}", section);
                        }
                    }
                },
                LineType::Directive(key, value) => {
                    debug!("Parsing directive: {} = {}", key, value);
                    match current_section.as_deref() {
                        Some("global") => {
                            if key == "stats" && value.starts_with("bind") {
                                let parts: Vec<&str> = value.split_whitespace().collect();
                                if parts.len() >= 2 {
                                    stats_binds.push(parts[1].to_string());
                                }
                            } else {
                                parse_global_directive(&mut config.global, &key, &value)?;
                            }
                        },
                        Some("defaults") => parse_defaults_directive(&mut config.defaults, &key, &value)?,
                        Some(section) if section.starts_with("frontend ") => {
                            if let Some(ref mut frontend) = current_frontend {
                                parse_frontend_directive(frontend, &key, &value)?;
                            }
                        },
                        Some(section) if section.starts_with("backend ") => {
                            if let Some(ref mut backend) = current_backend {
                                parse_backend_directive(backend, &key, &value)?;
                            }
                        },
                        _ => {
                            warn!("Directive outside section: {} {}", key, value);
                        }
                    }
                }
            }
        }

        if let Some(mut frontend) = current_frontend {
            let mode = frontend.mode.as_deref().unwrap_or("tcp");
            frontend.options = Some(Options::from_strings(&frontend.option, mode)?);
            config.frontends.push(frontend);
        }
        if let Some(mut backend) = current_backend {
            let mode = backend.mode.as_deref().unwrap_or("tcp");
            backend.options = Some(Options::from_strings(&backend.option, mode)?);
            
            backend.health_check = create_health_check_config(&backend);
            
            config.backends.push(backend);
        }

        let mode = config.defaults.mode.as_deref().unwrap_or("tcp");
        config.defaults.options = Some(Options::from_strings(&config.defaults.option, mode)?);

        if !stats_binds.is_empty() {
            config.metrics.bind = Some(stats_binds[0].clone());
            info!("Metrics will be available on: {}", stats_binds[0]);
        }

        Self::parse_rate_limit_config(&mut config)?;
        Self::parse_ddos_protection_config(&mut config)?;
        Self::parse_compression_config(&mut config)?;

        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        let backend_names: std::collections::HashSet<_> = self.backends.iter()
            .map(|b| &b.name)
            .collect();

        for frontend in &self.frontends {
            if let Some(ref backend_name) = frontend.default_backend {
                if !backend_names.contains(backend_name) {
                    return Err(anyhow!("Frontend '{}' references non-existent backend '{}'", 
                                     frontend.name, backend_name));
                }
            }

            for use_backend in &frontend.use_backend {
                if !backend_names.contains(&use_backend.backend) {
                    return Err(anyhow!("Frontend '{}' references non-existent backend '{}'", 
                                     frontend.name, use_backend.backend));
                }
            }
        }

        for backend in &self.backends {
            if backend.server.is_empty() {
                return Err(anyhow!("Backend '{}' has no servers", backend.name));
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
enum LineType {
    Section(String),
    Directive(String, String),
}

fn parse_line(line: &str, line_num: usize) -> Result<LineType> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.is_empty() {
        return Err(anyhow!("Empty line at {}", line_num));
    }

    let first = parts[0];
    if first == "global" || first == "defaults" || first.starts_with("frontend") || first.starts_with("backend") {
        Ok(LineType::Section(line.to_string()))
    } else {
        if parts.len() < 2 {
            return Err(anyhow!("Invalid directive at line {}: {}", line_num, line));
        }
        let key = parts[0].to_string();
        let value = parts[1..].join(" ");
        Ok(LineType::Directive(key, value))
    }
}

fn parse_global_directive(global: &mut GlobalConfig, key: &str, value: &str) -> Result<()> {
    match key {
        "maxconn" => global.maxconn = Some(value.parse()?),
        "log" => global.log = Some(value.to_string()),
        "user" => global.user = Some(value.to_string()),
        "group" => global.group = Some(value.to_string()),
        "daemon" => global.daemon = Some(match value {
            "on" | "true" | "yes" => true,
            "off" | "false" | "no" => false,
            _ => value.parse()?,
        }),
        "pidfile" => global.pidfile = Some(value.to_string()),
        "ssl-default-bind-ciphers" => global.ssl_default_bind_ciphers = Some(value.to_string()),
        "ssl-default-bind-options" => global.ssl_default_bind_options = Some(value.to_string()),
        "stats" => {
            if value.starts_with("bind") {
                let parts: Vec<&str> = value.split_whitespace().collect();
                if parts.len() >= 2 {
                }
            }
        },
        "rate-limit" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if parts.len() >= 2 {
                match parts[0] {
                    "requests-per-second" => {
                        if let Ok(rate) = parts[1].parse::<u32>() {
                            global.option.push(format!("rate-limit-rps {}", rate));
                        }
                    },
                    "burst-size" => {
                        if let Ok(burst) = parts[1].parse::<u32>() {
                            global.option.push(format!("rate-limit-burst {}", burst));
                        }
                    },
                    _ => {}
                }
            }
        },
        "ddos-protection" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if parts.len() >= 2 {
                match parts[0] {
                    "max-requests-per-minute" => {
                        if let Ok(max_req) = parts[1].parse::<u32>() {
                            global.option.push(format!("ddos-protection max-requests-per-minute {}", max_req));
                            debug!("Parsed DDoS max-requests-per-minute: {}", max_req);
                        }
                    },
                    "max-connections-per-ip" => {
                        if let Ok(max_conn) = parts[1].parse::<u32>() {
                            global.option.push(format!("ddos-protection max-connections-per-ip {}", max_conn));
                            debug!("Parsed DDoS max-connections-per-ip: {}", max_conn);
                        }
                    },
                    "reset-interval-seconds" => {
                        if let Ok(interval) = parts[1].parse::<u64>() {
                            global.option.push(format!("ddos-protection reset-interval-seconds {}", interval));
                            debug!("Parsed DDoS reset-interval-seconds: {}", interval);
                        }
                    },
                    "suspicious-pattern" => {
                        let patterns: Vec<&str> = parts[1].split(&[',', ' '][..]).filter(|s| !s.trim().is_empty()).map(|s| s.trim()).collect();
                        for pattern in patterns {
                            global.option.push(format!("ddos-protection suspicious-pattern {}", pattern));
                            debug!("Parsed DDoS suspicious-pattern: {}", pattern);
                        }
                    },
                    "whitelist" => {
                        let ips: Vec<&str> = parts[1].split(&[',', ' '][..]).filter(|s| !s.trim().is_empty()).map(|s| s.trim()).collect();
                        for ip in ips {
                            global.option.push(format!("ddos-protection whitelist {}", ip));
                            debug!("Parsed DDoS whitelist: {}", ip);
                        }
                    },
                    "blacklist" => {
                        let ips: Vec<&str> = parts[1].split(&[',', ' '][..]).filter(|s| !s.trim().is_empty()).map(|s| s.trim()).collect();
                        for ip in ips {
                            global.option.push(format!("ddos-protection blacklist {}", ip));
                            debug!("Parsed DDoS blacklist: {}", ip);
                        }
                    },
                    _ => {}
                }
            }
        },
        "compression-gzip" => {
            global.option.push(format!("compression-gzip {}", value));
        },
        "compression-brotli" => {
            global.option.push(format!("compression-brotli {}", value));
        },
        "compression-deflate" => {
            global.option.push(format!("compression-deflate {}", value));
        },
        "compression-min-size" => {
            if let Ok(size) = value.parse::<usize>() {
                global.option.push(format!("compression-min-size {}", size));
            }
        },
        "compression-max-size" => {
            if let Ok(size) = value.parse::<usize>() {
                global.option.push(format!("compression-max-size {}", size));
            }
        },
        "compression-level" => {
            if let Ok(level) = value.parse::<u32>() {
                global.option.push(format!("compression-level {}", level));
            }
        },
        _ => warn!("Unknown global directive: {}", key),
    }
    Ok(())
}

fn parse_defaults_directive(defaults: &mut DefaultsConfig, key: &str, value: &str) -> Result<()> {
    match key {
        "mode" => defaults.mode = Some(value.to_string()),
        "log" => defaults.log = Some(value.to_string()),
        "option" => defaults.option.push(value.to_string()),
        "timeout" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if parts.len() >= 2 {
                defaults.timeout.insert(parts[0].to_string(), parts[1].to_string());
                
                if defaults.options.is_none() {
                    defaults.options = Some(crate::options::Options::default());
                }
                if let Some(ref mut options) = defaults.options {
                    if let Err(e) = options.apply_timeout(parts[0], parts[1]) {
                        warn!("Failed to apply timeout {} {}: {}", parts[0], parts[1], e);
                    }
                }
            }
        },
        "retries" => defaults.retries = Some(value.parse()?),
        "rate-limit" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if parts.len() >= 2 {
                match parts[0] {
                    "requests-per-second" => {
                        if let Ok(rate) = parts[1].parse::<u32>() {
                            defaults.option.push(format!("rate-limit-rps {}", rate));
                        }
                    },
                    "burst-size" => {
                        if let Ok(burst) = parts[1].parse::<u32>() {
                            defaults.option.push(format!("rate-limit-burst {}", burst));
                        }
                    },
                    _ => {}
                }
            }
        },
        "ddos-protection" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if parts.len() >= 2 {
                match parts[0] {
                    "max-requests-per-minute" => {
                        if let Ok(max_req) = parts[1].parse::<u32>() {
                            defaults.option.push(format!("ddos-protection max-requests-per-minute {}", max_req));
                        }
                    },
                    "max-connections-per-ip" => {
                        if let Ok(max_conn) = parts[1].parse::<u32>() {
                            defaults.option.push(format!("ddos-protection max-connections-per-ip {}", max_conn));
                        }
                    },
                    "reset-interval-seconds" => {
                        if let Ok(interval) = parts[1].parse::<u64>() {
                            defaults.option.push(format!("ddos-protection reset-interval-seconds {}", interval));
                        }
                    },
                    "suspicious-pattern" => {
                        defaults.option.push(format!("ddos-protection suspicious-pattern {}", parts[1]));
                    },
                    "whitelist" => {
                        defaults.option.push(format!("ddos-protection whitelist {}", parts[1]));
                    },
                    "blacklist" => {
                        defaults.option.push(format!("ddos-protection blacklist {}", parts[1]));
                    },
                    _ => {}
                }
            }
        },
        "compression" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if parts.len() >= 2 {
                match parts[0] {
                    "gzip" | "brotli" | "deflate" => {
                        if parts[1] == "enabled" {
                            defaults.option.push(format!("compression-{} enabled", parts[0]));
                        }
                    },
                    "min-size" => {
                        if let Ok(size) = parts[1].parse::<usize>() {
                            defaults.option.push(format!("compression-min-size {}", size));
                        }
                    },
                    "max-size" => {
                        if let Ok(size) = parts[1].parse::<usize>() {
                            defaults.option.push(format!("compression-max-size {}", size));
                        }
                    },
                    _ => {}
                }
            }
        },
        "http2" | "http3" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if parts.len() >= 1 {
                if parts[0] == "enabled" {
                    defaults.option.push(format!("{}-enabled", key));
                }
                if parts.len() >= 2 {
                    defaults.option.push(format!("{}-{} {}", key, parts[0], parts[1]));
                }
            }
        },
        "hot-reload" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if parts.len() >= 1 {
                if parts[0] == "enabled" {
                    defaults.option.push("hot-reload-enabled".to_string());
                } else if parts.len() >= 2 {
                    defaults.option.push(format!("hot-reload-{} {}", parts[0], parts[1]));
                }
            }
        },
        _ => warn!("Unknown defaults directive: {}", key),
    }
    
    Ok(())
}

fn parse_frontend_directive(frontend: &mut FrontendConfig, key: &str, value: &str) -> Result<()> {
    match key {
        "bind" => {
            let bind_value = if value.trim().starts_with("*:") {
                let port = &value.trim()[2..];
                format!("0.0.0.0:{}", port)
            } else {
                value.to_string()
            };
            frontend.bind.push(bind_value)
        },
        "mode" => frontend.mode = Some(value.to_string()),
        "default_backend" => frontend.default_backend = Some(value.to_string()),
        "acl" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if parts.len() >= 2 {
                frontend.acl.push(AclConfig {
                    name: parts[0].to_string(),
                    criterion: parts[1..].join(" "),
                });
            }
        },
        "use_backend" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if parts.len() >= 2 {
                frontend.use_backend.push(UseBackendConfig {
                    backend: parts[0].to_string(),
                    condition: Some(parts[1..].join(" ")),
                });
            }
        },
        "option" => frontend.option.push(value.to_string()),
        "timeout" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if parts.len() >= 2 {
                frontend.timeout.insert(parts[0].to_string(), parts[1].to_string());
                
                if frontend.options.is_none() {
                    frontend.options = Some(crate::options::Options::default());
                }
                if let Some(ref mut options) = frontend.options {
                    if let Err(e) = options.apply_timeout(parts[0], parts[1]) {
                        warn!("Failed to apply timeout {} {}: {}", parts[0], parts[1], e);
                    }
                }
            }
        },
        "http-request" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if parts.len() >= 3 {
                match parts[0] {
                    "set-header" | "add-header" => {
                        frontend.option.push(format!("http-request-{}-header {} {}", parts[0], parts[1], parts[2]));
                    },
                    _ => {},
                }
            }
        },
        "http-response" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if parts.len() >= 3 {
                match parts[0] {
                    "set-header" | "add-header" => {
                        frontend.option.push(format!("http-response-{}-header {} {}", parts[0], parts[1], parts[2]));
                    },
                    _ => {},
                }
            }
        },
        "compression-gzip" => {
            frontend.option.push(format!("compression-gzip {}", value));
        },
        "compression-brotli" => {
            frontend.option.push(format!("compression-brotli {}", value));
        },
        "compression-deflate" => {
            frontend.option.push(format!("compression-deflate {}", value));
        },
        "compression-min-size" => {
            if let Ok(size) = value.parse::<usize>() {
                frontend.option.push(format!("compression-min-size {}", size));
            }
        },
        "compression-max-size" => {
            if let Ok(size) = value.parse::<usize>() {
                frontend.option.push(format!("compression-max-size {}", size));
            }
        },
        "compression-level" => {
            if let Ok(level) = value.parse::<u32>() {
                frontend.option.push(format!("compression-level {}", level));
            }
        },
        _ => warn!("Unknown frontend directive: {}", key),
    }
    
    Ok(())
}

fn parse_backend_directive(backend: &mut BackendConfig, key: &str, value: &str) -> Result<()> {
    match key {
        "mode" => backend.mode = Some(value.to_string()),
        "balance" => {
            let algorithm = value.split('#').next().unwrap_or(value).trim();
            backend.balance = Some(algorithm.to_string());
        },
        "server" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if parts.len() >= 2 {
                let server_name = parts[0].to_string();
                let server_addr = parts[1].to_string();
                let server_name_clone = server_name.clone();
                
                let (address, port) = if server_addr.contains(':') {
                    let addr_parts: Vec<&str> = server_addr.split(':').collect();
                    if addr_parts.len() == 2 {
                        (addr_parts[0].to_string(), addr_parts[1].parse().unwrap_or(80))
                    } else {
                        (server_addr, 80)
                    }
                } else {
                    (server_addr, 80)
                };
                
                let mut server = ServerConfig {
                    name: server_name,
                    address,
                    port,
                    weight: Some(1),
                    maxconn: None,
                    check: None,
                    inter: None,
                    rise: None,
                    fall: None,
                    backup: None,
                    disabled: None,
                };

                let mut i = 2;
                while i < parts.len() {
                    let part = parts[i];
                    match part {
                        "weight" => {
                            if i + 1 < parts.len() {
                                let weight_str = parts[i + 1];
                                server.weight = Some(weight_str.parse().unwrap_or(1));
                                tracing::debug!("Parsed weight for server {}: {}", server_name_clone, server.weight.unwrap());
                                i += 2;
                            }
                            i += 1;
                        },
                        "maxconn" => {
                            if i + 1 < parts.len() {
                                server.maxconn = Some(parts[i + 1].parse().unwrap_or(1000));
                                i += 2;
                            }
                            i += 1;
                        },
                        "check" => {
                            server.check = Some(true);
                            i += 1;
                        },
                        "inter" => {
                            if i + 1 < parts.len() {
                                server.inter = Some(parts[i + 1].to_string());
                                i += 2;
                            }
                            i += 1;
                        },
                        "rise" => {
                            if i + 1 < parts.len() {
                                server.rise = Some(parts[i + 1].parse().unwrap_or(2));
                                i += 2;
                            }
                            i += 1;
                        },
                        "fall" => {
                            if i + 1 < parts.len() {
                                server.fall = Some(parts[i + 1].parse().unwrap_or(3));
                                i += 2;
                            }
                            i += 1;
                        },
                        "backup" => {
                            server.backup = Some(true);
                            i += 1;
                        },
                        "disabled" => {
                            server.disabled = Some(true);
                            i += 1;
                        },
                        _ => {
                            i += 1;
                        },
                    }
                }

                backend.server.push(server);
            }
        },
        "option" => backend.option.push(value.to_string()),
        "tcp-check" => backend.option.push(value.to_string()),
        "retries" => backend.retries = Some(value.parse()?),
        "timeout" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if parts.len() >= 2 {
                backend.timeout.insert(parts[0].to_string(), parts[1].to_string());
                
                if backend.options.is_none() {
                    backend.options = Some(crate::options::Options::default());
                }
                if let Some(ref mut options) = backend.options {
                    if let Err(e) = options.apply_timeout(parts[0], parts[1]) {
                        warn!("Failed to apply timeout {} {}: {}", parts[0], parts[1], e);
                    }
                }
            }
        },
        _ => warn!("Unknown backend directive: {}", key),
    }
    
    Ok(())
}

fn create_health_check_config(backend: &BackendConfig) -> Option<HealthCheckConfig> {
    let mut interval = "2s".to_string();
    let timeout = "1s".to_string();
    let mut rise = 2;
    let mut fall = 3;
    
    for server in &backend.server {
        if server.check.unwrap_or(false) {
            if let Some(ref inter) = server.inter {
                interval = inter.clone();
            }
            if let Some(rise_val) = server.rise {
                rise = rise_val;
            }
            if let Some(fall_val) = server.fall {
                fall = fall_val;
            }
            break;
        }
    }
    
    let has_health_check = backend.server.iter().any(|s| s.check.unwrap_or(false));
    if has_health_check {
        Some(HealthCheckConfig {
            interval,
            timeout,
            rise,
            fall,
        })
    } else {
        None
    }
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            maxconn: Some(4096),
            log: Some("stdout".to_string()),
            user: None,
            group: None,
            daemon: Some(false),
            pidfile: None,
            ssl_default_bind_ciphers: Some("EECDH+AESGCM:EDH+AESGCM".to_string()),
            ssl_default_bind_options: Some("no-sslv3".to_string()),
            option: Vec::new(),
        }
    }
}

impl Default for DefaultsConfig {
    fn default() -> Self {
        Self {
            mode: Some("tcp".to_string()),
            log: Some("global".to_string()),
            option: vec!["dontlognull".to_string()],
            timeout: HashMap::new(),
            retries: Some(3),
            options: None,
        }
    }
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bind: Some("0.0.0.0:9090".to_string()),
            path: Some("/metrics".to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    pub requests_per_second: u32,
    pub burst_size: u32,
    pub window_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DdosProtectionConfig {
    pub reset_interval_seconds: u64,
    pub max_requests_per_minute: Option<u32>,
    pub max_connections_per_ip: Option<u32>,
    pub suspicious_patterns: Vec<String>,
    pub whitelist: Vec<String>,
    pub blacklist: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotReloadConfig {
    pub enabled: bool,
    pub watch_interval: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionConfig {
    pub gzip_enabled: bool,
    pub brotli_enabled: bool,
    pub deflate_enabled: bool,
    pub min_size: usize,
    pub max_size: usize,
    pub compression_level: u32,
    pub content_types: Vec<String>,
}

impl Config {
    fn parse_rate_limit_config(config: &mut Config) -> Result<()> {
        let mut requests_per_second = None;
        let mut burst_size = None;
        let window_size = 1;

        for option in &config.global.option {
            let parts: Vec<&str> = option.split_whitespace().collect();
            if parts.len() >= 2 {
                match parts[0] {
                    "rate-limit-rps" => {
                        if let Ok(rate) = parts[1].parse::<u32>() {
                            requests_per_second = Some(rate);
                        }
                    },
                    "rate-limit-burst" => {
                        if let Ok(burst) = parts[1].parse::<u32>() {
                            burst_size = Some(burst);
                        }
                    },
                    _ => {},
                }
            }
        }

        if let (Some(rps), Some(burst)) = (requests_per_second, burst_size) {
            config.rate_limit = Some(RateLimitConfig {
                requests_per_second: rps,
                burst_size: burst,
                window_size,
            });
            info!("Rate limiting configured: {} req/s, burst: {}", rps, burst);
        }

        Ok(())
    }

    fn parse_ddos_protection_config(config: &mut Config) -> Result<()> {
        let mut reset_interval_seconds = 60;
        let mut max_requests_per_minute = None;
        let mut max_connections_per_ip = None;
        let mut suspicious_patterns = Vec::new();
        let mut whitelist = Vec::new();
        let mut blacklist = Vec::new();

        for option in &config.global.option {
            let parts: Vec<&str> = option.split_whitespace().collect();
            if parts.len() >= 2 {
                match parts[0] {
                    "ddos-protection" => {
                        if parts.len() >= 3 {
                            match parts[1] {
                                "reset-interval-seconds" => {
                                    if let Ok(val) = parts[2].parse::<u64>() {
                                        reset_interval_seconds = val;
                                    }
                                },
                                "max-requests-per-minute" => {
                                    if let Ok(val) = parts[2].parse::<u32>() {
                                        max_requests_per_minute = Some(val);
                                    }
                                },
                                "max-connections-per-ip" => {
                                    if let Ok(val) = parts[2].parse::<u32>() {
                                        max_connections_per_ip = Some(val);
                                    }
                                },
                                "suspicious-pattern" => {
                                    suspicious_patterns.push(parts[2].to_string());
                                },
                                "whitelist" => {
                                    whitelist.push(parts[2].to_string());
                                },
                                "blacklist" => {
                                    blacklist.push(parts[2].to_string());
                                },
                                _ => {}
                            }
                        }
                    },
                    _ => {},
                }
            }
        }

        config.ddos_protection = Some(DdosProtectionConfig {
            reset_interval_seconds,
            max_requests_per_minute,
            max_connections_per_ip,
            suspicious_patterns,
            whitelist,
            blacklist,
        });

        Ok(())
    }

    fn parse_compression_config(config: &mut Config) -> Result<()> {
        let mut gzip_enabled = false;
        let mut brotli_enabled = false;
        let mut deflate_enabled = false;
        let mut min_size = 1024;
        let mut max_size = 1024 * 1024;
        let mut compression_level = 6;
        let content_types = vec![
            "text/plain".to_string(),
            "text/html".to_string(),
            "text/css".to_string(),
            "application/javascript".to_string(),
            "application/json".to_string(),
        ];

        for option in &config.global.option {
            let parts: Vec<&str> = option.split_whitespace().collect();
            if parts.len() >= 2 {
                match parts[0] {
                    "compression-gzip" => gzip_enabled = parts[1] == "enabled",
                    "compression-brotli" => brotli_enabled = parts[1] == "enabled",
                    "compression-deflate" => deflate_enabled = parts[1] == "enabled",
                    "compression-min-size" => {
                        if let Ok(size) = parts[1].parse::<usize>() {
                            min_size = size;
                        }
                    }
                    "compression-max-size" => {
                        if let Ok(size) = parts[1].parse::<usize>() {
                            max_size = size;
                        }
                    },
                    "compression-level" => {
                        if let Ok(level) = parts[1].parse::<u32>() {
                            compression_level = level;
                        }
                    }
                    _ => {}
                }
            }
        }

        if gzip_enabled || brotli_enabled || deflate_enabled {
            config.compression = Some(CompressionConfig {
                gzip_enabled,
                brotli_enabled,
                deflate_enabled,
                min_size,
                max_size,
                compression_level,
                content_types,
            });
            info!("Compression configured: gzip={}, brotli={}, deflate={}", gzip_enabled, brotli_enabled, deflate_enabled);
        }

        Ok(())
    }
}
