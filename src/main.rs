use clap::Parser;
use tracing::{info, error, Level};
use std::sync::Arc;

mod config;
mod proxy;
mod logging;
mod metrics;
mod health;
mod utils;
mod acl;
mod balancer;
mod options;
mod rate_limit;
mod ddos_protection;
mod hot_reload;
mod compression;
mod features;

use config::Config;
use proxy::ProxyServer;
use features::FeaturesManager;

#[derive(Parser)]
#[command(name = "turbogate")]
#[command(about = "High-performance L4 load balancer compatible with HAProxy")]
struct Cli {
    #[arg(short, long, default_value = "turbogate.cfg")]
    config: String,

    #[arg(short, long, default_value = "info")]
    log_level: Level,

    #[arg(long)]
    json_logs: bool,

    #[arg(long)]
    check: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    logging::init(cli.log_level, cli.json_logs)?;

    info!("Starting Turbogate L4 Load Balancer");
    info!("Log level: {}", cli.log_level);
    info!("Configuration file: {}", cli.config);

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

    if let Err(e) = config.validate() {
        error!("Configuration validation failed: {}", e);
        return Err(e.into());
    }

    if cli.check {
        info!("Configuration check passed");
        return Ok(());
    }

    metrics::init(&config.metrics).await?;

    let config_arc = Arc::new(config);
    let features_manager = FeaturesManager::new(config_arc.clone())?;
    
    let mut proxy = ProxyServer::new(Arc::new(features_manager));
    
    info!("Starting proxy server with enhanced features...");
    if let Err(e) = proxy.run().await {
        error!("Proxy server failed: {}", e);
        return Err(e);
    }

    Ok(())
}
