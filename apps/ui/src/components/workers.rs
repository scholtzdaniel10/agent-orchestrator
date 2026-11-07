use dioxus::prelude::*;
use legate_core::{ChangeSet, JobRecord, TerminalInfo};

use crate::logic::layout::PaneLayoutId;
use crate::logic::terminal_bus::TerminalBus;

#[component]
pub fn Workers(
    terminals: Vec<TerminalInfo>,
    initial_terminal_ids: Option<Vec<String>>,
    changes: Vec<ChangeSet>,
    selected_worktree: Option<String>,
    selected_worktrees: Vec<Option<String>>,
    place_terminal: Option<(String, i64)>,
    on_select_worktree: Callback<Option<String>>,
    project: String,
    bus: TerminalBus,
    pane_layout: PaneLayoutId,
    on_pane_layout: Callback<PaneLayoutId>,
    on_terminal: Callback<TerminalInfo>,
    agent_jobs: Vec<JobRecord>,
    focus_agent: Option<(String, i64)>,
    on_close_agent: Option<Callback<String>>,
) -> Element {
    let _ = (
        initial_terminal_ids,
        changes,
        selected_worktree,
        selected_worktrees,
        place_terminal,
        on_select_worktree,
        project,
        bus,
        pane_layout,
        on_pane_layout,
        on_terminal,
        agent_jobs,
        focus_agent,
        on_close_agent,
    );
    rsx! { section { class: "workers", p { "{terminals.len()} terminals" } } }
}
