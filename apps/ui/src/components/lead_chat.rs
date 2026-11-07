use dioxus::prelude::*;
use legate_core::{JobRecord, LeadMessage, PlanStatus, ProposedJob, ProviderId};

#[component]
pub fn LeadChat(
    messages: Vec<LeadMessage>,
    jobs: Vec<JobRecord>,
    plans: Option<Vec<PlanStatus>>,
    mode: Option<ProviderId>,
    on_reset: Callback<Option<ProviderId>>,
    on_open_terminal: Callback<ProviderId>,
    on_show_job: Callback<String>,
    on_run_plan: Callback<(String, Vec<ProposedJob>)>,
    settings_nonce: i64,
) -> Element {
    let _ = (
        jobs,
        plans,
        mode,
        on_reset,
        on_open_terminal,
        on_show_job,
        on_run_plan,
        settings_nonce,
    );
    rsx! { section { class: "lead-chat", p { "{messages.len()} messages" } } }
}
