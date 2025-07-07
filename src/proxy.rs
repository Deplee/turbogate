use crate::config::{FrontendConfig, BackendConfig, ServerConfig};
use crate::logging::{RequestLogger, log_startup_info, log_graceful_shutdown};
use crate::metrics;
use crate::health::{HealthChecker, ServerStatus};
use crate::balancer::BackendLoadBalancer;
use crate::acl::Acl;
use anyhow::{Result, anyhow};
use dashmap::DashMap;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tokio::task;
use tracing::{info, warn, error, debug};
use crate::features::FeaturesManager;

pub struct ProxyServer {
    frontends: Arc<DashMap<String, FrontendState>>,
    backends: Arc<DashMap<String, BackendState>>,
    health_checkers: Arc<DashMap<String, HealthChecker>>,
    active_connections: Arc<RwLock<HashMap<String, u64>>>,
    server_statuses: Arc<RwLock<HashMap<String, HashMap<String, ServerStatus>>>>,
    features_manager: Arc<FeaturesManager>,
}

struct FrontendState {
    config: FrontendConfig,
    listeners: Vec<Arc<TcpListener>>,
}

struct BackendState {
    config: BackendConfig,
    load_balancer: BackendLoadBalancer,
}

impl ProxyServer {
    pub fn new(features_manager: Arc<FeaturesManager>) -> Self {
        Self {
            frontends: Arc::new(DashMap::new()),
            backends: Arc::new(DashMap::new()),
            health_checkers: Arc::new(DashMap::new()),
            active_connections: Arc::new(RwLock::new(HashMap::new())),
            server_statuses: Arc::new(RwLock::new(HashMap::new())),
            features_manager,
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        self.initialize_frontends().await?;
        self.initialize_backends().await?;
        
        self.start_health_checkers().await;

        let bind_addresses: Vec<String> = self.features_manager.config.frontends.iter()
            .flat_map(|f| f.bind.clone())
            .collect();
        log_startup_info("0.1.0", "turbogate.toml", bind_addresses);

        let mut shutdown_signal = Self::setup_shutdown_signal();

        let ddos_reset_task = {
            let features_manager = Arc::clone(&self.features_manager);
            task::spawn(async move {
                loop {
                    let interval = {
                        if let Some(ddos) = &features_manager.ddos_protection {
                            let val = ddos.reset_interval_seconds();
                            if val < 1 { 60 } else { val }
                        } else {
                            60
                        }
                    };
                    info!("DDoS: ожидаю {} секунд до следующего сброса счетчиков", interval);
                    tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
                    if let Some(ddos) = &features_manager.ddos_protection {
                        ddos.reset_counters();
                        info!("DDoS: счетчики сброшены (reset_counters)");
                    }
                }
            })
        };

        let mut frontend_tasks = Vec::new();
        for frontend_name in self.frontends.iter().map(|f| f.key().clone()) {
            let frontends = Arc::clone(&self.frontends);
            let backends = Arc::clone(&self.backends);
            let active_connections = Arc::clone(&self.active_connections);
            let server_statuses = Arc::clone(&self.server_statuses);
            let features_manager = Arc::clone(&self.features_manager);
            
            let task = task::spawn(async move {
                if let Err(e) = Self::run_frontend(frontend_name, frontends, backends, active_connections, server_statuses, features_manager).await {
                    error!("Error running frontend: {}", e);
                }
            });
            frontend_tasks.push(task);
        }

        shutdown_signal.recv().await;
        
        let active_conns = self.active_connections.read().await.values().sum();
        log_graceful_shutdown(active_conns);
        
        ddos_reset_task.abort();
        for task in frontend_tasks {
            task.abort();
        }

        info!("Proxy server stopped");
        Ok(())
    }

    async fn initialize_frontends(&mut self) -> Result<()> {
        for frontend_config in &self.features_manager.config.frontends {
            let mut listeners = Vec::new();
            
            for bind_addr in &frontend_config.bind {
                let addr: SocketAddr = bind_addr.parse()?;
                let listener = TcpListener::bind(addr).await?;
                listeners.push(Arc::new(listener));
                info!("Frontend '{}' listening on {}", frontend_config.name, bind_addr);
            }

            let frontend_state = FrontendState {
                config: frontend_config.clone(),
                listeners,
            };

            self.frontends.insert(frontend_config.name.clone(), frontend_state);
        }

        Ok(())
    }

    async fn initialize_backends(&mut self) -> Result<()> {
        for backend_config in &self.features_manager.config.backends {
            let algorithm = backend_config.balance.as_deref().unwrap_or("roundrobin");
            let load_balancer = BackendLoadBalancer::new(backend_config.server.clone(), algorithm)?;
            
            let backend_state = BackendState {
                config: backend_config.clone(),
                load_balancer,
            };

            self.backends.insert(backend_config.name.clone(), backend_state);
        }

        Ok(())
    }

    async fn start_health_checkers(&self) {
        for backend_config in &self.features_manager.config.backends {
            if backend_config.health_check.is_some() {
                let health_checker = HealthChecker::new(backend_config.clone());
                self.health_checkers.insert(backend_config.name.clone(), health_checker);
            }
        }

        for health_checker in self.health_checkers.iter() {
            health_checker.value().start().await;
        }
    }

    async fn run_frontend(
        frontend_name: String,
        frontends: Arc<DashMap<String, FrontendState>>,
        backends: Arc<DashMap<String, BackendState>>,
        active_connections: Arc<RwLock<HashMap<String, u64>>>,
        server_statuses: Arc<RwLock<HashMap<String, HashMap<String, ServerStatus>>>>,
        features_manager: Arc<FeaturesManager>,
    ) -> Result<()> {
        if let Some(frontend_state) = frontends.get(&frontend_name) {
            for listener in &frontend_state.listeners {
                let listener = Arc::clone(listener);
                let frontend_name = frontend_name.clone();
                let frontends = Arc::clone(&frontends);
                let backends = Arc::clone(&backends);
                let active_connections = Arc::clone(&active_connections);
                let server_statuses = Arc::clone(&server_statuses);
                let features_manager = Arc::clone(&features_manager);
                
                task::spawn(async move {
                    if let Err(e) = Self::accept_connections(
                        &listener,
                        &frontend_name,
                        frontends,
                        backends,
                        active_connections,
                        server_statuses,
                        features_manager,
                    ).await {
                        error!("Error accepting connections on frontend {}: {}", frontend_name, e);
                    }
                });
            }
        }

        Ok(())
    }

    async fn accept_connections(
        listener: &TcpListener,
        frontend_name: &str,
        frontends: Arc<DashMap<String, FrontendState>>,
        backends: Arc<DashMap<String, BackendState>>,
        active_connections: Arc<RwLock<HashMap<String, u64>>>,
        server_statuses: Arc<RwLock<HashMap<String, HashMap<String, ServerStatus>>>>,
        features_manager: Arc<FeaturesManager>,
    ) -> Result<()> {
        loop {
            let (client_stream, client_addr) = listener.accept().await?;
            
            let max_connections = features_manager.config.global.maxconn.unwrap_or(4096);
            let current_connections = {
                let conns = active_connections.read().await;
                conns.values().sum::<u64>()
            };
            
            if current_connections >= max_connections as u64 {
                warn!("Max connections limit reached: {} >= {}", current_connections, max_connections);
                metrics::connection_error(frontend_name, "maxconn_limit");
                continue;
            }
            
            {
                let mut conns = active_connections.write().await;
                *conns.entry(frontend_name.to_string()).or_insert(0) += 1;
            }

            let frontend_name = frontend_name.to_string();
            let frontends = Arc::clone(&frontends);
            let backends = Arc::clone(&backends);
            let active_connections = Arc::clone(&active_connections);
            let server_statuses = Arc::clone(&server_statuses);
            let features_manager = Arc::clone(&features_manager);
            
            let handle_timeout = std::time::Duration::from_secs(30);

            task::spawn(async move {
                match tokio::time::timeout(handle_timeout, Self::handle_connection(
                    client_stream,
                    client_addr,
                    &frontend_name,
                    frontends,
                    backends,
                    server_statuses,
                    features_manager,
                )).await {
                    Ok(Ok(())) => {
                        debug!("Connection from {} handled successfully", client_addr);
                    }
                    Ok(Err(e)) => {
                        error!("Error handling connection from {}: {}", client_addr, e);
                        metrics::connection_error(&frontend_name, "handle_error");
                    }
                    Err(_) => {
                        error!("Connection from {} timed out after {:?}", client_addr, handle_timeout);
                        metrics::connection_error(&frontend_name, "handle_timeout");
                    }
                }

                {
                    let mut conns = active_connections.write().await;
                    if let Some(count) = conns.get_mut(&frontend_name) {
                        *count = count.saturating_sub(1);
                    }
                }

                metrics::connection_closed(&frontend_name);
            });
        }
    }

    async fn handle_connection(
        client_stream: TcpStream,
        client_addr: SocketAddr,
        frontend_name: &str,
        frontends: Arc<DashMap<String, FrontendState>>,
        backends: Arc<DashMap<String, BackendState>>,
        server_statuses: Arc<RwLock<HashMap<String, HashMap<String, ServerStatus>>>>,
        features_manager: Arc<FeaturesManager>,
    ) -> Result<()> {
        if let Some(rate_limiter) = &features_manager.rate_limiter {
            if !rate_limiter.check_rate_limit(client_addr.ip()) {
                warn!("Rate limit exceeded for client {} on frontend {}", client_addr.ip(), frontend_name);
                metrics::connection_error(frontend_name, "rate_limit_exceeded");
                return Err(anyhow!("Rate limit exceeded"));
            }
        }

        if let Some(ddos_protection) = &features_manager.ddos_protection {
            if !ddos_protection.check_connection_limit(client_addr.ip()) {
                warn!("DDoS protection: connection limit exceeded for client {} on frontend {}", client_addr.ip(), frontend_name);
                metrics::connection_error(frontend_name, "ddos_connection_limit");
                return Err(anyhow!("DDoS protection: connection limit exceeded"));
            }
        }

        let frontend_config = if let Some(frontend_state) = frontends.get(frontend_name) {
            frontend_state.config.clone()
        } else {
            return Err(anyhow!("Frontend '{}' not found", frontend_name));
        };

        let backend_name = Self::select_backend(&frontend_config, client_addr)?;
        
        let mut backend_state = if let Some(backend) = backends.get_mut(&backend_name) {
            backend
        } else {
            return Err(anyhow!("Backend '{}' not found", backend_name));
        };

        let server = Self::select_server(&mut backend_state, &server_statuses).await?;
        
        let start_time = std::time::Instant::now();
        let logger = RequestLogger::new(
            client_addr.ip().to_string(),
            backend_name.clone(),
            server.name.clone(),
        );

        logger.log_request_start();
        metrics::request_started(&backend_name, &server.name);

        if let Some(ddos_protection) = &features_manager.ddos_protection {
            if !ddos_protection.check_rate_limit(client_addr.ip()) {
                warn!("DDoS protection: rate limit exceeded for client {} on frontend {}", client_addr.ip(), frontend_name);
                metrics::connection_error(frontend_name, "ddos_rate_limit");
                return Err(anyhow!("DDoS protection: rate limit exceeded"));
            }
        }

        match Self::proxy_connection(client_stream, &server).await {
            Ok(()) => {
                let duration = start_time.elapsed();
                logger.log_request_end("success", 0);
                metrics::request_completed(&backend_name, &server.name, "success", duration.as_millis() as u64);
                
                if let Some(ddos_protection) = &features_manager.ddos_protection {
                    ddos_protection.connection_closed(client_addr.ip());
                }
                
                Ok(())
            }
            Err(e) => {
                logger.log_request_end("failure", 0);
                metrics::request_failed(&backend_name, &server.name, "connection_failed");
                
                if let Some(ddos_protection) = &features_manager.ddos_protection {
                    ddos_protection.connection_closed(client_addr.ip());
                }
                
                Err(e)
            }
        }
    }

    fn select_backend(frontend_config: &FrontendConfig, client_addr: SocketAddr) -> Result<String> {
        if let Some(ref backend_name) = frontend_config.default_backend {
            return Ok(backend_name.clone());
        }

        for use_backend in &frontend_config.use_backend {
            if let Some(ref condition) = use_backend.condition {
                if Self::evaluate_acl_condition(condition, client_addr)? {
                    return Ok(use_backend.backend.clone());
                }
            }
        }

        Err(anyhow!("No backend selected for frontend"))
    }

    async fn select_server(
        backend_state: &mut BackendState,
        server_statuses: &Arc<RwLock<HashMap<String, HashMap<String, ServerStatus>>>>,
    ) -> Result<ServerConfig> {
        let statuses = server_statuses.read().await;
        let backend_statuses = statuses.get(&backend_state.config.name);
        
        let available_servers: Vec<&ServerConfig> = backend_state.config.server.iter()
            .filter(|server| {
                if server.disabled.unwrap_or(false) {
                    return false;
                }
                
                if let Some(ref statuses) = backend_statuses {
                    if let Some(status) = statuses.get(&server.name) {
                        return status == &ServerStatus::Up;
                    }
                }
                
                true
            })
            .collect();

        if available_servers.is_empty() {
            return Err(anyhow!("No healthy servers available"));
        }

        let selected_server = backend_state.load_balancer.select_server()?;
        if let Some(server_state) = selected_server {
            Ok(server_state.config.clone())
        } else {
            Err(anyhow!("No server selected by load balancer"))
        }
    }

    async fn proxy_connection(client_stream: TcpStream, server: &ServerConfig) -> Result<()> {
        let server_addr = format!("{}:{}", server.address, server.port);
        let server_stream = TcpStream::connect(&server_addr).await?;

        let (mut client_read, mut client_write) = client_stream.into_split();
        let (mut server_read, mut server_write) = server_stream.into_split();

        let client_to_server = tokio::io::copy(&mut client_read, &mut server_write);
        let server_to_client = tokio::io::copy(&mut server_read, &mut client_write);

        tokio::select! {
            result = client_to_server => {
                if let Err(e) = result {
                    return Err(anyhow!("Client to server error: {}", e));
                }
            }
            result = server_to_client => {
                if let Err(e) = result {
                    return Err(anyhow!("Server to client error: {}", e));
                }
            }
        }

        Ok(())
    }

    fn evaluate_acl_condition(condition: &str, client_addr: SocketAddr) -> Result<bool> {
        let acl_config = crate::config::AclConfig {
            name: "temp".to_string(),
            criterion: condition.to_string(),
        };
        let acl = Acl::from_config(&acl_config)?;
        acl.evaluate(client_addr)
    }

    fn setup_shutdown_signal() -> tokio::signal::unix::Signal {
        use tokio::signal::unix::{signal, SignalKind};
        signal(SignalKind::terminate()).expect("Failed to create signal handler")
    }
}
