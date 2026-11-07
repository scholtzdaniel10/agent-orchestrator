use dioxus::prelude::*;

#[component]
pub fn HistoryPanel(r#where: Option<String>, label: String, refresh_key: String) -> Element {
    let place = r#where.unwrap_or_else(|| "this folder".into());
    rsx! { section { class: "history-panel", p { "{label}" }, p { class: "hint", "{place} · {refresh_key}" } } }
}
