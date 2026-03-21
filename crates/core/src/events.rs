use serde::Serialize;

use crate::models::{IndexingProgress, ProjectStatusInfo};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum LspEvent {
    ProjectStatusChanged {
        id: String,
        status: ProjectStatusInfo,
    },
    IndexingProgress {
        id: String,
        progress: IndexingProgress,
    },
    DiagnosticsUpdated {
        id: String,
        file: String,
        count: usize,
    },
    LogLine {
        id: String,
        line: String,
    },
    SettingsChanged {
        restart_required: bool,
    },
    ProjectsChanged,
}
