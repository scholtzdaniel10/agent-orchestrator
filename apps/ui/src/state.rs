//! Signals that mirror the React state in `App.tsx`.

use std::collections::HashMap;

use dioxus::prelude::*;
use legate_core::{
    ChangeSet, CursorModels, DEFAULT_LOCAL_MODEL, DEFAULT_LOCAL_NUM_CTX, DEFAULT_LOCAL_URL,
    JobRecord, LeadChat, PlanStatus, Prefs, ProjectEntry, ProjectInfo, TerminalInfo, UsageShows,
};
use wasm_bindgen_futures::spawn_local;

use crate::api::{self, LeadUpdate, TerminalData, TerminalUpdate};
use crate::logic::terminal_bus::{TerminalBus, create_terminal_bus};

#[derive(Clone, Copy)]
pub struct Store {
    pub jobs: Signal<Vec<JobRecord>>,
    pub plans: Signal<Option<Vec<PlanStatus>>>,
    pub changes: Signal<Vec<ChangeSet>>,
    pub prefs: Signal<Prefs>,
    pub terminals: Signal<Vec<TerminalInfo>>,
    pub projects: Signal<Vec<ProjectEntry>>,
    pub project: Signal<Option<ProjectInfo>>,
    pub chats: Signal<Vec<LeadChat>>,
    pub github_available: Signal<bool>,
    pub popped: Signal<Vec<String>>,
    pub popped_panels: Signal<Vec<String>>,
    pub error: Signal<Option<String>>,
    pub now: Signal<i64>,
    pub setup_open: Signal<bool>,
    pub active_terminal: Signal<String>,
    pub opening: Signal<bool>,
    pub activity: Signal<HashMap<String, i64>>,
    pub selected: Signal<Vec<Option<String>>>,
    pub pane_layout: Signal<String>,
    pub drawer_tab: Signal<String>,
    pub drawer_open: Signal<bool>,
    pub sidebar_shown: Signal<bool>,
    pub sidebar_width: Signal<i64>,
    pub bus: Signal<TerminalBus>,
}

pub fn provide_store() -> Store {
    let store = Store {
        jobs: use_signal(Vec::new),
        plans: use_signal(|| None),
        changes: use_signal(Vec::new),
        prefs: use_signal(default_prefs),
        terminals: use_signal(Vec::new),
        projects: use_signal(Vec::new),
        project: use_signal(|| None),
        chats: use_signal(Vec::new),
        github_available: use_signal(|| false),
        popped: use_signal(Vec::new),
        popped_panels: use_signal(Vec::new),
        error: use_signal(|| None),
        now: use_signal(now_ms),
        setup_open: use_signal(|| false),
        active_terminal: use_signal(String::new),
        opening: use_signal(|| false),
        activity: use_signal(HashMap::new),
        selected: use_signal(|| vec![None]),
        pane_layout: use_signal(|| "auto".to_string()),
        drawer_tab: use_signal(|| "usage".to_string()),
        drawer_open: use_signal(|| true),
        sidebar_shown: use_signal(|| true),
        sidebar_width: use_signal(|| 240),
        bus: use_signal(make_bus),
    };
    use_context_provider(|| store)
}

pub fn use_store() -> Store {
    use_context()
}

pub fn boot(store: Store) {
    let clock = store.now;
    spawn_local(async move {
        load(store).await;
        listen(store).await;
    });
    spawn_local(async move {
        let mut clock = clock;
        loop {
            gloo_timers::future::TimeoutFuture::new(30_000).await;
            clock.set(now_ms());
        }
    });
}

async fn load(mut store: Store) {
    if let Ok(list) = api::list_jobs().await {
        store.jobs.set(list);
    }
    if let Ok(list) = api::list_plans().await {
        store.plans.set(Some(list));
    }
    if let Ok(list) = api::list_changes().await {
        store.changes.set(list);
    }
    if let Ok(prefs) = api::get_prefs().await {
        store.prefs.set(prefs);
    }
    if let Ok(list) = api::list_terminals().await {
        if store.active_terminal.read().is_empty()
            && let Some(first) = list.first()
        {
            store.active_terminal.set(first.id.clone());
        }
        store.terminals.set(list);
    }
    if let Ok(list) = api::list_projects().await {
        store.projects.set(list);
    }
    if let Ok(info) = api::get_project().await {
        store.project.set(Some(info));
    }
    if let Ok(list) = api::list_lead_chats().await {
        store.chats.set(list);
    }
    if let Ok(ready) = api::github_available().await {
        store.github_available.set(ready);
    }
    if let Ok(list) = api::popped_out().await {
        store.popped.set(list);
    }
    if let Ok(list) = api::popped_panels().await {
        store.popped_panels.set(list);
    }
}

async fn listen(store: Store) {
    let mut jobs = store.jobs;
    let _ = api::on_jobs(move |job| {
        jobs.with_mut(|list| upsert(list, job, |item| &item.id));
    })
    .await;
    let mut plans = store.plans;
    let _ = api::on_plans(move |list| plans.set(Some(list))).await;
    let mut changes = store.changes;
    let _ = api::on_changes(move |list| changes.set(list)).await;
    let mut prefs = store.prefs;
    let _ = api::on_prefs(move |next| prefs.set(next)).await;
    let mut terminals = store.terminals;
    let mut active = store.active_terminal;
    let _ = api::on_terminal_update(move |TerminalUpdate { info, removed }| {
        if removed {
            terminals.with_mut(|list| list.retain(|item| item.id != info.id));
            if active.read().as_str() == info.id {
                let next = terminals
                    .read()
                    .first()
                    .map(|item| item.id.clone())
                    .unwrap_or_default();
                active.set(next);
            }
            return;
        }
        terminals.with_mut(|list| upsert(list, info, |item| &item.id));
    })
    .await;
    let mut activity = store.activity;
    let mut now = store.now;
    let _ = api::on_terminal_data(move |TerminalData { id, data: _ }| {
        let stamp = now_ms();
        now.set(stamp);
        activity.with_mut(|map| {
            map.insert(id, stamp);
        });
    })
    .await;
    let mut popped = store.popped;
    let _ = api::on_popped_out(move |list| popped.set(list)).await;
    let mut panels = store.popped_panels;
    let _ = api::on_popped_panels(move |list| panels.set(list)).await;
    let _ = api::on_lead(move |LeadUpdate { .. }| {}).await;
    let _ = api::on_goals(move |_| {}).await;
    let _ = api::on_triggers(move |_| {}).await;
}

fn upsert<T>(list: &mut Vec<T>, next: T, id: impl Fn(&T) -> &str) {
    let key = id(&next).to_string();
    if let Some(slot) = list.iter_mut().find(|item| id(item) == key) {
        *slot = next;
    } else {
        list.push(next);
    }
}

fn make_bus() -> TerminalBus {
    create_terminal_bus(
        |mut deliver| {
            spawn_local(async move {
                let _ = api::on_terminal_data(move |chunk| {
                    deliver(chunk.id, chunk.data);
                })
                .await;
            });
            Box::new(|| {})
        },
        None,
    )
}

pub fn now_ms() -> i64 {
    js_sys::Date::now() as i64
}

pub fn default_prefs() -> Prefs {
    Prefs {
        max_parallel: None,
        terminal_font_size: 13,
        notify: true,
        clone_dir: None,
        ui_scale: 100,
        usage_shows: UsageShows::Left,
        cursor_models: CursorModels::Own,
        usage_check: true,
        local_url: DEFAULT_LOCAL_URL.to_string(),
        local_model: DEFAULT_LOCAL_MODEL.to_string(),
        local_small_model: None,
        local_num_ctx: DEFAULT_LOCAL_NUM_CTX,
    }
}

pub fn note_error(mut store: Store, result: Result<(), String>) {
    if let Err(text) = result {
        store.error.set(Some(text));
    }
}
