use anyhow::{Result, anyhow};
use std::collections::HashMap;
use tracing::{debug, warn};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Options {
    pub http_options: HttpOptions,
    pub tcp_options: TcpOptions,
    pub general_options: GeneralOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpOptions {
    pub httpchk: Option<HttpCheck>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_keep_alive_timeout: Option<u64>, // Duration в миллисекундах
    pub dontlognull: bool,
    pub logasap: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TcpOptions {
    pub clitcpka: bool,
    pub tcp_check: bool,
    pub tcp_check_connect: bool,
    pub retries: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_connect: Option<u64>, // Duration в миллисекундах
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_client: Option<u64>, // Duration в миллисекундах
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_server: Option<u64>, // Duration в миллисекундах
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_queue: Option<u64>, // Duration в миллисекундах
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpCheck {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            http_options: HttpOptions::default(),
            tcp_options: TcpOptions::default(),
            general_options: GeneralOptions::default(),
        }
    }
}

impl Default for HttpOptions {
    fn default() -> Self {
        Self {
            httpchk: None,
            http_keep_alive_timeout: None,
            dontlognull: true,
            logasap: false,
        }
    }
}

impl Default for TcpOptions {
    fn default() -> Self {
        Self {
            clitcpka: false,
            tcp_check: false,
            tcp_check_connect: false,
            retries: Some(3),
        }
    }
}

impl Default for GeneralOptions {
    fn default() -> Self {
        Self {
            timeout_connect: Some(5000), // 5 секунд в миллисекундах
            timeout_client: Some(50000), // 50 секунд в миллисекундах
            timeout_server: Some(50000), // 50 секунд в миллисекундах
            timeout_queue: Some(10000), // 10 секунд в миллисекундах
        }
    }
}

impl Options {
    pub fn from_strings(options: &[String], mode: &str) -> Result<Self> {
        let mut opts = Self::default();
        
        for option in options {
            Self::parse_option(option, &mut opts, mode)?;
        }
        
        Ok(opts)
    }
    
    fn parse_option(option: &str, opts: &mut Options, mode: &str) -> Result<()> {
        let parts: Vec<&str> = option.split_whitespace().collect();
        if parts.is_empty() {
            return Ok(());
        }
        
        match parts[0] {
            "httpchk" => {
                if mode == "http" {
                    opts.http_options.httpchk = Some(Self::parse_httpchk(option)?);
                } else {
                    warn!("httpchk option ignored in {} mode", mode);
                }
            }
            "dontlognull" => {
                opts.http_options.dontlognull = true;
            }
            "logasap" => {
                opts.http_options.logasap = true;
            }
            "clitcpka" => {
                opts.tcp_options.clitcpka = true;
            }
            "tcp-check" => {
                opts.tcp_options.tcp_check = true;
                // Проверяем, есть ли дополнительные параметры
                if parts.len() > 1 && parts[1] == "connect" {
                    opts.tcp_options.tcp_check_connect = true;
                }
            }
            _ => {
                debug!("Unknown option: {}", option);
            }
        }
        
        Ok(())
    }
    
    fn parse_httpchk(option: &str) -> Result<HttpCheck> {
        // Парсим "httpchk GET /ping" или "httpchk"
        let parts: Vec<&str> = option.split_whitespace().collect();
        
        if parts.len() >= 3 {
            Ok(HttpCheck {
                method: parts[1].to_string(),
                path: parts[2].to_string(),
                headers: HashMap::new(),
            })
        } else {
            Ok(HttpCheck {
                method: "GET".to_string(),
                path: "/".to_string(),
                headers: HashMap::new(),
            })
        }
    }
    
    pub fn parse_timeout(timeout_str: &str) -> Result<std::time::Duration> {
        if timeout_str.ends_with("ms") {
            let ms: u64 = timeout_str[..timeout_str.len()-2].parse()?;
            Ok(std::time::Duration::from_millis(ms))
        } else if timeout_str.ends_with('s') {
            let secs: u64 = timeout_str[..timeout_str.len()-1].parse()?;
            Ok(std::time::Duration::from_secs(secs))
        } else if timeout_str.ends_with('m') {
            let mins: u64 = timeout_str[..timeout_str.len()-1].parse()?;
            Ok(std::time::Duration::from_secs(mins * 60))
        } else if timeout_str.ends_with('h') {
            let hours: u64 = timeout_str[..timeout_str.len()-1].parse()?;
            Ok(std::time::Duration::from_secs(hours * 3600))
        } else {
            // Попробуем как секунды
            let secs: u64 = timeout_str.parse()?;
            Ok(std::time::Duration::from_secs(secs))
        }
    }
    
    pub fn apply_timeout(&mut self, timeout_type: &str, value: &str) -> Result<()> {
        let duration = Self::parse_timeout(value)?;
        let duration_ms = duration.as_millis() as u64;
        
        match timeout_type {
            "connect" => self.general_options.timeout_connect = Some(duration_ms),
            "client" => self.general_options.timeout_client = Some(duration_ms),
            "server" => self.general_options.timeout_server = Some(duration_ms),
            "queue" => self.general_options.timeout_queue = Some(duration_ms),
            "http-keep-alive" => self.http_options.http_keep_alive_timeout = Some(duration_ms),
            _ => warn!("Unknown timeout type: {}", timeout_type),
        }
        
        Ok(())
    }
    
    // Методы для получения timeout'ов
    pub fn get_connect_timeout(&self) -> std::time::Duration {
        self.general_options.timeout_connect
            .map(|ms| std::time::Duration::from_millis(ms))
            .unwrap_or(std::time::Duration::from_secs(5))
    }
    
    pub fn get_client_timeout(&self) -> std::time::Duration {
        self.general_options.timeout_client
            .map(|ms| std::time::Duration::from_millis(ms))
            .unwrap_or(std::time::Duration::from_secs(50))
    }
    
    pub fn get_server_timeout(&self) -> std::time::Duration {
        self.general_options.timeout_server
            .map(|ms| std::time::Duration::from_millis(ms))
            .unwrap_or(std::time::Duration::from_secs(50))
    }
    
    pub fn get_queue_timeout(&self) -> std::time::Duration {
        self.general_options.timeout_queue
            .map(|ms| std::time::Duration::from_millis(ms))
            .unwrap_or(std::time::Duration::from_secs(10))
    }
}
