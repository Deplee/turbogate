use tracing::Level;
use tracing_subscriber::{
    fmt::time::ChronoUtc,
    prelude::*,
    EnvFilter,
};
use serde_json::json;
use std::time::Instant;

pub struct RequestLogger {
    start_time: Instant,
    request_id: String,
    client_ip: String,
    backend_name: String,
    server_name: String,
}

impl RequestLogger {
    pub fn new(client_ip: String, backend_name: String, server_name: String) -> Self {
        Self {
            start_time: Instant::now(),
            request_id: uuid::Uuid::new_v4().to_string(),
            client_ip,
            backend_name,
            server_name,
        }
    }

    pub fn log_request_start(&self) {
        tracing::info!(
            request_id = %self.request_id,
            client_ip = %self.client_ip,
            backend = %self.backend_name,
            server = %self.server_name,
            event = "request_start",
            "Request started"
        );
    }

    pub fn log_request_end(&self, status: &str, bytes_transferred: u64) {
        let duration = self.start_time.elapsed();
        tracing::info!(
            request_id = %self.request_id,
            client_ip = %self.client_ip,
            backend = %self.backend_name,
            server = %self.server_name,
            status = %status,
            duration_ms = duration.as_millis(),
            duration_us = duration.as_micros(),
            bytes_transferred = bytes_transferred,
            event = "request_end",
            "Request completed"
        );
    }

}

pub fn init(level: Level, json_logs: bool) -> anyhow::Result<()> {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("turbogate={}", level)));

    if json_logs {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_timer(ChronoUtc::rfc_3339())
                    .with_target(true)
                    .with_thread_ids(true)
                    .with_thread_names(true)
                    .with_file(true)
                    .with_line_number(true)
                    .json()
                    .with_current_span(true)
                    .with_span_list(true),
            )
            .init();
    } else {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_timer(ChronoUtc::rfc_3339())
                    .with_target(true)
                    .with_thread_ids(true)
                    .with_thread_names(true)
                    .with_file(true)
                    .with_line_number(true)
                    .with_ansi(true),
            )
            .init();
    }

    Ok(())
}

pub fn log_server_status(server_name: &str, status: &str, details: Option<&str>) {
    let mut log_data = json!({
        "server": server_name,
        "status": status,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "event": "server_status_change"
    });

    if let Some(details) = details {
        log_data["details"] = json!(details);
    }

    tracing::info!(
        server = %server_name,
        status = %status,
        details = ?details,
        event = "server_status_change",
        "Server status changed"
    );
}

pub fn log_backend_status(backend_name: &str, active_servers: usize, total_servers: usize) {
    tracing::info!(
        backend = %backend_name,
        active_servers = active_servers,
        total_servers = total_servers,
        health_percentage = (active_servers as f64 / total_servers as f64) * 100.0,
        event = "backend_status",
        "Backend status update"
    );
}

pub fn log_startup_info(version: &str, config_file: &str, bind_addresses: Vec<String>) {
    tracing::info!(
        version = %version,
        config_file = %config_file,
        bind_addresses = ?bind_addresses,
        event = "startup",
        "Turbogate started successfully"
    );
}

pub fn log_graceful_shutdown(active_connections: u64) {
    tracing::info!(
        active_connections = active_connections,
        event = "graceful_shutdown",
        "Starting graceful shutdown"
    );
}
