use dioxus::prelude::*;

#[component]
pub fn LogViewer(lines: Vec<String>) -> Element {
    rsx! {
        div { class: "rounded-box border border-base-300 bg-base-200 p-4 font-mono text-xs",
            if lines.is_empty() {
                p { class: "text-base-content/60", "Логов пока нет" }
            } else {
                div { class: "max-h-96 space-y-2 overflow-auto pr-2",
                    for line in lines {
                        pre { class: "whitespace-pre-wrap break-words", "{line}" }
                    }
                }
            }
        }
    }
}
