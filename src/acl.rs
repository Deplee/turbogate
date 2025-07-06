use crate::config::AclConfig;
use crate::utils;
use anyhow::{Result, anyhow};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use tracing::{debug, warn};

#[derive(Debug, Clone)]
pub enum AclCondition {
    SourceIp(IpNetwork),
    SourcePort(u16),
    DestinationPort(u16),
    Hostname(String),
    Path(String),
    Header(String, String),
    Custom(String),
}

#[derive(Debug, Clone)]
pub struct Acl {
    pub name: String,
    pub conditions: Vec<AclCondition>,
}

impl Acl {
    pub fn from_config(config: &AclConfig) -> Result<Self> {
        let conditions = Self::parse_criterion(&config.criterion)?;
        
        Ok(Self {
            name: config.name.clone(),
            conditions,
        })
    }

    pub fn evaluate(&self, client_addr: SocketAddr, request_data: Option<&RequestData>) -> Result<bool> {
        for condition in &self.conditions {
            if !Self::evaluate_condition(condition, client_addr, request_data)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn parse_criterion(criterion: &str) -> Result<Vec<AclCondition>> {
        let mut conditions = Vec::new();
        let parts: Vec<&str> = criterion.split_whitespace().collect();
        
        if parts.is_empty() {
            return Err(anyhow!("Empty ACL criterion"));
        }

        match parts[0] {
            "src" => {
                if parts.len() < 2 {
                    return Err(anyhow!("Invalid src ACL: missing IP/CIDR"));
                }
                let network = utils::parse_ip_or_cidr(parts[1])?;
                conditions.push(AclCondition::SourceIp(network));
            }
            "src_port" => {
                if parts.len() < 2 {
                    return Err(anyhow!("Invalid src_port ACL: missing port"));
                }
                let port: u16 = parts[1].parse()?;
                conditions.push(AclCondition::SourcePort(port));
            }
            "dst_port" => {
                if parts.len() < 2 {
                    return Err(anyhow!("Invalid dst_port ACL: missing port"));
                }
                let port: u16 = parts[1].parse()?;
                conditions.push(AclCondition::DestinationPort(port));
            }
            "hdr" => {
                if parts.len() < 3 {
                    return Err(anyhow!("Invalid hdr ACL: missing header name or value"));
                }
                let header_name = parts[1].to_string();
                let header_value = parts[2].to_string();
                conditions.push(AclCondition::Header(header_name, header_value));
            }
            "path" => {
                if parts.len() < 2 {
                    return Err(anyhow!("Invalid path ACL: missing path"));
                }
                conditions.push(AclCondition::Path(parts[1].to_string()));
            }
            "host" => {
                if parts.len() < 2 {
                    return Err(anyhow!("Invalid host ACL: missing hostname"));
                }
                conditions.push(AclCondition::Hostname(parts[1].to_string()));
            }
            _ => {
                warn!("Unknown ACL criterion: {}", parts[0]);
                conditions.push(AclCondition::Custom(criterion.to_string()));
            }
        }

        Ok(conditions)
    }

    fn evaluate_condition(
        condition: &AclCondition,
        client_addr: SocketAddr,
        request_data: Option<&RequestData>,
    ) -> Result<bool> {
        match condition {
            AclCondition::SourceIp(network) => {
                Ok(utils::ip_in_network(client_addr.ip(), network))
            }
            AclCondition::SourcePort(port) => {
                Ok(client_addr.port() == *port)
            }
            AclCondition::DestinationPort(_port) => {
                // В L4 режиме destination port - это порт frontend'а
                // Это будет обработано на уровне frontend
                Ok(true)
            }
            AclCondition::Hostname(hostname) => {
                if let Some(data) = request_data {
                    if let Some(host) = &data.host {
                        Ok(host == hostname)
                    } else {
                        Ok(false)
                    }
                } else {
                    Ok(false)
                }
            }
            AclCondition::Path(path) => {
                if let Some(data) = request_data {
                    if let Some(req_path) = &data.path {
                        Ok(req_path.starts_with(path))
                    } else {
                        Ok(false)
                    }
                } else {
                    Ok(false)
                }
            }
            AclCondition::Header(name, value) => {
                if let Some(data) = request_data {
                    if let Some(header_value) = data.headers.get(name) {
                        Ok(header_value == value)
                    } else {
                        Ok(false)
                    }
                } else {
                    Ok(false)
                }
            }
            AclCondition::Custom(_) => {
                // Для L4 режима большинство HTTP-специфичных ACL не применимы
                debug!("Custom ACL condition in L4 mode, allowing");
                Ok(true)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct RequestData {
    pub host: Option<String>,
    pub path: Option<String>,
    pub headers: HashMap<String, String>,
    pub method: Option<String>,
}

pub struct AclManager {
    acls: HashMap<String, Acl>,
}

impl AclManager {
    pub fn new() -> Self {
        Self {
            acls: HashMap::new(),
        }
    }

    pub fn add_acl(&mut self, acl: Acl) {
        self.acls.insert(acl.name.clone(), acl);
    }

    pub fn evaluate_acl(&self, acl_name: &str, client_addr: SocketAddr, request_data: Option<&RequestData>) -> Result<bool> {
        if let Some(acl) = self.acls.get(acl_name) {
            acl.evaluate(client_addr, request_data)
        } else {
            Err(anyhow!("ACL '{}' not found", acl_name))
        }
    }

    pub fn evaluate_condition(&self, condition: &str, client_addr: SocketAddr, request_data: Option<&RequestData>) -> Result<bool> {
        // Парсим условие вида "acl_name" или "!acl_name"
        let condition = condition.trim();
        let negated = condition.starts_with('!');
        let acl_name = if negated { &condition[1..] } else { condition };
        
        let result = self.evaluate_acl(acl_name, client_addr, request_data)?;
        Ok(if negated { !result } else { result })
    }
}

// Утилиты для работы с IP сетями
use ipnetwork::IpNetwork;

pub fn parse_ip_network(input: &str) -> Result<IpNetwork> {
    utils::parse_ip_or_cidr(input)
}

pub fn is_ip_in_network(ip: IpAddr, network: &IpNetwork) -> bool {
    utils::ip_in_network(ip, network)
}

// Утилиты для работы с портами
pub fn parse_port_range(input: &str) -> Result<Vec<u16>> {
    utils::parse_port_range(input)
}

pub fn is_port_in_range(port: u16, range: &[u16]) -> bool {
    range.contains(&port)
}

// Утилиты для работы с HTTP заголовками
pub fn parse_http_headers(headers_str: &str) -> Result<HashMap<String, String>> {
    Ok(utils::parse_http_header_value(headers_str))
}

// Утилиты для валидации
pub fn validate_acl_config(acls: &[AclConfig]) -> Result<()> {
    for acl in acls {
        let _ = Acl::from_config(acl)?;
    }
    Ok(())
} 