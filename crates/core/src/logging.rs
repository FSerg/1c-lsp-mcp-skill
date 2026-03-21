use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result};
use chrono::Utc;
use tracing_subscriber::EnvFilter;

const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 100 * 1024 * 1024;

static LOGGING_INIT: OnceLock<()> = OnceLock::new();

#[derive(Clone)]
struct RotatingFileWriter {
    inner: Arc<Mutex<RotatingFileState>>,
}

struct RotatingFileState {
    path: PathBuf,
    file: File,
    size: u64,
}

impl RotatingFileWriter {
    fn new(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("failed to create log dir")?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .context("failed to open server log")?;
        let size = file.metadata().map(|meta| meta.len()).unwrap_or(0);
        Ok(Self {
            inner: Arc::new(Mutex::new(RotatingFileState { path, file, size })),
        })
    }
}

impl Write for RotatingFileWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut state = self.inner.lock().expect("log writer poisoned");
        if state.size + buf.len() as u64 > MAX_FILE_BYTES {
            rotate_file(&state.path)?;
            state.file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&state.path)?;
            state.size = state.file.metadata().map(|meta| meta.len()).unwrap_or(0);
            if let Some(parent) = state.path.parent() {
                cleanup_logs_dir(parent).map_err(|err| std::io::Error::other(err.to_string()))?;
            }
        }

        let written = state.file.write(buf)?;
        state.size += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut state = self.inner.lock().expect("log writer poisoned");
        state.file.flush()
    }
}

pub fn init_logging(logs_dir: &Path, level: &str) -> Result<()> {
    if LOGGING_INIT.get().is_some() {
        return Ok(());
    }

    fs::create_dir_all(logs_dir).context("failed to create logs dir")?;
    cleanup_logs_dir(logs_dir)?;

    let writer = RotatingFileWriter::new(logs_dir.join("server.log"))?;
    let filter = EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_env_filter(filter)
        .with_writer(move || writer.clone())
        .init();

    let _ = LOGGING_INIT.set(());
    Ok(())
}

pub async fn append_project_log(logs_dir: PathBuf, project_id: String, line: String) -> Result<()> {
    tokio::task::spawn_blocking(move || append_project_log_sync(&logs_dir, &project_id, &line))
        .await
        .context("project log task failed")??;
    Ok(())
}

pub async fn tail_project_log(
    logs_dir: PathBuf,
    project_id: String,
    tail: usize,
) -> Result<Vec<String>> {
    tokio::task::spawn_blocking(move || {
        let path = project_log_path(&logs_dir, &project_id);
        if !path.exists() {
            return Ok(Vec::new());
        }

        let mut content = String::new();
        File::open(path)
            .and_then(|mut file| file.read_to_string(&mut content))
            .context("failed to read project log")?;

        let mut lines: Vec<String> = content.lines().map(ToString::to_string).collect();
        if lines.len() > tail {
            lines.drain(0..lines.len() - tail);
        }
        Ok(lines)
    })
    .await
    .context("log tail task failed")?
}

pub async fn clear_project_logs(logs_dir: PathBuf, project_id: String) -> Result<()> {
    tokio::task::spawn_blocking(move || clear_project_logs_sync(&logs_dir, &project_id))
        .await
        .context("project log clear task failed")??;
    Ok(())
}

fn append_project_log_sync(logs_dir: &Path, project_id: &str, line: &str) -> Result<()> {
    fs::create_dir_all(logs_dir).context("failed to create logs dir")?;
    let path = project_log_path(logs_dir, project_id);

    if path.exists() && path.metadata().map(|meta| meta.len()).unwrap_or(0) > MAX_FILE_BYTES {
        rotate_file(&path)?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .context("failed to open project log")?;
    writeln!(file, "{line}").context("failed to append project log")?;
    cleanup_logs_dir(logs_dir)?;
    Ok(())
}

fn clear_project_logs_sync(logs_dir: &Path, project_id: &str) -> Result<()> {
    fs::create_dir_all(logs_dir).context("failed to create logs dir")?;
    let prefix = project_log_prefix(project_id);

    for entry in fs::read_dir(logs_dir).context("failed to read logs dir")? {
        let entry = entry.context("failed to read log entry")?;
        let path = entry.path();
        if !entry
            .metadata()
            .context("failed to read log metadata")?
            .is_file()
        {
            continue;
        }

        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };

        if name == format!("{prefix}.log")
            || (name.starts_with(&format!("{prefix}-")) && name.ends_with(".log"))
        {
            let _ = fs::remove_file(path);
        }
    }

    Ok(())
}

fn project_log_path(logs_dir: &Path, project_id: &str) -> PathBuf {
    logs_dir.join(format!("{}.log", project_log_prefix(project_id)))
}

fn project_log_prefix(project_id: &str) -> String {
    format!("project-{project_id}")
}

fn rotate_file(path: &Path) -> std::io::Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("log");
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("log");
    let rotated = path.with_file_name(format!(
        "{stem}-{}.{}",
        Utc::now().format("%Y%m%d%H%M%S"),
        ext
    ));

    fs::rename(path, rotated)
}

fn cleanup_logs_dir(logs_dir: &Path) -> Result<()> {
    let mut files = Vec::new();
    let mut total_size = 0u64;

    for entry in fs::read_dir(logs_dir).context("failed to read logs dir")? {
        let entry = entry.context("failed to read log entry")?;
        let metadata = entry.metadata().context("failed to read log metadata")?;
        if !metadata.is_file() {
            continue;
        }

        total_size += metadata.len();
        files.push((entry.path(), metadata.modified().ok(), metadata.len()));
    }

    if total_size <= MAX_TOTAL_BYTES {
        return Ok(());
    }

    files.sort_by_key(|(_, modified, _)| *modified);
    for (path, _, len) in files {
        if total_size <= MAX_TOTAL_BYTES {
            break;
        }
        if path.file_name().and_then(|name| name.to_str()) == Some("server.log") {
            continue;
        }
        let _ = fs::remove_file(path);
        total_size = total_size.saturating_sub(len);
    }

    Ok(())
}
