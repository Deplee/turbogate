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

    pub fn log_error(&self, error: &str) {
        let duration = self.start_time.elapsed();
        tracing::error!(
            request_id = %self.request_id,
            client_ip = %self.client_ip,
            backend = %self.backend_name,
            server = %self.server_name,
            error = %error,
            duration_ms = duration.as_millis(),
            event = "request_error",
            "Request failed"
        );
    }

    pub fn log_health_check(&self, status: &str) {
        tracing::info!(
            server = %self.server_name,
            status = %status,
            event = "health_check",
            "Health check completed"
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

pub fn log_connection_stats(
    frontend_name: &str,
    connections_per_second: f64,
    total_connections: u64,
    active_connections: u64,
) {
    tracing::info!(
        frontend = %frontend_name,
        connections_per_second = connections_per_second,
        total_connections = total_connections,
        active_connections = active_connections,
        event = "connection_stats",
        "Connection statistics"
    );
}

pub fn log_ssl_handshake(client_ip: &str, ssl_version: &str, cipher: &str, success: bool) {
    let event = if success { "ssl_handshake_success" } else { "ssl_handshake_failed" };

    if success {
        tracing::info!(
            client_ip = %client_ip,
            ssl_version = %ssl_version,
            cipher = %cipher,
            success = success,
            event = %event,
            "SSL handshake completed"
        );
    } else {
        tracing::warn!(
            client_ip = %client_ip,
            ssl_version = %ssl_version,
            cipher = %cipher,
            success = success,
            event = %event,
            "SSL handshake completed"
        );
    }
}

pub fn log_rate_limit(client_ip: &str, frontend_name: &str, limit_exceeded: bool) {
    if limit_exceeded {
        tracing::warn!(
            client_ip = %client_ip,
            frontend = %frontend_name,
            event = "rate_limit_exceeded",
            "Rate limit exceeded"
        );
    } else {
        tracing::debug!(
            client_ip = %client_ip,
            frontend = %frontend_name,
            event = "rate_limit_check",
            "Rate limit check passed"
        );
    }
}

pub fn log_access_control(client_ip: &str, acl_name: &str, allowed: bool) {
    let event = if allowed { "acl_allowed" } else { "acl_denied" };

    if allowed {
        tracing::info!(
            client_ip = %client_ip,
            acl = %acl_name,
            allowed = allowed,
            event = %event,
            "Access control decision"
        );
    } else {
        tracing::warn!(
            client_ip = %client_ip,
            acl = %acl_name,
            allowed = allowed,
            event = %event,
            "Access control decision"
        );
    }
}

pub fn log_configuration_reload(config_file: &str, success: bool, errors: Option<Vec<String>>) {
    let event = if success { "config_reload_success" } else { "config_reload_failed" };

    if success {
        tracing::info!(
            config_file = %config_file,
            success = success,
            errors = ?errors,
            event = %event,
            "Configuration reload completed"
        );
    } else {
        tracing::error!(
            config_file = %config_file,
            success = success,
            errors = ?errors,
            event = %event,
            "Configuration reload completed"
        );
    }
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

pub fn log_shutdown_signal(signal: &str) {
    tracing::info!(
        signal = %signal,
        event = "shutdown_signal",
        "Received shutdown signal"
    );
}

pub fn log_graceful_shutdown(active_connections: u64) {
    tracing::info!(
        active_connections = active_connections,
        event = "graceful_shutdown",
        "Starting graceful shutdown"
    );
} 