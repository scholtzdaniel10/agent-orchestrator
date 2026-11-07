use dioxus::prelude::*;
use legate_core::JobRecord;

#[component]
pub fn JobSteps(job: JobRecord) -> Element {
    rsx! { div { class: "job-steps", "{job.id}" } }
}
