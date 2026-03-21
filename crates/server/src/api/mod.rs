use std::convert::Infallible;
use std::path::PathBuf;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::BroadcastStream;

use lsp_skill_core::{
    check_java, AppConfig, ErrorResponse, ProjectSnapshot, ProjectUpsert, ServiceError,
};

use crate::runtime::ServerState;

pub fn router() -> Router<ServerState> {
    Router::new()
        .route("/settings", get(get_settings).put(update_settings))
        .route("/settings/check-java", post(check_java_handler))
        .route("/browse", post(browse_directory))
        .route("/projects", get(list_projects).post(create_project))
        .route(
            "/projects/{id}",
            get(get_project).put(update_project).delete(delete_project),
        )
        .route("/projects/{id}/start", post(start_project))
        .route("/projects/{id}/stop", post(stop_project))
        .route("/projects/{id}/status", get(project_status))
        .route(
            "/projects/{id}/logs",
            get(project_logs).delete(clear_project_logs),
        )
        .route("/projects/{id}/diagnostics", post(project_diagnostics))
        .route("/projects/{id}/symbols", post(project_symbols))
        .route("/projects/{id}/references", post(project_references))
        .route("/projects/{id}/definition", post(project_definition))
        .route(
            "/projects/{id}/workspace-symbols",
            post(project_workspace_symbols),
        )
        .route("/events", get(events))
}

#[derive(Debug)]
struct ApiError(ServiceError);

impl From<ServiceError> for ApiError {
    fn from(value: ServiceError) -> Self {
        Self(value)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status =
            StatusCode::from_u16(self.0.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, Json(ErrorResponse::from(&self.0))).into_response()
    }
}

#[derive(Debug, Deserialize)]
struct SettingsUpdate {
    jar_path: String,
    listen_host: String,
    http_port: u16,
    log_level: String,
    #[serde(default)]
    mcp_diagnostics_enabled: bool,
    #[serde(default = "default_mcp_diagnostics_port")]
    mcp_diagnostics_port: u16,
    #[serde(default)]
    mcp_navigation_enabled: bool,
    #[serde(default = "default_mcp_navigation_port")]
    mcp_navigation_port: u16,
}

fn default_mcp_diagnostics_port() -> u16 {
    9011
}

fn default_mcp_navigation_port() -> u16 {
    9012
}

#[derive(Debug, Serialize)]
struct SettingsResponse {
    jar_path: String,
    listen_host: String,
    http_port: u16,
    log_level: String,
    mcp_diagnostics_enabled: bool,
    mcp_diagnostics_port: u16,
    mcp_navigation_enabled: bool,
    mcp_navigation_port: u16,
    config_path: String,
    db_path: String,
    logs_dir: String,
    restart_required: bool,
}

#[derive(Debug, Deserialize)]
struct LogsQuery {
    tail: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct FileRequest {
    file_path: String,
}

#[derive(Debug, Deserialize)]
struct ReferencesRequest {
    file_path: String,
    line: u32,
    character: u32,
}

#[derive(Debug, Deserialize)]
struct PositionRequest {
    file_path: String,
    line: u32,
    character: u32,
}

#[derive(Debug, Deserialize)]
struct WorkspaceSymbolsRequest {
    query: String,
}

#[derive(Debug, Deserialize)]
struct EventsQuery {
    project_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BrowseRequest {
    path: Option<String>,
    /// true — показывать файлы (с фильтрацией по extension), false — только каталоги
    #[serde(default)]
    show_files: bool,
    /// Фильтр по расширению файлов (например "jar")
    extension: Option<String>,
}

#[derive(Debug, Serialize)]
struct BrowseResponse {
    current: String,
    parent: Option<String>,
    entries: Vec<BrowseEntry>,
}

#[derive(Debug, Serialize)]
struct BrowseEntry {
    name: String,
    is_dir: bool,
}

async fn get_settings(
    State(state): State<ServerState>,
) -> Result<Json<SettingsResponse>, ApiError> {
    let config = state.manager.current_config().await;
    Ok(Json(settings_response(&state, config, false)))
}

async fn update_settings(
    State(state): State<ServerState>,
    Json(payload): Json<SettingsUpdate>,
) -> Result<Json<SettingsResponse>, ApiError> {
    let current = state.manager.current_config().await;
    let restart_required = current.listen_host != payload.listen_host
        || current.http_port != payload.http_port
        || current.mcp_diagnostics_enabled != payload.mcp_diagnostics_enabled
        || current.mcp_diagnostics_port != payload.mcp_diagnostics_port
        || current.mcp_navigation_enabled != payload.mcp_navigation_enabled
        || current.mcp_navigation_port != payload.mcp_navigation_port;
    let next = AppConfig {
        jar_path: payload.jar_path,
        listen_host: payload.listen_host,
        http_port: payload.http_port,
        log_level: payload.log_level,
        mcp_diagnostics_enabled: payload.mcp_diagnostics_enabled,
        mcp_diagnostics_port: payload.mcp_diagnostics_port,
        mcp_navigation_enabled: payload.mcp_navigation_enabled,
        mcp_navigation_port: payload.mcp_navigation_port,
    };
    next.save(&state.paths)
        .await
        .map_err(|err| ApiError(ServiceError::Internal(err.to_string())))?;
    state
        .manager
        .replace_config(next.clone(), restart_required)
        .await;
    Ok(Json(settings_response(&state, next, restart_required)))
}

async fn check_java_handler() -> Json<lsp_skill_core::JavaCheckResult> {
    Json(check_java().await)
}

async fn browse_directory(
    Json(payload): Json<BrowseRequest>,
) -> Result<Json<BrowseResponse>, ApiError> {
    let raw = match &payload.path {
        Some(p) if !p.is_empty() => PathBuf::from(strip_win_prefix(p)),
        _ => dirs_home(),
    };

    let base = raw
        .canonicalize()
        .map_err(|e| ApiError(ServiceError::InvalidRequest(format!("Путь не найден: {e}"))))?;
    let base = clean_path(base);

    if !base.is_dir() {
        return Err(ApiError(ServiceError::InvalidRequest(
            "Указанный путь не является каталогом".to_string(),
        )));
    }

    let parent = base
        .parent()
        .map(|p| clean_path(p.to_path_buf()).to_string_lossy().to_string());

    let mut entries = Vec::new();
    let mut dir = tokio::fs::read_dir(&base).await.map_err(|e| {
        ApiError(ServiceError::Internal(format!(
            "Не удалось прочитать каталог: {e}"
        )))
    })?;

    while let Some(entry) = dir
        .next_entry()
        .await
        .map_err(|e| ApiError(ServiceError::Internal(e.to_string())))?
    {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }

        let is_dir = entry.metadata().await.map(|m| m.is_dir()).unwrap_or(false);

        if !is_dir && !payload.show_files {
            continue;
        }

        if !is_dir {
            if let Some(ext) = &payload.extension {
                let file_ext = std::path::Path::new(&name)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                if !file_ext.eq_ignore_ascii_case(ext) {
                    continue;
                }
            }
        }

        entries.push(BrowseEntry { name, is_dir });
    }

    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    Ok(Json(BrowseResponse {
        current: base.to_string_lossy().to_string(),
        parent,
        entries,
    }))
}

/// Убирает Windows extended-length prefix `\\?\` из пути.
/// `canonicalize()` на Windows добавляет этот префикс, но он ломает повторный
/// разбор и выглядит плохо в UI.
fn clean_path(path: PathBuf) -> PathBuf {
    let s = path.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(stripped) => PathBuf::from(stripped),
        None => path,
    }
}

/// Защитная очистка входящего пути от `\\?\` перед использованием.
fn strip_win_prefix(path: &str) -> &str {
    path.strip_prefix(r"\\?\").unwrap_or(path)
}

fn dirs_home() -> PathBuf {
    #[cfg(unix)]
    {
        std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/"))
    }
    #[cfg(windows)]
    {
        std::env::var("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("C:\\"))
    }
    #[cfg(not(any(unix, windows)))]
    {
        PathBuf::from("/")
    }
}

async fn list_projects(State(state): State<ServerState>) -> Json<Vec<ProjectSnapshot>> {
    Json(state.manager.list_projects().await)
}

async fn create_project(
    State(state): State<ServerState>,
    Json(payload): Json<ProjectUpsert>,
) -> Result<Json<ProjectSnapshot>, ApiError> {
    Ok(Json(state.manager.create_project(payload).await?))
}

async fn get_project(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Result<Json<ProjectSnapshot>, ApiError> {
    Ok(Json(state.manager.get_project(&id).await?))
}

async fn update_project(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Json(payload): Json<ProjectUpsert>,
) -> Result<Json<ProjectSnapshot>, ApiError> {
    Ok(Json(state.manager.update_project(&id, payload).await?))
}

async fn delete_project(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.manager.delete_project(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn start_project(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Result<Json<ProjectSnapshot>, ApiError> {
    Ok(Json(state.manager.start_project(&id).await?))
}

async fn stop_project(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Result<Json<ProjectSnapshot>, ApiError> {
    Ok(Json(state.manager.stop_project(&id).await?))
}

async fn project_status(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Result<Json<ProjectSnapshot>, ApiError> {
    Ok(Json(state.manager.get_project(&id).await?))
}

async fn project_logs(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Query(query): Query<LogsQuery>,
) -> Result<Json<Vec<String>>, ApiError> {
    Ok(Json(
        state
            .manager
            .project_logs(&id, query.tail.unwrap_or(1000))
            .await?,
    ))
}

async fn clear_project_logs(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.manager.clear_project_logs(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn project_diagnostics(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Json(payload): Json<FileRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(
        state.manager.diagnostics(&id, &payload.file_path).await?,
    ))
}

async fn project_symbols(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Json(payload): Json<FileRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(state.manager.symbols(&id, &payload.file_path).await?))
}

async fn project_references(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Json(payload): Json<ReferencesRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(
        state
            .manager
            .references(&id, &payload.file_path, payload.line, payload.character)
            .await?,
    ))
}

async fn project_definition(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Json(payload): Json<PositionRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(
        state
            .manager
            .definition(&id, &payload.file_path, payload.line, payload.character)
            .await?,
    ))
}

async fn project_workspace_symbols(
    State(state): State<ServerState>,
    Path(id): Path<String>,
    Json(payload): Json<WorkspaceSymbolsRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(
        state.manager.workspace_symbols(&id, &payload.query).await?,
    ))
}

async fn events(
    State(state): State<ServerState>,
    Query(query): Query<EventsQuery>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let project_filter = query.project_id.clone();
    let shutdown_token = state.shutdown_token.clone();
    let stream = BroadcastStream::new(state.manager.subscribe())
        .take_while(move |_| {
            let cancelled = shutdown_token.is_cancelled();
            async move { !cancelled }
        })
        .filter_map(move |item| {
            let project_filter = project_filter.clone();
            async move {
                match item {
                    Ok(event) => {
                        if let Some(project_id) = project_filter.as_deref() {
                            if let Some(event_project_id) = event_project_id(&event) {
                                if event_project_id != project_id {
                                    return None;
                                }
                            }
                        }

                        let data = serde_json::to_string(&event).ok()?;
                        Some(Ok(Event::default().event("message").data(data)))
                    }
                    Err(_) => None,
                }
            }
        });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn settings_response(
    state: &ServerState,
    config: AppConfig,
    restart_required: bool,
) -> SettingsResponse {
    SettingsResponse {
        jar_path: config.jar_path,
        listen_host: config.listen_host,
        http_port: config.http_port,
        log_level: config.log_level,
        mcp_diagnostics_enabled: config.mcp_diagnostics_enabled,
        mcp_diagnostics_port: config.mcp_diagnostics_port,
        mcp_navigation_enabled: config.mcp_navigation_enabled,
        mcp_navigation_port: config.mcp_navigation_port,
        config_path: state.paths.config_path.display().to_string(),
        db_path: state.paths.db_path.display().to_string(),
        logs_dir: state.paths.logs_dir.display().to_string(),
        restart_required,
    }
}

fn event_project_id(event: &lsp_skill_core::LspEvent) -> Option<&str> {
    match event {
        lsp_skill_core::LspEvent::ProjectStatusChanged { id, .. } => Some(id),
        lsp_skill_core::LspEvent::IndexingProgress { id, .. } => Some(id),
        lsp_skill_core::LspEvent::DiagnosticsUpdated { id, .. } => Some(id),
        lsp_skill_core::LspEvent::LogLine { id, .. } => Some(id),
        lsp_skill_core::LspEvent::SettingsChanged { .. } => None,
        lsp_skill_core::LspEvent::ProjectsChanged => None,
    }
}
