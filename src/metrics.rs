use crate::config::MetricsConfig;
use metrics::{counter, gauge, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio::task;
use tracing::{info, error};
use std::sync::Arc;

pub struct Metrics {
    handle: PrometheusHandle,
}

impl Metrics {
    pub fn new() -> anyhow::Result<Self> {
        let builder = PrometheusBuilder::new();
        let handle = builder.install_recorder()?;
        
        Ok(Self { handle })
    }

    pub fn render(&self) -> String {
        self.handle.render()
    }
}

pub fn connection_closed(frontend: &str) {
    gauge!("turbogate_active_connections", -1.0, "frontend" => frontend.to_string());
}

pub fn connection_error(frontend: &str, error_type: &str) {
    counter!("turbogate_connection_errors_total", 1, 
            "frontend" => frontend.to_string(), 
            "error_type" => error_type.to_string());
}

pub fn request_started(backend: &str, server: &str) {
    counter!("turbogate_requests_total", 1, 
            "backend" => backend.to_string(), 
            "server" => server.to_string());
    gauge!("turbogate_active_requests", 1.0, 
           "backend" => backend.to_string(), 
           "server" => server.to_string());
}

pub fn request_completed(backend: &str, server: &str, status: &str, duration_ms: u64) {
    counter!("turbogate_requests_total", 1, 
            "backend" => backend.to_string(), 
            "server" => server.to_string(), 
            "status" => status.to_string());
    histogram!("turbogate_request_duration_ms", duration_ms as f64, 
              "backend" => backend.to_string(), 
              "server" => server.to_string());
    histogram!("turbogate_request_duration_us", (duration_ms * 1000) as f64, 
              "backend" => backend.to_string(), 
              "server" => server.to_string());
    gauge!("turbogate_request_avg_duration_ms", duration_ms as f64, 
           "backend" => backend.to_string(), 
           "server" => server.to_string());
    gauge!("turbogate_active_requests", -1.0, 
           "backend" => backend.to_string(), 
           "server" => server.to_string());
}

pub fn request_failed(backend: &str, server: &str, error_type: &str) {
    counter!("turbogate_request_errors_total", 1, 
            "backend" => backend.to_string(), 
            "server" => server.to_string(), 
            "error_type" => error_type.to_string());
    gauge!("turbogate_active_requests", -1.0, 
           "backend" => backend.to_string(), 
           "server" => server.to_string());
}

pub fn backend_active_servers(backend: &str, count: usize) {
    gauge!("turbogate_backend_active_servers", count as f64, 
           "backend" => backend.to_string());
}

pub fn backend_total_servers(backend: &str, count: usize) {
    gauge!("turbogate_backend_total_servers", count as f64, 
           "backend" => backend.to_string());
}

pub fn server_status_changed(server: &str, status: &str) {
    gauge!("turbogate_server_status", 
           if status == "up" { 1.0 } else { 0.0 }, 
           "server" => server.to_string());
}

pub fn health_check(server: &str, success: bool) {
    counter!("turbogate_health_checks_total", 1, 
            "server" => server.to_string(), 
            "success" => success.to_string());
}

pub async fn init(config: &MetricsConfig) -> anyhow::Result<()> {
    if !config.enabled {
        info!("Metrics disabled");
        return Ok(());
    }

    let metrics = Arc::new(Metrics::new()?);
    
    if let Some(bind_addr) = &config.bind {
        let addr: SocketAddr = bind_addr.parse()?;
        let listener = TcpListener::bind(addr).await?;
        let path = config.path.as_deref().unwrap_or("/metrics").to_string();
        
        info!("Starting metrics server on {} with path {}", bind_addr, path);
        let metrics_clone = Arc::clone(&metrics);
        let path_clone = path.clone();
        
        task::spawn(async move {
            if let Err(e) = run_metrics_server(listener, path_clone, metrics_clone).await {
                error!("Metrics server error: {}", e);
            }
        });
    }

    Ok(())
}

async fn run_metrics_server(
    listener: TcpListener,
    path: String,
    metrics: Arc<Metrics>,
) -> anyhow::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    loop {
        let (mut socket, _addr) = listener.accept().await?;
        let metrics = Arc::clone(&metrics);
        let path = path.clone();
        task::spawn(async move {
            let mut buffer = [0; 1024];
            let mut request = String::new();
            loop {
                let n = match socket.read(&mut buffer).await {
                    Ok(n) if n == 0 => return,
                    Ok(n) => n,
                    Err(_) => return,
                };
                request.push_str(&String::from_utf8_lossy(&buffer[..n]));
                if request.contains("\r\n\r\n") {
                    break;
                }
            }
            let response = if request.starts_with(&format!("GET {} HTTP/1.1", path)) {
                let metrics_data = metrics.render();
                format!(
                    "HTTP/1.1 200 OK\r\n\
                     Content-Type: text/plain; version=0.0.4\r\n\
                     Content-Length: {}\r\n\
                     \r\n\
                     {}",
                    metrics_data.len(),
                    metrics_data
                )
            } else {
                "HTTP/1.1 404 Not Found\r\n\
                 Content-Length: 0\r\n\
                 \r\n".to_string()
            };
            let _ = socket.write_all(response.as_bytes()).await;
        });
    }
}


