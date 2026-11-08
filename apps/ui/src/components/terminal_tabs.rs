use dioxus::prelude::*;
use legate_core::{ChangeSet, ProviderId, TerminalInfo, TerminalProvider, TerminalStatus};
use wasm_bindgen_futures::spawn_local;

use crate::api;
use crate::components::bot_avatar::{BotAvatar, BotName, BotState};
use crate::logic::layout::PaneLayoutId;
use crate::logic::worktree_label::{NamedChange, terminal_display_title};

#[derive(Clone, PartialEq)]
pub struct ReviewTab {
    pub label: String,
    pub active: bool,
    pub on_toggle: Callback,
}

#[derive(Clone, PartialEq)]
pub struct AgentTab {
    pub id: String,
    pub label: String,
    pub status: String,
    pub provider: Option<ProviderId>,
    pub avatar: BotState,
}

#[derive(Clone, PartialEq)]
pub struct AgentTabs {
    pub items: Vec<AgentTab>,
    pub active: Option<String>,
    pub on_select: Callback<String>,
    pub on_close: Callback<String>,
}

#[component]
pub fn TerminalTabs(
    terminals: Vec<TerminalInfo>,
    changes: Vec<ChangeSet>,
    project: String,
    active: String,
    pane_layout: String,
    opening: bool,
    on_select: Callback<(String, String)>,
    on_close: Callback<String>,
    on_open: Callback<(String, Option<String>)>,
    on_pane_layout: Callback<String>,
    review: Option<ReviewTab>,
    agents: Option<AgentTabs>,
) -> Element {
    let mut seen_project = use_signal(String::new);
    if !project.is_empty() && seen_project() != project {
        seen_project.set(project.clone());
        spawn_local(async move {
            let _ = api::restore_terminals(80.0, 24.0).await;
        });
    }
    let mut new_worktree = use_signal(|| false);
    let named: Vec<NamedChange> = changes
        .iter()
        .map(|change| NamedChange {
            id: change.id.clone(),
            name: change.name.clone(),
        })
        .collect();
    let tab_ids: Vec<String> = terminals.iter().map(|term| term.id.clone()).collect();
    let review_on = review.as_ref().is_some_and(|item| item.active);
    let agent_on = agents.as_ref().and_then(|item| item.active.clone());

    rsx! {
        div { class: "tab-bar",
            div { class: "tab-scroll", role: "tablist", aria_label: "Terminals",
                for (index, info) in terminals.iter().cloned().enumerate() {
                    {
                        let heading = terminal_display_title(&info.title, info.change.as_deref(), &named);
                        let selected = info.id == active && !review_on && agent_on.is_none();
                        let id = info.id.clone();
                        let close_id = id.clone();
                        let heading_label = heading.clone();
                        let next_right = tab_ids.get(index + 1).cloned();
                        let next_left = index.checked_sub(1).and_then(|slot| tab_ids.get(slot).cloned());
                        rsx! {
                            div { class: if selected { "tab-item is-selected" } else { "tab-item" }, key: "{id}",
                                button {
                                    id: "worker-tab-{id}",
                                    class: "tab",
                                    role: "tab",
                                    aria_selected: "{selected}",
                                    onclick: move |_| on_select.call((id.clone(), "click".into())),
                                    onkeydown: move |event| {
                                        let key = event.key().to_string();
                                        if key != "ArrowLeft" && key != "ArrowRight" {
                                            return;
                                        }
                                        let next = if key == "ArrowRight" { next_right.clone() } else { next_left.clone() };
                                        if let Some(term) = next {
                                            on_select.call((term, "arrow".into()));
                                        }
                                    },
                                    if info.provider != TerminalProvider::Shell {
                                        BotAvatar {
                                            bot: bot_of(info.provider),
                                            state: avatar_of(info.status),
                                            size: 16.0,
                                            title: None,
                                        }
                                    }
                                    span { class: "tab-label", "{heading}" }
                                }
                                button {
                                    class: "tab-close",
                                    aria_label: "Close {heading_label}",
                                    onclick: move |_| on_close.call(close_id.clone()),
                                    "×"
                                }
                            }
                        }
                    }
                }
                if let Some(agents) = agents.clone() {
                    for agent in agents.items.iter().cloned() {
                        {
                            let id = agent.id.clone();
                            let close_id = id.clone();
                            let on = agents.active.as_deref() == Some(id.as_str());
                            rsx! {
                                div { class: if on { "tab-item tab-agent is-selected" } else { "tab-item tab-agent" }, key: "{id}",
                                    button {
                                        class: "tab",
                                        title: "{agent.label} · {agent.status}",
                                        onclick: move |_| agents.on_select.call(id.clone()),
                                        if let Some(provider) = agent.provider {
                                            BotAvatar { bot: BotName::Plan(provider), state: agent.avatar, size: 16.0, title: None }
                                        }
                                        span { class: "tab-label", "{agent.label}" }
                                    }
                                    button {
                                        class: "tab-close",
                                        aria_label: "Close {agent.label}",
                                        onclick: move |_| agents.on_close.call(close_id.clone()),
                                        "×"
                                    }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "tab-actions",
                if let Some(review) = review.clone() {
                    button {
                        class: if review.active { "tab tab-review is-selected" } else { "tab tab-review" },
                        onclick: move |_| review.on_toggle.call(()),
                        span { class: "tab-label", "{review.label}" }
                    }
                }
                for id in [ProviderId::Claude, ProviderId::Cursor, ProviderId::Grok] {
                    button {
                        class: "btn btn-quiet tab-new",
                        disabled: opening,
                        aria_label: "New {worker_label(id)} terminal",
                        onclick: move |_| on_open.call((id.as_str().into(), worktree_pick(new_worktree()))),
                        BotAvatar { bot: BotName::Plan(id), state: BotState::Idle, size: 16.0, title: None }
                        span { class: "tab-new-plus", "+" }
                        span { class: "tab-new-label", "{worker_label(id)}" }
                    }
                }
                button {
                    class: "btn btn-quiet tab-worktree",
                    aria_pressed: "{new_worktree()}",
                    title: "Start new terminals in their own git worktree",
                    onclick: move |_| new_worktree.set(!new_worktree()),
                    "Worktree"
                }
                div { class: "pane-layout-picker",
                    label { r#for: "pane-layout",
                        span { class: "visually-hidden", "Panes" }
                        span { class: "pane-layout-label", aria_hidden: "true", "Panes" }
                    }
                    select {
                        id: "pane-layout",
                        class: "select",
                        onchange: move |event| on_pane_layout.call(event.value()),
                        for layout in PaneLayoutId::ALL {
                            option { value: "{layout.as_str()}", selected: layout.as_str() == pane_layout, "{layout.label()}" }
                        }
                    }
                }
            }
        }
    }
}

fn worktree_pick(on: bool) -> Option<String> {
    if on { Some("new".into()) } else { None }
}

fn worker_label(id: ProviderId) -> &'static str {
    match id {
        ProviderId::Claude => "Claude",
        ProviderId::Cursor => "Cursor",
        ProviderId::Grok => "Grok",
        ProviderId::Local => "Local",
    }
}

fn bot_of(provider: TerminalProvider) -> BotName {
    match provider {
        TerminalProvider::Claude => BotName::Plan(ProviderId::Claude),
        TerminalProvider::Cursor => BotName::Plan(ProviderId::Cursor),
        TerminalProvider::Grok => BotName::Plan(ProviderId::Grok),
        TerminalProvider::Local => BotName::Plan(ProviderId::Local),
        TerminalProvider::Shell => BotName::Plan(ProviderId::Local),
    }
}

fn avatar_of(status: TerminalStatus) -> BotState {
    match status {
        TerminalStatus::Sleeping => BotState::Resting,
        TerminalStatus::Exited => BotState::Error,
        TerminalStatus::Running => BotState::Idle,
    }
}
