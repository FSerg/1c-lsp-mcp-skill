use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::paths::AppPaths;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub jar_path: String,
    #[serde(default = "default_listen_host")]
    pub listen_host: String,
    #[serde(default = "default_http_port")]
    pub http_port: u16,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default)]
    pub mcp_diagnostics_enabled: bool,
    #[serde(default = "default_mcp_diagnostics_port")]
    pub mcp_diagnostics_port: u16,
    #[serde(default)]
    pub mcp_navigation_enabled: bool,
    #[serde(default = "default_mcp_navigation_port")]
    pub mcp_navigation_port: u16,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            jar_path: String::new(),
            listen_host: default_listen_host(),
            http_port: default_http_port(),
            log_level: default_log_level(),
            mcp_diagnostics_enabled: false,
            mcp_diagnostics_port: default_mcp_diagnostics_port(),
            mcp_navigation_enabled: false,
            mcp_navigation_port: default_mcp_navigation_port(),
        }
    }
}

impl AppConfig {
    pub async fn load_or_create(paths: &AppPaths) -> Result<Self> {
        match fs::read_to_string(&paths.config_path).await {
            Ok(content) => Ok(toml::from_str(&content).context("failed to parse config.toml")?),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let config = Self::default();
                config.save(paths).await?;
                Ok(config)
            }
            Err(err) => Err(err).context("failed to read config.toml"),
        }
    }

    pub async fn save(&self, paths: &AppPaths) -> Result<()> {
        let body = toml::to_string_pretty(self).context("failed to serialize config")?;
        fs::write(&paths.config_path, body)
            .await
            .context("failed to write config.toml")?;
        Ok(())
    }
}

fn default_listen_host() -> String {
    "0.0.0.0".to_string()
}

fn default_http_port() -> u16 {
    4000
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_mcp_diagnostics_port() -> u16 {
    9011
}

fn default_mcp_navigation_port() -> u16 {
    9012
}
