use crate::config::{Config, FrontendConfig, BackendConfig, ServerConfig};
use crate::logging::{RequestLogger, log_startup_info, log_shutdown_signal, log_graceful_shutdown};
use crate::metrics;
use crate::health::{HealthChecker, ServerStatus};
use crate::acl::{AclManager, Acl, RequestData};
use crate::balancer::{BackendLoadBalancer, ServerState};
use crate::utils;
use crate::options::Options;
use anyhow::{Result, anyhow};
use dashmap::DashMap;
use std::collections::HashMap;
use std::net::{SocketAddr, IpAddr};
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tokio::task;
use tracing::{info, warn, error, debug};

pub struct ProxyServer {
    config: Config,
    frontends: Arc<DashMap<String, FrontendState>>,
    backends: Arc<DashMap<String, BackendState>>,
    health_checkers: Arc<DashMap<String, HealthChecker>>,
    active_connections: Arc<RwLock<HashMap<String, u64>>>,
    server_statuses: Arc<RwLock<HashMap<String, HashMap<String, ServerStatus>>>>,
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
    pub fn new(config: Config) -> Self {
        Self {
            frontends: Arc::new(DashMap::new()),
            backends: Arc::new(DashMap::new()),
            health_checkers: Arc::new(DashMap::new()),
            active_connections: Arc::new(RwLock::new(HashMap::new())),
            server_statuses: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        // Инициализируем frontend'ы и backend'ы
        self.initialize_frontends().await?;
        self.initialize_backends().await?;
        
        // Запускаем health checkers
        self.start_health_checkers().await;

        // Логируем информацию о запуске
        let bind_addresses: Vec<String> = self.config.frontends.iter()
            .flat_map(|f| f.bind.clone())
            .collect();
        log_startup_info("0.1.0", "turbogate.toml", bind_addresses);

        // Запускаем обработку сигналов завершения
        let shutdown_signal = Self::setup_shutdown_signal();

        // Запускаем все frontend'ы
        let mut frontend_tasks = Vec::new();
        for frontend_name in self.frontends.iter().map(|f| f.key().clone()) {
            let frontends = Arc::clone(&self.frontends);
            let backends = Arc::clone(&self.backends);
            let active_connections = Arc::clone(&self.active_connections);
            let config = Arc::new(self.config.clone());
            let server_statuses = Arc::clone(&self.server_statuses);
            
            let task = task::spawn(async move {
                Self::run_frontend(frontend_name, frontends, backends, active_connections, config, server_statuses).await;
            });
            frontend_tasks.push(task);
        }

        // Ждем сигнала завершения
        shutdown_signal.await;
        
        // Graceful shutdown
        let active_conns = self.active_connections.read().await.values().sum();
        log_graceful_shutdown(active_conns);
        
        // Отменяем все задачи
        for task in frontend_tasks {
            task.abort();
        }

        info!("Proxy server stopped");
        Ok(())
    }

    async fn initialize_frontends(&mut self) -> Result<()> {
        for frontend_config in &self.config.frontends {
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
        for backend_config in &self.config.backends {
            let algorithm = backend_config.balance.as_deref().unwrap_or("roundrobin");
            let load_balancer = BackendLoadBalancer::new(backend_config.server.clone(), algorithm)?;
            
            let backend_state = BackendState {
                config: backend_config.clone(),
                load_balancer,
            };

            self.backends.insert(backend_config.name.clone(), backend_state);
            info!("Backend '{}' initialized with {} servers using {} algorithm", 
                  backend_config.name, backend_config.server.len(), algorithm);
        }

        Ok(())
    }

    async fn start_health_checkers(&self) {
        let server_statuses = Arc::clone(&self.server_statuses);
        
        for backend_config in &self.config.backends {
            let health_checker = HealthChecker::new(backend_config.clone());
            let backend_name = backend_config.name.clone();
            
            // Запускаем health checker с callback для обновления статусов
            let server_statuses_clone = Arc::clone(&server_statuses);
            health_checker.start_with_callback(backend_name.clone(), server_statuses_clone).await;
            
            self.health_checkers.insert(backend_config.name.clone(), health_checker);
            info!("Health checker started for backend '{}'", backend_config.name);
        }
    }

    async fn run_frontend(
        frontend_name: String,
        frontends: Arc<DashMap<String, FrontendState>>,
        backends: Arc<DashMap<String, BackendState>>,
        active_connections: Arc<RwLock<HashMap<String, u64>>>,
        config: Arc<Config>,
        server_statuses: Arc<RwLock<HashMap<String, HashMap<String, ServerStatus>>>>,
    ) {
        if let Some(frontend_state) = frontends.get(&frontend_name) {
            let listeners = frontend_state.listeners.clone();
            drop(frontend_state); // Освобождаем borrow
            
            for listener in listeners {
                let frontend_name = frontend_name.clone();
                let frontends = Arc::clone(&frontends);
                let backends = Arc::clone(&backends);
                let active_connections = Arc::clone(&active_connections);
                let config = Arc::clone(&config);
                let server_statuses = Arc::clone(&server_statuses);

                task::spawn(async move {
                    if let Err(e) = Self::accept_connections(
                        listener.as_ref(),
                        &frontend_name,
                        frontends,
                        backends,
                        active_connections,
                        config,
                        server_statuses,
                    ).await {
                        error!("Error accepting connections for frontend {}: {}", frontend_name, e);
                    }
                });
            }
        }
    }

    async fn accept_connections(
        listener: &TcpListener,
        frontend_name: &str,
        frontends: Arc<DashMap<String, FrontendState>>,
        backends: Arc<DashMap<String, BackendState>>,
        active_connections: Arc<RwLock<HashMap<String, u64>>>,
        config: Arc<Config>,
        server_statuses: Arc<RwLock<HashMap<String, HashMap<String, ServerStatus>>>>,
    ) -> Result<()> {
        loop {
            let (client_stream, client_addr) = listener.accept().await?;
            
            // Проверяем лимит maxconn
            let max_connections = config.global.maxconn.unwrap_or(4096);
            let current_connections = {
                let conns = active_connections.read().await;
                conns.values().sum::<u64>()
            };
            
            if current_connections >= max_connections as u64 {
                warn!("Max connections limit reached: {} >= {}", current_connections, max_connections);
                metrics::connection_error(frontend_name, "maxconn_limit");
                continue; // Отклоняем новое соединение
            }
            
            // Обновляем счетчик активных соединений
            {
                let mut conns = active_connections.write().await;
                *conns.entry(frontend_name.to_string()).or_insert(0) += 1;
            }

            metrics::connection_accepted(frontend_name);
            
            let frontend_name = frontend_name.to_string();
            let frontends = Arc::clone(&frontends);
            let backends = Arc::clone(&backends);
            let active_connections = Arc::clone(&active_connections);
            let config = Arc::clone(&config);
            let server_statuses = Arc::clone(&server_statuses);

            task::spawn(async move {
                if let Err(e) = Self::handle_connection(
                    client_stream,
                    client_addr,
                    &frontend_name,
                    frontends,
                    backends,
                    config,
                    server_statuses,
                ).await {
                    error!("Error handling connection from {}: {}", client_addr, e);
                    metrics::connection_error(&frontend_name, "handle_error");
                }

                // Уменьшаем счетчик активных соединений
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
        config: Arc<Config>,
        server_statuses: Arc<RwLock<HashMap<String, HashMap<String, ServerStatus>>>>,
    ) -> Result<()> {
        // Получаем конфигурацию frontend'а
        let frontend_config = if let Some(frontend_state) = frontends.get(frontend_name) {
            frontend_state.config.clone()
        } else {
            return Err(anyhow!("Frontend '{}' not found", frontend_name));
        };

        // Определяем backend
        let backend_name = Self::select_backend(&frontend_config, client_addr)?;
        
        // Получаем конфигурацию backend'а
        let mut backend_state = if let Some(backend) = backends.get_mut(&backend_name) {
            backend
        } else {
            return Err(anyhow!("Backend '{}' not found", backend_name));
        };

        // Клонируем конфигурацию до mutable borrow
        let backend_config = backend_state.config.clone();
        
        // Выбираем сервер с обновлением статусов из health checker
        let server = Self::select_server(&mut backend_state, &server_statuses).await?;
        
        // Создаем логгер для запроса и начинаем измерение времени
        let start_time = std::time::Instant::now();
        let logger = RequestLogger::new(
            client_addr.ip().to_string(),
            backend_name.clone(),
            server.name.clone(),
        );

        logger.log_request_start();
        metrics::request_started(&backend_name, &server.name);

        // Получаем timeout'ы из конфигурации
        let connect_timeout = backend_config.options
            .as_ref()
            .map(|opts| opts.get_connect_timeout())
            .unwrap_or_else(|| std::time::Duration::from_secs(5));

        // Устанавливаем соединение с сервером с timeout
        let connection_start = std::time::Instant::now();
        let server_addr = format!("{}:{}", server.address, server.port);
        let server_stream = match tokio::time::timeout(connect_timeout, TcpStream::connect(&server_addr)).await {
            Ok(Ok(stream)) => stream,
            Ok(Err(e)) => {
                let duration = start_time.elapsed();
                let connection_time = connection_start.elapsed();
                logger.log_error(&format!("Failed to connect to server: {}", e));
                metrics::request_failed(&backend_name, &server.name, "connection_failed");
                metrics::request_completed(&backend_name, &server.name, "connection_failed", duration.as_millis() as u64);
                metrics::record_detailed_timing_metrics(&backend_name, &server.name, duration.as_millis() as u64, Some(connection_time.as_millis() as u64), None);
                return Err(e.into());
            }
            Err(_) => {
                let duration = start_time.elapsed();
                let connection_time = connection_start.elapsed();
                logger.log_error(&format!("Connection timeout after {:?}", connect_timeout));
                metrics::request_failed(&backend_name, &server.name, "connection_timeout");
                metrics::request_completed(&backend_name, &server.name, "connection_timeout", duration.as_millis() as u64);
                metrics::record_detailed_timing_metrics(&backend_name, &server.name, duration.as_millis() as u64, Some(connection_time.as_millis() as u64), None);
                return Err(anyhow!("Connection timeout after {:?}", connect_timeout));
            }
        };
        let connection_time = connection_start.elapsed();

        // Проксируем данные с timeout'ами
        let transfer_start = std::time::Instant::now();
        let proxy_result = Self::proxy_data_with_timeouts(
            client_stream, 
            server_stream, 
            &backend_config
        ).await;
        let transfer_time = transfer_start.elapsed();
        let duration = start_time.elapsed();
        
        match proxy_result {
            Ok((bytes_client_to_server, bytes_server_to_client)) => {
                logger.log_request_end("success", bytes_client_to_server + bytes_server_to_client);
                metrics::request_completed(&backend_name, &server.name, "success", duration.as_millis() as u64);
                metrics::record_detailed_timing_metrics(
                    &backend_name, 
                    &server.name, 
                    duration.as_millis() as u64,
                    Some(connection_time.as_millis() as u64),
                    Some(transfer_time.as_millis() as u64)
                );
                metrics::bytes_transferred(frontend_name, "client_to_server", bytes_client_to_server);
                metrics::bytes_transferred(frontend_name, "server_to_client", bytes_server_to_client);
            }
            Err(e) => {
                logger.log_error(&format!("Proxy error: {}", e));
                metrics::request_failed(&backend_name, &server.name, "proxy_error");
                metrics::request_completed(&backend_name, &server.name, "proxy_error", duration.as_millis() as u64);
                metrics::record_detailed_timing_metrics(
                    &backend_name, 
                    &server.name, 
                    duration.as_millis() as u64,
                    Some(connection_time.as_millis() as u64),
                    Some(transfer_time.as_millis() as u64)
                );
                return Err(e);
            }
        }

        Ok(())
    }

    fn select_backend(frontend_config: &FrontendConfig, client_addr: SocketAddr) -> Result<String> {
        // Проверяем ACL'ы
        for acl in &frontend_config.acl {
            if !Self::evaluate_acl(acl, client_addr)? {
                return Err(anyhow!("Access denied by ACL '{}'", acl.name));
            }
        }

        // Проверяем use_backend правила
        for use_backend in &frontend_config.use_backend {
            if let Some(condition) = &use_backend.condition {
                if Self::evaluate_condition(condition, client_addr)? {
                    return Ok(use_backend.backend.clone());
                }
            }
        }

        // Используем default_backend
        if let Some(ref backend_name) = frontend_config.default_backend {
            Ok(backend_name.clone())
        } else {
            Err(anyhow!("No backend selected for frontend '{}'", frontend_config.name))
        }
    }

    async fn select_server<'a>(
        backend_state: &'a mut BackendState,
        server_statuses: &'a Arc<RwLock<HashMap<String, HashMap<String, ServerStatus>>>>,
    ) -> Result<&'a ServerConfig> {
        // Обновляем статусы серверов из health checker перед выбором
        Self::update_server_statuses_from_health_checker(backend_state, server_statuses).await;
        
        if let Some(server_state) = backend_state.load_balancer.select_server()? {
            Ok(&server_state.config)
        } else {
            Err(anyhow!("No available servers in backend"))
        }
    }

    async fn update_server_statuses_from_health_checker(
        backend_state: &mut BackendState,
        server_statuses: &Arc<RwLock<HashMap<String, HashMap<String, ServerStatus>>>>,
    ) {
        let backend_name = &backend_state.config.name;
        debug!("=== Updating server statuses for backend '{}' ===", backend_name);
        
        // Получаем статусы серверов из shared state
        let statuses = server_statuses.read().await;
        debug!("Total backends in shared state: {}", statuses.len());
        
        if let Some(backend_statuses) = statuses.get(backend_name) {
            debug!("Found {} server statuses for backend '{}'", backend_statuses.len(), backend_name);
            
            // Обновляем статусы в балансировщике
            for (server_name, status) in backend_statuses {
                let old_status = backend_state.load_balancer.get_server_status(server_name);
                backend_state.load_balancer.update_server_status(server_name, status.clone());
                if old_status != Some(status.clone()) {
                    info!("Updated server {} status from {:?} to {:?} in backend {}", 
                          server_name, old_status, status, backend_name);
                } else {
                    debug!("Updated server {} status to {:?} in backend {}", server_name, status, backend_name);
                }
            }
            
            debug!("Successfully updated {} server statuses in load balancer", backend_statuses.len());
        } else {
            debug!("No server statuses found for backend '{}'", backend_name);
            debug!("Available backends in shared state: {:?}", statuses.keys().collect::<Vec<_>>());
        }
        
        debug!("=== Completed updating server statuses for backend '{}' ===", backend_name);
    }

    fn evaluate_acl(acl: &crate::config::AclConfig, client_addr: SocketAddr) -> Result<bool> {
        let acl_instance = Acl::from_config(acl)?;
        acl_instance.evaluate(client_addr, None)
    }

    fn evaluate_condition(condition: &str, _client_addr: SocketAddr) -> Result<bool> {
        // В L4 режиме большинство условий не применимы
        // Простая реализация - разрешаем все
        debug!("Condition evaluation in L4 mode: {}", condition);
        Ok(true)
    }

    async fn proxy_data_with_timeouts(
        mut client_stream: TcpStream,
        mut server_stream: TcpStream,
        config: &BackendConfig,
    ) -> Result<(u64, u64)> {
        // Получаем timeout'ы из конфигурации
        let client_timeout = config.options
            .as_ref()
            .map(|opts| opts.get_client_timeout())
            .unwrap_or_else(|| std::time::Duration::from_secs(50));
        
        let server_timeout = config.options
            .as_ref()
            .map(|opts| opts.get_server_timeout())
            .unwrap_or_else(|| std::time::Duration::from_secs(50));

        let (mut client_read, mut client_write) = client_stream.split();
        let (mut server_read, mut server_write) = server_stream.split();
        
        // Проксируем данные с timeout'ами
        let client_to_server = tokio::time::timeout(
            client_timeout,
            tokio::io::copy(&mut client_read, &mut server_write)
        );
        let server_to_client = tokio::time::timeout(
            server_timeout,
            tokio::io::copy(&mut server_read, &mut client_write)
        );
        
        match tokio::try_join!(client_to_server, server_to_client) {
            Ok((Ok(bytes_client_to_server), Ok(bytes_server_to_client))) => {
                Ok((bytes_client_to_server, bytes_server_to_client))
            }
            Ok((Err(e), _)) => {
                Err(anyhow!("Client to server transfer failed: {}", e))
            }
            Ok((_, Err(e))) => {
                Err(anyhow!("Server to client transfer failed: {}", e))
            }
            Err(_) => {
                Err(anyhow!("Transfer timeout"))
            }
        }
    }

    async fn proxy_data(
        mut client_stream: TcpStream,
        mut server_stream: TcpStream,
    ) -> Result<(u64, u64)> {
        let (mut client_read, mut client_write) = client_stream.split();
        let (mut server_read, mut server_write) = server_stream.split();
        
        let client_to_server = tokio::io::copy(&mut client_read, &mut server_write);
        let server_to_client = tokio::io::copy(&mut server_read, &mut client_write);
        
        let (bytes_client_to_server, bytes_server_to_client) = tokio::try_join!(client_to_server, server_to_client)?;
        
        Ok((bytes_client_to_server, bytes_server_to_client))
    }

    async fn setup_shutdown_signal() {
        use tokio::signal;
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = signal(SignalKind::terminate()).expect("Failed to install SIGTERM handler");
        tokio::select! {
            _ = signal::ctrl_c() => {
                log_shutdown_signal("SIGINT");
            }
            _ = term.recv() => {
                log_shutdown_signal("SIGTERM");
            }
        }
    }
} 