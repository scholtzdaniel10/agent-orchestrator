use dioxus::prelude::*;

#[component]
pub fn FilesPanel(r#where: Option<String>, label: String) -> Element {
    let place = r#where.unwrap_or_else(|| "this folder".into());
    rsx! { section { class: "files-panel", p { "{label}" }, p { class: "hint", "{place}" } } }
}
