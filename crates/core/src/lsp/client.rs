use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use tokio::io::AsyncBufReadExt;
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

use super::transport::{LspReader, LspWriter};

type PendingMap = Arc<Mutex<HashMap<i64, oneshot::Sender<Value>>>>;
pub type NotificationHandler = Arc<dyn Fn(String, Value) + Send + Sync>;
pub type StderrHandler = Arc<dyn Fn(String) + Send + Sync>;

#[derive(Clone)]
pub struct LspClient {
    sender: mpsc::Sender<Value>,
    pending: PendingMap,
    next_id: Arc<AtomicI64>,
}

impl LspClient {
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        self.sender
            .send(message)
            .await
            .map_err(|_| anyhow!("LSP writer channel closed"))?;

        let response = rx
            .await
            .map_err(|_| anyhow!("LSP response channel dropped"))?;
        if let Some(error) = response.get("error") {
            bail!("LSP error: {error}");
        }

        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    pub async fn notify(&self, method: &str, params: Value) -> Result<()> {
        let message = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });

        self.sender
            .send(message)
            .await
            .map_err(|_| anyhow!("LSP writer channel closed"))?;
        Ok(())
    }
}

pub async fn spawn_lsp_server(
    java_path: &str,
    jar_path: &str,
    jvm_args: &str,
    bsl_config_path: Option<&str>,
    notification_handler: NotificationHandler,
    stderr_handler: StderrHandler,
) -> Result<(LspClient, Child)> {
    let mut command = Command::new(java_path);
    for arg in shlex::split(jvm_args).unwrap_or_default() {
        command.arg(arg);
    }
    command.arg("-jar").arg(jar_path).arg("lsp");
    if let Some(config_path) = bsl_config_path {
        command.arg("--configuration").arg(config_path);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn()?;
    let stdin = child.stdin.take().ok_or_else(|| anyhow!("missing stdin"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("missing stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("missing stderr"))?;

    let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
    let (tx, mut rx) = mpsc::channel::<Value>(64);

    tokio::spawn(async move {
        let mut writer = LspWriter::new(stdin);
        while let Some(message) = rx.recv().await {
            if let Err(err) = writer.send(&message).await {
                tracing::error!("LSP write error: {err}");
                break;
            }
        }
    });

    let pending_for_reader = pending.clone();
    let tx_for_reader = tx.clone();
    tokio::spawn(async move {
        let mut reader = LspReader::new(stdout);
        loop {
            match reader.recv().await {
                Ok(message) => {
                    if let Some(id) = message.get("id") {
                        if message.get("method").is_some() {
                            let response = json!({
                                "jsonrpc": "2.0",
                                "id": id.clone(),
                                "result": null,
                            });
                            let _ = tx_for_reader.send(response).await;
                        } else {
                            let request_id = id.as_i64().unwrap_or(-1);
                            let mut pending = pending_for_reader.lock().await;
                            if let Some(tx) = pending.remove(&request_id) {
                                let _ = tx.send(message);
                            }
                        }
                    } else {
                        let method = message
                            .get("method")
                            .and_then(|value| value.as_str())
                            .unwrap_or_default()
                            .to_string();
                        let params = message.get("params").cloned().unwrap_or(Value::Null);
                        let handler = notification_handler.clone();
                        tokio::spawn(async move {
                            handler(method, params);
                        });
                    }
                }
                Err(err) => {
                    tracing::debug!("LSP reader stopped: {err}");
                    break;
                }
            }
        }
    });

    tokio::spawn(async move {
        let reader = tokio::io::BufReader::new(stderr);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let handler = stderr_handler.clone();
            tokio::spawn(async move {
                handler(line);
            });
        }
    });

    Ok((
        LspClient {
            sender: tx,
            pending,
            next_id: Arc::new(AtomicI64::new(1)),
        },
        child,
    ))
}
