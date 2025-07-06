use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;
use std::collections::HashMap;
use anyhow::{Result, anyhow};
use regex::Regex;
use ipnetwork::IpNetwork;

pub fn parse_ip_or_cidr(input: &str) -> Result<IpNetwork> {
    if input.contains('/') {
        // CIDR notation
        IpNetwork::from_str(input).map_err(|e| anyhow!("Invalid CIDR: {}", e))
    } else {
        // Single IP
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

pub fn parse_port_range(input: &str) -> Result<Vec<u16>> {
    if input.contains('-') {
        let parts: Vec<&str> = input.split('-').collect();
        if parts.len() != 2 {
            return Err(anyhow!("Invalid port range format: {}", input));
        }
        
        let start: u16 = parts[0].parse().map_err(|_| anyhow!("Invalid start port: {}", parts[0]))?;
        let end: u16 = parts[1].parse().map_err(|_| anyhow!("Invalid end port: {}", parts[1]))?;
        
        if start > end {
            return Err(anyhow!("Start port {} is greater than end port {}", start, end));
        }
        
        Ok((start..=end).collect())
    } else {
        let port: u16 = input.parse().map_err(|_| anyhow!("Invalid port: {}", input))?;
        Ok(vec![port])
    }
}

pub fn parse_timeout(input: &str) -> Result<std::time::Duration> {
    if input.ends_with("ms") {
        let ms: u64 = input[..input.len()-2].parse()?;
        Ok(std::time::Duration::from_millis(ms))
    } else if input.ends_with('s') {
        let secs: u64 = input[..input.len()-1].parse()?;
        Ok(std::time::Duration::from_secs(secs))
    } else if input.ends_with('m') {
        let mins: u64 = input[..input.len()-1].parse()?;
        Ok(std::time::Duration::from_secs(mins * 60))
    } else if input.ends_with('h') {
        let hours: u64 = input[..input.len()-1].parse()?;
        Ok(std::time::Duration::from_secs(hours * 3600))
    } else {
        // Попробуем как секунды
        let secs: u64 = input.parse()?;
        Ok(std::time::Duration::from_secs(secs))
    }
}

pub fn parse_size(input: &str) -> Result<u64> {
    let input = input.to_lowercase();
    if input.ends_with("kb") {
        let kb: u64 = input[..input.len()-2].parse()?;
        Ok(kb * 1024)
    } else if input.ends_with("mb") {
        let mb: u64 = input[..input.len()-2].parse()?;
        Ok(mb * 1024 * 1024)
    } else if input.ends_with("gb") {
        let gb: u64 = input[..input.len()-2].parse()?;
        Ok(gb * 1024 * 1024 * 1024)
    } else {
        input.parse().map_err(|_| anyhow!("Invalid size format: {}", input))
    }
}

pub fn validate_hostname(hostname: &str) -> Result<()> {
    if hostname.is_empty() || hostname.len() > 253 {
        return Err(anyhow!("Hostname length must be between 1 and 253 characters"));
    }
    
    if hostname.starts_with('.') || hostname.ends_with('.') {
        return Err(anyhow!("Hostname cannot start or end with a dot"));
    }
    
    let parts: Vec<&str> = hostname.split('.').collect();
    for part in parts {
        if part.is_empty() || part.len() > 63 {
            return Err(anyhow!("Hostname part length must be between 1 and 63 characters"));
        }
        
        if !part.chars().all(|c| c.is_alphanumeric() || c == '-') {
            return Err(anyhow!("Hostname parts can only contain alphanumeric characters and hyphens"));
        }
        
        if part.starts_with('-') || part.ends_with('-') {
            return Err(anyhow!("Hostname parts cannot start or end with a hyphen"));
        }
    }
    
    Ok(())
}

pub fn extract_domain_from_hostname(hostname: &str) -> Option<String> {
    let parts: Vec<&str> = hostname.split('.').collect();
    if parts.len() >= 2 {
        Some(parts[parts.len()-2..].join("."))
    } else {
        None
    }
}

pub fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => {
            // RFC 1918 private networks
            ipv4.octets()[0] == 10 ||
            (ipv4.octets()[0] == 172 && ipv4.octets()[1] >= 16 && ipv4.octets()[1] <= 31) ||
            (ipv4.octets()[0] == 192 && ipv4.octets()[1] == 168) ||
            // Loopback
            ipv4.octets()[0] == 127 ||
            // Link-local
            (ipv4.octets()[0] == 169 && ipv4.octets()[1] == 254)
        }
        IpAddr::V6(ipv6) => {
            // RFC 4193 unique local addresses
            ipv6.octets()[0] == 0xfd ||
            // RFC 3879 site-local addresses (deprecated but still used)
            ipv6.octets()[0] == 0xfe && (ipv6.octets()[1] & 0xc0) == 0xc0 ||
            // Loopback
            ipv6.octets() == [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1] ||
            // Link-local
            ipv6.octets()[0] == 0xfe && (ipv6.octets()[1] & 0xc0) == 0x80
        }
    }
}

pub fn is_public_ip(ip: IpAddr) -> bool {
    !is_private_ip(ip) && !is_reserved_ip(ip)
}

pub fn is_reserved_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => {
            // Various reserved ranges
            ipv4.octets()[0] == 0 || // Current network
            ipv4.octets()[0] == 127 || // Loopback
            ipv4.octets()[0] == 224 || // Multicast
            ipv4.octets()[0] == 240 || // Reserved for future use
            (ipv4.octets()[0] == 192 && ipv4.octets()[1] == 0 && ipv4.octets()[2] == 2) || // TEST-NET-1
            (ipv4.octets()[0] == 198 && ipv4.octets()[1] == 51 && ipv4.octets()[2] == 100) || // TEST-NET-2
            (ipv4.octets()[0] == 203 && ipv4.octets()[1] == 0 && ipv4.octets()[2] == 113) // TEST-NET-3
        }
        IpAddr::V6(ipv6) => {
            // Various IPv6 reserved ranges
            ipv6.octets()[0] == 0 || // Unspecified
            ipv6.octets()[0] == 0xff || // Multicast
            ipv6.octets()[0] == 0xfe && (ipv6.octets()[1] & 0xc0) == 0x80 || // Link-local
            ipv6.octets()[0] == 0xfe && (ipv6.octets()[1] & 0xc0) == 0xc0 || // Site-local
            ipv6.octets()[0] == 0xfd || // Unique local
            ipv6.octets() == [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1] // Loopback
        }
    }
}

pub fn calculate_hash(input: &str) -> u32 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    
    let mut hasher = DefaultHasher::new();
    input.hash(&mut hasher);
    hasher.finish() as u32
}

pub fn calculate_consistent_hash<'a>(input: &str, nodes: &'a [String]) -> Option<&'a String> {
    if nodes.is_empty() {
        return None;
    }
    
    let hash = calculate_hash(input);
    let index = hash as usize % nodes.len();
    nodes.get(index)
}

pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit_index = 0;
    
    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }
    
    if unit_index == 0 {
        format!("{} {}", size as u64, UNITS[unit_index])
    } else {
        format!("{:.2} {}", size, UNITS[unit_index])
    }
}

pub fn format_duration(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();
    let millis = duration.subsec_millis();
    
    if secs == 0 {
        format!("{}ms", millis)
    } else if millis == 0 {
        format!("{}s", secs)
    } else {
        format!("{}s {}ms", secs, millis)
    }
}

pub fn parse_http_header_value(value: &str) -> HashMap<String, String> {
    let mut headers = HashMap::new();
    
    for part in value.split(',') {
        let part = part.trim();
        if let Some((key, value)) = part.split_once('=') {
            headers.insert(key.trim().to_string(), value.trim().trim_matches('"').to_string());
        }
    }
    
    headers
}

pub fn validate_ssl_certificate(cert_path: &str, key_path: &str) -> Result<()> {
    use std::fs;
    use std::path::Path;
    
    if !Path::new(cert_path).exists() {
        return Err(anyhow!("SSL certificate file not found: {}", cert_path));
    }
    
    if !Path::new(key_path).exists() {
        return Err(anyhow!("SSL private key file not found: {}", key_path));
    }
    
    // TODO: Add actual certificate validation
    Ok(())
}

pub fn get_system_info() -> HashMap<String, String> {
    let mut info = HashMap::new();
    
    // OS info
    if let Ok(os_type) = std::env::var("OSTYPE") {
        info.insert("os_type".to_string(), os_type);
    }
    
    // CPU info
    if let Ok(cpu_count) = std::thread::available_parallelism() {
        info.insert("cpu_count".to_string(), cpu_count.get().to_string());
    }
    
    // Memory info (Linux only)
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
            for line in content.lines() {
                if line.starts_with("MemTotal:") {
                    if let Some(kb_str) = line.split_whitespace().nth(1) {
                        if let Ok(kb) = kb_str.parse::<u64>() {
                            info.insert("total_memory_kb".to_string(), kb.to_string());
                            break;
                        }
                    }
                }
            }
        }
    }
    
    info
}

pub fn calculate_connection_rate(
    total_connections: u64,
    time_window: std::time::Duration,
) -> f64 {
    let window_secs = time_window.as_secs_f64();
    if window_secs > 0.0 {
        total_connections as f64 / window_secs
    } else {
        0.0
    }
}

pub fn is_rate_limit_exceeded(
    client_ip: &str,
    rate_limit: u32,
    time_window: std::time::Duration,
    connection_history: &[(String, std::time::Instant)],
) -> bool {
    let now = std::time::Instant::now();
    let cutoff = now - time_window;
    
    let recent_connections = connection_history
        .iter()
        .filter(|(ip, time)| ip == client_ip && *time > cutoff)
        .count();
    
    recent_connections >= rate_limit as usize
} 