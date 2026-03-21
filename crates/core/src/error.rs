use std::fmt::{Display, Formatter};

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("{0}")]
    ServerNotRunning(String),
    #[error("{0}")]
    ProjectNotReady(String),
    #[error("{0}")]
    FileNotFound(String),
    #[error("{0}")]
    InvalidRequest(String),
    #[error("{0}")]
    JavaNotFound(String),
    #[error("{0}")]
    PortInUse(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Internal(String),
}

impl ServiceError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::ServerNotRunning(_) => "server_not_running",
            Self::ProjectNotReady(_) => "project_not_ready",
            Self::FileNotFound(_) => "file_not_found",
            Self::InvalidRequest(_) => "invalid_request",
            Self::JavaNotFound(_) => "java_not_found",
            Self::PortInUse(_) => "port_in_use",
            Self::NotFound(_) => "not_found",
            Self::Internal(_) => "internal_error",
        }
    }

    pub fn message(&self) -> String {
        self.to_string()
    }

    pub fn http_status(&self) -> u16 {
        match self {
            Self::ServerNotRunning(_) => 503,
            Self::ProjectNotReady(_) => 409,
            Self::FileNotFound(_) => 404,
            Self::InvalidRequest(_) => 400,
            Self::JavaNotFound(_) => 400,
            Self::PortInUse(_) => 409,
            Self::NotFound(_) => 404,
            Self::Internal(_) => 500,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub code: String,
}

impl From<&ServiceError> for ErrorResponse {
    fn from(value: &ServiceError) -> Self {
        Self {
            error: value.message(),
            code: value.code().to_string(),
        }
    }
}

impl Display for ErrorResponse {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.error, self.code)
    }
}
