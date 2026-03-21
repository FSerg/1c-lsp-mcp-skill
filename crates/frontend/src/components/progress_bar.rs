use dioxus::prelude::*;

use crate::app::IndexingProgress;

#[component]
pub fn ProgressBar(progress: IndexingProgress) -> Element {
    let value = progress.percentage.unwrap_or(0);
    let text = progress
        .message
        .clone()
        .unwrap_or_else(|| "Нет активной задачи индексации".to_string());

    rsx! {
        div { class: "space-y-3",
            div { class: "flex items-center justify-between text-sm",
                span { "Индексация" }
                span { "{value}%" }
            }
            progress {
                class: "progress progress-neutral w-full",
                max: "100",
                value: "{value}",
            }
            p { class: "text-sm text-base-content/70", "{text}" }
        }
    }
}
