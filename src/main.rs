use clap::Parser;
use tracing::{info, error, Level};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod config;
mod proxy;
mod logging;
mod metrics;
mod health;
mod utils;
mod acl;
mod balancer;
mod options;

use config::Config;
use proxy::ProxyServer;

#[derive(Parser)]
#[command(name = "turbogate")]
#[command(about = "High-performance L4 load balancer compatible with HAProxy")]
struct Cli {
    /// Path to configuration file
    #[arg(short, long, default_value = "turbogate.cfg")]
    config: String,

    /// Log level (trace, debug, info, warn, error)
    #[arg(short, long, default_value = "info")]
    log_level: Level,

    /// Enable JSON logging
    #[arg(long)]
    json_logs: bool,

    /// Validate configuration and exit
    #[arg(long)]
    check: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Инициализация логирования
    logging::init(cli.log_level, cli.json_logs)?;

    info!("Starting Turbogate L4 Load Balancer");
    info!("Log level: {}", cli.log_level);
    info!("Configuration file: {}", cli.config);

    // Загрузка конфигурации
    let config = match Config::from_file(&cli.config).await {
        Ok(config) => {
            info!("Configuration loaded successfully");
            config
        }
        Err(e) => {
            error!("Failed to load configuration: {}", e);
            return Err(e);
        }
    };

    // Валидация конфигурации
    if let Err(e) = config.validate() {
        error!("Configuration validation failed: {}", e);
        return Err(e.into());
    }

    if cli.check {
        info!("Configuration check passed");
        return Ok(());
    }

    // Инициализация метрик
    metrics::init(&config.metrics).await?;

    // Запуск прокси сервера
    let mut proxy = ProxyServer::new(config);
    
    info!("Starting proxy server...");
    if let Err(e) = proxy.run().await {
        error!("Proxy server failed: {}", e);
        return Err(e);
    }

    Ok(())
} 