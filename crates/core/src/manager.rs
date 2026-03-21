use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result as AnyResult;
use chrono::Utc;
use serde_json::{json, Value};
use tokio::process::Child;
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio::time::sleep;
use url::Url;
use uuid::Uuid;

use crate::config::AppConfig;
use crate::db::Database;
use crate::error::ServiceError;
use crate::events::LspEvent;
use crate::logging::{append_project_log, clear_project_logs, tail_project_log};
use crate::lsp::{spawn_lsp_server, LspClient, NotificationHandler, StderrHandler};
use crate::models::{
    IndexingProgress, ProjectSnapshot, ProjectStatus, ProjectUpsert, StoredProject,
};
use crate::paths::AppPaths;
use crate::watcher::ProjectWatcher;

pub(crate) type SharedProject = Arc<RwLock<ProjectState>>;

pub(crate) struct ProjectState {
    pub(crate) config: StoredProject,
    status: ProjectStatus,
    client: Option<LspClient>,
    child: Option<Arc<Mutex<Child>>>,
    diagnostics: HashMap<String, Value>,
    opened_files: HashMap<String, u32>,
    progress: IndexingProgress,
    /// Token of the main progress (the one that sends "report" events).
    progress_token: Option<String>,
    watcher: Option<ProjectWatcher>,
}

impl ProjectState {
    fn from_config(config: StoredProject) -> Self {
        Self {
            config,
            status: ProjectStatus::Stopped,
            client: None,
            child: None,
            diagnostics: HashMap::new(),
            opened_files: HashMap::new(),
            progress: IndexingProgress::default(),
            progress_token: None,
            watcher: None,
        }
    }

    fn snapshot(&self) -> ProjectSnapshot {
        ProjectSnapshot {
            project: self.config.clone(),
            status: self.status.info(),
            progress: self.progress.clone(),
        }
    }
}

#[derive(Clone)]
pub struct LspManager {
    projects: Arc<RwLock<HashMap<String, SharedProject>>>,
    db: Database,
    paths: AppPaths,
    config: Arc<RwLock<AppConfig>>,
    event_tx: broadcast::Sender<LspEvent>,
}

impl LspManager {
    pub async fn load(
        paths: AppPaths,
        config: Arc<RwLock<AppConfig>>,
        db: Database,
    ) -> AnyResult<Self> {
        let mut projects = HashMap::new();
        for project in db.list_projects().await? {
            projects.insert(
                project.id.clone(),
                Arc::new(RwLock::new(ProjectState::from_config(project))),
            );
        }

        let (event_tx, _) = broadcast::channel(256);
        Ok(Self {
            projects: Arc::new(RwLock::new(projects)),
            db,
            paths,
            config,
            event_tx,
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LspEvent> {
        self.event_tx.subscribe()
    }

    pub async fn current_config(&self) -> AppConfig {
        self.config.read().await.clone()
    }

    pub async fn replace_config(&self, next: AppConfig, restart_required: bool) {
        *self.config.write().await = next;
        let _ = self
            .event_tx
            .send(LspEvent::SettingsChanged { restart_required });
    }

    pub async fn list_projects(&self) -> Vec<ProjectSnapshot> {
        let handles: Vec<SharedProject> = self.projects.read().await.values().cloned().collect();
        let mut snapshots = Vec::with_capacity(handles.len());
        for project in handles {
            snapshots.push(project.read().await.snapshot());
        }
        snapshots
    }

    pub async fn get_project(&self, id: &str) -> Result<ProjectSnapshot, ServiceError> {
        let project = self.project_handle(id).await?;
        let snapshot = project.read().await.snapshot();
        Ok(snapshot)
    }

    pub async fn create_project(
        &self,
        input: ProjectUpsert,
    ) -> Result<ProjectSnapshot, ServiceError> {
        validate_bsl_config(&input.bsl_config)?;
        let now = Utc::now().to_rfc3339();
        let project = StoredProject {
            id: Uuid::new_v4().to_string(),
            name: validate_name(&input.name)?,
            root_path: canonical_root_path(&input.root_path)?,
            project_root_path: canonical_root_path(&input.project_root_path)?,
            jvm_args: input.jvm_args.trim().to_string(),
            bsl_config: input.bsl_config.clone(),
            debug: input.debug,
            created_at: now.clone(),
            updated_at: now,
        };

        self.db
            .insert_project(&project)
            .await
            .map_err(|err| map_create_project_error(&err))?;

        let state = Arc::new(RwLock::new(ProjectState::from_config(project.clone())));
        self.projects
            .write()
            .await
            .insert(project.id.clone(), state.clone());

        let _ = self.event_tx.send(LspEvent::ProjectsChanged);
        let snapshot = state.read().await.snapshot();
        Ok(snapshot)
    }

    pub async fn update_project(
        &self,
        id: &str,
        input: ProjectUpsert,
    ) -> Result<ProjectSnapshot, ServiceError> {
        validate_bsl_config(&input.bsl_config)?;
        let project = self.project_handle(id).await?;
        {
            let state = project.read().await;
            if !state.status.is_stopped() {
                return Err(ServiceError::InvalidRequest(
                    "Чтобы изменить проект, сначала остановите его.".to_string(),
                ));
            }
        }

        let updated = {
            let mut state = project.write().await;
            state.config.name = validate_name(&input.name)?;
            state.config.root_path = canonical_root_path(&input.root_path)?;
            state.config.project_root_path = canonical_root_path(&input.project_root_path)?;
            state.config.jvm_args = input.jvm_args.trim().to_string();
            state.config.bsl_config = input.bsl_config.clone();
            state.config.debug = input.debug;
            state.config.updated_at = Utc::now().to_rfc3339();
            state.config.clone()
        };

        self.db
            .update_project(&updated)
            .await
            .map_err(|err| map_update_project_error(&err))?;

        let _ = self.event_tx.send(LspEvent::ProjectsChanged);
        let snapshot = project.read().await.snapshot();
        Ok(snapshot)
    }

    pub async fn delete_project(&self, id: &str) -> Result<(), ServiceError> {
        let project = self.project_handle(id).await?;
        {
            let state = project.read().await;
            if !state.status.is_stopped() {
                return Err(ServiceError::InvalidRequest(
                    "Нельзя удалить запущенный проект. Сначала остановите его.".to_string(),
                ));
            }
        }

        self.db
            .delete_project(id)
            .await
            .map_err(|err| ServiceError::Internal(err.to_string()))?;
        self.projects.write().await.remove(id);
        let _ = self.event_tx.send(LspEvent::ProjectsChanged);
        Ok(())
    }

    pub async fn start_project(&self, id: &str) -> Result<ProjectSnapshot, ServiceError> {
        let project = self.project_handle(id).await?;

        let (root_path, jvm_args, bsl_config, project_id) = {
            let mut state = project.write().await;
            if !state.status.is_stopped() {
                return Err(ServiceError::InvalidRequest(
                    "Проект уже запущен.".to_string(),
                ));
            }

            state.status = ProjectStatus::Starting;
            state.progress = IndexingProgress::default();
            state.diagnostics.clear();
            state.opened_files.clear();
            state.client = None;
            state.child = None;

            (
                state.config.root_path.clone(),
                state.config.jvm_args.clone(),
                state.config.bsl_config.clone(),
                state.config.id.clone(),
            )
        };
        self.emit_status(id, &project).await;

        let settings = self.current_config().await;
        if settings.jar_path.trim().is_empty() {
            self.set_error(
                id,
                &project,
                "Не указан путь к JAR-файлу BSL Language Server.",
            )
            .await;
            return Err(ServiceError::InvalidRequest(
                "Не указан путь к JAR-файлу BSL Language Server.".to_string(),
            ));
        }

        let notification_handler = project_notification_handler(
            project.clone(),
            self.paths.logs_dir.clone(),
            self.event_tx.clone(),
            id.to_string(),
        );
        let stderr_handler = project_stderr_handler(
            project.clone(),
            self.paths.logs_dir.clone(),
            self.event_tx.clone(),
            id.to_string(),
        );

        // Write per-project BSL config (always — use default if user didn't set one)
        let bsl_config_content = effective_bsl_config(&bsl_config);

        let bsl_config_file = {
            let config_dir = &self.paths.bsl_configs_dir;
            let config_file = config_dir.join(format!("{project_id}.json"));
            if let Err(err) = tokio::fs::create_dir_all(config_dir).await {
                self.set_error(
                    id,
                    &project,
                    &format!("Не удалось создать каталог конфигов: {err}"),
                )
                .await;
                return Err(ServiceError::Internal(format!(
                    "Не удалось создать каталог конфигов: {err}"
                )));
            }
            if let Err(err) = tokio::fs::write(&config_file, bsl_config_content).await {
                self.set_error(
                    id,
                    &project,
                    &format!("Не удалось записать конфигурацию BSL: {err}"),
                )
                .await;
                return Err(ServiceError::Internal(format!(
                    "Не удалось записать конфигурацию BSL: {err}"
                )));
            }
            Some(config_file)
        };

        let bsl_config_path_str = bsl_config_file
            .as_ref()
            .map(|p| p.to_string_lossy().to_string());

        let (client, child) = match spawn_lsp_server(
            "java",
            &settings.jar_path,
            &jvm_args,
            bsl_config_path_str.as_deref(),
            notification_handler,
            stderr_handler,
        )
        .await
        {
            Ok(value) => value,
            Err(err) => {
                let message = if err.to_string().contains("No such file or directory") {
                    "Java не найдена в PATH.".to_string()
                } else {
                    format!("Не удалось запустить BSL Language Server: {err}")
                };
                self.set_error(id, &project, &message).await;
                return if message.contains("Java не найдена") {
                    Err(ServiceError::JavaNotFound(message))
                } else {
                    Err(ServiceError::Internal(message))
                };
            }
        };

        // Log startup info
        {
            let jar_display = normalize_slashes(&settings.jar_path);
            let mut cmd_line = if jvm_args.is_empty() {
                format!("java -jar {jar_display} lsp")
            } else {
                format!("java {jvm_args} -jar {jar_display} lsp")
            };
            if let Some(ref cfg_path) = bsl_config_path_str {
                cmd_line.push_str(&format!(" --configuration {}", normalize_slashes(cfg_path)));
            }
            let root_uri = Url::from_directory_path(&root_path)
                .map(|u| u.to_string())
                .unwrap_or_else(|_| root_path.clone());

            let mut lines = vec![
                "─── Запуск BSL Language Server ───".to_string(),
                format!("Командная строка: {cmd_line}"),
                format!("Корневой каталог: {}", normalize_slashes(&root_path)),
                format!("rootUri: {root_uri}"),
            ];

            if let Some(ref cfg_path) = bsl_config_path_str {
                match tokio::fs::read_to_string(cfg_path).await {
                    Ok(content) => {
                        lines.push(format!(
                            "Конфигурация BSL ({}):",
                            normalize_slashes(cfg_path),
                        ));
                        lines.push(content);
                    }
                    Err(_) => {
                        lines.push("Конфигурация BSL: ошибка чтения файла".to_string());
                    }
                }
            } else {
                let config_path = PathBuf::from(&root_path).join(".bsl-language-server.json");
                match tokio::fs::read_to_string(&config_path).await {
                    Ok(content) => {
                        lines.push(format!(
                            "Конфигурация BSL ({}):",
                            normalize_slashes(&config_path.display().to_string()),
                        ));
                        lines.push(content);
                    }
                    Err(_) => {
                        lines.push(
                            "Конфигурация BSL (.bsl-language-server.json): не найдена".to_string(),
                        );
                    }
                }
            }

            for line in lines {
                let _ =
                    append_project_log(self.paths.logs_dir.clone(), id.to_string(), line.clone())
                        .await;
                let _ = self.event_tx.send(LspEvent::LogLine {
                    id: id.to_string(),
                    line,
                });
            }
        }

        initialize_client(&client, &root_path)
            .await
            .map_err(|err| {
                let message = err.to_string();
                tokio::spawn({
                    let project = project.clone();
                    let event_tx = self.event_tx.clone();
                    let id = id.to_string();
                    async move {
                        {
                            let mut state = project.write().await;
                            state.status = ProjectStatus::Error(message.clone());
                        }
                        let _ = event_tx.send(LspEvent::ProjectStatusChanged {
                            id,
                            status: ProjectStatus::Error(message).info(),
                        });
                    }
                });
                ServiceError::Internal("Не удалось инициализировать LSP-сервер.".to_string())
            })?;

        let watcher = match crate::watcher::start_project_watcher(
            &root_path,
            client.clone(),
            id.to_string(),
            project.clone(),
            self.paths.logs_dir.clone(),
            self.event_tx.clone(),
        ) {
            Ok(w) => {
                tracing::info!("File watcher started for project {id}");
                Some(w)
            }
            Err(err) => {
                tracing::warn!("File watcher failed to start for project {id}: {err}");
                None
            }
        };

        let child = Arc::new(Mutex::new(child));
        {
            let mut state = project.write().await;
            state.client = Some(client);
            state.child = Some(child.clone());
            state.watcher = watcher;
            state.status = ProjectStatus::WarmingUp;
        }
        self.emit_status(id, &project).await;

        tokio::spawn(watch_project_process(
            project.clone(),
            self.event_tx.clone(),
            id.to_string(),
            child,
        ));

        let snapshot = project.read().await.snapshot();
        Ok(snapshot)
    }

    pub async fn stop_project(&self, id: &str) -> Result<ProjectSnapshot, ServiceError> {
        let project = self.project_handle(id).await?;
        let (client, child) = {
            let state = project.read().await;
            (state.client.clone(), state.child.clone())
        };

        if let Some(client) = client {
            let _ = client.request("shutdown", Value::Null).await;
            let _ = client.notify("exit", Value::Null).await;
        }

        if let Some(child) = child {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                let exited = {
                    let mut child = child.lock().await;
                    match child.try_wait() {
                        Ok(Some(_)) => true,
                        Ok(None) => false,
                        Err(_) => true,
                    }
                };

                if exited {
                    break;
                }

                if Instant::now() >= deadline {
                    let _ = child.lock().await.kill().await;
                    break;
                }

                sleep(Duration::from_millis(250)).await;
            }
        }

        {
            let mut state = project.write().await;
            state.status = ProjectStatus::Stopped;
            state.client = None;
            state.child = None;
            state.watcher = None;
            state.diagnostics.clear();
            state.opened_files.clear();
            state.progress = IndexingProgress::default();
        }

        self.emit_status(id, &project).await;
        let snapshot = project.read().await.snapshot();
        Ok(snapshot)
    }

    pub async fn shutdown_all(&self) {
        let ids: Vec<String> = self.projects.read().await.keys().cloned().collect();
        for id in ids {
            let _ = self.stop_project(&id).await;
        }
    }

    pub async fn diagnostics(&self, id: &str, file_path: &str) -> Result<Value, ServiceError> {
        let (project, uri, _) = self.ensure_file_opened(id, file_path).await?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(value) = project.read().await.diagnostics.get(&uri).cloned() {
                return Ok(value);
            }
            if Instant::now() >= deadline {
                return Ok(json!({
                    "uri": uri,
                    "diagnostics": [],
                    "_note": "Истекло время ожидания диагностики"
                }));
            }
            sleep(Duration::from_millis(500)).await;
        }
    }

    pub async fn symbols(&self, id: &str, file_path: &str) -> Result<Value, ServiceError> {
        let (_, uri, client) = self.ensure_file_opened(id, file_path).await?;

        self.log(id, format!(">> symbols: {file_path}")).await;

        let result = client
            .request(
                "textDocument/documentSymbol",
                json!({ "textDocument": { "uri": uri } }),
            )
            .await
            .map_err(|err| ServiceError::Internal(err.to_string()));

        match &result {
            Ok(value) => {
                self.log(id, format!("<< symbols: {value}")).await;
            }
            Err(err) => {
                self.log(id, format!("<< symbols ошибка: {err}")).await;
            }
        }

        result
    }

    pub async fn references(
        &self,
        id: &str,
        file_path: &str,
        line: u32,
        character: u32,
    ) -> Result<Value, ServiceError> {
        let (_, uri, client) = self.ensure_file_opened(id, file_path).await?;

        self.log(
            id,
            format!(">> references: {file_path} ({line}:{character})"),
        )
        .await;

        let result = client
            .request(
                "textDocument/references",
                json!({
                    "textDocument": { "uri": uri },
                    "position": { "line": line, "character": character },
                    "context": { "includeDeclaration": true }
                }),
            )
            .await
            .map_err(|err| ServiceError::Internal(err.to_string()));

        match &result {
            Ok(value) => {
                self.log(id, format!("<< references: {value}")).await;
            }
            Err(err) => {
                self.log(id, format!("<< references ошибка: {err}")).await;
            }
        }

        result
    }

    pub async fn definition(
        &self,
        id: &str,
        file_path: &str,
        line: u32,
        character: u32,
    ) -> Result<Value, ServiceError> {
        let (_, uri, client) = self.ensure_file_opened(id, file_path).await?;

        self.log(
            id,
            format!(">> definition: {file_path} ({line}:{character})"),
        )
        .await;

        let result = client
            .request(
                "textDocument/definition",
                json!({
                    "textDocument": { "uri": uri },
                    "position": { "line": line, "character": character }
                }),
            )
            .await
            .map_err(|err| ServiceError::Internal(err.to_string()));

        match &result {
            Ok(value) => {
                self.log(id, format!("<< definition: {value}")).await;
            }
            Err(err) => {
                self.log(id, format!("<< definition ошибка: {err}")).await;
            }
        }

        result
    }

    pub async fn workspace_symbols(&self, id: &str, query: &str) -> Result<Value, ServiceError> {
        let project = self.project_handle(id).await?;
        let client = {
            let state = project.read().await;
            if !state.status.is_running() {
                return Err(ServiceError::ProjectNotReady(format!(
                    "Проект еще не готов. Текущий статус: {}.",
                    project_status_label(&state.status)
                )));
            }
            state.client.clone().ok_or_else(|| {
                ServiceError::ProjectNotReady("У проекта нет активного LSP-клиента.".to_string())
            })?
        };

        self.log(id, format!(">> workspace/symbol: \"{query}\""))
            .await;

        let result = client
            .request("workspace/symbol", json!({ "query": query }))
            .await
            .map_err(|err| ServiceError::Internal(err.to_string()));

        match &result {
            Ok(value) => {
                let count = value.as_array().map(|a| a.len()).unwrap_or(0);
                self.log(id, format!("<< workspace/symbol: {count} символов"))
                    .await;
            }
            Err(err) => {
                self.log(id, format!("<< workspace/symbol ошибка: {err}"))
                    .await;
            }
        }

        result
    }

    pub async fn project_logs(&self, id: &str, tail: usize) -> Result<Vec<String>, ServiceError> {
        self.project_handle(id).await?;
        tail_project_log(self.paths.logs_dir.clone(), id.to_string(), tail)
            .await
            .map_err(|err| ServiceError::Internal(err.to_string()))
    }

    pub async fn clear_project_logs(&self, id: &str) -> Result<(), ServiceError> {
        self.project_handle(id).await?;
        clear_project_logs(self.paths.logs_dir.clone(), id.to_string())
            .await
            .map_err(|err| ServiceError::Internal(err.to_string()))
    }

    async fn ensure_file_opened(
        &self,
        id: &str,
        file_path: &str,
    ) -> Result<(SharedProject, String, LspClient), ServiceError> {
        let project = self.project_handle(id).await?;
        let (root_path, project_root_path, client, ready, prev_version) = {
            let state = project.read().await;
            (
                state.config.root_path.clone(),
                state.config.project_root_path.clone(),
                state.client.clone(),
                state.status.clone(),
                state.opened_files.get(file_path).copied(),
            )
        };

        if !ready.is_running() {
            return Err(ServiceError::ProjectNotReady(format!(
                "Проект еще не готов. Текущий статус: {}.",
                project_status_label(&ready)
            )));
        }

        let client = client.ok_or_else(|| {
            ServiceError::ProjectNotReady("У проекта нет активного LSP-клиента.".to_string())
        })?;

        let absolute_path = resolve_project_file(
            Path::new(&root_path),
            Path::new(&project_root_path),
            file_path,
        )
        .await?;
        let uri = Url::from_file_path(&absolute_path)
            .map_err(|_| ServiceError::InvalidRequest("Некорректный путь к файлу.".to_string()))?
            .to_string();

        let content = tokio::fs::read_to_string(&absolute_path)
            .await
            .map_err(|err| ServiceError::FileNotFound(err.to_string()))?;

        match prev_version {
            None => {
                // First time — send didOpen
                client
                    .notify(
                        "textDocument/didOpen",
                        json!({
                            "textDocument": {
                                "uri": uri,
                                "languageId": "bsl",
                                "version": 1,
                                "text": content,
                            }
                        }),
                    )
                    .await
                    .map_err(|err| ServiceError::Internal(err.to_string()))?;

                project
                    .write()
                    .await
                    .opened_files
                    .insert(file_path.to_string(), 1);
            }
            Some(ver) => {
                // Already opened — send didChange with full content.
                // Requires computeTrigger: "onType" in bsl-language-server
                // config for diagnostics to be recalculated.
                let new_version = ver + 1;
                {
                    let mut state = project.write().await;
                    state.diagnostics.remove(&uri);
                    state
                        .opened_files
                        .insert(file_path.to_string(), new_version);
                }
                client
                    .notify(
                        "textDocument/didChange",
                        json!({
                            "textDocument": {
                                "uri": uri,
                                "version": new_version,
                            },
                            "contentChanges": [{ "text": content }],
                        }),
                    )
                    .await
                    .map_err(|err| ServiceError::Internal(err.to_string()))?;
            }
        }

        Ok((project, uri, client))
    }

    async fn log(&self, id: &str, line: String) {
        let is_debug = match self.projects.read().await.get(id) {
            Some(p) => p.read().await.config.debug,
            None => false,
        };
        if !is_debug {
            return;
        }
        let _ = append_project_log(self.paths.logs_dir.clone(), id.to_string(), line.clone()).await;
        let _ = self.event_tx.send(LspEvent::LogLine {
            id: id.to_string(),
            line,
        });
    }

    async fn project_handle(&self, id: &str) -> Result<SharedProject, ServiceError> {
        self.projects
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| ServiceError::NotFound(format!("Проект с id {id} не найден.")))
    }

    async fn emit_status(&self, id: &str, project: &SharedProject) {
        let status = project.read().await.status.info();
        let _ = self.event_tx.send(LspEvent::ProjectStatusChanged {
            id: id.to_string(),
            status,
        });
    }

    async fn set_error(&self, id: &str, project: &SharedProject, message: &str) {
        {
            let mut state = project.write().await;
            state.status = ProjectStatus::Error(message.to_string());
            state.client = None;
            state.child = None;
        }
        self.emit_status(id, project).await;
    }
}

fn validate_name(name: &str) -> Result<String, ServiceError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(ServiceError::InvalidRequest(
            "Имя проекта не должно быть пустым.".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

fn canonical_root_path(path: &str) -> Result<String, ServiceError> {
    let path = path.trim();
    if path.is_empty() {
        return Err(ServiceError::InvalidRequest(
            "Корневой путь не указан.".to_string(),
        ));
    }

    let canonical =
        clean_path(PathBuf::from(path).canonicalize().map_err(|_| {
            ServiceError::InvalidRequest(format!("Корневой путь не найден: {path}"))
        })?);
    if !canonical.is_dir() {
        return Err(ServiceError::InvalidRequest(format!(
            "Корневой путь не является каталогом: {path}"
        )));
    }
    Ok(canonical.to_string_lossy().to_string())
}

/// Разрешает относительный путь к файлу в абсолютный.
///
/// `project_root` — корень всего проекта, откуда работают LLM-агенты и CLI.
/// Путь разрешается относительно `project_root`, но итоговый абсолютный путь
/// должен находиться внутри `root_path` (корня BSL).
async fn resolve_project_file(
    root_path: &Path,
    project_root: &Path,
    relative_path: &str,
) -> Result<PathBuf, ServiceError> {
    let relative = PathBuf::from(relative_path);
    if relative.is_absolute() {
        return Err(ServiceError::InvalidRequest(
            "Путь к файлу должен быть относительным относительно корня проекта.".to_string(),
        ));
    }

    let root = clean_path(tokio::fs::canonicalize(root_path).await.map_err(|_| {
        ServiceError::InvalidRequest("Корневой каталог BSL недоступен.".to_string())
    })?);

    let resolve_base = clean_path(tokio::fs::canonicalize(project_root).await.map_err(|_| {
        ServiceError::InvalidRequest("Корневой каталог проекта недоступен.".to_string())
    })?);

    let absolute = clean_path(
        tokio::fs::canonicalize(resolve_base.join(&relative))
            .await
            .map_err(|_| ServiceError::FileNotFound(relative_path.to_string()))?,
    );

    if !absolute.starts_with(&root) {
        return Err(ServiceError::InvalidRequest(
            "Путь к файлу выходит за пределы корня BSL.".to_string(),
        ));
    }

    Ok(absolute)
}

/// Убирает Windows extended-length prefix `\\?\` из пути.
/// `canonicalize()` на Windows добавляет этот префикс, что ломает
/// повторную обработку пути и выглядит некорректно в UI.
fn clean_path(path: PathBuf) -> PathBuf {
    let s = path.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(stripped) => PathBuf::from(stripped),
        None => path,
    }
}

fn project_notification_handler(
    project: SharedProject,
    logs_dir: PathBuf,
    event_tx: broadcast::Sender<LspEvent>,
    project_id: String,
) -> NotificationHandler {
    Arc::new(move |method, params| {
        let project = project.clone();
        let logs_dir = logs_dir.clone();
        let event_tx = event_tx.clone();
        let project_id = project_id.clone();
        tokio::spawn(async move {
            let debug = project.read().await.config.debug;

            if debug {
                let line = format!("[LSP] {method}: {params}");
                let _ =
                    append_project_log(logs_dir.clone(), project_id.clone(), line.clone()).await;
                let _ = event_tx.send(LspEvent::LogLine {
                    id: project_id.clone(),
                    line,
                });
            }

            match method.as_str() {
                "textDocument/publishDiagnostics" => {
                    if let Some(raw_uri) = params.get("uri").and_then(|value| value.as_str()) {
                        // Normalize URI: BSL server may send non-encoded Cyrillic,
                        // but ensure_file_opened uses Url::from_file_path which percent-encodes.
                        let uri = Url::parse(raw_uri)
                            .map(|u| u.to_string())
                            .unwrap_or_else(|_| raw_uri.to_string());
                        let count = params
                            .get("diagnostics")
                            .and_then(|value| value.as_array())
                            .map(Vec::len)
                            .unwrap_or(0);
                        project
                            .write()
                            .await
                            .diagnostics
                            .insert(uri.clone(), params.clone());
                        let _ = event_tx.send(LspEvent::DiagnosticsUpdated {
                            id: project_id,
                            file: uri,
                            count,
                        });
                    }
                }
                "$/progress" => {
                    let token = params
                        .get("token")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let value = params.get("value").cloned().unwrap_or(Value::Null);
                    let kind = value
                        .get("kind")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();

                    let mut state = project.write().await;

                    // Filter by token: only track the main progress
                    // "report" always comes from the main task → adopt its token
                    // "begin" only processed if no main token yet
                    // "end" only processed if from the main token
                    let dominated = match kind {
                        "begin" => state.progress_token.is_some(),
                        "report" => {
                            if state.progress_token.is_none() {
                                state.progress_token = token.clone();
                            }
                            state.progress_token != token
                        }
                        "end" => state.progress_token != token,
                        _ => true,
                    };

                    if !dominated {
                        apply_progress_value(&mut state.progress, &value);

                        if kind == "end" {
                            state.progress_token = None;
                        }

                        let _ = event_tx.send(LspEvent::IndexingProgress {
                            id: project_id.clone(),
                            progress: state.progress.clone(),
                        });

                        let end_message = value
                            .get("message")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default();
                        let is_context_ready = kind == "end"
                            && (end_message.contains("Наполнение контекста завершено")
                                || end_message.contains("Context populated"));

                        if is_context_ready && matches!(state.status, ProjectStatus::WarmingUp) {
                            state.status = ProjectStatus::Ready;
                            let _ = event_tx.send(LspEvent::ProjectStatusChanged {
                                id: project_id,
                                status: state.status.info(),
                            });
                        }
                    }
                }
                _ => {}
            }
        });
    })
}

fn project_stderr_handler(
    project: SharedProject,
    logs_dir: PathBuf,
    event_tx: broadcast::Sender<LspEvent>,
    project_id: String,
) -> StderrHandler {
    Arc::new(move |line| {
        let project = project.clone();
        let logs_dir = logs_dir.clone();
        let event_tx = event_tx.clone();
        let project_id = project_id.clone();
        tokio::spawn(async move {
            let _ = append_project_log(logs_dir, project_id.clone(), line.clone()).await;
            let _ = event_tx.send(LspEvent::LogLine {
                id: project_id.clone(),
                line: line.clone(),
            });

            if line.contains("Наполнение контекста завершено") || line.contains("Context populated")
            {
                let mut state = project.write().await;
                if matches!(state.status, ProjectStatus::WarmingUp) {
                    state.status = ProjectStatus::Ready;
                    let _ = event_tx.send(LspEvent::ProjectStatusChanged {
                        id: project_id,
                        status: state.status.info(),
                    });
                }
            }
        });
    })
}

fn apply_progress_value(progress: &mut IndexingProgress, value: &Value) {
    let kind = value
        .get("kind")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    match kind {
        "begin" => {
            progress.active = true;
            progress.percentage = value
                .get("percentage")
                .and_then(|value| value.as_u64())
                .map(|value| value as u32);
            progress.message = value
                .get("title")
                .and_then(|value| value.as_str())
                .map(ToString::to_string);
        }
        "report" => {
            progress.active = true;
            if let Some(percentage) = value.get("percentage").and_then(|value| value.as_u64()) {
                progress.percentage = Some(percentage as u32);
            }
            if let Some(message) = value.get("message").and_then(|value| value.as_str()) {
                progress.message = Some(message.to_string());
            }
        }
        "end" => {
            progress.active = false;
            progress.message = value
                .get("message")
                .and_then(|value| value.as_str())
                .map(ToString::to_string);
            progress.percentage = Some(100);
        }
        _ => {}
    }
}

async fn initialize_client(client: &LspClient, root_path: &str) -> AnyResult<()> {
    let root_uri = Url::from_directory_path(root_path)
        .map_err(|_| anyhow::anyhow!("invalid root path"))?
        .to_string();

    client
        .request(
            "initialize",
            json!({
                "processId": std::process::id(),
                "rootUri": root_uri,
                "workspaceFolders": [{
                    "uri": root_uri,
                    "name": "project"
                }],
                "capabilities": {
                    "textDocument": {
                        "publishDiagnostics": { "relatedInformation": true },
                        "documentSymbol": {
                            "hierarchicalDocumentSymbolSupport": true
                        },
                        "references": {
                            "dynamicRegistration": false
                        },
                        "definition": {
                            "dynamicRegistration": false,
                            "linkSupport": true
                        }
                    },
                    "window": {
                        "workDoneProgress": true
                    },
                    "workspace": {
                        "symbol": {
                            "dynamicRegistration": false
                        },
                        "didChangeWatchedFiles": {
                            "dynamicRegistration": true
                        }
                    }
                }
            }),
        )
        .await?;
    client.notify("initialized", json!({})).await?;
    Ok(())
}

async fn watch_project_process(
    project: SharedProject,
    event_tx: broadcast::Sender<LspEvent>,
    project_id: String,
    child: Arc<Mutex<Child>>,
) {
    loop {
        let exit_status = {
            let mut child = child.lock().await;
            match child.try_wait() {
                Ok(status) => status,
                Err(err) => {
                    tracing::error!("failed to inspect child process: {err}");
                    None
                }
            }
        };

        if let Some(status) = exit_status {
            let maybe_status = {
                let mut state = project.write().await;
                state.client = None;
                state.child = None;
                state.watcher = None;
                state.opened_files.clear();
                if state.status.is_stopped() {
                    None
                } else if status.success() {
                    state.status = ProjectStatus::Stopped;
                    Some(state.status.info())
                } else {
                    state.status =
                        ProjectStatus::Error(format!("LSP-процесс завершился с ошибкой: {status}"));
                    Some(state.status.info())
                }
            };

            if let Some(status) = maybe_status {
                let _ = event_tx.send(LspEvent::ProjectStatusChanged {
                    id: project_id,
                    status,
                });
            }
            break;
        }

        sleep(Duration::from_millis(500)).await;
    }
}

fn map_create_project_error(err: &anyhow::Error) -> ServiceError {
    map_project_persist_error(err, "Не удалось создать проект.")
}

fn map_update_project_error(err: &anyhow::Error) -> ServiceError {
    map_project_persist_error(err, "Не удалось обновить проект.")
}

fn map_project_persist_error(err: &anyhow::Error, fallback: &str) -> ServiceError {
    let text = err.to_string();
    if text.contains("UNIQUE constraint failed: projects.root_path") {
        return ServiceError::InvalidRequest(
            "Проект с таким корневым путем уже существует.".to_string(),
        );
    }

    let reason = err
        .chain()
        .last()
        .map(|cause| cause.to_string())
        .unwrap_or(text);
    let reason = normalize_project_persist_reason(&reason);
    ServiceError::InvalidRequest(format!("{fallback} Причина: {reason}"))
}

fn project_status_label(status: &ProjectStatus) -> &'static str {
    match status {
        ProjectStatus::Stopped => "остановлен",
        ProjectStatus::Starting => "запускается",
        ProjectStatus::WarmingUp => "прогревается",
        ProjectStatus::Ready => "готов",
        ProjectStatus::Error(_) => "ошибка",
    }
}

fn normalize_project_persist_reason(reason: &str) -> String {
    let reason = reason
        .trim()
        .trim_start_matches("error returned from database:")
        .trim()
        .trim_start_matches("failed to insert project:")
        .trim_start_matches("failed to update project:")
        .trim();

    if reason.contains("database is locked") {
        return "база данных занята другим процессом".to_string();
    }
    if reason.contains("unable to open database file") {
        return "не удалось открыть файл базы данных".to_string();
    }
    if reason.contains("disk I/O error") {
        return "ошибка ввода-вывода при работе с базой данных".to_string();
    }
    if reason.is_empty() {
        return "неизвестная ошибка".to_string();
    }

    reason.to_string()
}

fn normalize_slashes(path: &str) -> String {
    path.replace('\\', "/")
}

const DEFAULT_BSL_CONFIG: &str = r#"{
  "$schema": "https://1c-syntax.github.io/bsl-language-server/configuration/schema.json",
  "language": "ru",
  "diagnostics": {
    "computeTrigger": "onType",
    "minimumLSPDiagnosticLevel": "Warning",
    "parameters": {
      "MethodSize": false
    }
  }
}"#;

/// Returns the effective BSL config: user-provided or default.
/// If the user config is empty, returns the default config.
fn effective_bsl_config(user_config: &str) -> &str {
    if user_config.trim().is_empty() {
        DEFAULT_BSL_CONFIG
    } else {
        user_config
    }
}

/// Validates BSL config JSON. Empty config is OK (default is used at start).
/// Non-empty config must be valid JSON with `diagnostics.computeTrigger: "onType"`.
fn validate_bsl_config(config: &str) -> Result<(), ServiceError> {
    if config.trim().is_empty() {
        return Ok(()); // default will be used at start
    }
    let parsed: Value = serde_json::from_str(config.trim()).map_err(|err| {
        ServiceError::InvalidRequest(format!("Конфигурация BSL содержит невалидный JSON: {err}"))
    })?;
    let trigger = parsed
        .get("diagnostics")
        .and_then(|d| d.get("computeTrigger"))
        .and_then(|v| v.as_str());
    match trigger {
        Some("onType") => Ok(()),
        Some(other) => Err(ServiceError::InvalidRequest(format!(
            "diagnostics.computeTrigger = \"{other}\", требуется \"onType\". \
             Без этого диагностики не будут обновляться после редактирования файлов."
        ))),
        None => Err(ServiceError::InvalidRequest(
            "В конфигурации BSL отсутствует diagnostics.computeTrigger = \"onType\". \
             Без этого диагностики не будут обновляться после редактирования файлов."
                .to_string(),
        )),
    }
}
