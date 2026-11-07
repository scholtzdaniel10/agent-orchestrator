use dioxus::prelude::*;

#[component]
pub fn RichText(text: String) -> Element {
    rsx! { div { class: "rich-text", "{text}" } }
}
