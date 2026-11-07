//! Top-level layout: setup, the sidebar, terminal panes, and pop-out routes.

use dioxus::prelude::*;
use legate_core::{ProjectInfo, ProviderId, TerminalInfo};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::api;
use crate::components::{
    ChangesTab, FilesPanel, FlowView, GoalsPanel, HistoryPanel, JobsPanel, LeadChat, NewTaskDialog,
    ProjectBar, Routines, SettingsPage, SetupScreen, Star, TabIcon, TerminalPane, TerminalTabs,
    TerminalWindow, UsageMeter,
};
use crate::logic::flow_events::FlowEvent;
use crate::logic::layout::{self, DrawerTab, PaneLayoutId};
use crate::logic::setup_seen::{self, SeenStorage};
use crate::logic::theme::{self, ThemeId, ThemeRoot, ThemeStorage, ThemeTarget};
use crate::state::{self, Store};
use crate::xterm;

const MAIN_CSS: manganis::Asset = manganis::asset!("/assets/main.css");
const XTERM_CSS: manganis::Asset = manganis::asset!("/assets/vendor/xterm.css");
const XTERM_JS: manganis::Asset = manganis::asset!("/assets/vendor/legate-xterm.js");

#[component]
pub fn App() -> Element {
    let mut store = state::provide_store();
    let mut started = use_signal(|| false);
    let mut focus_nonce = use_signal(|| 1i64);
    let mut settings_open = use_signal(|| false);
    let mut task_open = use_signal(|| false);
    use_effect(move || {
        if started() {
            return;
        }
        started.set(true);
        mount_assets();
        xterm::install_assets();
        let mut storage = WebStore;
        let theme = theme::load_theme(&storage);
        theme::apply_theme(theme, &mut storage, &mut WebRoot, &mut NoTarget);
        store
            .setup_open
            .set(!setup_seen::setup_seen(Some(&storage)));
        let saved = layout::load_layout(&storage);
        store.sidebar_width.set(saved.sizes.sidebar);
        store.sidebar_shown.set(saved.sidebar_shown);
        store.drawer_open.set(saved.drawer_open);
        store.drawer_tab.set(saved.drawer_tab.as_str().to_string());
        store
            .pane_layout
            .set(saved.pane_layout.as_str().to_string());
        state::boot(store);
        listen_for_new_window();
    });

    let terminal = query("terminal");
    let panel = query("panel").filter(|value| layout::is_drawer_tab(value));
    let projects = store.projects.read().clone();
    let project = store.project.read().clone();
    let terminals = store.terminals.read().clone();
    let changes = store.changes.read().clone();
    let jobs = store.jobs.read().clone();
    let plans = store.plans.read().clone();
    let chats = store.chats.read().clone();
    let active = store.active_terminal.read().clone();
    let width = *store.sidebar_width.read();
    let folder = project
        .as_ref()
        .map(|item| folder_name(&item.path))
        .unwrap_or_default();
    let branch = project
        .as_ref()
        .and_then(|item| item.branch.clone())
        .unwrap_or_default();

    if let Some(id) = terminal {
        return rsx! { TerminalWindow { id: id } };
    }
    if let Some(tab) = panel.clone() {
        let heading = tab.clone();
        return rsx! {
            div { class: "app is-solo",
                section { class: "solo-panel", aria_label: "{heading}",
                    div { class: "drawer-page", TabBody { tab: tab } }
                }
            }
        };
    }

    let slots = pane_slots(&terminals, &store.pane_layout.read(), &active);
    let stage: Vec<(i64, TerminalInfo)> = slots
        .iter()
        .enumerate()
        .filter_map(|(index, id)| {
            terminals
                .iter()
                .find(|item| &item.id == id)
                .cloned()
                .map(|info| (index as i64, info))
        })
        .collect();
    let claude = plans.as_ref().is_some_and(|list| {
        list.iter()
            .any(|plan| plan.id == ProviderId::Claude && plan.available)
    });
    let cursor = plans.as_ref().is_some_and(|list| {
        list.iter()
            .any(|plan| plan.id == ProviderId::Cursor && plan.available)
    });
    let running = jobs.iter().any(|job| {
        matches!(
            job.status,
            legate_core::JobStatus::Running | legate_core::JobStatus::Queued
        )
    });

    rsx! {
        div { class: "app",
            header { class: "app-header",
                div { class: "app-brand",
                    Star { size: 16.0 }
                    div { class: "app-name", "Legate" }
                    if !folder.is_empty() {
                        p { class: "app-context",
                            span { class: "app-context-project", "{folder}" }
                            if !branch.is_empty() { span { class: "app-context-branch", "{branch}" } }
                        }
                    }
                }
                if !*store.setup_open.read() {
                    div { class: "app-header-tools",
                        button {
                            class: "btn btn-quiet btn-compact",
                            onclick: move |_| spawn_local(async move { let _ = api::new_window().await; }),
                            "Window"
                        }
                        button {
                            class: "btn btn-quiet btn-compact",
                            onclick: move |_| {
                                let open = *store.drawer_open.read();
                                store.drawer_open.set(!open);
                            },
                            "Panel"
                        }
                    }
                }
            }
            div { class: "app-body",
                if *store.sidebar_shown.read() {
                    div { class: "sidebar-slot", style: "width: {width}px;",
                        ProjectBar {
                            projects: projects,
                            chats: chats,
                            busy: running,
                            lead_busy: false,
                            now: *store.now.read(),
                            error: store.error.read().clone(),
                            github_available: *store.github_available.read(),
                            changes: changes.clone(),
                            jobs: jobs.clone(),
                            terminals: terminals.clone(),
                            selected_worktrees: store.selected.read().clone(),
                            claude_available: claude,
                            cursor_available: cursor,
                            activity: store.activity.read().clone(),
                            goals_needing_you: 0,
                            on_add: Callback::new(move |_| choose_project(store)),
                            on_github_clone: Callback::new(move |_| refresh_projects(store)),
                            on_switch: Callback::new(move |(path, _worktree): (String, Option<String>)| switch_project(store, path)),
                            on_remove: Callback::new(move |path: String| remove_project(store, path)),
                            on_open_chat: Callback::new(move |id| open_chat(store, id)),
                            on_new_chat: Callback::new(move |_| reset_chat(store)),
                            on_rename_chat: Callback::new(move |(id, title): (String, String)| rename_chat(id, title)),
                            on_remove_chat: Callback::new(move |id| remove_chat(store, id)),
                            on_select_worktree: Callback::new(move |(id, _mode): (Option<String>, String)| store.selected.set(vec![id])),
                            on_select_all_worktrees: Callback::new(move |_| {
                                let mut all = vec![None];
                                for change in store.changes.read().iter() {
                                    all.push(Some(change.id.clone()));
                                }
                                store.selected.set(all);
                            }),
                            on_create_worktree: Callback::new(move |(name, _count, _start, _prompt): (Option<String>, i64, Option<ProviderId>, Option<String>)| {
                                spawn_local(async move {
                                    let _ = api::create_worktree(name.as_deref()).await;
                                });
                            }),
                            on_rename_worktree: Callback::new(move |(id, name): (String, String)| {
                                spawn_local(async move { let _ = api::rename_worktree(&id, &name).await; });
                            }),
                            on_discard_worktree: Callback::new(move |id: String| {
                                spawn_local(async move { let _ = api::discard_change(&id).await; });
                            }),
                            on_open_in_worktree: Callback::new(move |(provider, worktree): (ProviderId, Option<String>)| {
                                open_terminal(store, provider.as_str().into(), worktree);
                            }),
                            on_reveal: Callback::new(move |worktree: Option<String>| {
                                spawn_local(async move { let _ = api::reveal_folder(worktree.as_deref()).await; });
                            }),
                            on_reveal_project: Callback::new(move |path: String| {
                                spawn_local(async move { let _ = api::reveal_project(&path).await; });
                            }),
                            on_open_settings: Callback::new(move |_| settings_open.set(true)),
                            on_new_task: Callback::new(move |_| task_open.set(true)),
                            on_open_goals: Callback::new(move |_| {
                                store.drawer_tab.set("goals".into());
                                store.drawer_open.set(true);
                            }),
                            on_open_automations: Callback::new(move |_| {
                                store.drawer_tab.set("routines".into());
                                store.drawer_open.set(true);
                            }),
                            on_open_setup: Callback::new(move |_| store.setup_open.set(true)),
                        }
                    }
                }
                div { class: "workers",
                    TerminalTabs {
                        terminals: terminals.clone(),
                        changes: changes.clone(),
                        project: project.as_ref().map(|item| item.path.clone()).unwrap_or_default(),
                        active: active.clone(),
                        pane_layout: store.pane_layout.read().clone(),
                        opening: *store.opening.read(),
                        review: None,
                        agents: None,
                        on_select: Callback::new(move |(id, _source): (String, String)| {
                            store.active_terminal.set(id);
                            focus_nonce.set(focus_nonce() + 1);
                        }),
                        on_close: Callback::new(move |id: String| {
                            spawn_local(async move { let _ = api::close_terminal(&id).await; });
                        }),
                        on_open: Callback::new(move |(provider, worktree): (String, Option<String>)| open_terminal(store, provider, worktree)),
                        on_pane_layout: Callback::new(move |layout: String| store.pane_layout.set(layout)),
                    }
                    div { class: "terminal-stage",
                        if stage.is_empty() {
                            p { class: "hint", "Open a shell or a Claude terminal to start." }
                        }
                        for (index, info) in stage {
                            {
                                let id = info.id.clone();
                                let close_id = id.clone();
                                let focused = id == active;
                                rsx! {
                                    TerminalPane {
                                        key: "{id}",
                                        info: info,
                                        heading: None,
                                        bus: store.bus.read().clone(),
                                        restore: true,
                                        placement: crate::components::PanePlacement::Slot(index),
                                        focused: focused,
                                        focus_nonce: focus_nonce(),
                                        own_window: false,
                                        on_activate: Callback::new(move |_| store.active_terminal.set(id.clone())),
                                        on_close: Callback::new(move |_| {
                                            let closing = close_id.clone();
                                            spawn_local(async move { let _ = api::close_terminal(&closing).await; });
                                        }),
                                    }
                                }
                            }
                        }
                    }
                }
                if *store.drawer_open.read() {
                    aside { class: "drawer",
                        div { class: "drawer-tabs", role: "tablist",
                            for tab in DrawerTab::ALL {
                                button {
                                    class: if store.drawer_tab.read().as_str() == tab.as_str() { "drawer-tab is-selected" } else { "drawer-tab" },
                                    onclick: move |_| store.drawer_tab.set(tab.as_str().to_string()),
                                    TabIcon { tab: tab }
                                    "{tab.label()}"
                                }
                            }
                        }
                        div { class: "drawer-page", TabBody { tab: store.drawer_tab.read().clone() } }
                    }
                }
            }
            if *store.setup_open.read() {
                SetupScreen {
                    bus: store.bus.read().clone(),
                    on_close: Callback::new(move |_| finish_setup(store)),
                    on_skip: Callback::new(move |_| finish_setup(store)),
                }
            }
            if settings_open() {
                SettingsPage {
                    plans: plans.clone().unwrap_or_default(),
                    projects: store.projects.read().clone(),
                    theme: ThemeId::System,
                    tab_order_custom: false,
                    on_theme: Callback::new(|_| {}),
                    on_remove_project: Callback::new(move |path| remove_project(store, path)),
                    on_open_setup: Callback::new(move |_| store.setup_open.set(true)),
                    on_lead_changed: Callback::new(|_| {}),
                    on_reset_tabs: Callback::new(|_| {}),
                    on_close: Callback::new(move |_| settings_open.set(false)),
                }
            }
            NewTaskDialog {
                open: task_open(),
                is_repo: project.as_ref().is_some_and(|item| item.is_repo),
                claude_available: claude,
                cursor_available: cursor,
                on_close: Callback::new(move |_| task_open.set(false)),
                on_start_terminal: Callback::new(move |(text, provider, _count): (String, ProviderId, i64)| {
                    open_terminal(store, provider.as_str().into(), None);
                    let _ = text;
                }),
                on_created: Callback::new(move |_| task_open.set(false)),
            }
        }
    }
}

#[component]
fn TabBody(tab: String) -> Element {
    let mut store = state::use_store();
    let jobs = store.jobs.read().clone();
    let plans = store.plans.read().clone();
    let changes = store.changes.read().clone();
    let terminals = store.terminals.read().clone();
    let project = store.project.read().clone();
    match tab.as_str() {
        "files" => rsx! { FilesPanel { r#where: None, label: "Files".to_string() } },
        "lead" => rsx! { LeadChat {
            messages: Vec::new(),
            jobs: jobs,
            plans: plans,
            mode: None,
            settings_nonce: 0,
            on_reset: Callback::new(|_| {}),
            on_open_terminal: Callback::new(|_| {}),
            on_show_job: Callback::new(|_| {}),
            on_run_plan: Callback::new(|_| {}),
        } },
        "jobs" => rsx! { JobsPanel {
            jobs: jobs,
            plans: plans,
            changes: changes,
            is_repo: project.as_ref().is_some_and(|item| item.is_repo),
            selected_job_id: None,
            show_job: None,
            on_select_job: Callback::new(|_| {}),
            on_job: Callback::new(|_| {}),
            on_view_change: Callback::new(|_| {}),
            on_goal: Callback::new(|_| {}),
        } },
        "changes" => rsx! { ChangesTab {
            changes: changes,
            jobs: jobs,
            selected_id: None,
            claude_available: false,
            cursor_available: false,
            opening: false,
            on_select: Callback::new(|_| {}),
            on_open_here: Callback::new(|_| {}),
        } },
        "routines" => rsx! { Routines {
            project: project.map(|item| item.path).unwrap_or_default(),
            on_run: Callback::new(|_| {}),
        } },
        "usage" => rsx! { UsageMeter {
            plans: plans,
            now: *store.now.read(),
            shows: store.prefs.read().usage_shows,
            on_open_setup: Callback::new(move |_| store.setup_open.set(true)),
        } },
        "activity" => rsx! { FlowView {
            jobs: jobs,
            plans: plans.unwrap_or_default(),
            lead: Vec::new(),
            terminals: terminals,
            changes: changes,
            events: Vec::<FlowEvent>::new(),
            now: *store.now.read(),
            starts: std::collections::HashMap::new(),
            flashes: std::collections::HashMap::new(),
            on_show_terminal: Callback::new(move |id: String| store.active_terminal.set(id)),
        } },
        "history" => {
            rsx! { HistoryPanel { r#where: None, label: "History".to_string(), refresh_key: String::new() } }
        }
        "goals" => rsx! { GoalsPanel {
            jobs: jobs,
            on_view_change: Callback::new(|_| {}),
            on_show_job: Callback::new(|_| {}),
        } },
        _ => rsx! { p { class: "hint", "{tab}" } },
    }
}

fn pane_slots(terminals: &[TerminalInfo], layout: &str, active: &str) -> Vec<String> {
    let parsed = PaneLayoutId::parse(layout).unwrap_or(PaneLayoutId::Auto);
    let ids: Vec<String> = terminals.iter().map(|item| item.id.clone()).collect();
    let active = if active.is_empty() {
        None
    } else {
        Some(active)
    };
    layout::fill_panes(parsed, &ids, active)
        .into_iter()
        .flatten()
        .collect()
}

fn choose_project(mut store: Store) {
    spawn_local(async move {
        match api::choose_project().await {
            Ok(Some(info)) => adopt(store, info).await,
            Ok(None) => {}
            Err(text) => store.error.set(Some(text)),
        }
    });
}

fn switch_project(mut store: Store, path: String) {
    spawn_local(async move {
        match api::switch_project(&path).await {
            Ok(info) => adopt(store, info).await,
            Err(text) => store.error.set(Some(text)),
        }
    });
}

fn remove_project(mut store: Store, path: String) {
    spawn_local(async move {
        if let Err(text) = api::remove_project(&path).await {
            store.error.set(Some(text));
            return;
        }
        if let Ok(list) = api::list_projects().await {
            store.projects.set(list);
        }
    });
}

async fn adopt(mut store: Store, info: ProjectInfo) {
    store.project.set(Some(info));
    store.error.set(None);
    if let Ok(list) = api::list_projects().await {
        store.projects.set(list);
    }
    if let Ok(list) = api::list_changes().await {
        store.changes.set(list);
    }
}

fn refresh_projects(store: Store) {
    spawn_local(async move {
        if let Ok(info) = api::get_project().await {
            adopt(store, info).await;
        }
    });
}

fn open_terminal(mut store: Store, provider: String, worktree: Option<String>) {
    store.opening.set(true);
    spawn_local(async move {
        match api::open_terminal(&provider, 100.0, 30.0, worktree.as_deref(), None).await {
            Ok(info) => {
                let id = info.id.clone();
                store.terminals.with_mut(|list| {
                    if let Some(slot) = list.iter_mut().find(|item| item.id == info.id) {
                        *slot = info;
                    } else {
                        list.push(info);
                    }
                });
                store.active_terminal.set(id);
                store.error.set(None);
            }
            Err(text) => store.error.set(Some(text)),
        }
        store.opening.set(false);
    });
}

fn open_chat(mut store: Store, id: String) {
    spawn_local(async move {
        if let Ok(messages) = api::open_lead_chat(&id).await {
            let _ = messages;
            if let Ok(chats) = api::list_lead_chats().await {
                store.chats.set(chats);
            }
            store.drawer_tab.set("lead".into());
        }
    });
}

fn reset_chat(mut store: Store) {
    spawn_local(async move {
        let _ = api::reset_lead(None).await;
        if let Ok(chats) = api::list_lead_chats().await {
            store.chats.set(chats);
        }
        store.drawer_tab.set("lead".into());
    });
}

fn rename_chat(id: String, title: String) {
    spawn_local(async move {
        let _ = api::rename_lead_chat(&id, &title).await;
    });
}

fn remove_chat(mut store: Store, id: String) {
    spawn_local(async move {
        let _ = api::remove_lead_chat(&id).await;
        if let Ok(chats) = api::list_lead_chats().await {
            store.chats.set(chats);
        }
    });
}

fn finish_setup(mut store: Store) {
    let mut storage = WebStore;
    setup_seen::mark_setup_seen(Some(&mut storage));
    store.setup_open.set(false);
}

fn folder_name(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_string()
}

fn query(name: &str) -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    for pair in search.trim_start_matches('?').split('&') {
        let (key, value) = pair.split_once('=')?;
        if key == name {
            return Some(value.to_string());
        }
    }
    None
}

fn listen_for_new_window() {
    let closure = wasm_bindgen::closure::Closure::wrap(Box::new(|event: web_sys::KeyboardEvent| {
        if event.ctrl_key()
            && event.shift_key()
            && !event.alt_key()
            && (event.key() == "n" || event.key() == "N")
        {
            event.prevent_default();
            spawn_local(async {
                let _ = api::new_window().await;
            });
        }
    }) as Box<dyn FnMut(_)>);
    if let Some(window) = web_sys::window() {
        let _ =
            window.add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref());
    }
    closure.forget();
}

fn mount_assets() {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    add_style(&document, "legate-main-css", &MAIN_CSS.to_string());
    add_style(&document, "legate-xterm-css", &XTERM_CSS.to_string());
    if document.get_element_by_id("legate-xterm-script").is_none()
        && let Ok(script) = document.create_element("script")
    {
        let _ = script.set_attribute("id", "legate-xterm-script");
        let _ = script.set_attribute("type", "module");
        let _ = script.set_attribute("src", &XTERM_JS.to_string());
        if let Some(root) = document.document_element() {
            let _ = root.append_child(&script);
        }
    }
}

fn add_style(document: &web_sys::Document, id: &str, href: &str) {
    if document.get_element_by_id(id).is_some() {
        return;
    }
    let Ok(link) = document.create_element("link") else {
        return;
    };
    let _ = link.set_attribute("id", id);
    let _ = link.set_attribute("rel", "stylesheet");
    let _ = link.set_attribute("href", href);
    if let Some(root) = document.document_element() {
        let _ = root.append_child(&link);
    }
}

struct WebStore;

fn browser_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

impl SeenStorage for WebStore {
    fn get_item(&self, key: &str) -> Result<Option<String>, ()> {
        browser_storage().ok_or(())?.get_item(key).map_err(|_| ())
    }
    fn set_item(&mut self, key: &str, value: &str) -> Result<(), ()> {
        browser_storage()
            .ok_or(())?
            .set_item(key, value)
            .map_err(|_| ())
    }
}

impl ThemeStorage for WebStore {
    fn get_item(&self, key: &str) -> Result<Option<String>, ()> {
        browser_storage().ok_or(())?.get_item(key).map_err(|_| ())
    }
    fn set_item(&mut self, key: &str, value: &str) -> Result<(), ()> {
        browser_storage()
            .ok_or(())?
            .set_item(key, value)
            .map_err(|_| ())
    }
}

impl layout::LayoutStore for WebStore {
    fn get_item(&self, key: &str) -> Result<Option<String>, ()> {
        browser_storage().ok_or(())?.get_item(key).map_err(|_| ())
    }
    fn set_item(&mut self, key: &str, value: &str) -> Result<(), ()> {
        browser_storage()
            .ok_or(())?
            .set_item(key, value)
            .map_err(|_| ())
    }
}

struct WebRoot;

impl ThemeRoot for WebRoot {
    fn set_attribute(&mut self, name: &str, value: &str) {
        if let Some(root) = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.document_element())
        {
            let _ = root.set_attribute(name, value);
        }
    }
    fn remove_attribute(&mut self, name: &str) {
        if let Some(root) = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.document_element())
        {
            let _ = root.remove_attribute(name);
        }
    }
}

struct NoTarget;

impl ThemeTarget for NoTarget {
    fn dispatch(&mut self, _name: &str) {}
}
