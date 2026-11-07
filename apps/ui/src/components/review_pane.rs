use dioxus::prelude::*;
use legate_core::ChangeSet;

#[component]
pub fn ReviewPane(change: ChangeSet, on_close: Callback, on_done: Callback) -> Element {
    let id = change.id.clone();
    rsx! {
        section { class: "review-pane",
            p { "Review {id}" }
            button { class: "btn btn-quiet", onclick: move |_| on_close.call(()), "Close" }
            button { class: "btn btn-primary", onclick: move |_| on_done.call(()), "Done" }
        }
    }
}
