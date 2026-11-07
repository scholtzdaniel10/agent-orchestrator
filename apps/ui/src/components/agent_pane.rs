use dioxus::prelude::*;
use legate_core::JobRecord;

#[component]
pub fn AgentPane(job: JobRecord, on_take_over: Callback<String>, on_close: Callback) -> Element {
    let id = job.id.clone();
    rsx! {
        section { class: "agent-pane",
            p { "Agent {id}" }
            button { class: "btn btn-quiet", onclick: move |_| on_take_over.call(id.clone()), "Take over" }
            button { class: "btn btn-quiet", onclick: move |_| on_close.call(()), "Close" }
        }
    }
}
