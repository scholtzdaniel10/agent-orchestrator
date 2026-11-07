use dioxus::prelude::*;
use legate_core::{PlanStatus, ProjectEntry};

use crate::logic::theme::ThemeId;

#[component]
pub fn SettingsPage(
    plans: Vec<PlanStatus>,
    projects: Vec<ProjectEntry>,
    theme: ThemeId,
    on_theme: Callback<ThemeId>,
    on_remove_project: Callback<String>,
    on_open_setup: Callback,
    on_lead_changed: Callback,
    tab_order_custom: bool,
    on_reset_tabs: Callback,
    on_close: Callback,
) -> Element {
    let _ = (
        plans,
        projects,
        on_theme,
        on_remove_project,
        on_open_setup,
        on_lead_changed,
        tab_order_custom,
        on_reset_tabs,
    );
    rsx! {
        section { class: "settings-page",
            h1 { "Settings" }
            p { "{theme:?}" }
            button { class: "btn btn-quiet", onclick: move |_| on_close.call(()), "Close" }
        }
    }
}
