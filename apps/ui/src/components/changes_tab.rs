use dioxus::prelude::*;
use legate_core::{ChangeSet, JobRecord, ProviderId};

#[component]
pub fn ChangesTab(
    changes: Vec<ChangeSet>,
    jobs: Vec<JobRecord>,
    selected_id: Option<String>,
    on_select: Callback<Option<String>>,
    claude_available: bool,
    cursor_available: bool,
    opening: bool,
    on_open_here: Callback<(ProviderId, String)>,
) -> Element {
    let _ = (
        jobs,
        claude_available,
        cursor_available,
        opening,
        on_open_here,
    );
    let count = changes.len();
    let selected = selected_id.unwrap_or_default();
    rsx! {
        section { class: "changes-tab",
            p { "{count} changes" }
            button { class: "btn btn-quiet", onclick: move |_| on_select.call(None), "Clear {selected}" }
        }
    }
}
