use dioxus::prelude::*;
use legate_core::{LeadProposal, ProposedJob};

#[component]
pub fn LeadPlan(
    proposal: LeadProposal,
    on_run: Callback<Vec<ProposedJob>>,
    on_dismiss: Callback,
) -> Element {
    let summary = proposal.summary.clone();
    rsx! {
        section { class: "lead-plan",
            p { "{summary}" }
            button { class: "btn btn-primary", onclick: move |_| on_run.call(proposal.jobs.clone()), "Run" }
            button { class: "btn btn-quiet", onclick: move |_| on_dismiss.call(()), "Dismiss" }
        }
    }
}
