use notify::{Watcher, RecursiveMode, RecommendedWatcher, Config as NotifyConfig, EventKind};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;
use tokio::sync::broadcast;
use anyhow::{Result, anyhow};
use tracing::{info, error};
use crate::config::Config;

pub struct HotReload {
    config_path: String,
    reload_tx: broadcast::Sender<Config>,
}

impl HotReload {
    pub fn new(config_path: String) -> Result<Self> {
        let (reload_tx, _reload_rx) = broadcast::channel(10);
        
        Ok(Self {
            config_path,
            reload_tx,
        })
    }

    pub fn start_watching(&self) -> Result<()> {
        let config_path = self.config_path.clone();
        let reload_tx = self.reload_tx.clone();

        std::thread::spawn(move || {
            if let Err(e) = Self::watch_config_file(&config_path, reload_tx) {
                error!("Config file watcher failed: {}", e);
            }
        });

        info!("Hot reload watcher started for config file: {}", self.config_path);
        Ok(())
    }

    fn watch_config_file(config_path: &str, reload_tx: broadcast::Sender<Config>) -> Result<()> {
        let (tx, rx) = mpsc::channel();

        let mut watcher = RecommendedWatcher::new(tx, NotifyConfig::default())?;
        watcher.watch(Path::new(config_path), RecursiveMode::NonRecursive)?;

        info!("Watching config file for changes: {}", config_path);

        for res in rx {
            match res {
                Ok(event) => {
                    match event.kind {
                        EventKind::Modify(_) => {
                            info!("Config file modified, reloading...");
                            match Self::reload_config(config_path) {
                                Ok(config) => {
                                    if let Err(e) = reload_tx.send(config) {
                                        error!("Failed to send reload signal: {}", e);
                                    } else {
                                        info!("Configuration reloaded successfully");
                                    }
                                }
                                Err(e) => {
                                    error!("Failed to reload config: {}", e);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Err(e) => {
                    error!("Watch error: {}", e);
                }
            }
        }

        Ok(())
    }

    fn reload_config(config_path: &str) -> Result<Config> {
        std::thread::sleep(Duration::from_millis(100));
        
        let content = std::fs::read_to_string(config_path)
            .map_err(|e| anyhow!("Failed to read config file: {}", e))?;
        
        Config::from_haproxy_config(&content)
    }
}
