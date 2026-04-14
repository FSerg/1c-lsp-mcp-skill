use std::cell::Cell;
use std::rc::Rc;

use dioxus::prelude::*;
use gloo_timers::callback::Timeout;
use gloo_timers::future::TimeoutFuture;
use serde::{Deserialize, Serialize};
use wasm_bindgen_futures::spawn_local;

use crate::api::{client, sse};
use crate::components::file_browser::{BrowseMode, FileBrowser};
use crate::components::log_viewer::LogViewer;
use crate::components::progress_bar::ProgressBar;
use crate::components::status_badge::{status_label, StatusBadge};

const PROJECT_LOG_LINES_LIMIT: usize = 1000;
const SSE_DEBOUNCE_MS: u32 = 300;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Projects,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FlashKind {
    Success,
    Warning,
    Error,
}

#[derive(Clone, PartialEq, Eq)]
struct FlashMessage {
    id: u64,
    kind: FlashKind,
    text: String,
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexingProgress {
    pub percentage: Option<u32>,
    pub files_done: Option<u32>,
    pub files_total: Option<u32>,
    pub message: Option<String>,
    pub active: bool,
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectStatusInfo {
    pub status: String,
    pub error: Option<String>,
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSnapshot {
    pub id: String,
    pub name: String,
    pub root_path: String,
    pub project_root_path: String,
    pub jvm_args: String,
    #[serde(default)]
    pub bsl_config: String,
    #[serde(default)]
    pub debug: bool,
    pub created_at: String,
    pub updated_at: String,
    pub status: ProjectStatusInfo,
    pub progress: IndexingProgress,
}

const DEFAULT_BSL_CONFIG: &str = r#"{
  "$schema": "https://1c-syntax.github.io/bsl-language-server/configuration/schema.json",
  "language": "ru",
  "diagnostics": {
    "minimumLSPDiagnosticLevel": "Error",
    "parameters": {
      "MethodSize": false
    }
  }
}"#;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectPayload {
    pub name: String,
    pub root_path: String,
    pub project_root_path: String,
    pub jvm_args: String,
    #[serde(default)]
    pub bsl_config: String,
    #[serde(default)]
    pub debug: bool,
}

impl Default for ProjectPayload {
    fn default() -> Self {
        Self {
            name: String::new(),
            root_path: String::new(),
            project_root_path: String::new(),
            jvm_args: String::new(),
            bsl_config: DEFAULT_BSL_CONFIG.to_string(),
            debug: false,
        }
    }
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsPayload {
    pub jar_path: String,
    pub listen_host: String,
    pub http_port: u16,
    pub log_level: String,
    #[serde(default)]
    pub mcp_diagnostics_enabled: bool,
    #[serde(default)]
    pub mcp_diagnostics_port: u16,
    #[serde(default)]
    pub mcp_navigation_enabled: bool,
    #[serde(default)]
    pub mcp_navigation_port: u16,
    #[serde(default)]
    pub use_toon_format: bool,
    pub config_path: String,
    pub db_path: String,
    pub logs_dir: String,
    pub restart_required: bool,
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JavaCheckResult {
    pub found: bool,
    pub version: Option<String>,
    pub raw_output: String,
    pub ok: bool,
}

#[derive(Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
    pub code: String,
}

#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ServerEvent {
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

#[component]
pub fn App() -> Element {
    let mut active_tab = use_signal(Tab::default);
    let projects = use_signal(Vec::<ProjectSnapshot>::new);
    let mut selected_project_id = use_signal(|| None::<String>);
    let mut project_form = use_signal(ProjectPayload::default);
    let mut settings_form = use_signal(|| SettingsPayload {
        http_port: 4000,
        mcp_diagnostics_port: 9011,
        mcp_navigation_port: 9012,
        ..Default::default()
    });
    let mut project_logs = use_signal(Vec::<String>::new);
    let mut java_check = use_signal(|| None::<JavaCheckResult>);
    let toasts = use_signal(Vec::<FlashMessage>::new);
    let next_toast_id = use_signal(|| 1_u64);
    let mut creating_project = use_signal(|| false);
    let mut delete_confirm_open = use_signal(|| false);
    let mut initialized = use_signal(|| false);
    let mut browse_root_open = use_signal(|| false);
    let mut browse_project_root_open = use_signal(|| false);
    let mut browse_jar_open = use_signal(|| false);

    use_effect(move || {
        if initialized() {
            return;
        }

        initialized.set(true);

        spawn(async move {
            refresh_projects(
                projects,
                selected_project_id,
                project_form,
                creating_project,
                project_logs,
                toasts,
                next_toast_id,
            )
            .await;
            refresh_settings(settings_form, toasts, next_toast_id).await;
        });

        let mut projects_signal = projects;
        let selected_signal = selected_project_id;
        let form_signal = project_form;
        let creating_signal = creating_project;
        let mut logs_signal = project_logs;
        let toast_signal = toasts;
        let toast_id_signal = next_toast_id;
        let mut settings_signal = settings_form;

        // Debounced refresh: SSE events schedule a single refresh after SSE_DEBOUNCE_MS
        let pending_refresh: Rc<Cell<Option<Timeout>>> = Rc::new(Cell::new(None));

        let schedule_refresh = {
            let pending = pending_refresh.clone();
            move || {
                // Drop previous pending timer (cancels it)
                pending.set(None);
                let timeout = Timeout::new(SSE_DEBOUNCE_MS, move || {
                    spawn_local(async move {
                        silent_refresh_projects(
                            projects_signal,
                            selected_signal,
                            form_signal,
                            creating_signal,
                            logs_signal,
                        )
                        .await;
                    });
                });
                pending.set(Some(timeout));
            }
        };

        // Throttle flags: limit re-renders to max ~5/sec for frequent events
        let progress_busy: Rc<Cell<bool>> = Rc::new(Cell::new(false));

        if let Err(err) = sse::subscribe(move |event| match event {
            ServerEvent::ProjectStatusChanged { ref id, ref status } => {
                // Update status inline for immediate UI feedback
                projects_signal.with_mut(|list| {
                    if let Some(p) = list.iter_mut().find(|p| p.id == *id) {
                        p.status = status.clone();
                    }
                });
                if status.status == "ready" {
                    let project_name = projects_signal()
                        .iter()
                        .find(|p| p.id == *id)
                        .map(|p| p.name.clone())
                        .unwrap_or_default();
                    if !project_name.is_empty() {
                        push_success(
                            toast_signal,
                            toast_id_signal,
                            format!("Проект \"{project_name}\" запустился"),
                        );
                    }
                }
                schedule_refresh();
            }
            ServerEvent::ProjectsChanged => {
                schedule_refresh();
            }
            ServerEvent::IndexingProgress { id, progress } => {
                // Always show final state; throttle intermediate updates
                let force = !progress.active;
                if force || !progress_busy.get() {
                    projects_signal.with_mut(|list| {
                        if let Some(p) = list.iter_mut().find(|p| p.id == id) {
                            p.progress = progress;
                        }
                    });
                    if !progress_busy.get() {
                        progress_busy.set(true);
                        let busy = progress_busy.clone();
                        Timeout::new(200, move || busy.set(false)).forget();
                    }
                }
            }
            ServerEvent::DiagnosticsUpdated { .. } => {}
            ServerEvent::LogLine { id, line } => {
                if selected_signal().as_deref() == Some(id.as_str()) {
                    logs_signal.with_mut(|lines| {
                        lines.push(line);
                        if lines.len() > PROJECT_LOG_LINES_LIMIT {
                            let extra = lines.len() - PROJECT_LOG_LINES_LIMIT;
                            lines.drain(0..extra);
                        }
                    });
                }
            }
            ServerEvent::SettingsChanged { restart_required } => {
                settings_signal.with_mut(|settings| {
                    settings.restart_required = restart_required;
                });
            }
        }) {
            push_error(toasts, next_toast_id, err);
        }
    });

    use_effect(move || {
        let selected = selected_project_id();
        let creating = creating_project();
        if creating || selected.is_none() {
            project_logs.set(Vec::new());
            return;
        }

        spawn(async move {
            load_logs(selected_project_id, project_logs, toasts, next_toast_id).await;
        });
    });

    let selected_project = projects()
        .into_iter()
        .find(|project| Some(project.id.clone()) == selected_project_id());

    let save_project = {
        move |_| {
            let payload = project_form();
            let existing_id = selected_project_id();
            let mut creating_project = creating_project;
            let mut selected_project_id = selected_project_id;
            let mut project_form = project_form;
            let project_logs = project_logs;

            spawn(async move {
                let result = if creating_project() || existing_id.is_none() {
                    client::create_project(&payload).await
                } else {
                    client::update_project(existing_id.as_deref().unwrap(), &payload).await
                };

                match result {
                    Ok(project) => {
                        creating_project.set(false);
                        selected_project_id.set(Some(project.id.clone()));
                        project_form.set(ProjectPayload::from(&project));
                        push_success(
                            toasts,
                            next_toast_id,
                            format!("Проект \"{}\" сохранен", project.name),
                        );
                        refresh_projects(
                            projects,
                            selected_project_id,
                            project_form,
                            creating_project,
                            project_logs,
                            toasts,
                            next_toast_id,
                        )
                        .await;
                        load_logs(selected_project_id, project_logs, toasts, next_toast_id).await;
                    }
                    Err(err) => push_error(toasts, next_toast_id, err),
                }
            });
        }
    };

    let delete_project = {
        move |_| {
            if selected_project_id().is_none() {
                return;
            }
            delete_confirm_open.set(true);
        }
    };

    let confirm_delete_project = {
        move |_| {
            let Some(id) = selected_project_id() else {
                delete_confirm_open.set(false);
                return;
            };
            let project_name = project_form().name;
            let mut selected_project_id = selected_project_id;
            let mut project_form = project_form;
            let mut project_logs = project_logs;
            let mut creating_project = creating_project;
            let mut delete_confirm_open = delete_confirm_open;
            spawn(async move {
                match client::delete_project(&id).await {
                    Ok(()) => {
                        delete_confirm_open.set(false);
                        selected_project_id.set(None);
                        project_form.set(ProjectPayload::default());
                        project_logs.set(Vec::new());
                        creating_project.set(false);
                        push_success(
                            toasts,
                            next_toast_id,
                            format!("Проект \"{}\" удален", project_name),
                        );
                        refresh_projects(
                            projects,
                            selected_project_id,
                            project_form,
                            creating_project,
                            project_logs,
                            toasts,
                            next_toast_id,
                        )
                        .await;
                    }
                    Err(err) => push_error(toasts, next_toast_id, err),
                }
            });
        }
    };

    let start_project = {
        move |_| {
            let Some(id) = selected_project_id() else {
                return;
            };
            let project_name = project_form().name;
            spawn(async move {
                match client::start_project(&id).await {
                    Ok(_) => {
                        push_warning(
                            toasts,
                            next_toast_id,
                            format!("Проект \"{}\" запускается", project_name),
                        );
                        refresh_projects(
                            projects,
                            selected_project_id,
                            project_form,
                            creating_project,
                            project_logs,
                            toasts,
                            next_toast_id,
                        )
                        .await;
                    }
                    Err(err) => push_error(toasts, next_toast_id, err),
                }
            });
        }
    };

    let stop_project = {
        move |_| {
            let Some(id) = selected_project_id() else {
                return;
            };
            let project_name = project_form().name;
            spawn(async move {
                match client::stop_project(&id).await {
                    Ok(_) => {
                        push_success(
                            toasts,
                            next_toast_id,
                            format!("Проект \"{}\" остановлен", project_name),
                        );
                        refresh_projects(
                            projects,
                            selected_project_id,
                            project_form,
                            creating_project,
                            project_logs,
                            toasts,
                            next_toast_id,
                        )
                        .await;
                    }
                    Err(err) => push_error(toasts, next_toast_id, err),
                }
            });
        }
    };

    let save_settings = {
        move |_| {
            let payload = settings_form();
            spawn(async move {
                match client::update_settings(&payload).await {
                    Ok(saved) => {
                        settings_form.set(saved);
                        push_success(toasts, next_toast_id, "Настройки сохранены");
                    }
                    Err(err) => push_error(toasts, next_toast_id, err),
                }
            });
        }
    };

    let check_java = {
        move |_| {
            spawn(async move {
                match client::check_java().await {
                    Ok(result) => java_check.set(Some(result)),
                    Err(err) => push_error(toasts, next_toast_id, err),
                }
            });
        }
    };

    let choose_project_root = move |_| {
        browse_root_open.set(true);
    };

    let choose_project_root_path = move |_| {
        browse_project_root_open.set(true);
    };

    let choose_jar_path = move |_| {
        browse_jar_open.set(true);
    };

    let clear_logs = {
        move |_| {
            let Some(id) = selected_project_id() else {
                return;
            };
            let project_name = project_form().name;

            spawn(async move {
                match client::clear_project_logs(&id).await {
                    Ok(()) => {
                        project_logs.set(Vec::new());
                        push_success(
                            toasts,
                            next_toast_id,
                            format!("Логи проекта \"{}\" очищены", project_name),
                        );
                    }
                    Err(err) => push_error(toasts, next_toast_id, err),
                }
            });
        }
    };

    rsx! {
        document::Link { rel: "stylesheet", href: "/assets/app.css" }
        div { class: "min-h-screen bg-base-200 text-base-content",
            if delete_confirm_open() {
                div { class: "modal modal-open",
                    div { class: "modal-box",
                        h3 { class: "text-lg font-semibold", "Удалить проект?" }
                        p { class: "mt-3 text-sm text-base-content/70",
                            "Проект "
                            strong { "\"{project_form().name}\"" }
                            " будет удален из списка. Это действие нельзя отменить."
                        }
                        div { class: "modal-action",
                            button {
                                class: "btn btn-ghost",
                                onclick: move |_| delete_confirm_open.set(false),
                                "Нет"
                            }
                            button {
                                class: "btn btn-error",
                                onclick: confirm_delete_project,
                                "Да"
                            }
                        }
                    }
                    div {
                        class: "modal-backdrop",
                        onclick: move |_| delete_confirm_open.set(false),
                    }
                }
            }

            if !toasts().is_empty() {
                div { class: "toast toast-top toast-end z-[100]",
                    for toast in toasts() {
                        div {
                            key: "{toast.id}",
                            class: match toast.kind {
                                FlashKind::Error => "alert alert-error w-full max-w-md shadow-lg",
                                FlashKind::Warning => "alert alert-warning w-full max-w-md shadow-lg",
                                FlashKind::Success => "alert alert-success w-full max-w-md shadow-lg",
                            },
                            span { class: "flex-1", "{toast.text}" }
                            button {
                                class: "btn btn-xs btn-ghost",
                                onclick: {
                                    let toast_id = toast.id;
                                    move |_| dismiss_toast(toasts, toast_id)
                                },
                                "✕"
                            }
                        }
                    }
                }
            }

            header { class: "border-b border-base-300 bg-base-100",
                div { class: "mx-auto flex max-w-[1400px] items-center justify-between px-6 py-5",
                    div {
                        h1 { class: "text-2xl font-semibold tracking-tight", "1S LSP Skill" }
                        p { class: "text-sm text-base-content/60", "Управление несколькими bsl-language-server" }
                    }
                }
            }

            main { class: "mx-auto flex max-w-[1400px] flex-col gap-5 px-6 py-6",
                div { role: "tablist", class: "tabs tabs-box border border-base-300 bg-base-100 p-1",
                    button {
                        class: tab_class(active_tab() == Tab::Projects),
                        onclick: move |_| active_tab.set(Tab::Projects),
                        "Проекты"
                    }
                    button {
                        class: tab_class(active_tab() == Tab::Settings),
                        onclick: move |_| active_tab.set(Tab::Settings),
                        "Настройки"
                    }
                }

                if active_tab() == Tab::Projects {
                    div { class: "grid gap-5 xl:grid-cols-[340px_minmax(0,1fr)]",
                        aside { class: "card border border-base-300 bg-base-100",
                            div { class: "card-body gap-4",
                                div { class: "flex items-center justify-between",
                                    h2 { class: "card-title", "Проекты" }
                                    button {
                                        class: "btn btn-sm btn-primary",
                                        onclick: move |_| {
                                            creating_project.set(true);
                                            selected_project_id.set(None);
                                            project_form.set(ProjectPayload::default());
                                            project_logs.set(Vec::new());
                                        },
                                        "Добавить проект"
                                    }
                                }

                                div { class: "space-y-2",
                                    if projects().is_empty() {
                                        div { class: "rounded-box border border-dashed border-base-300 px-4 py-6 text-sm text-base-content/60",
                                            "Проектов пока нет."
                                        }
                                    } else {
                                        for project in projects() {
                                            div {
                                                key: "{project.id}",
                                                class: if Some(project.id.clone()) == selected_project_id() && !creating_project() {
                                                    "rounded-btn border border-base-300 bg-base-200 cursor-pointer"
                                                } else {
                                                    "rounded-btn border border-base-300 hover:bg-base-200 cursor-pointer"
                                                },
                                                onclick: {
                                                    let project = project.clone();
                                                    move |_| {
                                                        creating_project.set(false);
                                                        selected_project_id.set(Some(project.id.clone()));
                                                        project_form.set(ProjectPayload::from(&project));
                                                    }
                                                },
                                                div { class: "flex items-center justify-between px-4 py-2",
                                                    span { class: "truncate text-left text-sm font-medium", "{project.name}" }
                                                    span { class: "text-xs uppercase text-base-content/50 shrink-0 ml-2", "{status_label(&project.status.status)}" }
                                                }
                                                div { class: "px-4 pb-2",
                                                    progress {
                                                        class: if project.progress.active {
                                                            "progress progress-primary w-full h-1.5"
                                                        } else {
                                                            "progress w-full h-1.5"
                                                        },
                                                        value: project.progress.percentage.unwrap_or(0) as f64,
                                                        max: 100.0,
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        section { class: "card border border-base-300 bg-base-100",
                            div { class: "card-body gap-6",
                                if creating_project() || selected_project.is_some() {
                                    div { class: "flex items-start justify-between gap-3",
                                        h2 { class: "card-title text-2xl",
                                            if creating_project() { "Новый проект" } else { "{selected_project.as_ref().map(|project| project.name.clone()).unwrap_or_default()}" }
                                        }
                                        if let Some(project) = selected_project.as_ref() {
                                            StatusBadge {
                                                status: project.status.status.clone(),
                                                error: project.status.error.clone(),
                                            }
                                        } else {
                                            span { class: "badge badge-ghost", "черновик" }
                                        }
                                    }

                                    div { class: "grid gap-6 lg:grid-cols-[minmax(0,1fr)_320px]",
                                        div { class: "space-y-5",

                                            div { class: "grid gap-4 md:grid-cols-2",
                                                label { class: "form-control",
                                                    span { class: "label-text mb-2", "Имя проекта" }
                                                    input {
                                                        class: "input input-bordered w-full",
                                                        value: project_form().name,
                                                        oninput: move |evt| project_form.with_mut(|form| form.name = evt.value()),
                                                    }
                                                }
                                                label { class: "form-control",
                                                    span { class: "label-text mb-2", "Аргументы JVM" }
                                                    input {
                                                        class: "input input-bordered w-full",
                                                        value: project_form().jvm_args,
                                                        oninput: move |evt| project_form.with_mut(|form| form.jvm_args = evt.value()),
                                                    }
                                                }
                                            }

                                            div { class: "form-control gap-2",
                                                span { class: "label-text mb-2", "Корневой путь BSL" }
                                                div { class: "grid gap-3 lg:grid-cols-[minmax(0,1fr)_56px]",
                                                    input {
                                                        class: "input input-bordered w-full",
                                                        value: project_form().root_path,
                                                        oninput: move |evt| project_form.with_mut(|form| form.root_path = evt.value()),
                                                    }
                                                    button {
                                                        class: "btn btn-outline btn-square w-14 min-w-14",
                                                        r#type: "button",
                                                        onclick: choose_project_root,
                                                        title: "Выбрать папку",
                                                        aria_label: "Выбрать папку",
                                                        svg {
                                                            xmlns: "http://www.w3.org/2000/svg",
                                                            view_box: "0 0 24 24",
                                                            fill: "currentColor",
                                                            class: "size-5",
                                                            path {
                                                                d: "M19.906 9c.382 0 .749.057 1.094.162V9a3 3 0 0 0-3-3h-3.879a.75.75 0 0 1-.53-.22L11.47 3.66A2.25 2.25 0 0 0 9.879 3H6a3 3 0 0 0-3 3v3.162A3.756 3.756 0 0 1 4.094 9h15.812ZM4.094 10.5a2.25 2.25 0 0 0-2.227 2.568l.857 6A2.25 2.25 0 0 0 4.951 21H19.05a2.25 2.25 0 0 0 2.227-1.932l.857-6a2.25 2.25 0 0 0-2.227-2.568H4.094Z"
                                                            }
                                                        }
                                                    }
                                                }
                                            }

                                            div { class: "form-control gap-2",
                                                span { class: "label-text mb-2", "Корневой путь всего проекта" }
                                                p { class: "text-xs text-base-content/60 -mt-1 mb-1",
                                                    "Папка, из которой работают LLM-агенты. Пути файлов в запросах разрешаются относительно неё."
                                                }
                                                div { class: "grid gap-3 lg:grid-cols-[minmax(0,1fr)_56px]",
                                                    input {
                                                        class: "input input-bordered w-full",
                                                        value: project_form().project_root_path,
                                                        oninput: move |evt| project_form.with_mut(|form| form.project_root_path = evt.value()),
                                                    }
                                                    button {
                                                        class: "btn btn-outline btn-square w-14 min-w-14",
                                                        r#type: "button",
                                                        onclick: choose_project_root_path,
                                                        title: "Выбрать папку",
                                                        aria_label: "Выбрать папку",
                                                        svg {
                                                            xmlns: "http://www.w3.org/2000/svg",
                                                            view_box: "0 0 24 24",
                                                            fill: "currentColor",
                                                            class: "size-5",
                                                            path {
                                                                d: "M19.906 9c.382 0 .749.057 1.094.162V9a3 3 0 0 0-3-3h-3.879a.75.75 0 0 1-.53-.22L11.47 3.66A2.25 2.25 0 0 0 9.879 3H6a3 3 0 0 0-3 3v3.162A3.756 3.756 0 0 1 4.094 9h15.812ZM4.094 10.5a2.25 2.25 0 0 0-2.227 2.568l.857 6A2.25 2.25 0 0 0 4.951 21H19.05a2.25 2.25 0 0 0 2.227-1.932l.857-6a2.25 2.25 0 0 0-2.227-2.568H4.094Z"
                                                            }
                                                        }
                                                    }
                                                }
                                            }

                                            div { class: "rounded-box border border-base-300 p-4",
                                                h3 { class: "mb-3 font-medium", "Для агентов" }
                                                if let Some(project) = selected_project.as_ref() {
                                                    pre { class: "whitespace-pre-wrap font-mono text-xs select-all",
                                                        ".env: PROJECT_ID={project.id}\n\nmcp.json: \"x-project-id\": \"{project.id}\""
                                                    }
                                                } else {
                                                    p { class: "text-sm text-base-content/60", "Сохраните проект для получения идентификатора" }
                                                }
                                            }

                                        }

                                        div { class: "space-y-4",
                                            div { class: "rounded-box border border-base-300 p-4",
                                                h3 { class: "mb-3 font-medium", "Жизненный цикл" }
                                                div { class: "grid gap-2",
                                                    button { class: "btn btn-sm btn-primary", onclick: save_project, "Сохранить" }
                                                    button {
                                                        class: "btn btn-sm btn-outline",
                                                        disabled: selected_project.is_none() || selected_project.as_ref().map(|project| project.status.status != "stopped").unwrap_or(true),
                                                        onclick: start_project,
                                                        "Запустить"
                                                    }
                                                    button {
                                                        class: "btn btn-sm btn-outline",
                                                        disabled: selected_project.is_none() || selected_project.as_ref().map(|project| project.status.status == "stopped").unwrap_or(true),
                                                        onclick: stop_project,
                                                        "Остановить"
                                                    }
                                                    button {
                                                        class: "btn btn-sm btn-outline btn-error",
                                                        disabled: creating_project() || selected_project.is_none(),
                                                        onclick: delete_project,
                                                        "Удалить"
                                                    }
                                                }
                                            }

                                            label { class: "flex items-center gap-2 cursor-pointer select-none",
                                                input {
                                                    r#type: "checkbox",
                                                    class: "toggle toggle-sm",
                                                    checked: project_form().debug,
                                                    onchange: move |evt: Event<FormData>| project_form.with_mut(|form| form.debug = evt.checked()),
                                                }
                                                span { class: "label-text", "Отладка (больше логов)" }
                                            }
                                        }
                                    }

                                    label { class: "form-control gap-2",
                                        span { class: "label-text mb-2", "Конфигурация BSL Language Server" }
                                        p { class: "text-xs text-base-content/60 -mt-1 mb-1",
                                            "JSON-конфигурация, передаётся через --configuration при запуске. Оставьте пустым для использования .bsl-language-server.json из корня BSL."
                                        }
                                        textarea {
                                            class: "textarea textarea-bordered w-full font-mono text-xs leading-relaxed",
                                            rows: 6,
                                            spellcheck: false,
                                            value: project_form().bsl_config,
                                            oninput: move |evt| project_form.with_mut(|form| form.bsl_config = evt.value()),
                                        }
                                    }

                                    div { class: "rounded-box border border-base-300 p-4",
                                        ProgressBar {
                                            progress: selected_project
                                                .as_ref()
                                                .map(|project| project.progress.clone())
                                                .unwrap_or_default(),
                                        }
                                    }

                                    div { class: "rounded-box border border-base-300 p-4",
                                        div { class: "mb-3 flex items-center justify-between",
                                            h3 { class: "font-medium", "Последние логи" }
                                            button {
                                                class: "btn btn-sm btn-ghost",
                                                disabled: selected_project_id().is_none(),
                                                onclick: clear_logs,
                                                "Очистить"
                                            }
                                        }
                                        LogViewer { lines: project_logs() }
                                    }
                                } else {
                                    div { class: "hero rounded-box border border-dashed border-base-300 bg-base-200/40 py-16",
                                        div { class: "hero-content text-center",
                                            div { class: "max-w-xl space-y-3",
                                                h2 { class: "text-3xl font-semibold", "Проект не выбран" }
                                                p { class: "text-base-content/60",
                                                    "Выберите проект в списке или создайте новый. Панель деталей остается на этой странице в двухпанельной компоновке."
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    section { class: "card border border-base-300 bg-base-100",
                        div { class: "card-body gap-5",
                            div {
                                h2 { class: "card-title text-2xl", "Настройки" }
                                p { class: "text-sm text-base-content/60",
                                    "Хост и порт сохраняются сразу, но применяются только после перезапуска серверного приложения."
                                }
                            }

                            if settings_form().restart_required {
                                div { class: "alert alert-warning",
                                    span { "Для применения текущего хоста или порта нужен перезапуск." }
                                }
                            }

                            div { class: "space-y-4",
                                div { class: "form-control gap-2",
                                    span { class: "label-text mb-2", "JAR-файл BSL Language Server" }
                                    div { class: "grid gap-3 lg:grid-cols-[minmax(0,1fr)_96px_180px]",
                                        input {
                                            class: "input input-bordered w-full",
                                            value: settings_form().jar_path,
                                            oninput: move |evt| settings_form.with_mut(|form| form.jar_path = evt.value()),
                                        }
                                        button {
                                            class: "btn btn-outline gap-2 px-3",
                                            r#type: "button",
                                            onclick: choose_jar_path,
                                            title: "Выбрать JAR-файл",
                                            aria_label: "Выбрать JAR-файл",
                                            svg {
                                                xmlns: "http://www.w3.org/2000/svg",
                                                view_box: "0 0 24 24",
                                                fill: "currentColor",
                                                class: "size-5",
                                                path {
                                                    d: "M11.625 16.5a1.875 1.875 0 1 0 0-3.75 1.875 1.875 0 0 0 0 3.75Z"
                                                }
                                                path {
                                                    fill_rule: "evenodd",
                                                    clip_rule: "evenodd",
                                                    d: "M5.625 1.5H9a3.75 3.75 0 0 1 3.75 3.75v1.875c0 1.036.84 1.875 1.875 1.875H16.5a3.75 3.75 0 0 1 3.75 3.75v7.875c0 1.035-.84 1.875-1.875 1.875H5.625a1.875 1.875 0 0 1-1.875-1.875V3.375c0-1.036.84-1.875 1.875-1.875Zm6 16.5c.66 0 1.277-.19 1.797-.518l1.048 1.048a.75.75 0 0 0 1.06-1.06l-1.047-1.048A3.375 3.375 0 1 0 11.625 18Z"
                                                }
                                                path {
                                                    d: "M14.25 5.25a5.23 5.23 0 0 0-1.279-3.434 9.768 9.768 0 0 1 6.963 6.963A5.23 5.23 0 0 0 16.5 7.5h-1.875a.375.375 0 0 1-.375-.375V5.25Z"
                                                }
                                            }
                                            span { class: "text-xs font-semibold uppercase tracking-wide", "jar" }
                                        }
                                        button {
                                            class: "btn btn-outline w-full",
                                            onclick: check_java,
                                            "Проверить Java"
                                        }
                                    }
                                }

                                div { class: "grid gap-4 lg:grid-cols-[minmax(0,1fr)_140px_minmax(0,1fr)]",
                                    label { class: "form-control",
                                    span { class: "label-text mb-2", "Хост прослушивания" }
                                    input {
                                        class: "input input-bordered w-full",
                                        value: settings_form().listen_host,
                                        oninput: move |evt| settings_form.with_mut(|form| form.listen_host = evt.value()),
                                    }
                                    }

                                    label { class: "form-control",
                                    span { class: "label-text mb-2", "HTTP-порт" }
                                    input {
                                        class: "input input-bordered w-full",
                                        r#type: "number",
                                        value: settings_form().http_port,
                                        oninput: move |evt| {
                                            if let Ok(port) = evt.value().parse() {
                                                settings_form.with_mut(|form| form.http_port = port);
                                            }
                                        },
                                    }
                                    }

                                    label { class: "form-control",
                                    span { class: "label-text mb-2", "Уровень логирования" }
                                    input {
                                        class: "input input-bordered w-full",
                                        value: settings_form().log_level,
                                        oninput: move |evt| settings_form.with_mut(|form| form.log_level = evt.value()),
                                    }
                                    }
                                }
                            }

                            div { class: "space-y-4",
                                h3 { class: "text-lg font-semibold", "MCP серверы" }
                                p { class: "text-sm text-base-content/60",
                                    "Model Context Protocol серверы для интеграции с AI-помощниками. Изменение настроек MCP требует перезапуска."
                                }

                                div { class: "grid gap-4 lg:grid-cols-2",
                                    div { class: "rounded-box border border-base-300 p-4 space-y-3",
                                        div { class: "flex items-center justify-between",
                                            span { class: "font-medium", "Диагностика" }
                                            input {
                                                r#type: "checkbox",
                                                class: "toggle toggle-sm",
                                                checked: settings_form().mcp_diagnostics_enabled,
                                                onchange: move |evt: Event<FormData>| settings_form.with_mut(|form| form.mcp_diagnostics_enabled = evt.checked()),
                                            }
                                        }
                                        p { class: "text-xs text-base-content/60", "Проверка синтаксиса BSL-файлов" }
                                        label { class: "form-control",
                                            span { class: "label-text mb-1", "Порт" }
                                            input {
                                                class: "input input-bordered input-sm w-full",
                                                r#type: "number",
                                                value: settings_form().mcp_diagnostics_port,
                                                oninput: move |evt| {
                                                    if let Ok(port) = evt.value().parse() {
                                                        settings_form.with_mut(|form| form.mcp_diagnostics_port = port);
                                                    }
                                                },
                                            }
                                        }
                                    }

                                    div { class: "rounded-box border border-base-300 p-4 space-y-3",
                                        div { class: "flex items-center justify-between",
                                            span { class: "font-medium", "Навигация" }
                                            input {
                                                r#type: "checkbox",
                                                class: "toggle toggle-sm",
                                                checked: settings_form().mcp_navigation_enabled,
                                                onchange: move |evt: Event<FormData>| settings_form.with_mut(|form| form.mcp_navigation_enabled = evt.checked()),
                                            }
                                        }
                                        p { class: "text-xs text-base-content/60", "Символы, определения, ссылки" }
                                        label { class: "form-control",
                                            span { class: "label-text mb-1", "Порт" }
                                            input {
                                                class: "input input-bordered input-sm w-full",
                                                r#type: "number",
                                                value: settings_form().mcp_navigation_port,
                                                oninput: move |evt| {
                                                    if let Ok(port) = evt.value().parse() {
                                                        settings_form.with_mut(|form| form.mcp_navigation_port = port);
                                                    }
                                                },
                                            }
                                        }
                                    }
                                }
                            }

                            div { class: "space-y-4",
                                h3 { class: "text-lg font-semibold", "Формат ответов для LLM" }
                                p { class: "text-sm text-base-content/60",
                                    "Применяется к ответам MCP и CLI. Изменение вступает в силу немедленно, без перезапуска."
                                }

                                div { class: "rounded-box border border-base-300 p-4 space-y-3",
                                    div { class: "flex items-center justify-between",
                                        span { class: "font-medium", "Использовать TOON-формат" }
                                        input {
                                            r#type: "checkbox",
                                            class: "toggle toggle-sm",
                                            checked: settings_form().use_toon_format,
                                            onchange: move |evt: Event<FormData>| settings_form.with_mut(|form| form.use_toon_format = evt.checked()),
                                        }
                                    }
                                    p { class: "text-xs text-base-content/60",
                                        "Компактный формат вместо JSON. Экономит ~30–50% токенов для LLM-агентов. HTTP API продолжает отдавать JSON."
                                    }
                                }
                            }

                            div { class: "space-y-4",
                                ReadonlyField { label: "Путь к файлу настроек", value: settings_form().config_path.clone() }
                                ReadonlyField { label: "Путь к базе данных", value: settings_form().db_path.clone() }
                                ReadonlyField { label: "Каталог логов", value: settings_form().logs_dir.clone() }
                            }

                            div { class: "flex flex-wrap gap-3",
                                button { class: "btn btn-primary", onclick: save_settings, "Сохранить" }
                            }

                            if let Some(result) = java_check() {
                                div { class: if result.ok { "alert alert-success items-start" } else { "alert alert-warning items-start" },
                                    div { class: "space-y-2",
                                        p { class: "font-medium",
                                            if result.ok { "Проверка Java прошла успешно" } else { "Проверка Java требует внимания" }
                                        }
                                        p {
                                            "Найдена: "
                                            if result.found { "да" } else { "нет" }
                                        }
                                        p {
                                            "Версия: "
                                            {result.version.clone().unwrap_or_else(|| "неизвестно".to_string())}
                                        }
                                        pre { class: "max-h-56 overflow-auto whitespace-pre-wrap text-xs", "{result.raw_output}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Модальные окна файлового браузера
        if browse_root_open() {
            FileBrowser {
                mode: BrowseMode::Directory,
                initial_path: project_form().root_path,
                on_select: move |path: String| {
                    project_form.with_mut(|form| form.root_path = path);
                    browse_root_open.set(false);
                },
                on_close: move |_| browse_root_open.set(false),
            }
        }

        if browse_project_root_open() {
            FileBrowser {
                mode: BrowseMode::Directory,
                initial_path: project_form().project_root_path,
                on_select: move |path: String| {
                    project_form.with_mut(|form| form.project_root_path = path);
                    browse_project_root_open.set(false);
                },
                on_close: move |_| browse_project_root_open.set(false),
            }
        }

        if browse_jar_open() {
            FileBrowser {
                mode: BrowseMode::File,
                extension: Some("jar".to_string()),
                initial_path: settings_form().jar_path,
                on_select: move |path: String| {
                    settings_form.with_mut(|form| form.jar_path = path);
                    browse_jar_open.set(false);
                },
                on_close: move |_| browse_jar_open.set(false),
            }
        }
    }
}

#[component]
fn ReadonlyField(label: &'static str, value: String) -> Element {
    rsx! {
        label { class: "form-control",
            span { class: "label-text mb-2", "{label}" }
            input {
                class: "input input-bordered w-full font-mono text-xs",
                value,
                readonly: true,
            }
        }
    }
}

async fn refresh_projects(
    mut projects: Signal<Vec<ProjectSnapshot>>,
    mut selected_project_id: Signal<Option<String>>,
    mut project_form: Signal<ProjectPayload>,
    creating_project: Signal<bool>,
    mut project_logs: Signal<Vec<String>>,
    toasts: Signal<Vec<FlashMessage>>,
    next_toast_id: Signal<u64>,
) {
    match client::get_projects().await {
        Ok(next_projects) => {
            let current_selected = selected_project_id();
            let selected_exists = current_selected
                .as_ref()
                .map(|id| next_projects.iter().any(|project| &project.id == id))
                .unwrap_or(false);

            if next_projects.is_empty() && !creating_project() {
                selected_project_id.set(None);
                project_logs.set(Vec::new());
            } else if !creating_project() && !selected_exists {
                if let Some(first) = next_projects.first() {
                    selected_project_id.set(Some(first.id.clone()));
                    project_form.set(ProjectPayload::from(first));
                }
            }

            projects.set(next_projects);
        }
        Err(err) => push_error(toasts, next_toast_id, err),
    }
}

async fn silent_refresh_projects(
    mut projects: Signal<Vec<ProjectSnapshot>>,
    mut selected_project_id: Signal<Option<String>>,
    mut project_form: Signal<ProjectPayload>,
    creating_project: Signal<bool>,
    mut project_logs: Signal<Vec<String>>,
) {
    let Ok(next_projects) = client::get_projects().await else {
        return;
    };

    let current_selected = selected_project_id();
    let selected_exists = current_selected
        .as_ref()
        .map(|id| next_projects.iter().any(|project| &project.id == id))
        .unwrap_or(false);

    if next_projects.is_empty() && !creating_project() {
        selected_project_id.set(None);
        project_logs.set(Vec::new());
    } else if !creating_project() && !selected_exists {
        if let Some(first) = next_projects.first() {
            selected_project_id.set(Some(first.id.clone()));
            project_form.set(ProjectPayload::from(first));
        }
    }

    projects.set(next_projects);
}

async fn refresh_settings(
    mut settings_form: Signal<SettingsPayload>,
    toasts: Signal<Vec<FlashMessage>>,
    next_toast_id: Signal<u64>,
) {
    match client::get_settings().await {
        Ok(settings) => settings_form.set(settings),
        Err(err) => push_error(toasts, next_toast_id, err),
    }
}

async fn load_logs(
    selected_project_id: Signal<Option<String>>,
    mut project_logs: Signal<Vec<String>>,
    toasts: Signal<Vec<FlashMessage>>,
    next_toast_id: Signal<u64>,
) {
    let Some(id) = selected_project_id() else {
        project_logs.set(Vec::new());
        return;
    };

    match client::get_project_logs(&id, PROJECT_LOG_LINES_LIMIT).await {
        Ok(lines) => project_logs.set(lines),
        Err(err) => push_error(toasts, next_toast_id, err),
    }
}

fn tab_class(active: bool) -> &'static str {
    if active {
        "tab rounded-box border border-base-content bg-base-content text-base-100 [--tab-border-color:transparent]"
    } else {
        "tab rounded-box border border-transparent bg-base-100 text-base-content/60 hover:text-base-content [--tab-border-color:transparent]"
    }
}

fn push_success(
    toasts: Signal<Vec<FlashMessage>>,
    next_toast_id: Signal<u64>,
    text: impl Into<String>,
) {
    push_toast(toasts, next_toast_id, FlashKind::Success, text.into());
}

fn push_warning(
    toasts: Signal<Vec<FlashMessage>>,
    next_toast_id: Signal<u64>,
    text: impl Into<String>,
) {
    push_toast(toasts, next_toast_id, FlashKind::Warning, text.into());
}

fn push_error(
    toasts: Signal<Vec<FlashMessage>>,
    next_toast_id: Signal<u64>,
    text: impl Into<String>,
) {
    push_toast(toasts, next_toast_id, FlashKind::Error, text.into());
}

fn push_toast(
    mut toasts: Signal<Vec<FlashMessage>>,
    mut next_toast_id: Signal<u64>,
    kind: FlashKind,
    text: String,
) {
    let id = next_toast_id();
    next_toast_id.set(id + 1);
    toasts.with_mut(|items| items.push(FlashMessage { id, kind, text }));

    spawn(async move {
        TimeoutFuture::new(4000).await;
        dismiss_toast(toasts, id);
    });
}

fn dismiss_toast(mut toasts: Signal<Vec<FlashMessage>>, id: u64) {
    toasts.with_mut(|items| items.retain(|toast| toast.id != id));
}

impl From<&ProjectSnapshot> for ProjectPayload {
    fn from(value: &ProjectSnapshot) -> Self {
        Self {
            name: value.name.clone(),
            root_path: value.root_path.clone(),
            project_root_path: value.project_root_path.clone(),
            jvm_args: value.jvm_args.clone(),
            bsl_config: value.bsl_config.clone(),
            debug: value.debug,
        }
    }
}
