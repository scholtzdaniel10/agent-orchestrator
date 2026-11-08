//! Sidebar. The element tree matches `ProjectBar.tsx` so `main.css` applies.

use std::collections::HashMap;

use dioxus::prelude::dioxus_elements::{InteractionLocation, ModifiersInteraction};
use dioxus::prelude::*;
use legate_core::{
    ChangeSet, JobRecord, JobStatus, LeadChat, ProjectEntry, ProjectWorktree, ProviderId,
    TerminalInfo, TerminalProvider, TerminalStatus, plan_of,
};
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::api;
use crate::components::bot_avatar::{BotAvatar, BotName, BotState};
use crate::components::sidebar_menu::{MenuEntry, SidebarMenu};
use crate::components::star::Star;
use crate::logic::sidebar_cards::{
    Card, CardInput, CardLayout, CardStatus, DEFAULT_OPTIONS, GroupBy, SidebarOptions, SortBy,
    arrange_cards, card_status, read_options,
};
use crate::logic::worktree_label::{self, NamedChange};

type WorktreeCreate = Callback<(Option<String>, i64, Option<ProviderId>, Option<String>)>;

const AGENTS: [ProviderId; 3] = [ProviderId::Claude, ProviderId::Cursor, ProviderId::Grok];

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
    let _ = on_open_setup;
    let mut search = use_signal(String::new);
    let options = use_signal(stored_options);
    let mut folds = use_signal(stored_folds);
    let mut chats_open = use_signal(|| stored_bool("legate.sidebar.chatsOpen", true));
    let mut popup = use_signal(|| None::<Popup>);
    let mut github_open = use_signal(|| false);
    let repos = use_signal(Vec::<legate_core::GithubRepo>::new);
    let repos_loading = use_signal(|| false);
    let mut filter = use_signal(String::new);
    let mut cloning = use_signal(|| None::<String>);
    let needle = search.read().trim().to_lowercase();
    let mut tree: Vec<ProjectEntry> = projects
        .iter()
        .filter(|entry| entry.active)
        .cloned()
        .collect();
    tree.extend(projects.iter().filter(|entry| !entry.active).cloned());
    let active = projects.iter().find(|entry| entry.active).cloned();
    let shown_chats: Vec<LeadChat> = chats
        .iter()
        .filter(|chat| needle.is_empty() || chat.title.to_lowercase().contains(&needle))
        .take(30)
        .cloned()
        .collect();
    let option_value = *options.read();
    let popup_now = popup.read().clone();
    let repo_list = repos.read().clone();

    rsx! {
        aside { class: "project-bar", aria_label: "Projects",
            div { class: "sidebar-top",
                button {
                    class: "btn sidebar-new-task",
                    r#type: "button",
                    onclick: move |_| on_new_task.call(()),
                    span { class: "sidebar-new-task-plus", aria_hidden: "true", "+" }
                    "New task"
                }
                nav { class: "sidebar-nav", aria_label: "Sidebar",
                    button {
                        class: "sidebar-nav-row",
                        r#type: "button",
                        onclick: move |_| on_open_goals.call(()),
                        span { "Goals" }
                        if goals_needing_you > 0 {
                            span { class: "sidebar-nav-count", "{goals_needing_you}" }
                        }
                    }
                    button {
                        class: "sidebar-nav-row",
                        r#type: "button",
                        onclick: move |_| on_open_automations.call(()),
                        "Automations"
                    }
                }
            }
            div { class: "project-search-row",
                input {
                    class: "project-search",
                    r#type: "search",
                    value: "{search}",
                    placeholder: "Search  Ctrl+K",
                    aria_label: "Search projects, worktrees and chats",
                    oninput: move |event| search.set(event.value()),
                    onkeydown: move |event| {
                        if event.key().to_string() == "Escape" && !search.read().is_empty() {
                            event.prevent_default();
                            event.stop_propagation();
                            search.set(String::new());
                        }
                    },
                }
                button {
                    class: "btn btn-quiet btn-compact project-options",
                    r#type: "button",
                    aria_label: "View options",
                    title: "Group, sort and card options",
                    aria_haspopup: "menu",
                    aria_expanded: popup_now.as_ref().is_some_and(|item| matches!(item, Popup::Options { .. })),
                    onclick: move |event| {
                        let point = event.client_coordinates();
                        popup.set(Some(Popup::Options { x: point.x, y: point.y }));
                    },
                    svg {
                        width: "14",
                        height: "14",
                        view_box: "0 0 16 16",
                        path {
                            fill: "currentColor",
                            d: "M1 3h8v1.5H1zM12 3h3v1.5h-3zM9.5 1.5H11v4H9.5zM1 11.5h3V13H1zM7 11.5h8V13H7zM4.5 10H6v4H4.5zM1 7.25h11v1.5H1zM13.5 6.25H15v3.5h-1.5z",
                        }
                    }
                }
            }
            if let Some(Popup::Options { x, y }) = popup_now {
                SidebarMenu {
                    label: "View options".to_string(),
                    at: (x, y),
                    on_close: Callback::new(move |_| popup.set(None)),
                    entries: option_entries(option_value, move |next| {
                        let mut options = options;
                        write_json("legate.sidebar.options", &options_json(next));
                        options.set(next);
                    }),
                }
            }
            if let Some(Popup::Switcher { x, y }) = popup.read().clone() {
                SidebarMenu {
                    label: "Projects".to_string(),
                    at: (x, y),
                    on_close: Callback::new(move |_| popup.set(None)),
                    entries: switcher_entries(&projects, github_available, active.is_some(), on_switch, on_add, on_reveal_project, move || {
                        let mut github_open = github_open;
                        let mut repos_loading = repos_loading;
                        let repos = repos;
                        if !github_open() {
                            repos_loading.set(true);
                        }
                        github_open.set(true);
                        spawn_local(async move {
                            let mut repos = repos;
                            let mut repos_loading = repos_loading;
                            match api::github_repos().await {
                                Ok(list) => repos.set(list),
                                Err(_) => repos.set(Vec::new()),
                            }
                            repos_loading.set(false);
                        });
                    }),
                }
            }
            if let Some(Popup::Project { path, x, y }) = popup.read().clone() {
                SidebarMenu {
                    label: "Project actions".to_string(),
                    at: (x, y),
                    on_close: Callback::new(move |_| popup.set(None)),
                    entries: project_entries(&path, &projects, on_switch, on_reveal, on_reveal_project, on_remove),
                }
            }
            if github_open() {
                GithubPanel {
                    repos: repo_list,
                    loading: repos_loading(),
                    cloning: cloning.read().clone(),
                    filter: filter.read().clone(),
                    now,
                    on_filter: Callback::new(move |value: String| filter.set(value)),
                    on_clone: Callback::new(move |name: String| {
                        if cloning.read().is_some() {
                            return;
                        }
                        cloning.set(Some(name.clone()));
                        spawn_local(async move {
                            match api::github_clone(&name).await {
                                Ok(Some(_)) => {
                                    cloning.set(None);
                                    github_open.set(false);
                                    filter.set(String::new());
                                    on_github_clone.call(name);
                                }
                                _ => cloning.set(None),
                            }
                        });
                    }),
                    on_close: Callback::new(move |_| {
                        if cloning.read().is_some() {
                            return;
                        }
                        github_open.set(false);
                        filter.set(String::new());
                    }),
                }
            }
            div { class: "sidebar-scroll",
                div { class: "project-chats-head project-tree-head",
                    span { class: "project-chats-label project-tree-label",
                        Star { size: 12.0 }
                        "Projects"
                    }
                    button {
                        class: "btn btn-quiet btn-compact",
                        r#type: "button",
                        title: "Open a folder or clone a repository",
                        aria_label: "Add a project",
                        aria_haspopup: "menu",
                        aria_expanded: popup.read().as_ref().is_some_and(|item| matches!(item, Popup::Switcher { .. })),
                        onclick: move |event| {
                            let point = event.client_coordinates();
                            popup.set(Some(Popup::Switcher { x: point.x, y: point.y }));
                        },
                        "+"
                    }
                }
                div { class: "project-tree",
                    for entry in tree {
                        {
                            let path = entry.path.clone();
                            let folded = folds.read().get(&path).copied().unwrap_or(!entry.active && entry.worktrees.as_ref().map(|list| list.is_empty()).unwrap_or(true));
                            let name = folder_name(&path);
                            let toggle_path = path.clone();
                            let open_path = path.clone();
                            let menu_path = path.clone();
                            rsx! {
                                div {
                                    class: if entry.active { "project-group is-open" } else { "project-group" },
                                    key: "{path}",
                                    role: "group",
                                    aria_label: "{name}",
                                    div {
                                        class: if entry.active { "project-group-head is-open" } else { "project-group-head" },
                                        oncontextmenu: move |event| {
                                            event.prevent_default();
                                            let point = event.client_coordinates();
                                            popup.set(Some(Popup::Project { path: menu_path.clone(), x: point.x, y: point.y }));
                                        },
                                        button {
                                            class: "project-group-fold",
                                            r#type: "button",
                                            aria_expanded: "{!folded}",
                                            aria_label: if folded { format!("Show {name}") } else { format!("Fold {name}") },
                                            onclick: move |_| toggle_fold(&toggle_path, &mut folds),
                                            if folded { "▸" } else { "▾" }
                                        }
                                        button {
                                            class: "project-group-name",
                                            r#type: "button",
                                            title: if entry.active { entry.path.clone() } else { format!("Open {name}") },
                                            aria_current: if entry.active { "true" } else { "" },
                                            onclick: move |_| {
                                                if entry.active {
                                                    toggle_fold(&open_path, &mut folds);
                                                } else {
                                                    on_switch.call((open_path.clone(), None));
                                                }
                                            },
                                            FolderGlyph {}
                                            span { class: "project-row-body",
                                                span { class: "project-name", "{name}" }
                                                span { class: "project-row-meta",
                                                    if !entry.is_repo {
                                                        span { class: "project-note", "not a git repository" }
                                                    } else if let Some(branch) = entry.branch.clone() {
                                                        span { class: "project-branch", "{branch}" }
                                                    } else {
                                                        span { class: "project-note", "no commits yet" }
                                                    }
                                                    if (entry.active && busy) || entry.running > 0 {
                                                        span { class: "status status-running project-row-run",
                                                            span { class: "status-dot", aria_hidden: "true" }
                                                            if entry.running > 1 { "{entry.running} running" } else { "running" }
                                                        }
                                                    }
                                                }
                                                if entry.active {
                                                    ProjectStatus { entry: entry.clone() }
                                                }
                                            }
                                        }
                                    }
                                    if !folded && entry.active && entry.is_repo {
                                        WorktreesSection {
                                            project: entry.path.clone(),
                                            branch: entry.branch.clone(),
                                            changes: changes.clone(),
                                            jobs: jobs.clone(),
                                            terminals: terminals.clone(),
                                            activity: activity.clone(),
                                            options: option_value,
                                            filter: needle.clone(),
                                            selected_worktrees: selected_worktrees.clone(),
                                            claude_available,
                                            cursor_available,
                                            now,
                                            on_select: on_select_worktree,
                                            on_select_all: on_select_all_worktrees,
                                            on_create: on_create_worktree,
                                            on_rename: on_rename_worktree,
                                            on_discard: on_discard_worktree,
                                            on_open: on_open_in_worktree,
                                            on_reveal,
                                        }
                                    } else if !folded && !entry.active && entry.is_repo && entry.branch.is_some() {
                                        OtherWorktrees {
                                            entry: entry.clone(),
                                            filter: needle.clone(),
                                            on_open: on_switch,
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if active.is_some() {
                div { class: "project-chats sidebar-chats",
                    div { class: "project-chats-head",
                        button {
                            class: "sidebar-chats-toggle",
                            r#type: "button",
                            aria_expanded: "{chats_open()}",
                            aria_controls: "sidebar-chats",
                            onclick: move |_| {
                                let next = !chats_open();
                                chats_open.set(next);
                                write_json("legate.sidebar.chatsOpen", &Value::Bool(next));
                            },
                            span { class: "sidebar-chats-chevron", aria_hidden: "true",
                                if chats_open() { "▾" } else { "▸" }
                            }
                            span { class: "project-chats-label", "Chats" }
                        }
                        button {
                            class: "btn btn-quiet btn-compact",
                            r#type: "button",
                            disabled: lead_busy || !chats.iter().any(|chat| chat.active),
                            onclick: move |_| on_new_chat.call(()),
                            "New chat"
                        }
                    }
                    if chats_open() {
                        ul { class: "project-chat-list", id: "sidebar-chats",
                            for chat in shown_chats {
                                ChatRow {
                                    key: "{chat.id}",
                                    chat: chat.clone(),
                                    lead_busy,
                                    now,
                                    on_open: on_open_chat,
                                    on_rename: on_rename_chat,
                                    on_remove: on_remove_chat,
                                }
                            }
                        }
                    }
                }
            }
            if let Some(text) = error {
                p { class: "error-text project-error", role: "alert", "{text}" }
            }
            div { class: "project-foot",
                button {
                    class: "project-foot-button",
                    r#type: "button",
                    title: "Settings (Ctrl+,)",
                    onclick: move |_| on_open_settings.call(()),
                    svg {
                        width: "14",
                        height: "14",
                        view_box: "0 0 16 16",
                        path {
                            fill: "currentColor",
                            d: "M7 1h2l.4 1.9 1.3.6 1.7-1 1.4 1.4-1 1.7.6 1.3L15 7v2l-1.9.4-.6 1.3 1 1.7-1.4 1.4-1.7-1-1.3.6L9 15H7l-.4-1.9-1.3-.6-1.7 1-1.4-1.4 1-1.7-.6-1.3L1 9V7l1.9-.4.6-1.3-1-1.7 1.4-1.4 1.7 1 1.3-.6zm1 4.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5z",
                        }
                    }
                    span { class: "project-foot-label", "Settings" }
                }
            }
        }
    }
}

#[derive(Clone)]
enum Popup {
    Options { x: f64, y: f64 },
    Switcher { x: f64, y: f64 },
    Project { path: String, x: f64, y: f64 },
}

#[component]
fn GithubPanel(
    repos: Vec<legate_core::GithubRepo>,
    loading: bool,
    cloning: Option<String>,
    filter: String,
    now: i64,
    on_filter: Callback<String>,
    on_clone: Callback<String>,
    on_close: Callback,
) -> Element {
    let needle = filter.trim().to_lowercase();
    let shown: Vec<_> = repos
        .into_iter()
        .filter(|repo| {
            needle.is_empty()
                || repo.name_with_owner.to_lowercase().contains(&needle)
                || repo.description.to_lowercase().contains(&needle)
        })
        .collect();
    rsx! {
        div {
            class: "github-panel",
            role: "region",
            aria_label: "Clone from GitHub",
            onkeydown: move |event| {
                if event.key().to_string() == "Escape" {
                    event.stop_propagation();
                    on_close.call(());
                }
            },
            div { class: "github-panel-head",
                label { class: "github-filter-label", r#for: "github-filter", "Filter" }
                input {
                    id: "github-filter",
                    class: "github-filter",
                    r#type: "search",
                    value: "{filter}",
                    placeholder: "Filter repos",
                    disabled: cloning.is_some(),
                    oninput: move |event| on_filter.call(event.value()),
                }
                button {
                    class: "btn btn-quiet btn-compact",
                    r#type: "button",
                    disabled: cloning.is_some(),
                    onclick: move |_| on_close.call(()),
                    "Close"
                }
            }
            if loading {
                p { class: "github-status", "Loading…" }
            } else if shown.is_empty() {
                p { class: "github-status", "No repositories" }
            } else {
                ul { class: "github-list",
                    for repo in shown {
                        {
                            let name = repo.name_with_owner.clone();
                            let busy = cloning.as_deref() == Some(name.as_str());
                            let age = relative_updated(&repo.updated_at, now);
                            let private = repo.is_private;
                            let clone_label = if busy {
                                format!("Cloning {name}")
                            } else if private {
                                format!("Clone {name}, private")
                            } else {
                                format!("Clone {name}")
                            };
                            rsx! {
                                li { key: "{name}",
                                    button {
                                        class: "github-row",
                                        r#type: "button",
                                        disabled: cloning.is_some(),
                                        aria_label: "{clone_label}",
                                        onclick: move |_| on_clone.call(name.clone()),
                                        span { class: "github-row-name",
                                            span { class: "github-name", "{name}" }
                                            if private { LockGlyph {} }
                                        }
                                        span { class: "github-row-meta",
                                            if busy {
                                                span { class: "github-cloning", "Cloning…" }
                                            } else if !age.is_empty() {
                                                span { class: "github-updated", "{age}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn WorktreesSection(
    project: String,
    branch: Option<String>,
    changes: Vec<ChangeSet>,
    jobs: Vec<JobRecord>,
    terminals: Vec<TerminalInfo>,
    activity: HashMap<String, i64>,
    options: SidebarOptions,
    filter: String,
    selected_worktrees: Vec<Option<String>>,
    claude_available: bool,
    cursor_available: bool,
    now: i64,
    on_select: Callback<(Option<String>, String)>,
    on_select_all: Callback,
    on_create: WorktreeCreate,
    on_rename: Callback<(String, String)>,
    on_discard: Callback<String>,
    on_open: Callback<(ProviderId, Option<String>)>,
    on_reveal: Callback<Option<String>>,
) -> Element {
    let mut marks = use_signal(|| read_marks(&project));
    let mut seen_project = use_signal(String::new);
    if seen_project() != project {
        seen_project.set(project.clone());
        marks.set(read_marks(&project));
    }
    let mut creating = use_signal(|| false);
    let mut draft = use_signal(String::new);
    let mut task = use_signal(String::new);
    let mut how_many = use_signal(|| 1i64);
    let mut start = use_signal(String::new);
    let mut menu = use_signal(|| None::<(Option<String>, f64, f64)>);
    let mut renaming = use_signal(|| None::<String>);
    let mut confirming = use_signal(|| None::<String>);
    let stored = marks.read().clone();
    let inputs = card_inputs(
        &branch,
        &changes,
        &jobs,
        &terminals,
        &selected_worktrees,
        &stored,
    );
    let cards: Vec<Card> = inputs
        .into_iter()
        .map(|input| card_status(input, &activity, now))
        .collect();
    let groups = arrange_cards(cards.clone(), options, &filter);
    let working = cards
        .iter()
        .filter(|card| card.status == CardStatus::Working)
        .count();
    let all_selected = worktree_keys(&changes)
        .iter()
        .all(|key| selected_worktrees.iter().any(|item| item == key));
    let shown_count = selected_worktrees.len();
    let label = if changes.len() == 1 {
        "1 worktree".to_string()
    } else {
        format!("{} worktrees", changes.len())
    };
    let menu_at = menu.read().clone();
    let can_task = !start.read().is_empty();

    rsx! {
        div { class: "project-worktrees",
            div { class: "project-chats-head",
                span { class: "project-chats-label",
                    "{label}"
                    if working > 0 {
                        span { class: "project-worktree-shown", "{working} working" }
                    }
                    if shown_count > 1 {
                        span { class: "project-worktree-shown", "{shown_count} shown" }
                    }
                }
                span { class: "head-actions",
                    button {
                        class: "btn btn-quiet btn-compact",
                        r#type: "button",
                        title: "Show every worktree side by side",
                        aria_pressed: if all_selected { "true" } else { "false" },
                        onclick: move |_| on_select_all.call(()),
                        "All"
                    }
                    button {
                        class: "btn btn-quiet btn-compact",
                        r#type: "button",
                        title: "New worktree",
                        aria_label: "New worktree",
                        onclick: move |_| {
                            draft.set(String::new());
                            task.set(String::new());
                            how_many.set(1);
                            start.set(String::new());
                            creating.set(true);
                        },
                        "+"
                    }
                }
            }
            if creating() {
                form {
                    class: "project-worktree-create",
                    onsubmit: move |event| {
                        event.prevent_default();
                        finish_create(true, &draft, &task, how_many(), &start, &mut creating, on_create);
                    },
                    label { class: "project-worktree-field",
                        "Name (optional)"
                        input {
                            class: "project-worktree-rename",
                            value: "{draft}",
                            aria_label: "New worktree name",
                            maxlength: "40",
                            oninput: move |event| draft.set(event.value()),
                        }
                    }
                    label { class: "project-worktree-field",
                        "Task"
                        textarea {
                            class: "project-worktree-task",
                            value: "{task}",
                            rows: "2",
                            maxlength: "8000",
                            disabled: !can_task,
                            placeholder: if can_task { "What should the agent do? (optional)" } else { "" },
                            aria_label: "Task",
                            oninput: move |event| task.set(event.value()),
                        }
                        if !can_task {
                            span { class: "project-worktree-task-hint", "Choose an agent under Start to give it a task." }
                        }
                    }
                    label { class: "project-worktree-how",
                        "How many"
                        input {
                            r#type: "number",
                            min: "1",
                            max: "6",
                            value: "{how_many}",
                            aria_label: "How many",
                            oninput: move |event| {
                                if let Ok(value) = event.value().parse::<i64>() {
                                    how_many.set(value.clamp(1, 6));
                                }
                            },
                        }
                    }
                    label { class: "project-worktree-start",
                        "Start"
                        select {
                            class: "select",
                            aria_label: "Start",
                            onchange: move |event| start.set(event.value()),
                            option { value: "", "nothing" }
                            for id in AGENTS {
                                if plan_ready(id, claude_available, cursor_available) {
                                    option { value: "{id.as_str()}", "{worker_label(id)}" }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "project-worktrees-list",
                if groups.is_empty() {
                    p { class: "project-empty", "No worktrees match." }
                }
                for group in groups {
                    {
                        let heading = group.label.unwrap_or("Worktrees").to_string();
                        rsx! {
                            div { role: "group", aria_label: "{heading}", key: "{heading}",
                                if let Some(title) = group.label {
                                    div { class: "worktree-group-label",
                                        "{title}"
                                        span { class: "worktree-group-count", "{group.cards.len()}" }
                                    }
                                }
                                ul { class: "worktree-group-list",
                                    for card in group.cards {
                                        WorktreeRow {
                                            card: card.clone(),
                                            changes: changes.clone(),
                                            terminals: terminals.clone(),
                                            compact: options.layout == CardLayout::Compact,
                                            renaming: renaming.read().clone(),
                                            confirming: confirming.read().clone(),
                                            on_select,
                                            on_rename,
                                            on_discard,
                                            on_menu: Callback::new(move |(key, x, y): (Option<String>, f64, f64)| {
                                                menu.set(Some((key, x, y)));
                                            }),
                                            on_renaming: Callback::new(move |id: Option<String>| renaming.set(id)),
                                            on_confirming: Callback::new(move |id: Option<String>| confirming.set(id)),
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if let Some((key, x, y)) = menu_at {
                SidebarMenu {
                    label: "Worktree actions".to_string(),
                    at: (x, y),
                    on_close: Callback::new(move |_| menu.set(None)),
                    entries: worktree_menu(
                        &key,
                        &project,
                        &changes,
                        &jobs,
                        &cards,
                        &stored,
                        claude_available,
                        cursor_available,
                        on_select,
                        on_open,
                        on_reveal,
                        {
                            let marks_project = project.clone();
                            Callback::new(move |next: Marks| {
                                write_marks(&marks_project, &next);
                                marks.set(next);
                            })
                        },
                        Callback::new(move |id: String| renaming.set(Some(id))),
                        Callback::new(move |id: String| confirming.set(Some(id))),
                    ),
                }
            }
        }
    }
}

#[component]
fn WorktreeRow(
    card: Card,
    changes: Vec<ChangeSet>,
    terminals: Vec<TerminalInfo>,
    compact: bool,
    renaming: Option<String>,
    confirming: Option<String>,
    on_select: Callback<(Option<String>, String)>,
    on_rename: Callback<(String, String)>,
    on_discard: Callback<String>,
    on_menu: Callback<(Option<String>, f64, f64)>,
    on_renaming: Callback<Option<String>>,
    on_confirming: Callback<Option<String>>,
) -> Element {
    let mut text = use_signal(String::new);
    let mut text_for = use_signal(|| None::<String>);
    let key = card.input.key.clone();
    let title = card.input.title.clone();
    let status = status_name(card.status);
    let item = key
        .as_ref()
        .and_then(|id| changes.iter().find(|change| &change.id == id).cloned());
    if let Some(item) = item.clone()
        && renaming.as_deref() == Some(item.id.as_str())
    {
        let current = item.name.clone().unwrap_or_default();
        let item_id = item.id.clone();
        if text_for.read().as_deref() != Some(item_id.as_str()) {
            text.set(current.clone());
            text_for.set(Some(item_id.clone()));
        }
        let blur_id = item_id.clone();
        let key_id = item_id;
        let blur_current = current;
        return rsx! {
            li { class: "project-worktree-item",
                input {
                    class: "project-worktree-rename",
                    value: "{text}",
                    aria_label: "Rename {title}",
                    oninput: move |event| {
                        let mut text = text;
                        text.set(event.value());
                    },
                    onblur: move |_| {
                        let next = text.read().trim().to_string();
                        on_renaming.call(None);
                        if !next.is_empty() && next != blur_current {
                            on_rename.call((blur_id.clone(), next));
                        }
                    },
                    onkeydown: move |event| {
                        if event.key().to_string() == "Enter" {
                            event.prevent_default();
                            let next = text.read().trim().to_string();
                            on_renaming.call(None);
                            if !next.is_empty() {
                                on_rename.call((key_id.clone(), next));
                            }
                        } else if event.key().to_string() == "Escape" {
                            event.prevent_default();
                            on_renaming.call(None);
                        }
                    },
                }
            }
        };
    }
    if let Some(item) = item.clone()
        && confirming.as_deref() == Some(item.id.as_str())
    {
        let id = item.id.clone();
        return rsx! {
            li { class: "project-worktree-item",
                div { class: "project-chat-confirm", role: "group", aria_label: "Discard {title}?",
                    span { class: "project-chat-confirm-text", "Discard this worktree?" }
                    button {
                        class: "btn btn-quiet btn-compact is-danger-label",
                        r#type: "button",
                        onclick: move |_| {
                            on_confirming.call(None);
                            on_discard.call(id.clone());
                        },
                        "Discard"
                    }
                    button {
                        class: "btn btn-quiet btn-compact",
                        r#type: "button",
                        onclick: move |_| on_confirming.call(None),
                        "Cancel"
                    }
                }
            }
        };
    }
    let agents: Vec<TerminalInfo> = terminals
        .iter()
        .filter(|info| {
            info.change.as_ref() == key.as_ref() && info.status == TerminalStatus::Running
        })
        .cloned()
        .collect();
    let open = terminals
        .iter()
        .filter(|info| info.change.as_ref() == key.as_ref())
        .count();
    let selected = card.input.selected;
    let id_class = key.is_some() && title == key.clone().unwrap_or_default();
    let menu_key = key.clone();
    rsx! {
        li {
            class: if compact {
                format!("project-worktree-item is-{status} is-compact")
            } else {
                format!("project-worktree-item is-{status}")
            },
            oncontextmenu: move |event| {
                event.prevent_default();
                let point = event.client_coordinates();
                on_menu.call((menu_key.clone(), point.x, point.y));
            },
            button {
                class: if selected { "project-worktree-row is-selected" } else { "project-worktree-row" },
                r#type: "button",
                title: if key.is_none() { "Project folder".to_string() } else { title.clone() },
                aria_current: if selected { "true" } else { "" },
                onclick: move |event| {
                    let mods = event.modifiers();
                    let mode = if mods.shift() {
                        "range"
                    } else if mods.ctrl() || mods.meta() {
                        "toggle"
                    } else {
                        "only"
                    };
                    on_select.call((key.clone(), mode.into()));
                },
                span {
                    class: "worktree-dot is-{status}",
                    role: "img",
                    aria_label: "{status_text(card.status)}",
                    title: "{status_text(card.status)}",
                }
                span { class: "project-worktree-body",
                    span { class: "project-worktree-line",
                        span { class: if id_class { "project-worktree-id" } else { "project-worktree-name" }, "{title}" }
                        if key.is_none() {
                            span { class: "worktree-primary", "primary" }
                        }
                        if card.input.pinned {
                            span { class: "worktree-pin", aria_label: "pinned", title: "Pinned", "▲" }
                        }
                        if !agents.is_empty() {
                            span { class: "worktree-agents", aria_label: "{agents.len()} agents open",
                                for info in agents.iter().take(3).cloned() {
                                    if info.provider == TerminalProvider::Shell {
                                        span { class: "worktree-shell", aria_hidden: "true", key: "{info.id}", "$" }
                                    } else if let Some(provider) = provider_of(info.provider) {
                                        BotAvatar {
                                            key: "{info.id}",
                                            bot: BotName::Plan(provider),
                                            state: BotState::Working,
                                            size: 14.0,
                                            title: None,
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if !compact {
                        span { class: "project-worktree-meta",
                            if let Some(branch) = card.input.branch.clone().filter(|_branch| key.is_some()) {
                                span { class: "project-worktree-branch", "{branch}" }
                            }
                            if let Some(item) = item {
                                if item.setup == Some(legate_core::SetupState::Running) {
                                    span { "setting up…" }
                                } else if item.setup == Some(legate_core::SetupState::Failed) {
                                    span { class: "setup-failed", "setup failed" }
                                }
                                if item.files.is_empty() {
                                    span { "no changes yet" }
                                } else {
                                    span {
                                        span { class: "stat-add", "+{item.insertions}" }
                                        " "
                                        span { class: "stat-del", "−{item.deletions}" }
                                    }
                                }
                            }
                            if open > 0 {
                                span { class: "project-worktree-terms",
                                    if open == 1 { "1 terminal" } else { "{open} terminals" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn OtherWorktrees(
    entry: ProjectEntry,
    filter: String,
    on_open: Callback<(String, Option<String>)>,
) -> Element {
    let needle = filter.trim().to_lowercase();
    let worktrees: Vec<ProjectWorktree> = entry
        .worktrees
        .clone()
        .unwrap_or_default()
        .into_iter()
        .filter(|item| {
            needle.is_empty()
                || item.name.to_lowercase().contains(&needle)
                || item.id.to_lowercase().contains(&needle)
        })
        .collect();
    let path = entry.path.clone();
    let branch = entry
        .branch
        .clone()
        .unwrap_or_else(|| "project folder".into());
    rsx! {
        ul { class: "worktree-group-list project-other-list",
            li { class: "project-worktree-item is-idle is-compact",
                button {
                    class: "project-worktree-row",
                    r#type: "button",
                    title: "Open {folder_name(&path)} on its project folder",
                    onclick: move |_| on_open.call((path.clone(), None)),
                    span { class: "worktree-dot is-idle", aria_hidden: "true" }
                    span { class: "project-worktree-body",
                        span { class: "project-worktree-line",
                            span { class: "project-worktree-name", "{branch}" }
                            span { class: "worktree-primary", "primary" }
                        }
                    }
                }
            }
            for item in worktrees {
                {
                    let path = entry.path.clone();
                    let id = item.id.clone();
                    let status = if item.working { "working" } else { "idle" };
                    let name = if item.name.is_empty() { item.id.clone() } else { item.name.clone() };
                    rsx! {
                        li { class: "project-worktree-item is-{status}", key: "{id}",
                            button {
                                class: "project-worktree-row",
                                r#type: "button",
                                onclick: move |_| on_open.call((path.clone(), Some(id.clone()))),
                                span { class: "worktree-dot is-{status}", aria_hidden: "true" }
                                span { class: "project-worktree-body",
                                    span { class: "project-worktree-line",
                                        span { class: if item.name.is_empty() { "project-worktree-id" } else { "project-worktree-name" }, "{name}" }
                                    }
                                    span { class: "project-worktree-meta",
                                        span { class: "project-worktree-branch", "{item.branch}" }
                                        if item.files > 0 {
                                            span {
                                                span { class: "stat-add", "+{item.insertions}" }
                                                " "
                                                span { class: "stat-del", "−{item.deletions}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ChatRow(
    chat: LeadChat,
    lead_busy: bool,
    now: i64,
    on_open: Callback<String>,
    on_rename: Callback<(String, String)>,
    on_remove: Callback<String>,
) -> Element {
    let mut mode = use_signal(|| "idle");
    let draft = use_signal(|| chat.title.clone());
    let title = chat.title.clone();
    let id = chat.id.clone();
    if mode() == "confirm" {
        return rsx! {
            li { class: "project-chat-item",
                div { class: "project-chat-confirm", role: "group", aria_label: "Delete {title}?",
                    span { class: "project-chat-confirm-text", "Delete this chat?" }
                    button {
                        class: "btn btn-quiet btn-compact is-danger-label",
                        r#type: "button",
                        onclick: move |_| {
                            mode.set("idle");
                            on_remove.call(id.clone());
                        },
                        "Delete"
                    }
                    button {
                        class: "btn btn-quiet btn-compact",
                        r#type: "button",
                        onclick: move |_| mode.set("idle"),
                        "Cancel"
                    }
                }
            }
        };
    }
    if mode() == "rename" {
        let label = title.clone();
        let blur_id = id.clone();
        let blur_title = title.clone();
        let key_id = id.clone();
        let key_title = title.clone();
        let escape_title = title;
        return rsx! {
            li { class: "project-chat-item",
                input {
                    class: "project-chat-rename",
                    value: "{draft}",
                    aria_label: "Rename {label}",
                    maxlength: "80",
                    oninput: move |event| {
                        let mut draft = draft;
                        draft.set(event.value());
                    },
                    onblur: move |_| {
                        let mut mode = mode;
                        save_rename(&blur_id, &blur_title, &draft, &mut mode, on_rename);
                    },
                    onkeydown: move |event| {
                        if event.key().to_string() == "Enter" {
                            event.prevent_default();
                            let mut mode = mode;
                            save_rename(&key_id, &key_title, &draft, &mut mode, on_rename);
                        } else if event.key().to_string() == "Escape" {
                            event.prevent_default();
                            let mut draft = draft;
                            draft.set(escape_title.clone());
                            let mut mode = mode;
                            mode.set("idle");
                        }
                    },
                }
            }
        };
    }
    let open_id = id.clone();
    rsx! {
        li { class: "project-chat-item",
            button {
                class: if chat.active { "project-chat-row is-selected" } else { "project-chat-row" },
                r#type: "button",
                title: "{title}",
                aria_current: if chat.active { "true" } else { "" },
                onclick: move |_| on_open.call(open_id.clone()),
                span { class: "project-chat-title", "{title}" }
                span { class: "project-chat-meta",
                    if lead_busy && chat.active {
                        span { class: "status status-running project-chat-run",
                            span { class: "status-dot", aria_hidden: "true" }
                        }
                    }
                    span { class: "project-chat-age", "{relative_age(chat.updated_at, now)}" }
                }
            }
            div { class: "project-chat-actions",
                button {
                    class: "project-chat-action",
                    r#type: "button",
                    aria_label: "Rename {title}",
                    onclick: move |_| mode.set("rename"),
                    "Rename"
                }
                button {
                    class: "project-chat-action",
                    r#type: "button",
                    aria_label: "Delete {title}",
                    onclick: move |_| mode.set("confirm"),
                    "Delete"
                }
            }
        }
    }
}

#[component]
fn ProjectStatus(entry: ProjectEntry) -> Element {
    let upstream = entry.ahead > 0 || entry.behind > 0;
    let lines = entry.insertions > 0 || entry.deletions > 0;
    if !entry.is_repo || (!upstream && entry.dirty == 0 && !lines) {
        return rsx! {};
    }
    let mut said = Vec::new();
    if entry.ahead > 0 {
        said.push(format!("{} ahead", entry.ahead));
    }
    if entry.behind > 0 {
        said.push(format!("{} behind", entry.behind));
    }
    if entry.dirty > 0 {
        said.push(format!("{} uncommitted", entry.dirty));
    }
    if lines {
        said.push(format!(
            "{} lines added, {} removed",
            entry.insertions, entry.deletions
        ));
    }
    let label = said.join(", ");
    rsx! {
        span { class: "project-status", aria_label: "{label}",
            if upstream {
                span { class: "project-status-item", title: "Commits ahead of and behind upstream",
                    if entry.ahead > 0 { "↑{entry.ahead}" }
                    if entry.ahead > 0 && entry.behind > 0 { " " }
                    if entry.behind > 0 { "↓{entry.behind}" }
                }
            }
            if entry.dirty > 0 {
                span { class: "project-status-item", title: "Uncommitted files in the project folder", "{entry.dirty} uncommitted" }
            }
            if lines {
                span { class: "project-status-item", title: "Lines in changes waiting for review",
                    span { class: "stat-add", "+{entry.insertions}" }
                    " "
                    span { class: "stat-del", "−{entry.deletions}" }
                }
            }
        }
    }
}

#[component]
fn FolderGlyph() -> Element {
    rsx! {
        svg {
            class: "project-folder",
            width: "14",
            height: "14",
            view_box: "0 0 16 16",
            path { fill: "currentColor", d: "M1.5 3.5h4.2l1.3 1.5H14.5v7.5H1.5V3.5zm1 1v7h11v-5.5H6.6L5.3 4.5H2.5z" }
        }
    }
}

#[component]
fn LockGlyph() -> Element {
    rsx! {
        svg {
            class: "github-lock",
            width: "10",
            height: "12",
            view_box: "0 0 10 12",
            path { fill: "currentColor", d: "M5 0a3 3 0 0 0-3 3v2H1v7h8V5H8V3a3 3 0 0 0-3-3zm-2 3a2 2 0 1 1 4 0v2H3V3zm1 5h2v2H4V8z" }
        }
    }
}

#[derive(Clone)]
struct Marks {
    pinned: Vec<String>,
    seen: HashMap<String, i64>,
}

fn card_inputs(
    branch: &Option<String>,
    changes: &[ChangeSet],
    jobs: &[JobRecord],
    terminals: &[TerminalInfo],
    selected: &[Option<String>],
    marks: &Marks,
) -> Vec<CardInput> {
    let mut inputs = vec![CardInput {
        key: None,
        title: branch.clone().unwrap_or_else(|| "HEAD".into()),
        branch: None,
        terminal_ids: terminals
            .iter()
            .filter(|info| info.change.is_none())
            .map(|info| info.id.clone())
            .collect(),
        job_running: job_running(jobs, None),
        has_changes: false,
        created_index: 0,
        pinned: marks.pinned.iter().any(|id| id.is_empty()),
        seen_at: marks.seen.get("").copied().unwrap_or(0),
        selected: selected.iter().any(|item| item.is_none()),
    }];
    for (index, change) in changes.iter().enumerate() {
        let named = NamedChange {
            id: change.id.clone(),
            name: change.name.clone(),
        };
        inputs.push(CardInput {
            key: Some(change.id.clone()),
            title: worktree_label::worktree_label(&named),
            branch: Some(change.branch.clone()),
            terminal_ids: terminals
                .iter()
                .filter(|info| info.change.as_deref() == Some(change.id.as_str()))
                .map(|info| info.id.clone())
                .collect(),
            job_running: job_running(jobs, Some(change.id.as_str())),
            has_changes: !change.files.is_empty(),
            created_index: index as i64 + 1,
            pinned: marks.pinned.iter().any(|id| id == &change.id),
            seen_at: marks.seen.get(&change.id).copied().unwrap_or(0),
            selected: selected
                .iter()
                .any(|item| item.as_deref() == Some(change.id.as_str())),
        });
    }
    inputs
}

fn job_running(jobs: &[JobRecord], id: Option<&str>) -> bool {
    jobs.iter().any(|job| {
        job.change.as_deref() == id && matches!(job.status, JobStatus::Queued | JobStatus::Running)
    })
}

fn worktree_keys(changes: &[ChangeSet]) -> Vec<Option<String>> {
    let mut keys = vec![None];
    keys.extend(changes.iter().map(|change| Some(change.id.clone())));
    keys
}

fn status_name(status: CardStatus) -> &'static str {
    match status {
        CardStatus::Working => "working",
        CardStatus::Unread => "unread",
        CardStatus::Idle => "idle",
    }
}

fn status_text(status: CardStatus) -> &'static str {
    match status {
        CardStatus::Working => "agent working",
        CardStatus::Unread => "new output since you looked",
        CardStatus::Idle => "quiet",
    }
}

fn worker_label(id: ProviderId) -> &'static str {
    match id {
        ProviderId::Claude => "Claude",
        ProviderId::Grok => "Grok",
        ProviderId::Cursor | ProviderId::Local => "Cursor",
    }
}

fn plan_ready(id: ProviderId, claude: bool, cursor: bool) -> bool {
    if plan_of(id) == ProviderId::Claude {
        claude
    } else {
        cursor
    }
}

fn provider_of(provider: TerminalProvider) -> Option<ProviderId> {
    Some(match provider {
        TerminalProvider::Claude => ProviderId::Claude,
        TerminalProvider::Cursor => ProviderId::Cursor,
        TerminalProvider::Grok => ProviderId::Grok,
        TerminalProvider::Local => ProviderId::Local,
        TerminalProvider::Shell => return None,
    })
}

fn folder_name(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_string()
}

fn relative_age(updated_at: i64, now: i64) -> String {
    let minutes = ((now - updated_at).max(0)) / 60_000;
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h");
    }
    format!("{}d", hours / 24)
}

fn relative_updated(iso: &str, now: i64) -> String {
    let Ok(parsed) =
        time::OffsetDateTime::parse(iso, &time::format_description::well_known::Rfc3339)
    else {
        return String::new();
    };
    relative_age(parsed.unix_timestamp() * 1000, now)
}

fn toggle_fold(path: &str, folds: &mut Signal<HashMap<String, bool>>) {
    let mut next = folds.read().clone();
    let current = next.get(path).copied().unwrap_or(false);
    next.insert(path.to_string(), !current);
    write_json(
        "legate.sidebar.folds",
        &serde_json::to_value(&next).unwrap_or(Value::Null),
    );
    folds.set(next);
}

fn save_rename(
    id: &str,
    title: &str,
    draft: &Signal<String>,
    mode: &mut Signal<&'static str>,
    on_rename: Callback<(String, String)>,
) {
    let next = draft.read().trim().to_string();
    mode.set("idle");
    if next.is_empty() || next == title {
        return;
    }
    on_rename.call((id.to_string(), next));
}

fn finish_create(
    save: bool,
    draft: &Signal<String>,
    task: &Signal<String>,
    how_many: i64,
    start: &Signal<String>,
    creating: &mut Signal<bool>,
    on_create: WorktreeCreate,
) {
    let name = draft.read().trim().to_string();
    let task_text = task.read().trim().to_string();
    let agent = ProviderId::parse(start.read().as_str());
    let named = if !name.is_empty() {
        Some(name)
    } else if !task_text.is_empty() {
        let from_task = worktree_label::worktree_name_from_task(&task_text);
        if from_task.is_empty() {
            None
        } else {
            Some(from_task)
        }
    } else {
        None
    };
    let prompt = if agent.is_some() && !task_text.is_empty() {
        Some(task_text)
    } else {
        None
    };
    creating.set(false);
    if save {
        on_create.call((named, how_many.clamp(1, 6), agent, prompt));
    }
}

fn option_entries(
    options: SidebarOptions,
    set: impl Fn(SidebarOptions) + Copy + 'static,
) -> Vec<MenuEntry> {
    let group = MenuEntry::heading("Group by");
    let mut sort = MenuEntry::heading("Sort by");
    sort.separator = true;
    let mut cards = MenuEntry::heading("Cards");
    cards.separator = true;
    let mut hide = MenuEntry::check(
        "Hide quiet worktrees",
        options.hide_clean,
        Callback::new(move |_| {
            let mut next = options;
            next.hide_clean = !options.hide_clean;
            set(next);
        }),
    );
    hide.separator = true;
    hide.title = "Hide worktrees with no changes, no terminals and nothing happening".into();
    vec![
        group,
        MenuEntry::check(
            "Nothing",
            options.group_by == GroupBy::None,
            Callback::new(move |_| {
                let mut next = options;
                next.group_by = GroupBy::None;
                set(next);
            }),
        ),
        MenuEntry::check(
            "Status",
            options.group_by == GroupBy::Status,
            Callback::new(move |_| {
                let mut next = options;
                next.group_by = GroupBy::Status;
                set(next);
            }),
        ),
        sort,
        MenuEntry::check(
            "Created",
            options.sort_by == SortBy::Created,
            Callback::new(move |_| {
                let mut next = options;
                next.sort_by = SortBy::Created;
                set(next);
            }),
        ),
        MenuEntry::check(
            "Recent activity",
            options.sort_by == SortBy::Recent,
            Callback::new(move |_| {
                let mut next = options;
                next.sort_by = SortBy::Recent;
                set(next);
            }),
        ),
        MenuEntry::check(
            "Name",
            options.sort_by == SortBy::Name,
            Callback::new(move |_| {
                let mut next = options;
                next.sort_by = SortBy::Name;
                set(next);
            }),
        ),
        cards,
        MenuEntry::check(
            "Detailed",
            options.layout == CardLayout::Detailed,
            Callback::new(move |_| {
                let mut next = options;
                next.layout = CardLayout::Detailed;
                set(next);
            }),
        ),
        MenuEntry::check(
            "Compact",
            options.layout == CardLayout::Compact,
            Callback::new(move |_| {
                let mut next = options;
                next.layout = CardLayout::Compact;
                set(next);
            }),
        ),
        hide,
    ]
}

fn switcher_entries(
    projects: &[ProjectEntry],
    github: bool,
    has_active: bool,
    on_switch: Callback<(String, Option<String>)>,
    on_add: Callback,
    on_reveal_project: Callback<String>,
    on_github: impl Fn() + Copy + 'static,
) -> Vec<MenuEntry> {
    let mut entries = Vec::new();
    for entry in projects {
        let path = entry.path.clone();
        let active = entry.active;
        let item = MenuEntry::check(
            project_choice(entry),
            active,
            Callback::new(move |_| {
                if !active {
                    on_switch.call((path.clone(), None));
                }
            }),
        );
        entries.push(item);
    }
    let mut open = MenuEntry::item("Open folder…", on_add);
    open.separator = !projects.is_empty();
    entries.push(open);
    if github {
        entries.push(MenuEntry::item(
            "Clone from GitHub…",
            Callback::new(move |_| on_github()),
        ));
    }
    let active = projects
        .iter()
        .find(|entry| entry.active)
        .map(|entry| entry.path.clone());
    let mut reveal = MenuEntry::item(
        "Reveal in file manager",
        Callback::new(move |_| {
            if let Some(path) = active.clone() {
                on_reveal_project.call(path);
            }
        }),
    );
    reveal.disabled = !has_active;
    entries.push(reveal);
    entries
}

fn project_choice(entry: &ProjectEntry) -> String {
    let name = folder_name(&entry.path);
    if entry.active {
        return name;
    }
    let mut extra = Vec::new();
    if entry.running > 0 {
        extra.push(format!("{} running", entry.running));
    }
    if entry.changes > 0 {
        extra.push(format!("{} changes", entry.changes));
    }
    if extra.is_empty() {
        name
    } else {
        format!("{name} · {}", extra.join(" · "))
    }
}

fn project_entries(
    path: &str,
    projects: &[ProjectEntry],
    on_switch: Callback<(String, Option<String>)>,
    on_reveal: Callback<Option<String>>,
    on_reveal_project: Callback<String>,
    on_remove: Callback<String>,
) -> Vec<MenuEntry> {
    let active = projects
        .iter()
        .any(|entry| entry.path == path && entry.active);
    let path_owned = path.to_string();
    let mut entries = Vec::new();
    if active {
        entries.push(MenuEntry::item(
            "Open folder",
            Callback::new(move |_| on_reveal.call(None)),
        ));
    } else {
        let switch_path = path_owned.clone();
        let reveal_path = path_owned.clone();
        entries.push(MenuEntry::item(
            "Switch to this project",
            Callback::new(move |_| {
                on_switch.call((switch_path.clone(), None));
            }),
        ));
        entries.push(MenuEntry::item(
            "Open folder",
            Callback::new(move |_| on_reveal_project.call(reveal_path.clone())),
        ));
    }
    let copy_path = path_owned.clone();
    entries.push(MenuEntry::item(
        "Copy path",
        Callback::new(move |_| {
            let copy_path = copy_path.clone();
            spawn_local(async move {
                let _ = copy_text(&copy_path).await;
            });
        }),
    ));
    let remove_path = path_owned;
    let mut remove = MenuEntry::item(
        "Remove from list",
        Callback::new(move |_| on_remove.call(remove_path.clone())),
    );
    remove.separator = true;
    remove.danger = true;
    remove.disabled = active;
    remove.title = if active {
        "Switch to another project first".into()
    } else {
        "Removes it from this list. The folder is not deleted.".into()
    };
    entries.push(remove);
    entries
}

#[allow(clippy::too_many_arguments)]
fn worktree_menu(
    key: &Option<String>,
    project: &str,
    changes: &[ChangeSet],
    jobs: &[JobRecord],
    cards: &[Card],
    marks: &Marks,
    claude: bool,
    cursor: bool,
    on_select: Callback<(Option<String>, String)>,
    on_open: Callback<(ProviderId, Option<String>)>,
    on_reveal: Callback<Option<String>>,
    save: Callback<Marks>,
    rename: Callback<String>,
    confirm: Callback<String>,
) -> Vec<MenuEntry> {
    let item = key
        .as_ref()
        .and_then(|id| changes.iter().find(|change| &change.id == id));
    let path = item
        .map(|change| change.path.clone())
        .unwrap_or_else(|| project.to_string());
    let mark = key.clone().unwrap_or_default();
    let owned = marks.clone();
    let pinned = owned.pinned.iter().any(|id| id == &mark);
    let last_active = cards
        .iter()
        .find(|card| &card.input.key == key)
        .map(|card| card.last_active)
        .unwrap_or(0);
    let unread = cards
        .iter()
        .find(|card| &card.input.key == key)
        .is_some_and(|card| card.status == CardStatus::Unread);
    let blocked = key.is_some() && job_running(jobs, key.as_deref());
    let mut entries = Vec::new();
    for id in AGENTS {
        let ready = plan_ready(id, claude, cursor);
        let open_key = key.clone();
        let mut row = MenuEntry::item(
            format!("Open {} here", worker_label(id)),
            Callback::new(move |_| {
                on_select.call((open_key.clone(), "only".into()));
                on_open.call((id, open_key.clone()));
            }),
        );
        row.disabled = !ready;
        entries.push(row);
    }
    let pin_key = mark.clone();
    let mut pin = MenuEntry::item(
        if pinned { "Unpin" } else { "Pin to top" },
        Callback::new({
            let owned = owned.clone();
            move |_| {
                let mut next = owned.clone();
                if pinned {
                    next.pinned.retain(|id| id != &pin_key);
                } else {
                    next.pinned.push(pin_key.clone());
                }
                save.call(next);
            }
        }),
    );
    pin.separator = true;
    entries.push(pin);
    let seen_key = mark.clone();
    if unread {
        entries.push(MenuEntry::item(
            "Mark as read",
            Callback::new({
                let owned = owned.clone();
                move |_| {
                    let mut next = owned.clone();
                    next.seen.insert(seen_key.clone(), last_active);
                    save.call(next);
                }
            }),
        ));
    } else {
        let mut unread_row = MenuEntry::item(
            "Mark as unread",
            Callback::new({
                let owned = owned.clone();
                move |_| {
                    let mut next = owned.clone();
                    next.seen.insert(seen_key.clone(), 0);
                    save.call(next);
                }
            }),
        );
        unread_row.disabled = last_active == 0;
        entries.push(unread_row);
    }
    let reveal_key = key.clone();
    let mut reveal = MenuEntry::item(
        "Open folder",
        Callback::new(move |_| on_reveal.call(reveal_key.clone())),
    );
    reveal.separator = true;
    entries.push(reveal);
    let copy = path.clone();
    entries.push(MenuEntry::item(
        "Copy path",
        Callback::new(move |_| {
            let copy = copy.clone();
            spawn_local(async move {
                let _ = copy_text(&copy).await;
            });
        }),
    ));
    if let Some(change) = item {
        let branch = change.branch.clone();
        entries.push(MenuEntry::item(
            "Copy branch name",
            Callback::new(move |_| {
                let branch = branch.clone();
                spawn_local(async move {
                    let _ = copy_text(&branch).await;
                });
            }),
        ));
        let rename_id = change.id.clone();
        let mut rename_row = MenuEntry::item(
            "Rename…",
            Callback::new(move |_| rename.call(rename_id.clone())),
        );
        rename_row.separator = true;
        entries.push(rename_row);
        let confirm_id = change.id.clone();
        let mut discard = MenuEntry::item(
            if blocked {
                "Discard (a job is running)"
            } else {
                "Discard worktree…"
            },
            Callback::new(move |_| confirm.call(confirm_id.clone())),
        );
        discard.danger = true;
        discard.disabled = blocked;
        entries.push(discard);
    }
    entries
}

fn options_json(options: SidebarOptions) -> Value {
    json!({
        "groupBy": match options.group_by { GroupBy::None => "none", GroupBy::Status => "status" },
        "sortBy": match options.sort_by { SortBy::Created => "created", SortBy::Recent => "recent", SortBy::Name => "name" },
        "layout": match options.layout { CardLayout::Detailed => "detailed", CardLayout::Compact => "compact" },
        "hideClean": options.hide_clean,
    })
}

fn stored_options() -> SidebarOptions {
    read_json("legate.sidebar.options")
        .map(|value| read_options(&value))
        .unwrap_or(DEFAULT_OPTIONS)
}

fn stored_folds() -> HashMap<String, bool> {
    let Some(value) = read_json("legate.sidebar.folds") else {
        return HashMap::new();
    };
    let Some(object) = value.as_object() else {
        return HashMap::new();
    };
    object
        .iter()
        .filter_map(|(key, value)| value.as_bool().map(|on| (key.clone(), on)))
        .collect()
}

fn stored_bool(key: &str, default: bool) -> bool {
    read_json(key)
        .and_then(|value| value.as_bool())
        .unwrap_or(default)
}

fn read_marks(project: &str) -> Marks {
    let Some(value) = read_json(&format!("legate.sidebar.marks:{project}")) else {
        return Marks {
            pinned: Vec::new(),
            seen: HashMap::new(),
        };
    };
    let pinned = value
        .get("pinned")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let mut seen = HashMap::new();
    if let Some(object) = value.get("seen").and_then(Value::as_object) {
        for (key, at) in object {
            if let Some(number) = at.as_i64() {
                seen.insert(key.clone(), number);
            } else if let Some(number) = at.as_f64() {
                seen.insert(key.clone(), number as i64);
            }
        }
    }
    Marks { pinned, seen }
}

fn write_marks(project: &str, marks: &Marks) {
    let seen = marks
        .seen
        .iter()
        .map(|(key, value)| (key.clone(), Value::from(*value)))
        .collect::<serde_json::Map<_, _>>();
    write_json(
        &format!("legate.sidebar.marks:{project}"),
        &json!({ "pinned": marks.pinned, "seen": seen }),
    );
}

fn read_json(key: &str) -> Option<Value> {
    let storage = web_sys::window()?.local_storage().ok()??;
    let raw = storage.get_item(key).ok()??;
    serde_json::from_str(&raw).ok()
}

fn write_json(key: &str, value: &Value) {
    let Some(storage) = web_sys::window().and_then(|window| window.local_storage().ok().flatten())
    else {
        return;
    };
    if let Ok(raw) = serde_json::to_string(value) {
        let _ = storage.set_item(key, &raw);
    }
}

async fn copy_text(text: &str) -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let Ok(clipboard) = js_sys::Reflect::get(window.navigator().as_ref(), &"clipboard".into())
    else {
        return false;
    };
    let Ok(write) = js_sys::Reflect::get(&clipboard, &"writeText".into()) else {
        return false;
    };
    let Ok(function) = write.dyn_into::<js_sys::Function>() else {
        return false;
    };
    function.call1(&clipboard, &text.into()).is_ok()
}
