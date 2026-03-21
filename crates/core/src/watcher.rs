use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, DebouncedEvent, Debouncer};
use serde_json::json;
use tokio::sync::{broadcast, mpsc};

use crate::events::LspEvent;
use crate::logging::append_project_log;
use crate::lsp::LspClient;
use crate::manager::SharedProject;

/// Handle for a running file watcher. Dropping it stops watching.
pub struct ProjectWatcher {
    _debouncer: Debouncer<notify::RecommendedWatcher>,
}

/// Start watching `.bsl` and `.os` files under `root_path` and send
/// `workspace/didChangeWatchedFiles` notifications to the LSP server.
///
/// Watcher failure is non-fatal — the project works without it.
pub(crate) fn start_project_watcher(
    root_path: &str,
    client: LspClient,
    project_id: String,
    project: SharedProject,
    logs_dir: PathBuf,
    event_tx: broadcast::Sender<LspEvent>,
) -> anyhow::Result<ProjectWatcher> {
    let root = PathBuf::from(root_path);
    if !root.is_dir() {
        anyhow::bail!("root_path is not a directory: {}", root.display());
    }

    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<DebouncedEvent>>();

    let mut debouncer = new_debouncer(Duration::from_millis(500), DebounceHandler(tx))?;

    debouncer.watcher().watch(&root, RecursiveMode::Recursive)?;

    tokio::spawn(async move {
        while let Some(events) = rx.recv().await {
            let changes = map_events(&events);
            if changes.is_empty() {
                continue;
            }

            let count = changes.len();
            if let Err(err) = client
                .notify(
                    "workspace/didChangeWatchedFiles",
                    json!({ "changes": changes }),
                )
                .await
            {
                tracing::warn!(
                    "Failed to send didChangeWatchedFiles for project {project_id}: {err}"
                );
                break;
            }

            // Log to project logs (visible in UI) only when debug is on
            let is_debug = project.read().await.config.debug;
            if is_debug {
                let summary = build_change_summary(&events, &root);
                let line =
                    format!("[FileWatcher] didChangeWatchedFiles ({count} файлов): {summary}");
                let _ =
                    append_project_log(logs_dir.clone(), project_id.clone(), line.clone()).await;
                let _ = event_tx.send(LspEvent::LogLine {
                    id: project_id.clone(),
                    line,
                });
            }
        }
    });

    Ok(ProjectWatcher {
        _debouncer: debouncer,
    })
}

/// Bridge: `notify` callback (std thread) → tokio channel.
struct DebounceHandler(mpsc::UnboundedSender<Vec<DebouncedEvent>>);

impl notify_debouncer_mini::DebounceEventHandler for DebounceHandler {
    fn handle_event(&mut self, event: DebounceEventResult) {
        match event {
            Ok(events) => {
                let _ = self.0.send(events);
            }
            Err(err) => {
                tracing::warn!("File watcher error: {err}");
            }
        }
    }
}

fn is_bsl_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let lower = e.to_ascii_lowercase();
            lower == "bsl" || lower == "os"
        })
        .unwrap_or(false)
}

/// Filter events to `.bsl`/`.os` files, deduplicate, and map to LSP change objects.
fn map_events(events: &[DebouncedEvent]) -> Vec<serde_json::Value> {
    let mut changes = Vec::new();
    let mut seen = HashSet::new();

    for event in events {
        let path = &event.path;
        if !is_bsl_file(path) {
            continue;
        }
        if !seen.insert(path.clone()) {
            continue;
        }

        let uri = match url::Url::from_file_path(path) {
            Ok(u) => u.to_string(),
            Err(_) => continue,
        };

        // notify-debouncer-mini doesn't distinguish Create/Change/Delete.
        // Check existence: gone → Deleted(3), otherwise → Changed(2).
        let change_type = if path.exists() { 2 } else { 3 };

        changes.push(json!({ "uri": uri, "type": change_type }));
    }

    changes
}

/// Build a human-readable summary of changed files (relative paths).
fn build_change_summary(events: &[DebouncedEvent], root: &Path) -> String {
    let mut seen = HashSet::new();
    let mut parts = Vec::new();

    for event in events {
        let path = &event.path;
        if !is_bsl_file(path) || !seen.insert(path.clone()) {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string();
        let label = if path.exists() { "changed" } else { "deleted" };
        parts.push(format!("{rel} ({label})"));
    }

    if parts.len() <= 5 {
        parts.join(", ")
    } else {
        let first: Vec<_> = parts.iter().take(5).cloned().collect();
        format!("{}, ... и ещё {}", first.join(", "), parts.len() - 5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify_debouncer_mini::DebouncedEventKind;

    fn event(path: &str, kind: DebouncedEventKind) -> DebouncedEvent {
        DebouncedEvent::new(PathBuf::from(path), kind)
    }

    #[test]
    fn filters_non_bsl_files() {
        let events = vec![
            event("/project/src/module.bsl", DebouncedEventKind::Any),
            event("/project/src/readme.md", DebouncedEventKind::Any),
            event("/project/src/script.os", DebouncedEventKind::Any),
            event("/project/src/data.json", DebouncedEventKind::Any),
        ];
        let changes = map_events(&events);
        assert_eq!(changes.len(), 2);
    }

    #[test]
    fn deduplicates_same_path() {
        let events = vec![
            event("/project/src/module.bsl", DebouncedEventKind::Any),
            event("/project/src/module.bsl", DebouncedEventKind::AnyContinuous),
        ];
        let changes = map_events(&events);
        assert_eq!(changes.len(), 1);
    }

    #[test]
    fn case_insensitive_extension() {
        let events = vec![
            event("/project/src/module.BSL", DebouncedEventKind::Any),
            event("/project/src/script.Os", DebouncedEventKind::Any),
        ];
        let changes = map_events(&events);
        assert_eq!(changes.len(), 2);
    }

    #[test]
    fn empty_events() {
        let changes = map_events(&[]);
        assert!(changes.is_empty());
    }

    #[test]
    fn deleted_file_gets_type_3() {
        // Path that doesn't exist on disk
        let events = vec![event(
            "/nonexistent/path/deleted.bsl",
            DebouncedEventKind::Any,
        )];
        let changes = map_events(&events);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0]["type"], 3);
    }
}
