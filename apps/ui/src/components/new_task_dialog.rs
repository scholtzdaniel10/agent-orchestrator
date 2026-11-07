use dioxus::prelude::*;
use legate_core::ProviderId;

#[component]
pub fn NewTaskDialog(
    open: bool,
    on_close: Callback,
    is_repo: bool,
    claude_available: bool,
    cursor_available: bool,
    on_start_terminal: Callback<(String, ProviderId, i64)>,
    on_created: Callback<String>,
) -> Element {
    let _ = (
        is_repo,
        claude_available,
        cursor_available,
        on_start_terminal,
        on_created,
    );
    if !open {
        return rsx! {};
    }
    rsx! {
        div { class: "new-task", role: "dialog",
            p { "New task" }
            button { class: "btn btn-quiet", onclick: move |_| on_close.call(()), "Close" }
        }
    }
}
