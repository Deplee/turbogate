use crate::config::{BackendConfig, ServerConfig};
use crate::logging;
use crate::metrics;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::{debug, info, warn, error};

#[derive(Debug, Clone, PartialEq)]
pub enum ServerStatus {
    Up,
    Down,
    Maintenance,
}

#[derive(Debug, Clone)]
pub struct HealthState {
    pub status: ServerStatus,
    pub last_check: Instant,
    pub consecutive_failures: u32,
    pub consecutive_successes: u32,
    pub last_success: Option<Instant>,
    pub last_failure: Option<Instant>,
}

impl Default for HealthState {
    fn default() -> Self {
        Self {
            status: ServerStatus::Up,
            last_check: Instant::now(),
            consecutive_failures: 0,
            consecutive_successes: 0,
            last_success: None,
            last_failure: None,
        }
    }
}

pub struct HealthChecker {
    backends: Arc<RwLock<HashMap<String, BackendHealthState>>>,
    config: BackendConfig,
}

#[derive(Clone)]
struct BackendHealthState {
    servers: HashMap<String, HealthState>,
    rise_threshold: u32,
    fall_threshold: u32,
    check_interval: Duration,
    check_timeout: Duration,
}

impl HealthChecker {
    pub fn new(config: BackendConfig) -> Self {
        let mut servers = HashMap::new();
        let rise_threshold = config.health_check.as_ref()
            .map(|hc| hc.rise)
            .unwrap_or(2);
        let fall_threshold = config.health_check.as_ref()
            .map(|hc| hc.fall)
            .unwrap_or(3);
        let check_interval = config.health_check.as_ref()
            .and_then(|hc| parse_duration(&hc.interval))
            .unwrap_or(Duration::from_secs(2));
        let check_timeout = config.health_check.as_ref()
            .and_then(|hc| parse_duration(&hc.timeout))
            .unwrap_or(Duration::from_secs(1));

        for server in &config.server {
            if server.check.unwrap_or(false) {
                servers.insert(server.name.clone(), HealthState::default());
            }
        }

        let backend_state = BackendHealthState {
            servers,
            rise_threshold,
            fall_threshold,
            check_interval,
            check_timeout,
        };

        let mut backends = HashMap::new();
        backends.insert(config.name.clone(), backend_state);

        Self {
            backends: Arc::new(RwLock::new(backends)),
            config,
        }
    }

    pub async fn start(&self) {
        let backends = Arc::clone(&self.backends);
        let config = self.config.clone();

        tokio::spawn(async move {
            Self::run_health_checks(backends, config).await;
        });
    }

    async fn run_health_checks(
        backends: Arc<RwLock<HashMap<String, BackendHealthState>>>,
        config: BackendConfig,
    ) {
        let check_interval = config.health_check.as_ref()
            .and_then(|hc| parse_duration(&hc.interval))
            .unwrap_or(Duration::from_secs(2));

        loop {
            let backend_name = config.name.clone();
            
            if let Some(backend_state) = backends.read().await.get(&backend_name) {
                let mut updated_servers = backend_state.servers.clone();
                
                for server in &config.server {
                    if server.check.unwrap_or(false) {
                        if let Some(health_state) = updated_servers.get_mut(&server.name) {
                            Self::check_server_health(server, health_state, &backend_state).await;
                        }
                    }
                }

                // Освобождаем read lock перед получением write lock
                drop(backends.read().await);

                // Обновляем состояние
                {
                    debug!("Updating backend state");
                    match tokio::time::timeout(Duration::from_secs(1), async {
                        let mut backends_write = backends.write().await;
                        if let Some(backend_state) = backends_write.get_mut(&backend_name) {
                            backend_state.servers = updated_servers.clone();
                            debug!("Backend state updated successfully");
                        } else {
                            warn!("Failed to update backend state - backend not found");
                        }
                    }).await {
                        Ok(_) => debug!("Backend state update completed"),
                        Err(_) => {
                            error!("Backend state update timed out - possible deadlock");
                            return;
                        }
                    }
                }

                // Логируем статистику
                let active_servers = updated_servers.values()
                    .filter(|state| matches!(state.status, ServerStatus::Up))
                    .count();
                let total_servers = updated_servers.len();

                logging::log_backend_status(&backend_name, active_servers, total_servers);
                metrics::backend_active_servers(&backend_name, active_servers);
                metrics::backend_total_servers(&backend_name, total_servers);
            }

            sleep(check_interval).await;
        }
    }

    async fn check_server_health(
        server: &ServerConfig,
        health_state: &mut HealthState,
        backend_state: &BackendHealthState,
    ) {
        let server_addr = format!("{}:{}", server.address, server.port);
        let start_time = Instant::now();

        debug!("Performing health check for server '{}' at {}", server.name, server_addr);

        match Self::perform_health_check(&server_addr, backend_state.check_timeout).await {
            Ok(_) => {
                health_state.consecutive_successes += 1;
                health_state.consecutive_failures = 0;
                health_state.last_success = Some(Instant::now());
                health_state.last_check = Instant::now();

                debug!("Health check SUCCESS for server '{}': consecutive_successes={}, rise_threshold={}", 
                       server.name, health_state.consecutive_successes, backend_state.rise_threshold);

                if health_state.consecutive_successes >= backend_state.rise_threshold {
                    if !matches!(health_state.status, ServerStatus::Up) {
                        health_state.status = ServerStatus::Up;
                        logging::log_server_status(&server.name, "up", None);
                        metrics::server_status_changed(&server.name, "up");
                        info!("Server {} is now UP", server.name);
                    }
                }

                metrics::health_check(&server.name, true);
                logging::log_server_status(&server.name, "healthy", None);
            }
            Err(e) => {
                health_state.consecutive_failures += 1;
                health_state.consecutive_successes = 0;
                health_state.last_failure = Some(Instant::now());
                health_state.last_check = Instant::now();

                debug!("Health check FAILED for server '{}': consecutive_failures={}, fall_threshold={}, error={}", 
                       server.name, health_state.consecutive_failures, backend_state.fall_threshold, e);

                if health_state.consecutive_failures >= backend_state.fall_threshold {
                    if !matches!(health_state.status, ServerStatus::Down) {
                        health_state.status = ServerStatus::Down;
                        logging::log_server_status(&server.name, "down", Some(&e.to_string()));
                        metrics::server_status_changed(&server.name, "down");
                        warn!("Server {} is now DOWN: {}", server.name, e);
                    }
                }

                metrics::health_check(&server.name, false);
                debug!("Health check failed for {}: {}", server.name, e);
            }
        }

        let duration = start_time.elapsed();
        debug!("Health check completed for server '{}' in {:?}", server.name, duration);
    }

    async fn perform_health_check(addr: &str, timeout: Duration) -> anyhow::Result<()> {
        let socket_addr: SocketAddr = addr.parse()?;
        
        match tokio::time::timeout(timeout, TcpStream::connect(socket_addr)).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(anyhow::anyhow!("Connection failed: {}", e)),
            Err(_) => Err(anyhow::anyhow!("Health check timeout")),
        }
    }

    pub async fn get_server_status(&self, server_name: &str) -> Option<ServerStatus> {
        let backends = self.backends.read().await;
        if let Some(backend_state) = backends.get(&self.config.name) {
            backend_state.servers.get(server_name)
                .map(|state| state.status.clone())
        } else {
            None
        }
    }

    pub async fn get_healthy_servers(&self) -> Vec<String> {
        let backends = self.backends.read().await;
        if let Some(backend_state) = backends.get(&self.config.name) {
            backend_state.servers.iter()
                .filter(|(_, state)| matches!(state.status, ServerStatus::Up))
                .map(|(name, _)| name.clone())
                .collect()
        } else {
            Vec::new()
        }
    }

    pub async fn get_all_servers(&self) -> Vec<String> {
        let backends = self.backends.read().await;
        if let Some(backend_state) = backends.get(&self.config.name) {
            backend_state.servers.keys().cloned().collect()
        } else {
            Vec::new()
        }
    }

    pub async fn set_server_maintenance(&self, server_name: &str, maintenance: bool) {
        let mut backends = self.backends.write().await;
        if let Some(backend_state) = backends.get_mut(&self.config.name) {
            if let Some(health_state) = backend_state.servers.get_mut(server_name) {
                health_state.status = if maintenance {
                    ServerStatus::Maintenance
                } else {
                    ServerStatus::Up
                };
                
                let status_str = if maintenance { "maintenance" } else { "up" };
                logging::log_server_status(server_name, status_str, None);
                metrics::server_status_changed(server_name, status_str);
            }
        }
    }

    pub async fn get_all_server_statuses(&self) -> HashMap<String, ServerStatus> {
        let backends = self.backends.read().await;
        if let Some(backend_state) = backends.get(&self.config.name) {
            backend_state.servers.iter()
                .map(|(name, state)| (name.clone(), state.status.clone()))
                .collect()
        } else {
            HashMap::new()
        }
    }

    pub async fn start_with_callback(
        &self,
        backend_name: String,
        server_statuses: Arc<RwLock<HashMap<String, HashMap<String, ServerStatus>>>>,
    ) {
        let backends = Arc::clone(&self.backends);
        let config = self.config.clone();

        tokio::spawn(async move {
            Self::run_health_checks_with_callback(backends, config, backend_name, server_statuses).await;
        });
    }

    async fn run_health_checks_with_callback(
        backends: Arc<RwLock<HashMap<String, BackendHealthState>>>,
        config: BackendConfig,
        backend_name: String,
        server_statuses: Arc<RwLock<HashMap<String, HashMap<String, ServerStatus>>>>,
    ) {
        let check_interval = config.health_check.as_ref()
            .and_then(|hc| parse_duration(&hc.interval))
            .unwrap_or(Duration::from_secs(2));

        info!("Health checker started for backend '{}' with interval {:?}", backend_name, check_interval);

        loop {
            debug!("=== Starting health check cycle for backend '{}' ===", backend_name);
            
            if let Err(e) = async {
                debug!("Running health checks for backend '{}'", backend_name);
                
                // Получаем данные и сразу освобождаем read lock
                let backend_state_opt = backends.read().await.get(&backend_name).cloned();
                if let Some(backend_state) = backend_state_opt {
                    debug!("Found backend state, checking {} servers", backend_state.servers.len());
                    let mut updated_servers = backend_state.servers.clone();
                    
                    for server in &config.server {
                        if server.check.unwrap_or(false) {
                            debug!("Checking server '{}' at {}:{}", server.name, server.address, server.port);
                            if let Some(health_state) = updated_servers.get_mut(&server.name) {
                                Self::check_server_health(server, health_state, &backend_state).await;
                            } else {
                                warn!("Server '{}' not found in health state", server.name);
                            }
                        } else {
                            debug!("Server '{}' has health check disabled", server.name);
                        }
                    }

                    // Сохраняем обновленное состояние обратно в основное хранилище
                    {
                        debug!("Saving updated health state for backend '{}'", backend_name);
                        match tokio::time::timeout(Duration::from_secs(1), async {
                            let mut backends_write = backends.write().await;
                            if let Some(backend_state) = backends_write.get_mut(&backend_name) {
                                backend_state.servers = updated_servers.clone();
                                debug!("Health state saved successfully");
                            } else {
                                warn!("Failed to save health state - backend not found");
                            }
                        }).await {
                            Ok(_) => debug!("Health state save completed"),
                            Err(_) => {
                                error!("Health state save timed out - possible deadlock");
                                return Err(anyhow::anyhow!("Health state save timed out"));
                            }
                        }
                    }

                    // Обновляем shared state для интеграции с балансировщиком
                    {
                        debug!("Updating shared state for backend '{}'", backend_name);
                        match tokio::time::timeout(Duration::from_secs(1), async {
                            let mut statuses = server_statuses.write().await;
                            let backend_statuses = statuses.entry(backend_name.clone()).or_insert_with(HashMap::new);
                            debug!("Found {} existing server statuses", backend_statuses.len());
                            
                            for (server_name, health_state) in &updated_servers {
                                let old_status = backend_statuses.get(server_name).cloned();
                                backend_statuses.insert(server_name.clone(), health_state.status.clone());
                                if old_status != Some(health_state.status.clone()) {
                                    info!("Shared state updated: server '{}' status changed from {:?} to {:?}", 
                                          server_name, old_status, health_state.status);
                                } else {
                                    debug!("Updated shared state: server '{}' = {:?}", server_name, health_state.status);
                                }
                            }
                            debug!("Shared state updated, now has {} server statuses", backend_statuses.len());
                        }).await {
                            Ok(_) => debug!("Shared state update completed"),
                            Err(_) => {
                                error!("Shared state update timed out - possible deadlock");
                                return Err(anyhow::anyhow!("Shared state update timed out"));
                            }
                        }
                    }

                    // Логируем статистику
                    let active_servers = updated_servers.values()
                        .filter(|state| matches!(state.status, ServerStatus::Up))
                        .count();
                    let total_servers = updated_servers.len();

                    info!("Backend '{}' health check summary: {}/{} servers active", 
                          backend_name, active_servers, total_servers);
                    
                    logging::log_backend_status(&backend_name, active_servers, total_servers);
                    metrics::backend_active_servers(&backend_name, active_servers);
                    metrics::backend_total_servers(&backend_name, total_servers);
                } else {
                    warn!("Backend '{}' not found in health checker state", backend_name);
                }

                debug!("Health check cycle completed for backend '{}', sleeping for {:?}", backend_name, check_interval);
                sleep(check_interval).await;
                debug!("Woke up from sleep, starting next cycle");
                Ok::<(), anyhow::Error>(())
            }.await {
                error!("Health check cycle failed for backend '{}': {}", backend_name, e);
                debug!("Sleeping before retry due to error");
                sleep(check_interval).await;
            }
            
            debug!("=== Completed health check cycle for backend '{}' ===", backend_name);
        }
    }
}

fn parse_duration(s: &str) -> Option<Duration> {
    if s.ends_with("ms") {
        s[..s.len()-2].parse::<u64>().ok().map(Duration::from_millis)
    } else if s.ends_with('s') {
        s[..s.len()-1].parse::<u64>().ok().map(Duration::from_secs)
    } else if s.ends_with('m') {
        s[..s.len()-1].parse::<u64>().ok().map(|m| Duration::from_secs(m * 60))
    } else if s.ends_with('h') {
        s[..s.len()-1].parse::<u64>().ok().map(|h| Duration::from_secs(h * 3600))
    } else {
        // Попробуем как секунды
        s.parse::<u64>().ok().map(Duration::from_secs)
    }
}

// HTTP health check (для будущего расширения)
pub async fn perform_http_health_check(
    addr: &str,
    path: &str,
    timeout: Duration,
) -> anyhow::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    
    let socket_addr: SocketAddr = addr.parse()?;
    let stream = tokio::time::timeout(timeout, TcpStream::connect(socket_addr)).await??;
    
    let request = format!(
        "GET {} HTTP/1.1\r\n\
         Host: {}\r\n\
         Connection: close\r\n\
         \r\n",
        path, addr
    );
    
    let (mut read, mut write) = stream.into_split();
    write.write_all(request.as_bytes()).await?;
    drop(write);
    
    let mut response = Vec::new();
    read.read_to_end(&mut response).await?;
    
    let response_str = String::from_utf8_lossy(&response);
    if response_str.contains("200 OK") {
        Ok(())
    } else {
        Err(anyhow::anyhow!("HTTP health check failed: {}", response_str.lines().next().unwrap_or("Unknown")))
    }
}

// TCP health check с кастомными данными
pub async fn perform_tcp_health_check_with_data(
    addr: &str,
    data: &[u8],
    expected_response: Option<&[u8]>,
    timeout: Duration,
) -> anyhow::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    
    let socket_addr: SocketAddr = addr.parse()?;
    let mut stream = tokio::time::timeout(timeout, TcpStream::connect(socket_addr)).await??;
    
    stream.write_all(data).await?;
    
    if let Some(expected) = expected_response {
        let mut response = vec![0; expected.len()];
        let n = tokio::time::timeout(timeout, stream.read(&mut response)).await??;
        
        if n != expected.len() || &response[..n] != expected {
            return Err(anyhow::anyhow!("Unexpected response from health check"));
        }
    }
    
    Ok(())
}
