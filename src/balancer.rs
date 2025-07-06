use crate::config::ServerConfig;
use crate::health::ServerStatus;
use anyhow::{Result, anyhow};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn};

#[derive(Debug, Clone)]
pub struct ServerState {
    pub config: ServerConfig,
    pub active_connections: u32,
    pub total_connections: u64,
    pub last_used: std::time::Instant,
    pub weight: u32,
    pub status: ServerStatus,
}

impl ServerState {
    pub fn new(config: ServerConfig) -> Self {
        let weight = config.weight.unwrap_or(1);
        Self {
            config,
            active_connections: 0,
            total_connections: 0,
            last_used: std::time::Instant::now(),
            weight,
            status: ServerStatus::Up,
        }
    }

    pub fn increment_connections(&mut self) {
        self.active_connections += 1;
        self.total_connections += 1;
        self.last_used = std::time::Instant::now();
    }

    pub fn decrement_connections(&mut self) {
        if self.active_connections > 0 {
            self.active_connections -= 1;
        }
    }

    pub fn is_available(&self) -> bool {
        matches!(self.status, ServerStatus::Up) && 
        !self.config.disabled.unwrap_or(false) &&
        !self.config.backup.unwrap_or(false) &&
        self.weight > 0 // Сервер с весом 0 недоступен
    }

    pub fn is_backup(&self) -> bool {
        self.config.backup.unwrap_or(false)
    }
}

pub trait LoadBalancer {
    fn select_server<'a>(&mut self, servers: &'a [ServerState]) -> Result<Option<&'a ServerState>>;
    fn name(&self) -> &str;
}

pub struct RoundRobinBalancer {
    current_index: usize,
}

impl RoundRobinBalancer {
    pub fn new() -> Self {
        Self { current_index: 0 }
    }
}

impl LoadBalancer for RoundRobinBalancer {
    fn select_server<'a>(&mut self, servers: &'a [ServerState]) -> Result<Option<&'a ServerState>> {
        if servers.is_empty() {
            return Ok(None);
        }

        let available_servers: Vec<&ServerState> = servers.iter()
            .filter(|s| s.is_available())
            .collect();

        if available_servers.is_empty() {
            // Если нет доступных серверов, попробуем backup серверы
            let backup_servers: Vec<&ServerState> = servers.iter()
                .filter(|s| s.is_backup() && matches!(s.status, ServerStatus::Up))
                .collect();

            if backup_servers.is_empty() {
                return Ok(None);
            }

            self.current_index = (self.current_index + 1) % backup_servers.len();
            Ok(Some(backup_servers[self.current_index]))
        } else {
            self.current_index = (self.current_index + 1) % available_servers.len();
            Ok(Some(available_servers[self.current_index]))
        }
    }

    fn name(&self) -> &str {
        "roundrobin"
    }
}

pub struct LeastConnectionBalancer {
    connection_counts: Arc<RwLock<HashMap<String, u32>>>,
}

impl LeastConnectionBalancer {
    pub fn new() -> Self {
        Self {
            connection_counts: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl LoadBalancer for LeastConnectionBalancer {
    fn select_server<'a>(&mut self, servers: &'a [ServerState]) -> Result<Option<&'a ServerState>> {
        let available_servers: Vec<&ServerState> = servers.iter()
            .filter(|s| s.is_available())
            .collect();

        debug!("LeastConnectionBalancer: {} total servers, {} available", servers.len(), available_servers.len());
        
        for server in servers {
            debug!("Server {}: status={:?}, connections={}, available={}", 
                   server.config.name, server.status, server.active_connections, server.is_available());
        }

        if available_servers.is_empty() {
            warn!("No available servers for least connection balancing");
            return Ok(None);
        }

        // Находим сервер с наименьшим количеством соединений
        let min_connections = available_servers.iter()
            .map(|s| s.active_connections)
            .min()
            .unwrap();

        // Если несколько серверов имеют одинаковое минимальное количество соединений,
        // выбираем случайный из них
        let servers_with_min_connections: Vec<&ServerState> = available_servers.iter()
            .filter(|s| s.active_connections == min_connections)
            .copied()
            .collect();

        let selected_server = if servers_with_min_connections.len() == 1 {
            servers_with_min_connections[0]
        } else {
            // Случайный выбор из серверов с минимальным количеством соединений
            let index = rand::random::<usize>() % servers_with_min_connections.len();
            servers_with_min_connections[index]
        };

        debug!("Selected server: {} with {} active connections", 
               selected_server.config.name, selected_server.active_connections);

        Ok(Some(selected_server))
    }

    fn name(&self) -> &str {
        "leastconn"
    }
}

pub struct WeightedRoundRobinBalancer {
    current_index: usize,
    current_weight: u32,
    max_weight: u32,
    gcd: u32,
}

impl WeightedRoundRobinBalancer {
    pub fn new() -> Self {
        Self {
            current_index: 0,
            current_weight: 0,
            max_weight: 0,
            gcd: 1,
        }
    }

    fn calculate_gcd(a: u32, b: u32) -> u32 {
        if b == 0 {
            a
        } else {
            Self::calculate_gcd(b, a % b)
        }
    }

    fn calculate_gcd_of_weights(servers: &[ServerState]) -> u32 {
        let weights: Vec<u32> = servers.iter()
            .filter(|s| s.is_available() && s.weight > 0) // Исключаем серверы с весом 0
            .map(|s| s.weight)
            .collect();

        if weights.is_empty() {
            return 1;
        }

        let mut gcd = weights[0];
        for &weight in weights.iter().skip(1) {
            gcd = Self::calculate_gcd(gcd, weight);
        }
        gcd
    }
}

impl LoadBalancer for WeightedRoundRobinBalancer {
    fn select_server<'a>(&mut self, servers: &'a [ServerState]) -> Result<Option<&'a ServerState>> {
        let available_servers: Vec<&ServerState> = servers.iter()
            .filter(|s| s.is_available() && s.weight > 0) // Исключаем серверы с весом 0
            .collect();

        // Отладочная информация
        for server in servers {
            tracing::debug!(
                "Server {}: status={:?}, weight={}, available={}, disabled={}, backup={}",
                server.config.name,
                server.status,
                server.weight,
                server.is_available(),
                server.config.disabled.unwrap_or(false),
                server.config.backup.unwrap_or(false)
            );
        }

        tracing::debug!("Available servers with weight > 0: {}", available_servers.len());

        if available_servers.is_empty() {
            tracing::warn!("No available servers with weight > 0");
            return Ok(None);
        }

        // Вычисляем GCD весов
        self.gcd = Self::calculate_gcd_of_weights(servers);
        self.max_weight = available_servers.iter()
            .map(|s| s.weight)
            .max()
            .unwrap_or(1);

        // Инициализируем current_weight если нужно
        if self.current_weight == 0 {
            self.current_weight = self.max_weight;
        }

        loop {
            self.current_index = (self.current_index + 1) % available_servers.len();
            
            if self.current_index == 0 {
                self.current_weight = self.current_weight.saturating_sub(self.gcd);
                if self.current_weight == 0 {
                    self.current_weight = self.max_weight;
                }
            }

            let server = &available_servers[self.current_index];
            if server.weight >= self.current_weight {
                tracing::debug!("Selected server: {} with weight: {}", server.config.name, server.weight);
                return Ok(Some(server));
            }
        }
    }

    fn name(&self) -> &str {
        "weighted_roundrobin"
    }
}

pub struct SourceHashBalancer {
    hash_cache: HashMap<String, usize>,
}

impl SourceHashBalancer {
    pub fn new() -> Self {
        Self {
            hash_cache: HashMap::new(),
        }
    }

    fn hash_source(source: &str) -> u32 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut hasher = DefaultHasher::new();
        source.hash(&mut hasher);
        hasher.finish() as u32
    }
}

impl LoadBalancer for SourceHashBalancer {
    fn select_server<'a>(&mut self, servers: &'a [ServerState]) -> Result<Option<&'a ServerState>> {
        // В L4 режиме мы не можем получить source IP на этом уровне
        // Это будет обработано на уровне frontend
        warn!("Source hash balancing not fully implemented for L4 mode");
        
        let available_servers: Vec<&ServerState> = servers.iter()
            .filter(|s| s.is_available())
            .collect();

        if available_servers.is_empty() {
            return Ok(None);
        }

        // Fallback to round robin
        let index = rand::random::<usize>() % available_servers.len();
        Ok(Some(available_servers[index]))
    }

    fn name(&self) -> &str {
        "source"
    }
}

pub struct RandomBalancer;

impl LoadBalancer for RandomBalancer {
    fn select_server<'a>(&mut self, servers: &'a [ServerState]) -> Result<Option<&'a ServerState>> {
        let available_servers: Vec<&ServerState> = servers.iter()
            .filter(|s| s.is_available())
            .collect();

        if available_servers.is_empty() {
            return Ok(None);
        }

        let index = rand::random::<usize>() % available_servers.len();
        Ok(Some(available_servers[index]))
    }

    fn name(&self) -> &str {
        "random"
    }
}

pub struct LoadBalancerFactory;

impl LoadBalancerFactory {
    pub fn create(algorithm: &str) -> Result<Box<dyn LoadBalancer + Send + Sync>> {
        match algorithm.to_lowercase().as_str() {
            "roundrobin" => Ok(Box::new(WeightedRoundRobinBalancer::new())), // Всегда используем взвешенную версию
            "leastconn" => Ok(Box::new(LeastConnectionBalancer::new())),
            "weighted_roundrobin" => Ok(Box::new(WeightedRoundRobinBalancer::new())),
            "source" => Ok(Box::new(SourceHashBalancer::new())),
            "random" => Ok(Box::new(RandomBalancer)),
            _ => {
                warn!("Unknown load balancing algorithm '{}', using weighted roundrobin", algorithm);
                Ok(Box::new(WeightedRoundRobinBalancer::new()))
            }
        }
    }
}

pub struct BackendLoadBalancer {
    servers: Vec<ServerState>,
    balancer: Box<dyn LoadBalancer + Send + Sync>,
    algorithm: String,
}

impl BackendLoadBalancer {
    pub fn new(servers: Vec<ServerConfig>, algorithm: &str) -> Result<Self> {
        let server_states: Vec<ServerState> = servers.into_iter()
            .map(ServerState::new)
            .collect();

        let balancer = LoadBalancerFactory::create(algorithm)?;

        Ok(Self {
            servers: server_states,
            balancer,
            algorithm: algorithm.to_string(),
        })
    }

    pub fn select_server(&mut self) -> Result<Option<&ServerState>> {
        self.balancer.select_server(&self.servers)
    }

    pub fn update_server_status(&mut self, server_name: &str, status: ServerStatus) {
        for server in &mut self.servers {
            if server.config.name == server_name {
                server.status = status;
                break;
            }
        }
    }

    pub fn increment_server_connections(&mut self, server_name: &str) {
        for server in &mut self.servers {
            if server.config.name == server_name {
                server.increment_connections();
                break;
            }
        }
    }

    pub fn decrement_server_connections(&mut self, server_name: &str) {
        for server in &mut self.servers {
            if server.config.name == server_name {
                server.decrement_connections();
                break;
            }
        }
    }

    pub fn get_available_servers(&self) -> Vec<&ServerState> {
        self.servers.iter()
            .filter(|s| s.is_available())
            .collect()
    }

    pub fn get_all_servers(&self) -> &[ServerState] {
        &self.servers
    }

    pub fn get_server_status(&self, server_name: &str) -> Option<ServerStatus> {
        for server in &self.servers {
            if server.config.name == server_name {
                return Some(server.status.clone());
            }
        }
        None
    }

    pub fn algorithm(&self) -> &str {
        &self.algorithm
    }
} 