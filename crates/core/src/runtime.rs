use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sysinfo::{Pid, System};
use tokio::fs;

use crate::paths::AppPaths;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeMetadata {
    pub pid: u32,
    pub listen_host: String,
    pub port: u16,
    pub connect_url: String,
}

impl RuntimeMetadata {
    pub fn new(pid: u32, listen_host: String, port: u16) -> Self {
        let connect_url = compute_connect_url(&listen_host, port);
        Self {
            pid,
            listen_host,
            port,
            connect_url,
        }
    }

    pub async fn read(paths: &AppPaths) -> Result<Option<Self>> {
        match fs::read_to_string(&paths.runtime_path).await {
            Ok(body) => Ok(Some(
                serde_json::from_str(&body).context("failed to parse runtime.json")?,
            )),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err).context("failed to read runtime.json"),
        }
    }

    pub async fn write(&self, paths: &AppPaths) -> Result<()> {
        let body =
            serde_json::to_string_pretty(self).context("failed to serialize runtime metadata")?;
        fs::write(&paths.runtime_path, body)
            .await
            .context("failed to write runtime.json")?;
        Ok(())
    }

    pub async fn remove(paths: &AppPaths) -> Result<()> {
        match fs::remove_file(&paths.runtime_path).await {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err).context("failed to remove runtime.json"),
        }
    }

    pub fn process_is_alive(&self) -> bool {
        let system = System::new_all();
        system.process(Pid::from_u32(self.pid)).is_some()
    }
}

pub fn compute_connect_url(listen_host: &str, port: u16) -> String {
    let host = match listen_host {
        "0.0.0.0" | "::" => "127.0.0.1",
        other => other,
    };
    format!("http://{host}:{port}")
}

#[cfg(test)]
mod tests {
    use super::compute_connect_url;

    #[test]
    fn maps_wildcard_host_to_loopback() {
        assert_eq!(
            compute_connect_url("0.0.0.0", 4000),
            "http://127.0.0.1:4000"
        );
        assert_eq!(compute_connect_url("::", 4000), "http://127.0.0.1:4000");
    }
}
