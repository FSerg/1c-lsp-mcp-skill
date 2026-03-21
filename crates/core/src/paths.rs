use std::fs;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use directories::ProjectDirs;

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_local_dir: PathBuf,
    pub state_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub bsl_configs_dir: PathBuf,
    pub config_path: PathBuf,
    pub db_path: PathBuf,
    pub runtime_path: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        let dirs = ProjectDirs::from("ru", "Infostart", "LspSkill")
            .ok_or_else(|| anyhow!("failed to resolve application directories"))?;

        let config_dir = dirs.config_dir().to_path_buf();
        let data_local_dir = dirs.data_local_dir().to_path_buf();
        let state_dir = dirs
            .state_dir()
            .map(PathBuf::from)
            .unwrap_or_else(|| data_local_dir.clone());
        let logs_dir = state_dir.join("logs");
        let bsl_configs_dir = data_local_dir.join("bsl-configs");

        Ok(Self {
            config_path: config_dir.join("config.toml"),
            db_path: data_local_dir.join("data.db"),
            runtime_path: state_dir.join("runtime.json"),
            config_dir,
            data_local_dir,
            state_dir,
            logs_dir,
            bsl_configs_dir,
        })
    }

    pub fn ensure(&self) -> Result<()> {
        fs::create_dir_all(&self.config_dir).context("failed to create config dir")?;
        fs::create_dir_all(&self.data_local_dir).context("failed to create data dir")?;
        fs::create_dir_all(&self.state_dir).context("failed to create state dir")?;
        fs::create_dir_all(&self.logs_dir).context("failed to create logs dir")?;
        fs::create_dir_all(&self.bsl_configs_dir).context("failed to create bsl-configs dir")?;
        Ok(())
    }
}
