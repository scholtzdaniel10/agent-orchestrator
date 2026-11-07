use dioxus::prelude::*;
use legate_core::JobRecord;

#[component]
pub fn Routines(project: String, on_run: Callback<JobRecord>) -> Element {
    let _ = on_run;
    rsx! { section { class: "routines", p { "Routines for {project}" } } }
}
