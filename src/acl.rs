use crate::config::AclConfig;
use crate::utils;
use anyhow::{Result, anyhow};
use std::net::SocketAddr;
use tracing::{debug, warn};
use ipnetwork::IpNetwork;

#[derive(Debug, Clone)]
pub enum AclCondition {
    SourceIp(IpNetwork),
    SourcePort(u16),
    DestinationPort(u16),
    Hostname(String),
    Path(String),
    Header(String, String),
    Custom(()),
}

#[derive(Debug, Clone)]
pub struct Acl {
    pub conditions: Vec<AclCondition>,
}

impl Acl {
    pub fn from_config(config: &AclConfig) -> Result<Self> {
        let conditions = Self::parse_criterion(&config.criterion)?;
        
        Ok(Self {
            conditions,
        })
    }

    pub fn evaluate(&self, client_addr: SocketAddr) -> Result<bool> {
        for condition in &self.conditions {
            if !Self::evaluate_condition(condition, client_addr)? {
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
                conditions.push(AclCondition::Custom(()));
            }
        }

        Ok(conditions)
    }

    fn evaluate_condition(
        condition: &AclCondition,
        client_addr: SocketAddr,
    ) -> Result<bool> {
        match condition {
            AclCondition::SourceIp(network) => {
                Ok(utils::ip_in_network(client_addr.ip(), network))
            }
            AclCondition::SourcePort(port) => {
                Ok(client_addr.port() == *port)
            }
            AclCondition::DestinationPort(_port) => {
                Ok(true)
            }
            AclCondition::Hostname(_hostname) => {
                debug!("Hostname ACL condition in L4 mode, allowing");
                Ok(true)
            }
            AclCondition::Path(_path) => {
                debug!("Path ACL condition in L4 mode, allowing");
                Ok(true)
            }
            AclCondition::Header(_name, _value) => {
                debug!("Header ACL condition in L4 mode, allowing");
                Ok(true)
            }
            AclCondition::Custom(_) => {
                debug!("Custom ACL condition in L4 mode, allowing");
                Ok(true)
            }
        }
    }
}
