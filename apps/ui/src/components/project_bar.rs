use std::collections::HashMap;

use dioxus::prelude::*;
use legate_core::{
    ChangeSet, JobRecord, JobStatus, LeadChat, ProjectEntry, ProviderId, TerminalInfo,
};
use wasm_bindgen_futures::spawn_local;

use crate::api;
use crate::components::sidebar_menu::{MenuEntry, SidebarMenu};
use crate::components::usage_meter::UsageMeter;
use crate::logic::sidebar_cards::{CardInput, DEFAULT_OPTIONS, arrange_cards, card_status};
use crate::state::use_store;

type WorktreeCreate = Callback<(Option<String>, i64, Option<ProviderId>, Option<String>)>;

#[component]
pub fn ProjectBar(
    projects: Vec<ProjectEntry>,
    chats: Vec<LeadChat>,
    busy: bool,
    lead_busy: bool,
    now: i64,
    error: Option<String>,
    github_available: bool,
    changes: Vec<ChangeSet>,
    jobs: Vec<JobRecord>,
    terminals: Vec<TerminalInfo>,
    selected_worktrees: Vec<Option<String>>,
    claude_available: bool,
    cursor_available: bool,
    on_add: Callback,
    on_github_clone: Callback<String>,
    on_switch: Callback<(String, Option<String>)>,
    on_remove: Callback<String>,
    on_open_chat: Callback<String>,
    on_new_chat: Callback,
    on_rename_chat: Callback<(String, String)>,
    on_remove_chat: Callback<String>,
    on_select_worktree: Callback<(Option<String>, String)>,
    on_select_all_worktrees: Callback,
    on_create_worktree: WorktreeCreate,
    on_rename_worktree: Callback<(String, String)>,
    on_discard_worktree: Callback<String>,
    on_open_in_worktree: Callback<(ProviderId, Option<String>)>,
    activity: HashMap<String, i64>,
    on_reveal: Callback<Option<String>>,
    on_reveal_project: Callback<String>,
    on_open_settings: Callback,
    on_new_task: Callback,
    on_open_goals: Callback,
    on_open_automations: Callback,
    goals_needing_you: i64,
    on_open_setup: Callback,
) -> Element {
    let store = use_store();
    let mut search = use_signal(String::new);
    let mut github_open = use_signal(|| false);
    let mut repos = use_signal(Vec::<legate_core::GithubRepo>::new);
    let mut repos_loading = use_signal(|| false);
    let mut filter = use_signal(String::new);
    let mut cloning = use_signal(|| None::<String>);
    let mut menu = use_signal(|| None::<(String, f64, f64)>);
    let mut folded = use_signal(HashMap::<String, bool>::new);
    let mut worktree_name = use_signal(String::new);
    let needle = search.read().trim().to_lowercase();
    let mut ordered = projects.clone();
    ordered.sort_by_key(|entry| !entry.active);
    let cards = worktree_cards(
        &changes,
        &jobs,
        &terminals,
        &selected_worktrees,
        &activity,
        now,
        &needle,
    );
    let plans = store.plans.read().clone();
    let shows = store.prefs.read().usage_shows;
    let shown_chats: Vec<LeadChat> = chats
        .iter()
        .filter(|chat| needle.is_empty() || chat.title.to_lowercase().contains(&needle))
        .take(30)
        .cloned()
        .collect();
    let repo_list = repos.read().clone();
    let repo_needle = filter.read().trim().to_lowercase();
    let menu_at = menu.read().clone();

    rsx! {
        div { class: "project-bar",
            div { class: "project-bar-tools",
                input {
                    class: "project-search",
                    placeholder: "Search",
                    value: "{search}",
                    oninput: move |event| search.set(event.value()),
                }
                button { class: "btn btn-quiet btn-compact", onclick: move |_| on_add.call(()), "Open folder" }
                if github_available {
                    button {
                        class: "btn btn-quiet btn-compact",
                        onclick: move |_| {
                            github_open.set(true);
                            repos_loading.set(true);
                            spawn_local(async move {
                                match api::github_repos().await {
                                    Ok(list) => repos.set(list),
                                    Err(_) => repos.set(Vec::new()),
                                }
                                repos_loading.set(false);
                            });
                        },
                        "Clone from GitHub"
                    }
                }
                button { class: "btn btn-quiet btn-compact", onclick: move |_| on_new_task.call(()), "New task" }
                button { class: "btn btn-quiet btn-compact", onclick: move |_| on_open_settings.call(()), "Settings" }
                button { class: "btn btn-quiet btn-compact", onclick: move |_| on_open_goals.call(()),
                    "Goals"
                    if goals_needing_you > 0 { span { class: "badge", "{goals_needing_you}" } }
                }
                button { class: "btn btn-quiet btn-compact", onclick: move |_| on_open_automations.call(()), "Automations" }
            }
            if let Some(error) = error.clone() {
                p { class: "setup-error", "{error}" }
            }
            if github_open() {
                div { class: "github-panel", role: "region", aria_label: "Clone from GitHub",
                    div { class: "github-panel-head",
                        input {
                            placeholder: "Filter repositories",
                            value: "{filter}",
                            oninput: move |event| filter.set(event.value()),
                        }
                        button { class: "btn btn-quiet btn-compact", onclick: move |_| github_open.set(false), "Close" }
                    }
                    if repos_loading() {
                        p { class: "hint", "Loading repositories…" }
                    }
                    ul { class: "github-list",
                        for repo in repo_list.iter().filter(|repo| {
                            repo_needle.is_empty()
                                || repo.name_with_owner.to_lowercase().contains(&repo_needle)
                                || repo.description.to_lowercase().contains(&repo_needle)
                        }).cloned() {
                            {
                                let name = repo.name_with_owner.clone();
                                let age = relative_updated(&repo.updated_at, now);
                                let busy_name = cloning.read().clone();
                                rsx! {
                                    li { class: "github-row", key: "{name}",
                                        div { class: "github-row-name",
                                            if repo.is_private { span { class: "github-lock", title: "Private", "🔒" } }
                                            span { "{name}" }
                                            span { class: "hint", "{age}" }
                                        }
                                        p { class: "hint", "{repo.description}" }
                                        button {
                                            class: "btn btn-primary btn-compact",
                                            disabled: busy_name.is_some(),
                                            onclick: move |_| {
                                                let name = name.clone();
                                                cloning.set(Some(name.clone()));
                                                spawn_local(async move {
                                                    match api::github_clone(&name).await {
                                                        Ok(Some(_)) => {
                                                            cloning.set(None);
                                                            github_open.set(false);
                                                            on_github_clone.call(name);
                                                        }
                                                        Ok(None) => cloning.set(None),
                                                        Err(text) => {
                                                            cloning.set(None);
                                                            let mut store = use_store();
                                                            store.error.set(Some(text));
                                                        }
                                                    }
                                                });
                                            },
                                            if busy_name.as_deref() == Some(name.as_str()) { "Cloning…" } else { "Clone" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            ul { class: "project-list",
                for entry in ordered {
                    {
                        let path = entry.path.clone();
                        let name = folder_name(&path);
                        let is_folded = folded.read().get(&path).copied().unwrap_or(!entry.active && entry.worktrees.as_ref().map(|list| list.is_empty()).unwrap_or(true));
                        let switch_path = path.clone();
                        let menu_path = path.clone();
                        rsx! {
                            li { class: if entry.active { "project-group is-open" } else { "project-group" }, key: "{path}",
                                div { class: if entry.active { "project-group-head is-open" } else { "project-group-head" },
                                    button {
                                        class: "project-group-fold",
                                        onclick: move |_| {
                                            let path = path.clone();
                                            folded.with_mut(|map| {
                                                let open = map.get(&path).copied().unwrap_or(false);
                                                map.insert(path, !open);
                                            });
                                        },
                                        if is_folded { "▸" } else { "▾" }
                                    }
                                    button {
                                        class: "project-group-name",
                                        title: "{entry.path}",
                                        onclick: move |_| {
                                            if !entry.active {
                                                on_switch.call((switch_path.clone(), None));
                                            }
                                        },
                                        span { class: "project-name", "{name}" }
                                        span { class: "project-row-meta",
                                            if let Some(branch) = entry.branch.clone() {
                                                span { class: "project-branch", "{branch}" }
                                            } else if !entry.is_repo {
                                                span { class: "project-note", "not a git repository" }
                                            }
                                            if entry.running > 0 || (entry.active && busy) {
                                                span { class: "status status-running", "running" }
                                            }
                                        }
                                        if entry.active { ProjectStatus { entry: entry.clone() } }
                                    }
                                    button {
                                        class: "btn btn-quiet btn-compact",
                                        aria_label: "Project menu",
                                        onclick: move |_| menu.set(Some((menu_path.clone(), 240.0, 120.0))),
                                        "⋯"
                                    }
                                }
                                if entry.active && !is_folded {
                                    div { class: "worktree-cards",
                                        button { class: "btn btn-quiet btn-compact", onclick: move |_| on_select_all_worktrees.call(()), "All" }
                                        for group in cards.clone() {
                                            for card in group.cards {
                                                {
                                                    let key = card.input.key.clone();
                                                    let title = card.input.title.clone();
                                                    let selected = card.input.selected;
                                                    rsx! {
                                                        button {
                                                            class: if selected { "worktree-card is-selected" } else { "worktree-card" },
                                                            onclick: move |_| on_select_worktree.call((key.clone(), "toggle".into())),
                                                            span { "{title}" }
                                                            span { class: "hint", "{status_word(card.status)}" }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        div { class: "worktree-create",
                                            input {
                                                placeholder: "New worktree",
                                                value: "{worktree_name}",
                                                oninput: move |event| worktree_name.set(event.value()),
                                            }
                                            button {
                                                class: "btn btn-quiet btn-compact",
                                                onclick: move |_| {
                                                    let name = worktree_name.read().trim().to_string();
                                                    let named = if name.is_empty() { None } else { Some(name) };
                                                    let start = if cursor_available {
                                                        Some(ProviderId::Cursor)
                                                    } else if claude_available {
                                                        Some(ProviderId::Claude)
                                                    } else {
                                                        None
                                                    };
                                                    on_create_worktree.call((named, 1, start, None));
                                                },
                                                "Create"
                                            }
                                            if let Some(first) = changes.first() {
                                                {
                                                    let rename_id = first.id.clone();
                                                    let discard_id = first.id.clone();
                                                    let open_id = first.id.clone();
                                                    rsx! {
                                                        button {
                                                            class: "btn btn-quiet btn-compact",
                                                            onclick: move |_| {
                                                                let name = worktree_name.read().to_string();
                                                                on_rename_worktree.call((rename_id.clone(), name));
                                                            },
                                                            "Rename"
                                                        }
                                                        button {
                                                            class: "btn btn-quiet btn-compact",
                                                            onclick: move |_| on_discard_worktree.call(discard_id.clone()),
                                                            "Discard"
                                                        }
                                                        button {
                                                            class: "btn btn-quiet btn-compact",
                                                            onclick: move |_| on_open_in_worktree.call((ProviderId::Claude, Some(open_id.clone()))),
                                                            "Open Claude here"
                                                        }
                                                    }
                                                }
                                            }
                                            button { class: "btn btn-quiet btn-compact", onclick: move |_| on_reveal.call(None), "Reveal folder" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "lead-chats",
                div { class: "lead-chats-head",
                    span { "Chats" }
                    if lead_busy { span { class: "status status-running", "working" } }
                    button { class: "btn btn-quiet btn-compact", onclick: move |_| on_new_chat.call(()), "New chat" }
                }
                for chat in shown_chats {
                    {
                        let open_id = chat.id.clone();
                        let rename_id = chat.id.clone();
                        let remove_id = chat.id.clone();
                        let title = chat.title.clone();
                        rsx! {
                            div { class: if chat.active { "chat-row is-active" } else { "chat-row" }, key: "{open_id}",
                                button { class: "chat-title", onclick: move |_| on_open_chat.call(open_id.clone()), "{title}" }
                                button {
                                    class: "btn btn-quiet btn-compact",
                                    onclick: move |_| on_rename_chat.call((rename_id.clone(), format!("{title} 2"))),
                                    "Rename"
                                }
                                button { class: "btn btn-quiet btn-compact", onclick: move |_| on_remove_chat.call(remove_id.clone()), "Remove" }
                            }
                        }
                    }
                }
            }
            UsageMeter { plans: plans, now: now, on_open_setup: on_open_setup, shows: shows }
            if let Some((path, x, y)) = menu_at {
                SidebarMenu {
                    label: "Project".to_string(),
                    at: (x, y),
                    on_close: Callback::new(move |_| menu.set(None)),
                    entries: vec![
                        MenuEntry::item("Open", Callback::new({
                            let path = path.clone();
                            move |_| on_switch.call((path.clone(), None))
                        })),
                        MenuEntry::item("Reveal", Callback::new({
                            let path = path.clone();
                            move |_| on_reveal_project.call(path.clone())
                        })),
                        MenuEntry {
                            label: "Remove".into(),
                            on_select: Some(Callback::new({
                                let path = path.clone();
                                move |_| on_remove.call(path.clone())
                            })),
                            disabled: false,
                            danger: true,
                            checked: false,
                            heading: false,
                            separator: true,
                            title: String::new(),
                        },
                    ],
                }
            }
        }
    }
}

fn worktree_cards(
    changes: &[ChangeSet],
    jobs: &[JobRecord],
    terminals: &[TerminalInfo],
    selected: &[Option<String>],
    activity: &HashMap<String, i64>,
    now: i64,
    filter: &str,
) -> Vec<crate::logic::sidebar_cards::CardGroup> {
    let mut inputs = vec![CardInput {
        key: None,
        title: "This folder".into(),
        branch: None,
        terminal_ids: terminals
            .iter()
            .filter(|term| term.change.is_none())
            .map(|term| term.id.clone())
            .collect(),
        job_running: jobs.iter().any(|job| {
            job.change.is_none() && matches!(job.status, JobStatus::Queued | JobStatus::Running)
        }),
        has_changes: false,
        created_index: -1,
        pinned: false,
        seen_at: 0,
        selected: selected.iter().any(|item| item.is_none()),
    }];
    for (index, change) in changes.iter().enumerate() {
        inputs.push(CardInput {
            key: Some(change.id.clone()),
            title: change.name.clone().unwrap_or_else(|| change.id.clone()),
            branch: Some(change.branch.clone()),
            terminal_ids: terminals
                .iter()
                .filter(|term| term.change.as_deref() == Some(change.id.as_str()))
                .map(|term| term.id.clone())
                .collect(),
            job_running: jobs.iter().any(|job| {
                job.change.as_deref() == Some(change.id.as_str())
                    && matches!(job.status, JobStatus::Queued | JobStatus::Running)
            }),
            has_changes: !change.files.is_empty(),
            created_index: index as i64,
            pinned: false,
            seen_at: 0,
            selected: selected
                .iter()
                .any(|item| item.as_deref() == Some(change.id.as_str())),
        });
    }
    let cards = inputs
        .into_iter()
        .map(|input| card_status(input, activity, now))
        .collect();
    arrange_cards(cards, DEFAULT_OPTIONS, filter)
}

fn status_word(status: crate::logic::sidebar_cards::CardStatus) -> &'static str {
    match status {
        crate::logic::sidebar_cards::CardStatus::Working => "working",
        crate::logic::sidebar_cards::CardStatus::Unread => "unread",
        crate::logic::sidebar_cards::CardStatus::Idle => "idle",
    }
}

#[component]
fn ProjectStatus(entry: ProjectEntry) -> Element {
    let upstream = entry.ahead > 0 || entry.behind > 0;
    let lines = entry.insertions > 0 || entry.deletions > 0;
    if !entry.is_repo || (!upstream && entry.dirty == 0 && !lines) {
        return rsx! {};
    }
    rsx! {
        span { class: "project-status",
            if entry.ahead > 0 { span { class: "project-status-item", "↑{entry.ahead}" } }
            if entry.behind > 0 { span { class: "project-status-item", "↓{entry.behind}" } }
            if entry.dirty > 0 { span { class: "project-status-item", "{entry.dirty} uncommitted" } }
            if lines {
                span { class: "project-status-item",
                    span { class: "stat-add", "+{entry.insertions}" }
                    " "
                    span { class: "stat-del", "−{entry.deletions}" }
                }
            }
        }
    }
}

fn folder_name(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_string()
}

fn relative_updated(iso: &str, now: i64) -> String {
    let Ok(parsed) =
        time::OffsetDateTime::parse(iso, &time::format_description::well_known::Rfc3339)
    else {
        return String::new();
    };
    crate::logic::usage_format::ago_phrase(parsed.unix_timestamp() * 1000, now)
}
