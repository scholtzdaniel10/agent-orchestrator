use dioxus::prelude::*;
use legate_core::{ChangeSet, JobRecord, PlanStatus};

#[component]
pub fn JobsPanel(
    jobs: Vec<JobRecord>,
    plans: Option<Vec<PlanStatus>>,
    changes: Vec<ChangeSet>,
    is_repo: bool,
    selected_job_id: Option<String>,
    show_job: Option<(String, i64)>,
    on_select_job: Callback<Option<String>>,
    on_job: Callback<JobRecord>,
    on_view_change: Callback<String>,
    on_goal: Callback,
) -> Element {
    let _ = (
        plans,
        changes,
        is_repo,
        selected_job_id,
        show_job,
        on_select_job,
        on_job,
        on_view_change,
        on_goal,
    );
    rsx! { section { class: "jobs-panel", p { "{jobs.len()} jobs" } } }
}
