use dioxus::prelude::*;

pub fn status_label(status: &str) -> &'static str {
    match status {
        "ready" => "Готов",
        "warming_up" => "Прогревается",
        "starting" => "Запускается",
        "error" => "Ошибка",
        "stopped" => "Остановлен",
        other if other.is_empty() => "Неизвестно",
        _ => "Неизвестно",
    }
}

#[component]
pub fn StatusBadge(status: String, error: Option<String>) -> Element {
    let class = match status.as_str() {
        "ready" => "badge badge-success badge-outline",
        "warming_up" => "badge badge-warning badge-outline",
        "starting" => "badge badge-warning badge-outline",
        "error" => "badge badge-error badge-outline",
        _ => "badge badge-ghost",
    };

    rsx! {
        div { class: "flex items-center gap-3",
            span { class, "{status_label(&status)}" }
            if let Some(error) = error {
                span { class: "text-xs text-error", "{error}" }
            }
        }
    }
}
