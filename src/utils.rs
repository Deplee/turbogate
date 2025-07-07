use std::net::IpAddr;
use std::str::FromStr;
use anyhow::{Result, anyhow};
use ipnetwork::IpNetwork;

pub fn parse_ip_or_cidr(input: &str) -> Result<IpNetwork> {
    if input.contains('/') {
        IpNetwork::from_str(input).map_err(|e| anyhow!("Invalid CIDR: {}", e))
    } else {
        let ip = IpAddr::from_str(input).map_err(|e| anyhow!("Invalid IP: {}", e))?;
        match ip {
            IpAddr::V4(ipv4) => Ok(IpNetwork::V4(ipv4.into())),
            IpAddr::V6(ipv6) => Ok(IpNetwork::V6(ipv6.into())),
        }
    }
}

pub fn ip_in_network(ip: IpAddr, network: &IpNetwork) -> bool {
    network.contains(ip)
}
