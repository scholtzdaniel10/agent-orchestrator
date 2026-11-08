//! Top-level layout. The element tree matches `App.tsx` so `main.css` applies.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use dioxus::prelude::dioxus_elements::InteractionLocation;
use dioxus::prelude::*;
use legate_core::{
    JobRecord, JobStatus, LeadStatus, PlanProblem, ProjectInfo, ProviderId, TerminalInfo,
    TerminalProvider, UsageShows, plan_of,
};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;

use crate::api;
use crate::components::{
    ChangesTab, ErrorBoundary, FilesPanel, FlowView, GoalsPanel, HistoryPanel, JobsPanel, LeadChat,
    NewTaskDialog, ProjectBar, RayBurst, Routines, SettingsPage, SetupScreen, Star, TabIcon,
    TerminalWindow, UsageMeter, Workers,
};
use crate::logic::agent_tabs::agent_jobs;
use crate::logic::flow_events::FlowEvent;
use crate::logic::layout::{
    self, DockId, DrawerTab, LayoutFlags, LayoutShortcut, LayoutState, PaneLayoutId, SizeKey,
    Viewport,
};
use crate::logic::setup_seen::{self, SeenStorage};
use crate::logic::theme::{self, ThemeId, ThemeRoot, ThemeStorage, ThemeTarget};
use crate::logic::usage_format::usage_figure;
use crate::state::{self, Store};
use crate::xterm;

const MAIN_CSS: manganis::Asset = manganis::asset!("/assets/main.css");
const XTERM_CSS: manganis::Asset = manganis::asset!("/assets/vendor/xterm.css");
const XTERM_JS: manganis::Asset = manganis::asset!("/assets/vendor/legate-xterm.js");

#[component]
pub fn App() -> Element {
    let mut store = state::provide_store();
    let layout = use_signal(|| layout::load_layout(&WebStore));
    let mut theme = use_signal(|| theme::load_theme(&WebStore));
    let mut started = use_signal(|| false);
    let mut applied = use_signal(|| false);
    let mut settings_open = use_signal(|| false);
    let mut task_open = use_signal(|| false);
    let mut dragging = use_signal(|| None::<String>);
    let mut initial_ids = use_signal(|| None::<Vec<String>>);
    let mut closed_agents = use_signal(Vec::<String>::new);
    if !applied() {
        applied.set(true);
        let saved = layout.read().clone();
        store.sidebar_width.set(saved.sizes.sidebar);
        store.drawer_width.set(saved.sizes.drawer);
        store.sidebar_shown.set(saved.sidebar_shown);
        store.drawer_open.set(saved.drawer_open);
        store.drawer_tab.set(saved.drawer_tab.as_str().to_string());
        store
            .pane_layout
            .set(saved.pane_layout.as_str().to_string());
        let id = theme();
        theme::apply_theme(id, &mut WebStore, &mut WebRoot, &mut WebTarget);
        store
            .setup_open
            .set(!setup_seen::setup_seen(Some(&WebStore)));
    }
    use_effect(move || {
        if started() {
            return;
        }
        started.set(true);
        mount_assets();
        xterm::install_assets();
        state::boot(store);
        install_keys(store, layout, settings_open, task_open);
    });

    let terminal = query("terminal");
    let panel = query("panel").filter(|value| layout::is_drawer_tab(value));
    if *store.booted.read() && initial_ids.read().is_none() {
        let ids = store
            .terminals
            .read()
            .iter()
            .map(|item| item.id.clone())
            .collect();
        initial_ids.set(Some(ids));
    }
    prune_selection(store);

    let projects = store.projects.read().clone();
    let project = store.project.read().clone();
    let terminals = store.terminals.read().clone();
    let changes = store.changes.read().clone();
    let jobs = store.jobs.read().clone();
    let plans = store.plans.read().clone();
    let chats = store.chats.read().clone();
    let messages = store.messages.read().clone();
    let frame = layout.read().clone();
    let flags = LayoutFlags {
        sidebar_shown: frame.sidebar_shown,
        drawer_open: frame.drawer_open,
    };
    let view = viewport();
    let displayed = layout::clamp_sizes(frame.sizes, view, flags);
    let overlay = layout::drawer_overlays(displayed, view, flags);
    let docks = docks_of(&frame);
    let folder = project
        .as_ref()
        .map(|item| folder_name(&item.path))
        .unwrap_or_default();
    let branch = project.as_ref().and_then(|item| item.branch.clone());
    let claude = plan_ready(&plans, ProviderId::Claude);
    let cursor = plan_ready(&plans, ProviderId::Cursor);
    let running = jobs
        .iter()
        .filter(|job| job.status == JobStatus::Running)
        .count();
    let queued = jobs
        .iter()
        .filter(|job| job.status == JobStatus::Queued)
        .count();
    let lead_busy = messages
        .iter()
        .any(|message| message.status == LeadStatus::Streaming);
    let shows = store.prefs.read().usage_shows;
    let now = *store.now.read();
    let setup_open = *store.setup_open.read();
    let agents = agent_jobs(&jobs, &[], &closed_agents.read());
    let visible_terminals: Vec<TerminalInfo> = terminals
        .iter()
        .filter(|info| !(info.provider == TerminalProvider::Shell && info.change.is_none()))
        .cloned()
        .collect();
    let pane = PaneLayoutId::parse(&store.pane_layout.read()).unwrap_or(frame.pane_layout);
    let app_class = app_class(
        dragging.read().as_deref(),
        flags,
        overlay && flags.drawer_open,
    );
    let app_style = format!(
        "--layout-sidebar: {}px; --layout-drawer: {}px;",
        if flags.sidebar_shown {
            displayed.sidebar
        } else {
            0
        },
        if flags.drawer_open {
            displayed.drawer
        } else {
            0
        }
    );

    if let Some(id) = terminal {
        return rsx! { TerminalWindow { id: id } };
    }
    if let Some(tab) = panel.clone() {
        let state = frame.clone();
        return rsx! {
            div { class: "app is-solo", style: "{app_style}",
                section { class: "solo-panel", aria_label: "{DrawerTab::parse(&tab).map(|item| item.label()).unwrap_or(\"Panel\")}",
                    div { class: "drawer-page",
                        ErrorBoundary { class_name: None,
                            TabBody { tab: tab.clone(), frame: state }
                        }
                    }
                }
            }
        };
    }

    let sidebar_max = layout::max_size(SizeKey::Sidebar, displayed, view, flags);
    let drawer_max = layout::max_size(SizeKey::Drawer, displayed, view, flags);

    rsx! {
        div { class: "{app_class}", style: "{app_style}",
            header { class: "app-header",
                div { class: "app-brand",
                    RayBurst {}
                    span { class: "app-mark", aria_hidden: "true", Star { size: 16.0 } }
                    div { class: "app-name", "Legate" }
                    if !folder.is_empty() {
                        p { class: "app-context",
                            span { class: "app-context-project", "{folder}" }
                            if let Some(branch) = branch.clone() {
                                span { class: "app-context-branch", "{branch}" }
                            }
                        }
                    }
                }
                if !setup_open {
                    div { class: "app-header-tools",
                        button {
                            class: "btn btn-quiet btn-compact",
                            r#type: "button",
                            onclick: move |_| spawn_local(async move { let _ = api::new_window().await; }),
                            "Window"
                        }
                        label { class: "theme-picker",
                            span { class: "theme-picker-label", "Theme" }
                            select {
                                class: "select",
                                aria_label: "Theme",
                                onchange: move |event| {
                                    if let Some(next) = ThemeId::parse(&event.value()) {
                                        theme.set(next);
                                        theme::apply_theme(next, &mut WebStore, &mut WebRoot, &mut WebTarget);
                                    }
                                },
                                for (id, name) in theme::THEMES {
                                    option { value: "{id}", selected: theme().as_str() == *id, "{name}" }
                                }
                            }
                        }
                        button {
                            class: "btn btn-quiet btn-compact",
                            r#type: "button",
                            aria_pressed: if frame.drawer_open { "true" } else { "false" },
                            title: "Panel (Ctrl+B)",
                            onclick: move |_| {
                                let mut next = layout.read().clone();
                                next.drawer_open = !next.drawer_open;
                                write_layout(layout, store, next);
                            },
                            "Panel"
                        }
                    }
                }
            }
            div { class: "app-body",
                if frame.sidebar_shown {
                    div { class: "sidebar-slot",
                        ErrorBoundary { class_name: Some("sidebar-main".into()),
                            ProjectBar {
                                projects: projects.clone(),
                                chats: chats.clone(),
                                busy: running > 0 || queued > 0,
                                lead_busy: lead_busy,
                                now: now,
                                error: store.error.read().clone(),
                                github_available: *store.github_available.read(),
                                changes: changes.clone(),
                                jobs: jobs.clone(),
                                terminals: terminals.clone(),
                                selected_worktrees: store.selected.read().clone(),
                                claude_available: claude,
                                cursor_available: cursor,
                                activity: store.activity.read().clone(),
                                goals_needing_you: store.goals_waiting.read().len() as i64,
                                on_add: Callback::new(move |_| choose_project(store)),
                                on_github_clone: Callback::new(move |_| refresh_projects(store)),
                                on_switch: Callback::new(move |(path, worktree): (String, Option<String>)| switch_project(store, path, worktree)),
                                on_remove: Callback::new(move |path: String| remove_project(store, path)),
                                on_open_chat: Callback::new(move |id: String| open_chat(store, layout, id)),
                                on_new_chat: Callback::new(move |_| reset_chat(store, layout)),
                                on_rename_chat: Callback::new(move |(id, title): (String, String)| rename_chat(id, title)),
                                on_remove_chat: Callback::new(move |id: String| remove_chat(store, id)),
                                on_select_worktree: Callback::new(move |(id, mode): (Option<String>, String)| select_worktree(store, id, &mode)),
                                on_select_all_worktrees: Callback::new(move |_| select_all(store)),
                                on_create_worktree: Callback::new(move |(name, count, start, prompt): (Option<String>, i64, Option<ProviderId>, Option<String>)| {
                                    create_worktrees(store, name, count, start, prompt);
                                }),
                                on_rename_worktree: Callback::new(move |(id, name): (String, String)| {
                                    spawn_local(async move { let _ = api::rename_worktree(&id, &name).await; });
                                }),
                                on_discard_worktree: Callback::new(move |id: String| {
                                    spawn_local(async move { let _ = api::discard_change(&id).await; });
                                }),
                                on_open_in_worktree: Callback::new(move |(provider, worktree): (ProviderId, Option<String>)| {
                                    open_terminal(store, provider.as_str().into(), worktree, None);
                                }),
                                on_reveal: Callback::new(move |worktree: Option<String>| {
                                    spawn_local(async move { let _ = api::reveal_folder(worktree.as_deref()).await; });
                                }),
                                on_reveal_project: Callback::new(move |path: String| {
                                    spawn_local(async move { let _ = api::reveal_project(&path).await; });
                                }),
                                on_open_settings: Callback::new(move |_| settings_open.set(true)),
                                on_new_task: Callback::new(move |_| task_open.set(true)),
                                on_open_goals: Callback::new(move |_| open_tab(layout, store, DrawerTab::Goals)),
                                on_open_automations: Callback::new(move |_| {
                                    open_tab(layout, store, DrawerTab::Goals);
                                    spawn_local(async move {
                                        gloo_timers::future::TimeoutFuture::new(50).await;
                                        if let Some(node) = document().and_then(|doc| doc.query_selector(".goal-triggers").ok().flatten()) {
                                            node.scroll_into_view();
                                        }
                                    });
                                }),
                                on_open_setup: Callback::new(move |_| store.setup_open.set(true)),
                            }
                        }
                        SideDock { dock: DockId::Left, frame: frame.clone(), docks: docks.clone(), jobs_badge: (running + queued) as i64, change_badge: changes.len() as i64, lead_busy: lead_busy, popped: store.popped_panels.read().clone(), on_open: Callback::new(move |tab: DrawerTab| open_tab(layout, store, tab)), on_resize: Callback::new(move |value: i64| resize_dock(layout, store, DockId::Left, value)), on_reset: Callback::new(move |_| reset_dock(layout, store, DockId::Left)), on_drag: Callback::new(move |value: Option<String>| dragging.set(value)) }
                    }
                    ResizeHandle {
                        class_name: "layout-handle layout-handle-sidebar".to_string(),
                        orientation: "vertical",
                        value: displayed.sidebar,
                        min: layout::MIN_SIDEBAR,
                        max: sidebar_max,
                        label: "Projects width".to_string(),
                        invert: false,
                        on_change: Callback::new(move |value: i64| resize_bar(layout, store, SizeKey::Sidebar, value)),
                        on_reset: Callback::new(move |_| reset_bar(layout, store, SizeKey::Sidebar)),
                        on_drag: Callback::new(move |value: Option<String>| dragging.set(value)),
                    }
                }
                section { class: "workspace", aria_label: "Workspace",
                    SideDock { dock: DockId::Top, frame: frame.clone(), docks: docks.clone(), jobs_badge: (running + queued) as i64, change_badge: changes.len() as i64, lead_busy: lead_busy, popped: store.popped_panels.read().clone(), on_open: Callback::new(move |tab: DrawerTab| open_tab(layout, store, tab)), on_resize: Callback::new(move |value: i64| resize_dock(layout, store, DockId::Top, value)), on_reset: Callback::new(move |_| reset_dock(layout, store, DockId::Top)), on_drag: Callback::new(move |value: Option<String>| dragging.set(value)) }
                    ErrorBoundary { class_name: Some("workspace-slot".into()),
                        Workers {
                            terminals: visible_terminals,
                            initial_terminal_ids: initial_ids.read().clone(),
                            changes: changes.clone(),
                            selected_worktree: store.primary.read().clone(),
                            selected_worktrees: store.selected.read().clone(),
                            place_terminal: None,
                            on_select_worktree: Callback::new(move |id: Option<String>| select_worktree(store, id, "only")),
                            project: project.as_ref().map(|item| item.path.clone()).unwrap_or_default(),
                            bus: store.bus.read().clone(),
                            pane_layout: pane,
                            on_pane_layout: Callback::new(move |next: PaneLayoutId| {
                                let mut saved = layout.read().clone();
                                saved.pane_layout = next;
                                write_layout(layout, store, saved);
                            }),
                            on_terminal: Callback::new(move |info: TerminalInfo| remember_terminal(store, info)),
                            agent_jobs: agents,
                            focus_agent: None,
                            on_close_agent: Some(Callback::new(move |id: String| {
                                closed_agents.with_mut(|list| {
                                    if !list.iter().any(|item| item == &id) {
                                        list.push(id.clone());
                                    }
                                });
                            })),
                        }
                    }
                    SideDock { dock: DockId::Bottom, frame: frame.clone(), docks: docks.clone(), jobs_badge: (running + queued) as i64, change_badge: changes.len() as i64, lead_busy: lead_busy, popped: store.popped_panels.read().clone(), on_open: Callback::new(move |tab: DrawerTab| open_tab(layout, store, tab)), on_resize: Callback::new(move |value: i64| resize_dock(layout, store, DockId::Bottom, value)), on_reset: Callback::new(move |_| reset_dock(layout, store, DockId::Bottom)), on_drag: Callback::new(move |value: Option<String>| dragging.set(value)) }
                }
                if frame.drawer_open {
                    ResizeHandle {
                        class_name: "layout-handle layout-handle-drawer".to_string(),
                        orientation: "vertical",
                        value: displayed.drawer,
                        min: layout::MIN_DRAWER,
                        max: drawer_max,
                        label: "Panel width".to_string(),
                        invert: true,
                        on_change: Callback::new(move |value: i64| resize_bar(layout, store, SizeKey::Drawer, value)),
                        on_reset: Callback::new(move |_| reset_bar(layout, store, SizeKey::Drawer)),
                        on_drag: Callback::new(move |value: Option<String>| dragging.set(value)),
                    }
                    aside { class: "drawer", aria_label: "Panel",
                        TabBar { dock: DockId::Right, frame: frame.clone(), docks: docks.clone(), jobs_badge: (running + queued) as i64, change_badge: changes.len() as i64, lead_busy: lead_busy, popped: store.popped_panels.read().clone(), on_open: Callback::new(move |tab: DrawerTab| open_tab(layout, store, tab)) }
                        Pages { dock: DockId::Right, frame: frame.clone(), docks: docks.clone() }
                    }
                }
            }
            NewTaskDialog {
                open: task_open(),
                is_repo: project.as_ref().is_some_and(|item| item.is_repo),
                claude_available: claude,
                cursor_available: cursor,
                on_close: Callback::new(move |_| task_open.set(false)),
                on_start_terminal: Callback::new(move |(text, provider, count): (String, ProviderId, i64)| {
                    create_worktrees(store, None, count, Some(provider), Some(text));
                    task_open.set(false);
                }),
                on_created: Callback::new(move |kind: String| {
                    task_open.set(false);
                    open_tab(layout, store, if kind == "goal" { DrawerTab::Goals } else { DrawerTab::Jobs });
                }),
            }
            footer { class: "status-bar",
                for plan in meter_plans(plans.as_deref().unwrap_or(&[])) {
                    {
                        let (text, bad) = plan_bar_label(&plan, now, shows);
                        let problem = plan.problem;
                        rsx! {
                            button {
                                key: "{plan.id.as_str()}",
                                class: if bad { "status-bar-item is-bad" } else { "status-bar-item" },
                                r#type: "button",
                                onclick: move |_| {
                                    if problem.is_some() {
                                        store.setup_open.set(true);
                                    } else {
                                        open_tab(layout, store, DrawerTab::Usage);
                                    }
                                },
                                "{text}"
                            }
                        }
                    }
                }
                button {
                    class: "status-bar-item",
                    r#type: "button",
                    onclick: move |_| open_tab(layout, store, DrawerTab::Jobs),
                    "{running} running"
                }
                button {
                    class: "status-bar-item",
                    r#type: "button",
                    onclick: move |_| open_tab(layout, store, DrawerTab::Jobs),
                    "{queued} queued"
                }
                button {
                    class: "status-bar-item status-bar-activity",
                    r#type: "button",
                    onclick: move |_| open_tab(layout, store, DrawerTab::Activity),
                    "No activity yet"
                }
            }
            if settings_open() && !setup_open {
                ErrorBoundary { class_name: Some("setup-slot".into()),
                    SettingsPage {
                        plans: plans.clone().unwrap_or_default(),
                        projects: projects,
                        theme: theme(),
                        tab_order_custom: !layout::is_default_tab_order(&frame.tab_order) || !docks.top.is_empty() || !docks.bottom.is_empty() || !docks.left.is_empty(),
                        on_theme: Callback::new(move |next: ThemeId| {
                            theme.set(next);
                            theme::apply_theme(next, &mut WebStore, &mut WebRoot, &mut WebTarget);
                        }),
                        on_remove_project: Callback::new(move |path: String| remove_project(store, path)),
                        on_open_setup: Callback::new(move |_| {
                            settings_open.set(false);
                            store.setup_open.set(true);
                        }),
                        on_lead_changed: Callback::new(move |_| store.settings_nonce.with_mut(|n| *n += 1)),
                        on_reset_tabs: Callback::new(move |_| {
                            let mut next = layout.read().clone();
                            next.tab_order = DrawerTab::ALL.to_vec();
                            next.dock_tabs.top.clear();
                            next.dock_tabs.bottom.clear();
                            next.dock_tabs.left.clear();
                            write_layout(layout, store, next);
                        }),
                        on_close: Callback::new(move |_| settings_open.set(false)),
                    }
                }
            }
            if setup_open {
                ErrorBoundary { class_name: Some("setup-slot".into()),
                    SetupScreen {
                        bus: store.bus.read().clone(),
                        on_close: Callback::new(move |_| finish_setup(store)),
                        on_skip: Callback::new(move |_| finish_setup(store)),
                    }
                }
            }
        }
    }
}

#[component]
fn SideDock(
    dock: DockId,
    frame: LayoutState,
    docks: layout::Docks,
    jobs_badge: i64,
    change_badge: i64,
    lead_busy: bool,
    popped: Vec<String>,
    on_open: Callback<DrawerTab>,
    on_resize: Callback<i64>,
    on_reset: Callback,
    on_drag: Callback<Option<String>>,
) -> Element {
    let tabs = tabs_of(&docks, dock);
    if tabs.is_empty() {
        return rsx! {};
    }
    let height = match dock {
        DockId::Top => frame.dock_sizes.top,
        DockId::Bottom => frame.dock_sizes.bottom,
        DockId::Left => frame.dock_sizes.left,
        DockId::Right => 0,
    };
    let invert = dock != DockId::Top;
    let handle = rsx! {
        ResizeHandle {
            class_name: format!("layout-handle layout-handle-dock layout-handle-{}", dock_name(dock)),
            orientation: "horizontal",
            value: height,
            min: layout::MIN_DOCK,
            max: layout::MAX_DOCK,
            label: format!("{} height", dock_label(dock)),
            invert: invert,
            on_change: on_resize,
            on_reset: on_reset,
            on_drag: on_drag,
        }
    };
    let body = rsx! {
        section { class: "dock dock-{dock_name(dock)}", aria_label: "{dock_label(dock)}", style: "height: {height}px;",
            TabBar { dock: dock, frame: frame.clone(), docks: docks.clone(), jobs_badge: jobs_badge, change_badge: change_badge, lead_busy: lead_busy, popped: popped.clone(), on_open: on_open }
            Pages { dock: dock, frame: frame, docks: docks }
        }
    };
    if dock == DockId::Top {
        rsx! { {body} {handle} }
    } else {
        rsx! { {handle} {body} }
    }
}

#[component]
fn TabBar(
    dock: DockId,
    frame: LayoutState,
    docks: layout::Docks,
    jobs_badge: i64,
    change_badge: i64,
    lead_busy: bool,
    popped: Vec<String>,
    on_open: Callback<DrawerTab>,
) -> Element {
    let tabs = tabs_of(&docks, dock);
    let active = active_in(&frame, &docks, dock);
    let order = layout::tabs_in_order(&docks);
    rsx! {
        div { class: "drawer-tabs", role: "tablist", aria_label: "{dock_label(dock)}",
            for (index, tab) in tabs.iter().copied().enumerate() {
                {
                    let key_tabs = tabs.clone();
                    let selected = active == Some(tab);
                    let place = order.iter().position(|item| *item == tab);
                    let popped_out = popped.iter().any(|item| item == tab.as_str());
                    let mut class_name = String::from("tab");
                    if selected {
                        class_name.push_str(" is-selected");
                    }
                    if popped_out {
                        class_name.push_str(" is-popped");
                    }
                    rsx! {
                        button {
                            key: "{tab.as_str()}",
                            id: "drawer-tab-{tab.as_str()}",
                            class: "{class_name}",
                            r#type: "button",
                            role: "tab",
                            aria_selected: if selected { "true" } else { "false" },
                            aria_label: "{tab.label()}",
                            aria_keyshortcuts: if place.is_some_and(|place| place < 9) { format!("Control+{}", place.unwrap_or(0) + 1) } else { String::new() },
                            title: "Right-click is not required. Click to show this tab.",
                            tabindex: if selected { "0" } else { "-1" },
                            onclick: move |_| on_open.call(tab),
                            onkeydown: move |event| on_tab_key(&event, &key_tabs, index, on_open),
                            TabIcon { tab: tab }
                            span { class: "tab-label", "{tab.label()}" }
                            if tab == DrawerTab::Jobs && jobs_badge > 0 {
                                span { class: "tab-badge", "{jobs_badge}" }
                            }
                            if tab == DrawerTab::Changes && change_badge > 0 {
                                span { class: "tab-badge", "{change_badge}" }
                            }
                            if tab == DrawerTab::Lead && lead_busy {
                                span { class: "tab-dot", aria_hidden: "true" }
                            }
                            if popped_out {
                                span { class: "tab-popped", aria_label: "in its own window", "↗" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Pages(dock: DockId, frame: LayoutState, docks: layout::Docks) -> Element {
    let tabs = tabs_of(&docks, dock);
    let active = active_in(&frame, &docks, dock);
    rsx! {
        div { class: "drawer-body",
            if tabs.is_empty() {
                p { class: "dock-empty", "Drag a tab here." }
            }
            for tab in tabs {
                {
                    let shown = active == Some(tab);
                    let name = tab.as_str().to_string();
                    rsx! {
                        div { class: if shown { "drawer-page" } else { "drawer-page is-hidden" }, key: "{tab.as_str()}",
                            ErrorBoundary { class_name: None,
                                TabBody { tab: name, frame: frame.clone() }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn TabBody(tab: String, frame: LayoutState) -> Element {
    let mut store = state::use_store();
    let jobs = store.jobs.read().clone();
    let plans = store.plans.read().clone();
    let changes = store.changes.read().clone();
    let terminals = store.terminals.read().clone();
    let messages = store.messages.read().clone();
    let project = store.project.read().clone();
    let primary = store.primary.read().clone();
    let folder = project
        .as_ref()
        .map(|item| folder_name(&item.path))
        .unwrap_or_default();
    let files_label = match &primary {
        None => folder,
        Some(id) => changes
            .iter()
            .find(|change| &change.id == id)
            .and_then(|change| change.name.clone())
            .unwrap_or_else(|| id.clone()),
    };
    let claude = plan_ready(&plans, ProviderId::Claude);
    let cursor = plan_ready(&plans, ProviderId::Cursor);
    let done = jobs
        .iter()
        .filter(|job| job.status == JobStatus::Done)
        .count();
    let refresh = format!("{}:{done}", changes.len());
    let _ = frame;
    match tab.as_str() {
        "files" => rsx! { FilesPanel { r#where: primary.clone(), label: files_label } },
        "lead" => rsx! { LeadChat {
            messages: messages.clone(),
            jobs: jobs,
            plans: plans,
            mode: *store.lead_mode.read(),
            settings_nonce: *store.settings_nonce.read(),
            on_reset: Callback::new(move |direct: Option<ProviderId>| {
                spawn_local(async move {
                    let _ = api::reset_lead(direct).await;
                    if let Ok(list) = api::list_lead_messages().await {
                        store.messages.set(list);
                    }
                    if let Ok(list) = api::list_lead_chats().await {
                        store.chats.set(list);
                    }
                });
            }),
            on_open_terminal: Callback::new(move |provider: ProviderId| {
                open_terminal(store, provider.as_str().into(), store.primary.read().clone(), None);
            }),
            on_show_job: Callback::new(move |id: String| {
                store.selected_job.set(Some(id.clone()));
                let nonce = js_sys::Date::now() as i64;
                store.show_job.set(Some((id, nonce)));
                store.drawer_tab.set("jobs".into());
                store.drawer_open.set(true);
            }),
            on_run_plan: Callback::new(move |(id, jobs): (String, Vec<legate_core::ProposedJob>)| {
                spawn_local(async move {
                    let _ = api::run_lead_proposal(&id, Some(&jobs)).await;
                });
            }),
        } },
        "jobs" => rsx! { JobsPanel {
            jobs: jobs,
            plans: plans,
            changes: changes,
            is_repo: project.as_ref().is_some_and(|item| item.is_repo),
            selected_job_id: store.selected_job.read().clone(),
            show_job: store.show_job.read().clone(),
            on_select_job: Callback::new(move |id: Option<String>| store.selected_job.set(id)),
            on_job: Callback::new(|_| {}),
            on_view_change: Callback::new(move |id: String| {
                store.selected_change.set(Some(id.clone()));
                store.primary.set(Some(id));
                store.drawer_tab.set("changes".into());
            }),
            on_goal: Callback::new(move |_| store.drawer_tab.set("goals".into())),
        } },
        "changes" => rsx! { ChangesTab {
            changes: changes,
            jobs: jobs,
            selected_id: store.selected_change.read().clone().or(primary),
            claude_available: claude,
            cursor_available: cursor,
            opening: *store.opening.read(),
            on_select: Callback::new(move |id: Option<String>| store.selected_change.set(id)),
            on_open_here: Callback::new(move |(provider, id): (ProviderId, String)| {
                open_terminal(store, provider.as_str().into(), Some(id), None);
            }),
        } },
        "routines" => rsx! { Routines {
            project: project.map(|item| item.path).unwrap_or_default(),
            on_run: Callback::new(move |job: JobRecord| {
                store.jobs.with_mut(|list| {
                    if let Some(slot) = list.iter_mut().find(|item| item.id == job.id) {
                        *slot = job.clone();
                    } else {
                        list.push(job.clone());
                    }
                });
                store.drawer_tab.set("jobs".into());
            }),
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
            lead: messages,
            terminals: terminals,
            changes: changes,
            events: Vec::<FlowEvent>::new(),
            now: *store.now.read(),
            starts: std::collections::HashMap::new(),
            flashes: std::collections::HashMap::new(),
            on_show_terminal: Callback::new(move |id: String| store.active_terminal.set(id)),
        } },
        "history" => {
            rsx! { HistoryPanel { r#where: primary, label: files_label, refresh_key: refresh } }
        }
        "goals" => rsx! { GoalsPanel {
            jobs: jobs,
            on_view_change: Callback::new(move |id: String| {
                store.selected_change.set(Some(id.clone()));
                store.primary.set(Some(id));
                store.drawer_tab.set("changes".into());
            }),
            on_show_job: Callback::new(move |id: String| {
                store.selected_job.set(Some(id.clone()));
                store.show_job.set(Some((id, js_sys::Date::now() as i64)));
                store.drawer_tab.set("jobs".into());
            }),
        } },
        _ => rsx! { p { class: "hint", "{tab}" } },
    }
}

#[component]
fn ResizeHandle(
    class_name: String,
    orientation: &'static str,
    value: i64,
    min: i64,
    max: i64,
    label: String,
    invert: bool,
    on_change: Callback<i64>,
    on_reset: Callback,
    on_drag: Callback<Option<String>>,
) -> Element {
    let dragging = use_hook(|| Rc::new(Cell::new(false)));
    let origin = use_hook(|| Rc::new(Cell::new(0.0)));
    let start_value = use_hook(|| Rc::new(Cell::new(0i64)));
    let change_slot = use_hook(|| Rc::new(RefCell::new(on_change)));
    let drag_slot = use_hook(|| Rc::new(RefCell::new(on_drag)));
    *change_slot.borrow_mut() = on_change;
    *drag_slot.borrow_mut() = on_drag;
    let vertical = orientation == "vertical";
    use_hook({
        let dragging = Rc::clone(&dragging);
        let origin = Rc::clone(&origin);
        let start_value = Rc::clone(&start_value);
        let change_slot = Rc::clone(&change_slot);
        let drag_slot = Rc::clone(&drag_slot);
        move || {
            let drag_flag = Rc::clone(&dragging);
            let origin_cell = Rc::clone(&origin);
            let start_cell = Rc::clone(&start_value);
            let changes = Rc::clone(&change_slot);
            let move_listener = Closure::wrap(Box::new(move |event: web_sys::Event| {
                if !drag_flag.get() {
                    return;
                }
                let pos = client_axis(&event, vertical);
                let delta = pos - origin_cell.get();
                let next = if invert {
                    start_cell.get() - delta as i64
                } else {
                    start_cell.get() + delta as i64
                };
                changes.borrow().call(next.clamp(min, max));
            }) as Box<dyn FnMut(web_sys::Event)>);
            let drag_flag = Rc::clone(&dragging);
            let drags = Rc::clone(&drag_slot);
            let up_listener = Closure::wrap(Box::new(move |_event: web_sys::Event| {
                if !drag_flag.get() {
                    return;
                }
                drag_flag.set(false);
                drags.borrow().call(None);
                layout::request_layout_fit();
            }) as Box<dyn FnMut(web_sys::Event)>);
            if let Some(window) = web_sys::window() {
                let _ = window.add_event_listener_with_callback(
                    "pointermove",
                    move_listener.as_ref().unchecked_ref(),
                );
                let _ = window.add_event_listener_with_callback(
                    "pointerup",
                    up_listener.as_ref().unchecked_ref(),
                );
                let _ = window.add_event_listener_with_callback(
                    "pointercancel",
                    up_listener.as_ref().unchecked_ref(),
                );
            }
            move_listener.forget();
            up_listener.forget();
        }
    });
    let drag_name = orientation.to_string();
    rsx! {
        button {
            class: "{class_name}",
            r#type: "button",
            role: "separator",
            aria_label: "{label}",
            aria_orientation: "{orientation}",
            aria_valuenow: "{value}",
            aria_valuemin: "{min}",
            aria_valuemax: "{max}",
            tabindex: "0",
            onpointerdown: move |event| {
                let point = event.client_coordinates();
                origin.set(if vertical { point.x } else { point.y });
                start_value.set(value);
                dragging.set(true);
                on_drag.call(Some(drag_name.clone()));
            },
            ondoubleclick: move |_| {
                on_reset.call(());
                layout::request_layout_fit();
            },
            onkeydown: move |event| {
                if event.key().to_string() == "Enter" {
                    event.prevent_default();
                    on_reset.call(());
                    layout::request_layout_fit();
                    return;
                }
                let step = 16;
                let delta = if vertical {
                    if event.key().to_string() == "ArrowLeft" { -step } else if event.key().to_string() == "ArrowRight" { step } else { 0 }
                } else if event.key().to_string() == "ArrowUp" {
                    -step
                } else if event.key().to_string() == "ArrowDown" {
                    step
                } else {
                    0
                };
                if delta == 0 {
                    return;
                }
                event.prevent_default();
                let next = if invert { value - delta } else { value + delta };
                on_change.call(next.clamp(min, max));
            },
        }
    }
}

fn on_tab_key(
    event: &Event<KeyboardData>,
    tabs: &[DrawerTab],
    index: usize,
    on_open: Callback<DrawerTab>,
) {
    let next = if event.key().to_string() == "ArrowRight" {
        tabs.get(index + 1).copied()
    } else if event.key().to_string() == "ArrowLeft" {
        index
            .checked_sub(1)
            .and_then(|slot| tabs.get(slot).copied())
    } else {
        None
    };
    if let Some(tab) = next {
        event.prevent_default();
        on_open.call(tab);
    }
}

fn app_class(dragging: Option<&str>, flags: LayoutFlags, overlay: bool) -> String {
    let mut names = vec!["app".to_string()];
    if dragging.is_some() {
        names.push("is-dragging".into());
    }
    if dragging == Some("vertical") {
        names.push("is-dragging-col".into());
    }
    if dragging == Some("horizontal") {
        names.push("is-dragging-row".into());
    }
    if !flags.sidebar_shown {
        names.push("is-hide-sidebar".into());
    }
    if !flags.drawer_open {
        names.push("is-hide-drawer".into());
    }
    if overlay {
        names.push("is-drawer-overlay".into());
    }
    names.join(" ")
}

fn docks_of(state: &LayoutState) -> layout::Docks {
    let mut seen = Vec::new();
    let take = |list: &[DrawerTab], seen: &mut Vec<DrawerTab>| {
        let mut out = Vec::new();
        for tab in list {
            if !seen.contains(tab) {
                seen.push(*tab);
                out.push(*tab);
            }
        }
        out
    };
    let top = take(&state.dock_tabs.top, &mut seen);
    let bottom = take(&state.dock_tabs.bottom, &mut seen);
    let left = take(&state.dock_tabs.left, &mut seen);
    let mut right = Vec::new();
    for tab in &state.tab_order {
        if !seen.contains(tab) {
            seen.push(*tab);
            right.push(*tab);
        }
    }
    for tab in DrawerTab::ALL {
        if !seen.contains(&tab) {
            right.push(tab);
        }
    }
    layout::Docks {
        right,
        top,
        bottom,
        left,
    }
}

fn tabs_of(docks: &layout::Docks, dock: DockId) -> Vec<DrawerTab> {
    match dock {
        DockId::Right => docks.right.clone(),
        DockId::Top => docks.top.clone(),
        DockId::Bottom => docks.bottom.clone(),
        DockId::Left => docks.left.clone(),
    }
}

fn active_in(state: &LayoutState, docks: &layout::Docks, dock: DockId) -> Option<DrawerTab> {
    let tabs = tabs_of(docks, dock);
    let preferred = match dock {
        DockId::Right => Some(state.drawer_tab),
        DockId::Top => state.dock_active.top,
        DockId::Bottom => state.dock_active.bottom,
        DockId::Left => state.dock_active.left,
    };
    preferred
        .filter(|tab| tabs.contains(tab))
        .or_else(|| tabs.first().copied())
}

fn dock_name(dock: DockId) -> &'static str {
    match dock {
        DockId::Right => "right",
        DockId::Top => "top",
        DockId::Bottom => "bottom",
        DockId::Left => "left",
    }
}

fn dock_label(dock: DockId) -> &'static str {
    match dock {
        DockId::Right => "Right panel",
        DockId::Top => "Above the terminals",
        DockId::Bottom => "Below the terminals",
        DockId::Left => "Under the projects",
    }
}

fn write_layout(mut layout: Signal<LayoutState>, mut store: Store, mut next: LayoutState) {
    let flags = LayoutFlags {
        sidebar_shown: next.sidebar_shown,
        drawer_open: next.drawer_open,
    };
    next.sizes = layout::clamp_sizes(next.sizes, viewport(), flags);
    store.sidebar_width.set(next.sizes.sidebar);
    store.drawer_width.set(next.sizes.drawer);
    store.sidebar_shown.set(next.sidebar_shown);
    store.drawer_open.set(next.drawer_open);
    store.drawer_tab.set(next.drawer_tab.as_str().to_string());
    store.pane_layout.set(next.pane_layout.as_str().to_string());
    let mut storage = WebStore;
    layout::save_layout(&next, &mut storage);
    layout.set(next);
}

fn open_tab(layout: Signal<LayoutState>, store: Store, tab: DrawerTab) {
    let current = layout.read().clone();
    let docks = docks_of(&current);
    let mut next = current;
    match layout::dock_of(&docks, tab) {
        DockId::Right => {
            next.drawer_tab = tab;
            next.drawer_open = true;
        }
        DockId::Top => next.dock_active.top = Some(tab),
        DockId::Bottom => next.dock_active.bottom = Some(tab),
        DockId::Left => next.dock_active.left = Some(tab),
    }
    write_layout(layout, store, next);
}

fn resize_bar(layout: Signal<LayoutState>, store: Store, key: SizeKey, value: i64) {
    let mut next = layout.read().clone();
    match key {
        SizeKey::Sidebar => next.sizes.sidebar = value,
        SizeKey::Drawer => next.sizes.drawer = value,
    }
    write_layout(layout, store, next);
}

fn reset_bar(layout: Signal<LayoutState>, store: Store, key: SizeKey) {
    resize_bar(
        layout,
        store,
        key,
        match key {
            SizeKey::Sidebar => layout::DEFAULT_SIZES.sidebar,
            SizeKey::Drawer => layout::DEFAULT_SIZES.drawer,
        },
    );
}

fn resize_dock(layout: Signal<LayoutState>, store: Store, dock: DockId, value: i64) {
    let mut next = layout.read().clone();
    let value = value.clamp(layout::MIN_DOCK, layout::MAX_DOCK);
    match dock {
        DockId::Top => next.dock_sizes.top = value,
        DockId::Bottom => next.dock_sizes.bottom = value,
        DockId::Left => next.dock_sizes.left = value,
        DockId::Right => {}
    }
    write_layout(layout, store, next);
}

fn reset_dock(layout: Signal<LayoutState>, store: Store, dock: DockId) {
    let value = match dock {
        DockId::Top => layout::DEFAULT_DOCK_SIZES.top,
        DockId::Bottom => layout::DEFAULT_DOCK_SIZES.bottom,
        DockId::Left => layout::DEFAULT_DOCK_SIZES.left,
        DockId::Right => return,
    };
    resize_dock(layout, store, dock, value);
}

fn viewport() -> Viewport {
    let Some(window) = web_sys::window() else {
        return Viewport {
            width: 1280.0,
            height: 800.0,
        };
    };
    Viewport {
        width: window
            .inner_width()
            .ok()
            .and_then(|value| value.as_f64())
            .unwrap_or(1280.0),
        height: window
            .inner_height()
            .ok()
            .and_then(|value| value.as_f64())
            .unwrap_or(800.0),
    }
}

fn plan_ready(plans: &Option<Vec<legate_core::PlanStatus>>, id: ProviderId) -> bool {
    plans
        .as_ref()
        .is_some_and(|list| list.iter().any(|plan| plan.id == id && plan.available))
}

fn meter_plans(plans: &[legate_core::PlanStatus]) -> Vec<legate_core::PlanStatus> {
    plans
        .iter()
        .filter(|plan| plan_of(plan.id) == plan.id)
        .cloned()
        .collect()
}

fn plan_bar_label(plan: &legate_core::PlanStatus, now: i64, shows: UsageShows) -> (String, bool) {
    let name = plan.id.as_str();
    if plan.problem == Some(PlanProblem::NotInstalled) {
        return (format!("{name} not installed"), true);
    }
    if plan.problem == Some(PlanProblem::SignedOut) || !plan.available {
        return (format!("{name} signed out"), true);
    }
    if plan.resting_until.is_some_and(|until| until > now) {
        return (format!("{name} resting"), false);
    }
    let word = match shows {
        UsageShows::Left => "left",
        UsageShows::Used => "used",
    };
    (
        format!("{name} {}% {word}", usage_figure(plan.used, shows)),
        false,
    )
}

fn prune_selection(mut store: Store) {
    let changes = store.changes.read().clone();
    let mut selected = store.selected.read().clone();
    selected
        .retain(|id| id.is_none() || changes.iter().any(|change| id.as_ref() == Some(&change.id)));
    if selected.is_empty() {
        selected.push(None);
    }
    if selected != *store.selected.read() {
        store.selected.set(selected);
    }
    let primary = store.primary.read().clone();
    if primary
        .as_ref()
        .is_some_and(|id| changes.iter().all(|change| &change.id != id))
    {
        store.primary.set(None);
    }
}

fn select_worktree(mut store: Store, id: Option<String>, mode: &str) {
    let changes = store.changes.read().clone();
    let mut keys = vec![None];
    keys.extend(changes.into_iter().map(|change| Some(change.id)));
    let mut selected = store.selected.read().clone();
    match mode {
        "toggle" => {
            if selected.iter().any(|item| item == &id) {
                selected.retain(|item| item != &id);
                if selected.is_empty() {
                    selected.push(None);
                }
            } else {
                selected.push(id.clone());
            }
        }
        "range" => {
            let anchor = store.primary.read().clone();
            let start = keys.iter().position(|key| key == &anchor).unwrap_or(0);
            let end = keys.iter().position(|key| key == &id).unwrap_or(start);
            let (from, to) = if start <= end {
                (start, end)
            } else {
                (end, start)
            };
            selected = keys[from..=to].to_vec();
        }
        _ => selected = vec![id.clone()],
    }
    store.primary.set(id);
    store.selected.set(selected);
}

fn select_all(mut store: Store) {
    let changes = store.changes.read().clone();
    let mut keys = vec![None];
    keys.extend(changes.into_iter().map(|change| Some(change.id)));
    let selected = store.selected.read().clone();
    let all = keys
        .iter()
        .all(|key| selected.iter().any(|item| item == key));
    if all {
        store.selected.set(vec![None]);
        store.primary.set(None);
    } else {
        store.primary.set(keys.first().cloned().flatten());
        store.selected.set(keys);
    }
}

fn create_worktrees(
    mut store: Store,
    name: Option<String>,
    count: i64,
    start: Option<ProviderId>,
    prompt: Option<String>,
) {
    let count = count.clamp(1, 6);
    spawn_local(async move {
        let mut made = Vec::new();
        for index in 1..=count {
            let named = match &name {
                Some(base) if count > 1 => Some(format!("{base} {index}")),
                other => other.clone(),
            };
            match api::create_worktree(named.as_deref()).await {
                Ok(change) => {
                    let id = change.id.clone();
                    store.changes.with_mut(|list| list.push(change));
                    if let Some(provider) = start {
                        let _ = open_terminal_now(
                            store,
                            provider.as_str(),
                            Some(&id),
                            prompt.as_deref(),
                        )
                        .await;
                    }
                    made.push(Some(id));
                }
                Err(text) => {
                    store.error.set(Some(text));
                    break;
                }
            }
        }
        if !made.is_empty() {
            store.primary.set(made.first().cloned().flatten());
            store.selected.set(made);
        }
    });
}

async fn open_terminal_now(
    mut store: Store,
    provider: &str,
    worktree: Option<&str>,
    prompt: Option<&str>,
) -> Result<(), String> {
    match api::open_terminal(provider, 100.0, 30.0, worktree, prompt).await {
        Ok(info) => {
            remember_terminal(store, info);
            Ok(())
        }
        Err(text) => {
            store.error.set(Some(text.clone()));
            Err(text)
        }
    }
}

fn remember_terminal(mut store: Store, info: TerminalInfo) {
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

fn choose_project(mut store: Store) {
    spawn_local(async move {
        match api::choose_project().await {
            Ok(Some(info)) => adopt(store, info).await,
            Ok(None) => {}
            Err(text) => store.error.set(Some(text)),
        }
    });
}

fn switch_project(mut store: Store, path: String, worktree: Option<String>) {
    spawn_local(async move {
        match api::switch_project(&path).await {
            Ok(info) => {
                adopt(store, info).await;
                store.primary.set(worktree.clone());
                store.selected.set(vec![worktree]);
            }
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
    store.primary.set(None);
    store.selected.set(vec![None]);
    if let Ok(list) = api::list_projects().await {
        store.projects.set(list);
    }
    if let Ok(list) = api::list_changes().await {
        store.changes.set(list);
    }
    if let Ok(list) = api::list_lead_chats().await {
        store.chats.set(list);
    }
}

fn refresh_projects(store: Store) {
    spawn_local(async move {
        if let Ok(info) = api::get_project().await {
            adopt(store, info).await;
        }
    });
}

fn open_terminal(
    mut store: Store,
    provider: String,
    worktree: Option<String>,
    prompt: Option<String>,
) {
    store.opening.set(true);
    spawn_local(async move {
        let _ = open_terminal_now(store, &provider, worktree.as_deref(), prompt.as_deref()).await;
        store.opening.set(false);
    });
}

fn open_chat(mut store: Store, layout: Signal<LayoutState>, id: String) {
    spawn_local(async move {
        if let Ok(messages) = api::open_lead_chat(&id).await {
            store.messages.set(messages);
            if let Ok(chats) = api::list_lead_chats().await {
                store.chats.set(chats);
            }
            open_tab(layout, store, DrawerTab::Lead);
        }
    });
}

fn reset_chat(mut store: Store, layout: Signal<LayoutState>) {
    spawn_local(async move {
        let _ = api::reset_lead(None).await;
        if let Ok(chats) = api::list_lead_chats().await {
            store.chats.set(chats);
        }
        if let Ok(messages) = api::list_lead_messages().await {
            store.messages.set(messages);
        }
        open_tab(layout, store, DrawerTab::Lead);
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

fn document() -> Option<web_sys::Document> {
    web_sys::window()?.document()
}

fn client_axis(event: &web_sys::Event, vertical: bool) -> f64 {
    let key = if vertical { "clientX" } else { "clientY" };
    js_sys::Reflect::get(event, &wasm_bindgen::JsValue::from_str(key))
        .ok()
        .and_then(|value| value.as_f64())
        .unwrap_or(0.0)
}

fn in_terminal(event: &web_sys::KeyboardEvent) -> bool {
    let Some(target) = event.target() else {
        return false;
    };
    let Ok(element) = target.dyn_into::<web_sys::Element>() else {
        return false;
    };
    element
        .closest(".terminal-host, .xterm")
        .ok()
        .flatten()
        .is_some()
}

fn install_keys(
    store: Store,
    layout: Signal<LayoutState>,
    mut settings_open: Signal<bool>,
    mut task_open: Signal<bool>,
) {
    let closure = Closure::wrap(Box::new(move |event: web_sys::KeyboardEvent| {
        if in_terminal(&event) {
            return;
        }
        let key = event.key();
        let code = event.code();
        if event.ctrl_key() && event.shift_key() && !event.alt_key() && (key == "n" || key == "N") {
            event.prevent_default();
            spawn_local(async {
                let _ = api::new_window().await;
            });
            return;
        }
        if event.ctrl_key() && !event.shift_key() && !event.alt_key() && key == "," {
            event.prevent_default();
            let open = settings_open();
            settings_open.set(!open);
            return;
        }
        if event.ctrl_key() && !event.shift_key() && !event.alt_key() && (key == "n" || key == "N")
        {
            event.prevent_default();
            task_open.set(true);
            return;
        }
        let shortcut = layout::layout_shortcut(layout::KeyPress {
            key: &key,
            code: Some(&code),
            ctrl_key: event.ctrl_key(),
            shift_key: event.shift_key(),
            alt_key: event.alt_key(),
            meta_key: event.meta_key(),
        });
        let Some(shortcut) = shortcut else {
            return;
        };
        event.prevent_default();
        let current = layout.read().clone();
        match shortcut {
            LayoutShortcut::Drawer => {
                let mut next = current;
                next.drawer_open = !next.drawer_open;
                write_layout(layout, store, next);
            }
            LayoutShortcut::Sidebar => {
                let mut next = current;
                next.sidebar_shown = !next.sidebar_shown;
                write_layout(layout, store, next);
            }
            LayoutShortcut::Tab(tab) => {
                let order = layout::tabs_in_order(&docks_of(&current));
                let index = DrawerTab::ALL.iter().position(|item| *item == tab);
                let target = index
                    .and_then(|slot| order.get(slot).copied())
                    .unwrap_or(tab);
                open_tab(layout, store, target);
            }
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
    // React mounts on `#root`, which `main.css` sizes to the window. Dioxus
    // mounts on `#main`, so the grid's `height: 100%` otherwise collapses.
    if let Some(mount) = document
        .get_element_by_id("main")
        .or_else(|| document.get_element_by_id("root"))
    {
        let _ = mount.set_attribute("id", "root");
    }
    if document.get_element_by_id("legate-mount-fill").is_none()
        && let Ok(style) = document.create_element("style")
    {
        let _ = style.set_attribute("id", "legate-mount-fill");
        style.set_text_content(Some(
            "#main,#root{height:100%;min-height:0;overflow:hidden}#main>*,#root>*{height:100%;min-height:0}",
        ));
        if let Some(head) = document.head() {
            let _ = head.append_child(&style);
        }
    }
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

struct WebTarget;

impl ThemeTarget for WebTarget {
    fn dispatch(&mut self, name: &str) {
        let Some(window) = web_sys::window() else {
            return;
        };
        if let Ok(event) = web_sys::Event::new(name) {
            let _ = window.dispatch_event(&event);
        }
    }
}
