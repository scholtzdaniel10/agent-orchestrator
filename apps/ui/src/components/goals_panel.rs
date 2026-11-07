use dioxus::prelude::*;
use legate_core::JobRecord;

#[component]
pub fn GoalsPanel(
    jobs: Vec<JobRecord>,
    on_view_change: Callback<String>,
    on_show_job: Callback<String>,
) -> Element {
    let _ = (on_view_change, on_show_job);
    rsx! { section { class: "goals-panel", p { "{jobs.len()} jobs beside the goals" } } }
}
