use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_mini::{
    new_debouncer, DebounceEventResult, DebouncedEvent, DebouncedEventKind, Debouncer,
};
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileChange {
    path: PathBuf,
    change_type: u8,
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

    let mut known_files = collect_known_files(&root);

    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<DebouncedEvent>>();

    let mut debouncer = new_debouncer(Duration::from_millis(500), DebounceHandler(tx))?;

    debouncer.watcher().watch(&root, RecursiveMode::Recursive)?;

    tokio::spawn(async move {
        while let Some(events) = rx.recv().await {
            let changes = map_events(&events, &mut known_files);
            if changes.is_empty() {
                continue;
            }

            let count = changes.len();
            let payload: Vec<_> = changes
                .iter()
                .filter_map(|change| {
                    let uri = url::Url::from_file_path(&change.path).ok()?.to_string();
                    Some(json!({ "uri": uri, "type": change.change_type }))
                })
                .collect();
            if payload.is_empty() {
                continue;
            }

            if let Err(err) = client
                .notify(
                    "workspace/didChangeWatchedFiles",
                    json!({ "changes": payload }),
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
                let summary = build_change_summary(&changes, &root);
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

fn collect_known_files(root: &Path) -> HashSet<PathBuf> {
    let mut files = HashSet::new();
    collect_known_files_recursive(root, &mut files);
    files
}

fn collect_known_files_recursive(path: &Path, files: &mut HashSet<PathBuf>) {
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };

    if metadata.is_file() {
        if is_bsl_file(path) {
            files.insert(path.to_path_buf());
        }
        return;
    }

    let Ok(entries) = fs::read_dir(path) else {
        return;
    };

    for entry in entries.flatten() {
        collect_known_files_recursive(&entry.path(), files);
    }
}

/// Filter events to `.bsl`/`.os` files, drop intermediate continuous events,
/// deduplicate per batch, and map to LSP change objects.
fn map_events(events: &[DebouncedEvent], known_files: &mut HashSet<PathBuf>) -> Vec<FileChange> {
    let mut changes = Vec::new();
    let mut seen = HashMap::new();

    for event in events {
        let path = &event.path;
        if !is_bsl_file(path) {
            continue;
        }
        seen.entry(path.clone())
            .and_modify(|kind| {
                if event.kind == DebouncedEventKind::Any {
                    *kind = DebouncedEventKind::Any;
                }
            })
            .or_insert(event.kind);
    }

    for (path, kind) in seen {
        if kind != DebouncedEventKind::Any {
            continue;
        }

        if let Some(change_type) = classify_change(&path, known_files) {
            changes.push(FileChange { path, change_type });
        }
    }

    changes
}

fn classify_change(path: &Path, known_files: &mut HashSet<PathBuf>) -> Option<u8> {
    let exists = path.exists();
    let was_known = known_files.contains(path);

    match (was_known, exists) {
        (false, true) => {
            known_files.insert(path.to_path_buf());
            Some(1)
        }
        (true, true) => Some(2),
        (true, false) => {
            known_files.remove(path);
            Some(3)
        }
        (false, false) => None,
    }
}

/// Build a human-readable summary of changed files (relative paths).
fn build_change_summary(changes: &[FileChange], root: &Path) -> String {
    let mut parts = Vec::new();

    for change in changes {
        let rel = change
            .path
            .strip_prefix(root)
            .unwrap_or(&change.path)
            .display()
            .to_string();
        let label = match change.change_type {
            1 => "created",
            2 => "changed",
            3 => "deleted",
            _ => "changed",
        };
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
        let mut known_files = HashSet::from([
            PathBuf::from("/project/src/module.bsl"),
            PathBuf::from("/project/src/script.os"),
        ]);
        let events = vec![
            event("/project/src/module.bsl", DebouncedEventKind::Any),
            event("/project/src/readme.md", DebouncedEventKind::Any),
            event("/project/src/script.os", DebouncedEventKind::Any),
            event("/project/src/data.json", DebouncedEventKind::Any),
        ];
        let changes = map_events(&events, &mut known_files);
        assert_eq!(changes.len(), 2);
    }

    #[test]
    fn deduplicates_same_path() {
        let mut known_files = HashSet::from([PathBuf::from("/project/src/module.bsl")]);
        let events = vec![
            event("/project/src/module.bsl", DebouncedEventKind::Any),
            event("/project/src/module.bsl", DebouncedEventKind::AnyContinuous),
        ];
        let changes = map_events(&events, &mut known_files);
        assert_eq!(changes.len(), 1);
    }

    #[test]
    fn case_insensitive_extension() {
        let mut known_files = HashSet::from([
            PathBuf::from("/project/src/module.BSL"),
            PathBuf::from("/project/src/script.Os"),
        ]);
        let events = vec![
            event("/project/src/module.BSL", DebouncedEventKind::Any),
            event("/project/src/script.Os", DebouncedEventKind::Any),
        ];
        let changes = map_events(&events, &mut known_files);
        assert_eq!(changes.len(), 2);
    }

    #[test]
    fn empty_events() {
        let mut known_files = HashSet::new();
        let changes = map_events(&[], &mut known_files);
        assert!(changes.is_empty());
    }

    #[test]
    fn deleted_file_gets_type_3() {
        let mut known_files = HashSet::from([PathBuf::from("/nonexistent/path/deleted.bsl")]);
        // Path that doesn't exist on disk
        let events = vec![event(
            "/nonexistent/path/deleted.bsl",
            DebouncedEventKind::Any,
        )];
        let changes = map_events(&events, &mut known_files);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].change_type, 3);
    }

    #[test]
    fn ignores_any_continuous_without_terminal_any() {
        let mut known_files = HashSet::from([PathBuf::from("/project/src/module.bsl")]);
        let events = vec![event(
            "/project/src/module.bsl",
            DebouncedEventKind::AnyContinuous,
        )];
        let changes = map_events(&events, &mut known_files);
        assert!(changes.is_empty());
    }

    #[test]
    fn new_file_gets_type_1() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("module.bsl");
        fs::write(&path, "Procedure Test() EndProcedure").expect("write file");

        let mut known_files = HashSet::new();
        let events = vec![DebouncedEvent::new(path.clone(), DebouncedEventKind::Any)];
        let changes = map_events(&events, &mut known_files);

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].change_type, 1);
        assert!(known_files.contains(&path));
    }

    #[test]
    fn build_change_summary_uses_change_types() {
        let changes = vec![
            FileChange {
                path: PathBuf::from("/project/src/created.bsl"),
                change_type: 1,
            },
            FileChange {
                path: PathBuf::from("/project/src/deleted.bsl"),
                change_type: 3,
            },
        ];

        let summary = build_change_summary(&changes, Path::new("/project"));
        assert_eq!(
            summary,
            "src/created.bsl (created), src/deleted.bsl (deleted)"
        );
    }
}
