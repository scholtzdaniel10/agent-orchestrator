//! Centre of the window: terminal tabs, panes, a live agent, or a review.

use dioxus::prelude::*;
use legate_core::{ChangeSet, JobRecord, JobStatus, ProviderId, TerminalInfo};
use wasm_bindgen_futures::spawn_local;

use crate::api;
use crate::components::agent_pane::AgentPane;
use crate::components::review_pane::ReviewPane;
use crate::components::terminal_pane::{PanePlacement, TerminalPane};
use crate::components::terminal_tabs::{AgentTab, AgentTabs, ReviewTab, TerminalTabs};
use crate::logic::agent_tabs::{agent_status_label, agent_tab_label};
use crate::logic::layout::{
    self, PaneLayoutId, assign_tab_to_pane, drop_missing_terminals, empty_pane_for_terminal,
    fill_panes, pane_count, place_new_terminal, place_terminal_in_pane, resolved_pane_layout,
    seat_unplaced,
};
use crate::logic::terminal_bus::TerminalBus;
use crate::logic::worktree_label::{
    NamedChange, terminal_display_title, terminals_in_sidebar_order,
};

const AGENTS: [ProviderId; 4] = [
    ProviderId::Claude,
    ProviderId::Codex,
    ProviderId::Cursor,
    ProviderId::Grok,
];

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
    let mut slots = use_signal(Vec::<Option<String>>::new);
    let mut focused = use_signal(|| 0i64);
    let opening = use_signal(|| false);
    let mut open_error = use_signal(|| None::<String>);
    let mut focus_id = use_signal(|| None::<String>);
    let mut focus_tick = use_signal(|| 0i64);
    let mut seen_place = use_signal(|| None::<i64>);
    let mut reviewing = use_signal(|| None::<String>);
    let mut viewing = use_signal(|| None::<String>);
    let mut seen_agent = use_signal(|| None::<i64>);
    let mut seen_selection = use_signal(String::new);
    if let Some((id, nonce)) = focus_agent.clone()
        && seen_agent() != Some(nonce)
    {
        seen_agent.set(Some(nonce));
        viewing.set(Some(id));
        reviewing.set(None);
    }
    if let Some((id, nonce)) = place_terminal.clone()
        && seen_place() != Some(nonce)
    {
        seen_place.set(Some(nonce));
        focus_id.set(Some(id));
        focus_tick.set(focus_tick() + 1);
    }

    let named: Vec<NamedChange> = changes
        .iter()
        .map(|change| NamedChange {
            id: change.id.clone(),
            name: change.name.clone(),
        })
        .collect();
    let filtered: Vec<TerminalInfo> = terminals
        .iter()
        .filter(|info| {
            let key = info.change.clone();
            selected_worktrees.iter().any(|item| item == &key)
        })
        .cloned()
        .collect();
    let visible = terminals_in_sidebar_order(&filtered, &named, |info| info.change.as_deref());
    let visible_ids: Vec<String> = visible.iter().map(|info| info.id.clone()).collect();
    let selection_key = selected_worktrees
        .iter()
        .map(|id| id.clone().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\u{1}");
    let shown = resolved_pane_layout(pane_layout, visible_ids.len());
    let selection_changed = seen_selection() != selection_key;
    if selection_changed {
        seen_selection.set(selection_key);
        focused.set(0);
    }
    let current = slots.read().clone();
    let base = if selection_changed || current.len() != pane_count(shown, visible_ids.len()) {
        fill_panes(
            pane_layout,
            &visible_ids,
            visible_ids.first().map(String::as_str),
        )
    } else {
        current
    };
    let live = seat_unplaced(&drop_missing_terminals(&base, &visible_ids), &visible_ids);
    if live != *slots.read() {
        slots.set(live.clone());
    }
    let slots_now = live;
    let agent_view = viewing
        .read()
        .as_ref()
        .and_then(|id| agent_jobs.iter().find(|job| &job.id == id))
        .cloned();
    let review_change = selected_worktree.as_ref().and_then(|id| {
        changes
            .iter()
            .find(|change| &change.id == id && !change.files.is_empty())
            .cloned()
    });
    let reviewing_on = agent_view.is_none()
        && review_change
            .as_ref()
            .is_some_and(|change| reviewing.read().as_ref() == Some(&change.id));
    let tab_active = focus_id.read().clone().unwrap_or_else(|| {
        slots_now
            .get(focused() as usize)
            .cloned()
            .flatten()
            .unwrap_or_default()
    });
    let error = open_error.read().clone();
    let busy = opening();
    let known = initial_terminal_ids.clone();

    rsx! {
        div { class: "workers",
            TerminalTabs {
                terminals: visible.clone(),
                changes: changes.clone(),
                project: project.clone(),
                active: tab_active.clone(),
                pane_layout: pane_layout.as_str().to_string(),
                opening: busy,
                on_select: Callback::new(move |(id, source): (String, String)| {
                    viewing.set(None);
                    let next = assign_tab_to_pane(&slots.read(), focused(), &id);
                    slots.set(next);
                    if source == "click" {
                        focus_id.set(Some(id));
                        focus_tick.set(focus_tick() + 1);
                    }
                }),
                on_close: Callback::new(move |id: String| {
                    let next = empty_pane_for_terminal(&slots.read(), &id);
                    slots.set(next);
                    let closing = id.clone();
                    spawn_local(async move {
                        if let Err(text) = api::close_terminal(&closing).await {
                            open_error.set(Some(text));
                        }
                    });
                }),
                on_open: Callback::new(move |(provider, worktree): (String, Option<String>)| {
                    open_provider(
                        provider,
                        worktree,
                        None,
                        opening,
                        open_error,
                        slots,
                        focused(),
                        on_terminal,
                        on_select_worktree,
                        focus_id,
                        focus_tick,
                    );
                }),
                on_pane_layout: Callback::new(move |value: String| {
                    if let Some(next) = PaneLayoutId::parse(&value) {
                        on_pane_layout.call(next);
                        let count = visible_ids.len();
                        let fresh = layout::empty_slots(next, count);
                        let seated = seat_unplaced(&fresh, &visible_ids);
                        slots.set(seated);
                    }
                }),
                review: review_change.as_ref().map(|change| {
                    let review_id = change.id.clone();
                    ReviewTab {
                        label: format!("Review +{} −{}", change.insertions, change.deletions),
                        active: reviewing_on,
                        on_toggle: Callback::new(move |_| {
                            viewing.set(None);
                            let current = reviewing.read().clone();
                            reviewing.set(if current.as_deref() == Some(review_id.as_str()) {
                                None
                            } else {
                                Some(review_id.clone())
                            });
                        }),
                    }
                }),
                agents: if agent_jobs.is_empty() {
                    None
                } else {
                    Some(AgentTabs {
                        items: agent_jobs.iter().map(|job| AgentTab {
                            id: job.id.clone(),
                            label: agent_tab_label(job),
                            status: agent_status_label(job).to_string(),
                            provider: job.provider,
                            avatar: agent_avatar(job.status),
                        }).collect(),
                        active: agent_view.as_ref().map(|job| job.id.clone()),
                        on_select: Callback::new(move |id: String| {
                            reviewing.set(None);
                            let current = viewing.read().clone();
                            viewing.set(if current.as_deref() == Some(id.as_str()) { None } else { Some(id) });
                        }),
                        on_close: Callback::new(move |id: String| {
                            if viewing.read().as_deref() == Some(id.as_str()) {
                                viewing.set(None);
                            }
                            if let Some(close) = on_close_agent {
                                close.call(id);
                            }
                        }),
                    })
                },
            }
            if let Some(text) = error {
                p { class: "error-text", role: "alert", "{text}" }
            } else if busy {
                p { class: "hint", "Opening terminal…" }
            }
            div {
                class: if reviewing_on || agent_view.is_some() { "workers-stage is-hidden" } else { "workers-stage" },
                if visible.is_empty() {
                    div { class: "workspace-empty",
                        p { class: "workspace-empty-copy", "Open an agent here, or ask the lead in the panel." }
                        div { class: "workspace-empty-actions",
                            for id in AGENTS {
                                button {
                                    class: "btn btn-quiet tab-new",
                                    r#type: "button",
                                    aria_label: "New {worker_label(id)} terminal",
                                    disabled: busy,
                                    onclick: move |_| {
                                        open_provider(
                                            id.as_str().into(),
                                            new_worktree_flag(),
                                            None,
                                            opening,
                                            open_error,
                                            slots,
                                            focused(),
                                            on_terminal,
                                            on_select_worktree,
                                            focus_id,
                                            focus_tick,
                                        );
                                    },
                                    "+ {worker_label(id)}"
                                }
                            }
                        }
                    }
                } else {
                    div { class: "terminal-stage is-{shown.as_str()}",
                        for (index, id) in slots_now.iter().cloned().enumerate() {
                            if id.is_none() {
                                {
                                    let slot = index as i64;
                                    rsx! {
                                        div {
                                            key: "empty-{index}",
                                            class: if focused() == slot {
                                                format!("pane-empty is-slot-{index} is-focused")
                                            } else {
                                                format!("pane-empty is-slot-{index}")
                                            },
                                            onmousedown: move |_| focused.set(slot),
                                            div { class: "pane-empty-actions",
                                                for provider in AGENTS {
                                                    button {
                                                        class: "btn btn-quiet",
                                                        r#type: "button",
                                                        disabled: busy,
                                                        onclick: move |_| {
                                                            open_provider(
                                                                provider.as_str().into(),
                                                                new_worktree_flag(),
                                                                Some(slot),
                                                                opening,
                                                                open_error,
                                                                slots,
                                                                focused(),
                                                                on_terminal,
                                                                on_select_worktree,
                                                                focus_id,
                                                                focus_tick,
                                                            );
                                                        },
                                                        "+ {worker_label(provider)}"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if let Some(known) = known {
                            for info in terminals.iter().cloned() {
                                {
                                    let placement = slots_now.iter().position(|slot| slot.as_deref() == Some(info.id.as_str()));
                                    let slot = placement.map(|index| index as i64).unwrap_or(-1);
                                    let id = info.id.clone();
                                    let activate_id = id.clone();
                                    let close_id = id.clone();
                                    let heading = terminal_display_title(&info.title, info.change.as_deref(), &named);
                                    let focused_pane = placement.is_some_and(|index| index as i64 == focused());
                                    let nonce = if focus_id.read().as_deref() == Some(id.as_str()) { focus_tick() } else { 0 };
                                    let restore = known.iter().any(|item| item == &id);
                                    rsx! {
                                        TerminalPane {
                                            key: "{id}",
                                            info: info,
                                            heading: Some(heading),
                                            bus: bus.clone(),
                                            restore: restore,
                                            placement: if slot >= 0 { PanePlacement::Slot(slot) } else { PanePlacement::Hidden },
                                            focused: focused_pane,
                                            focus_nonce: nonce,
                                            own_window: false,
                                            on_activate: Callback::new(move |_| {
                                                if slot >= 0 { focused.set(slot); }
                                                focus_id.set(Some(activate_id.clone()));
                                                focus_tick.set(focus_tick() + 1);
                                            }),
                                            on_close: Callback::new(move |_| {
                                                let next = empty_pane_for_terminal(&slots.read(), &close_id);
                                                slots.set(next);
                                                let closing = close_id.clone();
                                                spawn_local(async move {
                                                    if let Err(text) = api::close_terminal(&closing).await {
                                                        open_error.set(Some(text));
                                                    }
                                                });
                                            }),
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if let Some(job) = agent_view.clone() {
                AgentPane {
                    key: "{job.id}",
                    job: job.clone(),
                    on_take_over: Callback::new(move |id: String| {
                        spawn_local(async move {
                            match api::take_over_job(&id, 100.0, 30.0).await {
                                Ok(info) => {
                                    on_select_worktree.call(info.change.clone());
                                    on_terminal.call(info.clone());
                                    viewing.set(None);
                                    if let Some(close) = on_close_agent {
                                        close.call(id);
                                    }
                                    focus_id.set(Some(info.id));
                                    focus_tick.set(focus_tick() + 1);
                                }
                                Err(text) => open_error.set(Some(text)),
                            }
                        });
                    }),
                    on_close: Callback::new(move |_| viewing.set(None)),
                }
            }
            if reviewing_on {
                if let Some(change) = review_change.clone() {
                    ReviewPane {
                        change: change,
                        on_close: Callback::new(move |_| reviewing.set(None)),
                        on_done: Callback::new(move |_| reviewing.set(None)),
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn open_provider(
    provider: String,
    worktree: Option<String>,
    pane: Option<i64>,
    mut opening: Signal<bool>,
    mut open_error: Signal<Option<String>>,
    mut slots: Signal<Vec<Option<String>>>,
    focused: i64,
    on_terminal: Callback<TerminalInfo>,
    on_select_worktree: Callback<Option<String>>,
    mut focus_id: Signal<Option<String>>,
    mut focus_tick: Signal<i64>,
) {
    if opening() {
        return;
    }
    opening.set(true);
    spawn_local(async move {
        match api::open_terminal(&provider, 100.0, 30.0, worktree.as_deref(), None).await {
            Ok(info) => {
                let next = match pane {
                    Some(index) => place_terminal_in_pane(&slots.read(), index, &info.id),
                    None => place_new_terminal(&slots.read(), focused, &info.id),
                };
                slots.set(next);
                if worktree.as_deref() == Some("new") {
                    on_select_worktree.call(info.change.clone());
                }
                focus_id.set(Some(info.id.clone()));
                focus_tick.set(focus_tick() + 1);
                on_terminal.call(info);
                open_error.set(None);
            }
            Err(text) => open_error.set(Some(text)),
        }
        opening.set(false);
    });
}

fn worker_label(id: ProviderId) -> &'static str {
    match id {
        ProviderId::Claude => "Claude",
        ProviderId::Codex => "Codex",
        ProviderId::Grok => "Grok",
        ProviderId::Cursor | ProviderId::Local => "Cursor",
    }
}

fn agent_avatar(status: JobStatus) -> crate::components::BotState {
    match status {
        JobStatus::Running => crate::components::BotState::Working,
        JobStatus::Failed => crate::components::BotState::Error,
        JobStatus::Queued => crate::components::BotState::Resting,
        JobStatus::Done => crate::components::BotState::Idle,
    }
}

fn new_worktree_flag() -> Option<String> {
    let storage = web_sys::window()?.local_storage().ok()??;
    match storage.get_item("orch.newWorktree").ok()?? {
        value if value == "1" => Some("new".into()),
        _ => None,
    }
}
