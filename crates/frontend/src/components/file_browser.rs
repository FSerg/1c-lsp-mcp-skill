use dioxus::prelude::*;

use crate::api::client::{self, BrowseEntry, BrowseRequest};

/// Режим выбора файлового браузера
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BrowseMode {
    /// Выбрать каталог
    Directory,
    /// Выбрать файл с заданным расширением
    File,
}

/// Свойства компонента файлового браузера
#[derive(Props, Clone, PartialEq)]
pub struct FileBrowserProps {
    /// Режим: выбор каталога или файла
    pub mode: BrowseMode,
    /// Фильтр расширения (для режима File, например "jar")
    #[props(default)]
    pub extension: Option<String>,
    /// Начальный путь (если пуст — домашний каталог)
    #[props(default)]
    pub initial_path: String,
    /// Колбэк: выбран путь
    pub on_select: EventHandler<String>,
    /// Колбэк: закрыть модалку
    pub on_close: EventHandler<()>,
}

#[component]
pub fn FileBrowser(props: FileBrowserProps) -> Element {
    let mut current_path = use_signal(String::new);
    let mut parent_path: Signal<Option<String>> = use_signal(|| None);
    let mut entries: Signal<Vec<BrowseEntry>> = use_signal(Vec::new);
    let mut loading = use_signal(|| true);
    let mut error: Signal<Option<String>> = use_signal(|| None);

    let mode = props.mode;
    let extension = props.extension.clone();
    let on_select = props.on_select.clone();
    let on_close = props.on_close.clone();

    // Начальная загрузка
    let initial_path = props.initial_path.clone();
    let ext_for_load = extension.clone();
    use_effect(move || {
        let initial = initial_path.clone();
        let ext = ext_for_load.clone();
        spawn(async move {
            load_directory(
                if initial.is_empty() {
                    None
                } else {
                    Some(initial)
                },
                mode,
                ext,
                &mut current_path,
                &mut parent_path,
                &mut entries,
                &mut loading,
                &mut error,
            )
            .await;
        });
    });

    let ext_for_nav = extension.clone();
    let navigate = move |path: String| {
        let ext = ext_for_nav.clone();
        spawn(async move {
            load_directory(
                Some(path),
                mode,
                ext,
                &mut current_path,
                &mut parent_path,
                &mut entries,
                &mut loading,
                &mut error,
            )
            .await;
        });
    };

    let ext_for_input = extension.clone();
    let navigate_to_input = move |path: String| {
        let ext = ext_for_input.clone();
        spawn(async move {
            load_directory(
                Some(path),
                mode,
                ext,
                &mut current_path,
                &mut parent_path,
                &mut entries,
                &mut loading,
                &mut error,
            )
            .await;
        });
    };

    rsx! {
        div { class: "modal modal-open",
            div { class: "modal-box max-w-2xl max-h-[80vh] flex flex-col",
                // Заголовок
                h3 { class: "font-bold text-lg mb-3",
                    match mode {
                        BrowseMode::Directory => "Выбор каталога",
                        BrowseMode::File => "Выбор файла",
                    }
                }

                // Строка текущего пути с возможностью ввода
                div { class: "flex gap-2 mb-3",
                    input {
                        class: "input input-bordered input-sm flex-1 font-mono text-sm",
                        value: current_path(),
                        onchange: move |evt| {
                            let path = evt.value();
                            navigate_to_input(path);
                        },
                    }
                    // Кнопка "Вверх"
                    if let Some(parent) = parent_path() {
                        {
                            let navigate = navigate.clone();
                            rsx! {
                                button {
                                    class: "btn btn-sm btn-outline",
                                    title: "На уровень выше",
                                    onclick: move |_| navigate(parent.clone()),
                                    "↑"
                                }
                            }
                        }
                    }
                }

                // Ошибка
                if let Some(err) = error() {
                    div { class: "alert alert-error text-sm mb-3",
                        "{err}"
                    }
                }

                // Список файлов
                div { class: "flex-1 overflow-y-auto border border-base-300 rounded-box",
                    if loading() {
                        div { class: "flex items-center justify-center p-8",
                            span { class: "loading loading-spinner loading-md" }
                        }
                    } else {
                        ul { class: "menu menu-sm p-0",
                            for entry in entries() {
                                {
                                    let entry_name = entry.name.clone();
                                    let is_dir = entry.is_dir;
                                    let full_path = format!(
                                        "{}{}{}", current_path(),
                                        if current_path().ends_with('/') || current_path().ends_with('\\') { "" } else { "/" },
                                        entry_name
                                    );
                                    let path_for_click = full_path.clone();
                                    let navigate = navigate.clone();
                                    let on_select = on_select.clone();

                                    rsx! {
                                        li {
                                            a {
                                                class: "flex items-center gap-2 rounded-none",
                                                onclick: move |_| {
                                                    if is_dir {
                                                        navigate(path_for_click.clone());
                                                    } else {
                                                        on_select.call(path_for_click.clone());
                                                    }
                                                },
                                                if is_dir {
                                                    // Иконка каталога
                                                    svg {
                                                        xmlns: "http://www.w3.org/2000/svg",
                                                        view_box: "0 0 20 20",
                                                        fill: "currentColor",
                                                        class: "size-4 text-warning shrink-0",
                                                        path {
                                                            d: "M3.75 3A1.75 1.75 0 0 0 2 4.75v3.26a3.235 3.235 0 0 1 1.75-.51h12.5c.644 0 1.245.188 1.75.51V6.75A1.75 1.75 0 0 0 16.25 5h-4.836a.25.25 0 0 1-.177-.073L9.823 3.513A1.75 1.75 0 0 0 8.586 3H3.75ZM3.75 9A1.75 1.75 0 0 0 2 10.75v4.5c0 .966.784 1.75 1.75 1.75h12.5A1.75 1.75 0 0 0 18 15.25v-4.5A1.75 1.75 0 0 0 16.25 9H3.75Z"
                                                        }
                                                    }
                                                } else {
                                                    // Иконка файла
                                                    svg {
                                                        xmlns: "http://www.w3.org/2000/svg",
                                                        view_box: "0 0 20 20",
                                                        fill: "currentColor",
                                                        class: "size-4 text-base-content/50 shrink-0",
                                                        path {
                                                            fill_rule: "evenodd",
                                                            d: "M4.5 2A1.5 1.5 0 0 0 3 3.5v13A1.5 1.5 0 0 0 4.5 18h11a1.5 1.5 0 0 0 1.5-1.5V7.621a1.5 1.5 0 0 0-.44-1.06l-4.12-4.122A1.5 1.5 0 0 0 11.378 2H4.5Z",
                                                            clip_rule: "evenodd",
                                                        }
                                                    }
                                                }
                                                span { class: "truncate", "{entry_name}" }
                                            }
                                        }
                                    }
                                }
                            }
                            if entries().is_empty() && !loading() {
                                li {
                                    span { class: "text-base-content/50 py-4 text-center",
                                        "Каталог пуст"
                                    }
                                }
                            }
                        }
                    }
                }

                // Кнопки
                div { class: "modal-action",
                    if mode == BrowseMode::Directory {
                        {
                            let on_select = on_select.clone();
                            let current = current_path();
                            rsx! {
                                button {
                                    class: "btn btn-primary",
                                    onclick: move |_| on_select.call(current.clone()),
                                    "Выбрать этот каталог"
                                }
                            }
                        }
                    }
                    button {
                        class: "btn",
                        onclick: move |_| on_close.call(()),
                        "Отмена"
                    }
                }
            }
            // Фон модалки
            div {
                class: "modal-backdrop",
                onclick: move |_| on_close.call(()),
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn load_directory(
    path: Option<String>,
    mode: BrowseMode,
    extension: Option<String>,
    current_path: &mut Signal<String>,
    parent_path: &mut Signal<Option<String>>,
    entries: &mut Signal<Vec<BrowseEntry>>,
    loading: &mut Signal<bool>,
    error: &mut Signal<Option<String>>,
) {
    loading.set(true);
    error.set(None);

    let request = BrowseRequest {
        path,
        show_files: mode == BrowseMode::File,
        extension,
    };

    match client::browse(&request).await {
        Ok(response) => {
            current_path.set(response.current);
            parent_path.set(response.parent);
            entries.set(response.entries);
        }
        Err(err) => {
            error.set(Some(err));
        }
    }

    loading.set(false);
}
