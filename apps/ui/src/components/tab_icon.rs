use dioxus::prelude::*;

use crate::logic::layout::DrawerTab;

#[component]
pub fn TabIcon(tab: DrawerTab) -> Element {
    rsx! { span { class: "tab-icon", aria_hidden: "true", "data-tab": tab.as_str() } }
}
