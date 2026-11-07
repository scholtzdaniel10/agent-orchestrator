use std::collections::HashMap;

use dioxus::prelude::*;
use legate_core::{ChangeSet, JobRecord, LeadMessage, PlanStatus, TerminalInfo};

use crate::logic::flow_events::FlowEvent;

#[component]
pub fn FlowView(
    jobs: Vec<JobRecord>,
    plans: Vec<PlanStatus>,
    lead: Vec<LeadMessage>,
    terminals: Vec<TerminalInfo>,
    changes: Vec<ChangeSet>,
    events: Vec<FlowEvent>,
    now: i64,
    starts: HashMap<String, i64>,
    flashes: HashMap<String, i64>,
    on_show_terminal: Callback<String>,
) -> Element {
    let _ = (
        plans,
        lead,
        terminals,
        changes,
        events,
        now,
        starts,
        flashes,
        on_show_terminal,
    );
    rsx! { section { class: "flow-view", p { "{jobs.len()} jobs in the flow" } } }
}
