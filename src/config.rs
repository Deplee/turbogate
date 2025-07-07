use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use tokio::fs;
use anyhow::{Result, anyhow};
use tracing::{debug, warn, info};
use crate::options::Options;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub global: GlobalConfig,
    pub defaults: DefaultsConfig,
    pub frontends: Vec<FrontendConfig>,
    pub backends: Vec<BackendConfig>,
    pub metrics: MetricsConfig,
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
        };
        
        // Список stats bind адресов
        let mut stats_binds = Vec::new();

        let mut current_section = None;
        let mut current_frontend: Option<FrontendConfig> = None;
        let mut current_backend: Option<BackendConfig> = None;

        for (line_num, line) in content.lines().enumerate() {
            let line = line.trim();
            
            // Пропускаем комментарии и пустые строки
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            debug!("Parsing line {}: '{}'", line_num + 1, line);

            match parse_line(line, line_num + 1)? {
                LineType::Section(section) => {
                        // Сохраняем предыдущую секцию
    if let Some(mut frontend) = current_frontend.take() {
        // Создаем Options для frontend
        let mode = frontend.mode.as_deref().unwrap_or("tcp");
        frontend.options = Some(Options::from_strings(&frontend.option, mode)?);
        config.frontends.push(frontend);
    }
    if let Some(mut backend) = current_backend.take() {
        // Создаем Options для backend
        let mode = backend.mode.as_deref().unwrap_or("tcp");
        backend.options = Some(Options::from_strings(&backend.option, mode)?);
        
        // Создаем HealthCheckConfig из параметров серверов
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

        // Сохраняем последние секции
        if let Some(mut frontend) = current_frontend {
            let mode = frontend.mode.as_deref().unwrap_or("tcp");
            frontend.options = Some(Options::from_strings(&frontend.option, mode)?);
            config.frontends.push(frontend);
        }
        if let Some(mut backend) = current_backend {
            let mode = backend.mode.as_deref().unwrap_or("tcp");
            backend.options = Some(Options::from_strings(&backend.option, mode)?);
            
            // Создаем HealthCheckConfig из параметров серверов
            backend.health_check = create_health_check_config(&backend);
            
            config.backends.push(backend);
        }

        // Создаем Options для defaults
        let mode = config.defaults.mode.as_deref().unwrap_or("tcp");
        config.defaults.options = Some(Options::from_strings(&config.defaults.option, mode)?);

        // Настраиваем метрики на основе stats bind
        if !stats_binds.is_empty() {
            // Используем первый stats bind адрес
            config.metrics.bind = Some(stats_binds[0].clone());
            info!("Metrics will be available on: {}", stats_binds[0]);
        }

        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        // Проверяем, что все frontend'ы ссылаются на существующие backend'ы
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

        // Проверяем, что у каждого backend'а есть хотя бы один сервер
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
            // Поддержка stats bind для метрик
            if value.starts_with("bind") {
                let parts: Vec<&str> = value.split_whitespace().collect();
                if parts.len() >= 2 {
                    // Это будет обработано в Config::from_haproxy_config
                }
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
                
                // Применяем timeout к options
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
        _ => warn!("Unknown defaults directive: {}", key),
    }
    
    Ok(())
}

fn parse_frontend_directive(frontend: &mut FrontendConfig, key: &str, value: &str) -> Result<()> {
    match key {
        "bind" => frontend.bind.push(value.to_string()),
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
                
                // Применяем timeout к options
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
        _ => warn!("Unknown frontend directive: {}", key),
    }
    
    Ok(())
}

fn parse_backend_directive(backend: &mut BackendConfig, key: &str, value: &str) -> Result<()> {
    match key {
        "mode" => backend.mode = Some(value.to_string()),
        "balance" => {
            // Извлекаем первый алгоритм балансировки (до комментария или пробела)
            let algorithm = value.split('#').next().unwrap_or(value).trim();
            backend.balance = Some(algorithm.to_string());
        },
        "server" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if parts.len() >= 2 {
                let server_name = parts[0].to_string();
                let server_addr = parts[1].to_string();
                let server_name_clone = server_name.clone();
                
                // Разбираем адрес и порт
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
                    weight: Some(1), // По умолчанию вес 1
                    maxconn: None,
                    check: None,
                    inter: None,
                    rise: None,
                    fall: None,
                    backup: None,
                    disabled: None,
                };

                // Парсим дополнительные параметры
                let mut i = 2;
                while i < parts.len() {
                    let part = parts[i];
                    match part {
                        "weight" => {
                            if i + 1 < parts.len() {
                                let weight_str = parts[i + 1];
                                server.weight = Some(weight_str.parse().unwrap_or(1));
                                tracing::debug!("Parsed weight for server {}: {}", server_name_clone, server.weight.unwrap());
                                i += 2; // Пропускаем и "weight" и значение
                            } else {
                                i += 1;
                            }
                        },
                        "maxconn" => {
                            if i + 1 < parts.len() {
                                server.maxconn = Some(parts[i + 1].parse().unwrap_or(1000));
                                i += 2;
                            } else {
                                i += 1;
                            }
                        },
                        "check" => {
                            server.check = Some(true);
                            i += 1;
                        },
                        "inter" => {
                            if i + 1 < parts.len() {
                                server.inter = Some(parts[i + 1].to_string());
                                i += 2;
                            } else {
                                i += 1;
                            }
                        },
                        "rise" => {
                            if i + 1 < parts.len() {
                                server.rise = Some(parts[i + 1].parse().unwrap_or(2));
                                i += 2;
                            } else {
                                i += 1;
                            }
                        },
                        "fall" => {
                            if i + 1 < parts.len() {
                                server.fall = Some(parts[i + 1].parse().unwrap_or(3));
                                i += 2;
                            } else {
                                i += 1;
                            }
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
                            // Неизвестный параметр, пропускаем
                            i += 1;
                        }
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
                
                // Применяем timeout к options
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

// Создаем HealthCheckConfig из параметров серверов
fn create_health_check_config(backend: &BackendConfig) -> Option<HealthCheckConfig> {
    let mut interval = "2s".to_string();
    let timeout = "1s".to_string();
    let mut rise = 2;
    let mut fall = 3;
    
    // Ищем параметры health check среди серверов с health check
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
            // Берем параметры только от первого сервера с health check
            break;
        }
    }
    
    // Создаем конфигурацию только если есть серверы с health check
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
