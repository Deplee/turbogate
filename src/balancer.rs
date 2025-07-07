use crate::config::ServerConfig;
use crate::health::ServerStatus;
use anyhow::Result;
use tracing::{debug, warn};

#[derive(Debug, Clone)]
pub struct ServerState {
    pub config: ServerConfig,
    pub active_connections: u32,
    pub weight: u32,
    pub status: ServerStatus,
}

impl ServerState {
    pub fn new(config: ServerConfig) -> Self {
        let weight = config.weight.unwrap_or(1);
        Self {
            config,
            active_connections: 0,
            weight,
            status: ServerStatus::Up,
        }
    }

    pub fn is_available(&self) -> bool {
        matches!(self.status, ServerStatus::Up) && 
        !self.config.disabled.unwrap_or(false) &&
        !self.config.backup.unwrap_or(false) &&
        self.weight > 0
    }

    pub fn is_backup(&self) -> bool {
        self.config.backup.unwrap_or(false)
    }
}

pub trait LoadBalancer {
    fn select_server<'a>(&mut self, servers: &'a [ServerState]) -> Result<Option<&'a ServerState>>;
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
}

pub struct LeastConnectionBalancer;

impl LoadBalancer for LeastConnectionBalancer {
    fn select_server<'a>(&mut self, servers: &'a [ServerState]) -> Result<Option<&'a ServerState>> {
        let available_servers: Vec<&ServerState> = servers.iter()
            .filter(|s| s.is_available())
            .collect();

        if available_servers.is_empty() {
            warn!("No available servers for least connection balancing");
            return Ok(None);
        }

        let min_connections = available_servers.iter()
            .map(|s| s.active_connections)
            .min()
            .unwrap();

        let servers_with_min_connections: Vec<&ServerState> = available_servers.iter()
            .filter(|s| s.active_connections == min_connections)
            .copied()
            .collect();

        let selected_server = if servers_with_min_connections.len() == 1 {
            servers_with_min_connections[0]
        } else {
            let index = rand::random::<usize>() % servers_with_min_connections.len();
            servers_with_min_connections[index]
        };

        debug!("Selected server: {} with {} active connections", 
               selected_server.config.name, selected_server.active_connections);

        Ok(Some(selected_server))
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
}

pub struct LoadBalancerFactory;

impl LoadBalancerFactory {
    pub fn create(algorithm: &str) -> Result<Box<dyn LoadBalancer + Send + Sync>> {
        match algorithm {
            "roundrobin" => Ok(Box::new(RoundRobinBalancer::new())),
            "leastconn" => Ok(Box::new(LeastConnectionBalancer)),
            "random" => Ok(Box::new(RandomBalancer)),
            _ => {
                warn!("Unknown load balancing algorithm: {}, using roundrobin", algorithm);
                Ok(Box::new(RoundRobinBalancer::new()))
            }
        }
    }
}

pub struct BackendLoadBalancer {
    servers: Vec<ServerState>,
    balancer: Box<dyn LoadBalancer + Send + Sync>,
}

impl BackendLoadBalancer {
    pub fn new(servers: Vec<ServerConfig>, algorithm: &str) -> Result<Self> {
        let server_states: Vec<ServerState> = servers.into_iter().map(ServerState::new).collect();
        let balancer = LoadBalancerFactory::create(algorithm)?;

        Ok(Self {
            servers: server_states,
            balancer,
        })
    }

    pub fn select_server(&mut self) -> Result<Option<&ServerState>> {
        self.balancer.select_server(&self.servers)
    }
}
