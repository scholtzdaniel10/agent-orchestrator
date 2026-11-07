use dioxus::prelude::*;

#[component]
pub fn ErrorBoundary(class_name: Option<String>, children: Element) -> Element {
    let class = class_name.unwrap_or_default();
    rsx! { div { class: "{class}", {children} } }
}
