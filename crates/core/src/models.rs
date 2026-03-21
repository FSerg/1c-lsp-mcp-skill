use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, FromRow)]
pub struct StoredProject {
    pub id: String,
    pub name: String,
    pub root_path: String,
    /// Корневой путь всего проекта (где работают LLM-агенты).
    /// Входящие пути к файлам разрешаются относительно него.
    pub project_root_path: String,
    pub jvm_args: String,
    pub bsl_config: String,
    pub debug: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProjectUpsert {
    pub name: String,
    pub root_path: String,
    pub project_root_path: String,
    #[serde(default)]
    pub jvm_args: String,
    #[serde(default)]
    pub bsl_config: String,
    #[serde(default)]
    pub debug: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct IndexingProgress {
    pub percentage: Option<u32>,
    pub files_done: Option<u32>,
    pub files_total: Option<u32>,
    pub message: Option<String>,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectStatus {
    Stopped,
    Starting,
    WarmingUp,
    Ready,
    Error(String),
}

impl ProjectStatus {
    pub fn info(&self) -> ProjectStatusInfo {
        match self {
            Self::Stopped => ProjectStatusInfo {
                status: "stopped".to_string(),
                error: None,
            },
            Self::Starting => ProjectStatusInfo {
                status: "starting".to_string(),
                error: None,
            },
            Self::WarmingUp => ProjectStatusInfo {
                status: "warming_up".to_string(),
                error: None,
            },
            Self::Ready => ProjectStatusInfo {
                status: "ready".to_string(),
                error: None,
            },
            Self::Error(message) => ProjectStatusInfo {
                status: "error".to_string(),
                error: Some(message.clone()),
            },
        }
    }

    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }

    pub fn is_running(&self) -> bool {
        matches!(self, Self::Ready | Self::WarmingUp)
    }

    pub fn is_stopped(&self) -> bool {
        matches!(self, Self::Stopped)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectStatusInfo {
    pub status: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectSnapshot {
    #[serde(flatten)]
    pub project: StoredProject,
    pub status: ProjectStatusInfo,
    pub progress: IndexingProgress,
}

#[derive(Debug, Clone, Serialize)]
pub struct JavaCheckResult {
    pub found: bool,
    pub version: Option<String>,
    pub raw_output: String,
    pub ok: bool,
}
