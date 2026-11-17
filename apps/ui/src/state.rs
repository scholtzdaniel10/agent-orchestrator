//! Signals that mirror the React state in `App.tsx`.

use std::collections::HashMap;

use dioxus::prelude::*;
use legate_core::{
    ChangeSet, CursorModels, DEFAULT_LOCAL_MODEL, DEFAULT_LOCAL_NUM_CTX, DEFAULT_LOCAL_URL,
    JobRecord, LeadChat, LeadMessage, PlanStatus, Prefs, ProjectEntry, ProjectInfo, ProviderId,
    TerminalInfo, UsageShows,
};
use wasm_bindgen_futures::spawn_local;

use crate::api::{self, LeadUpdate, TerminalData, TerminalUpdate};
use crate::logic::terminal_bus::{TerminalBus, create_terminal_bus};
use crate::wire::{Goal, GoalStatus};

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
    pub drawer_width: Signal<i64>,
    pub bus: Signal<TerminalBus>,
    pub messages: Signal<Vec<LeadMessage>>,
    pub lead_mode: Signal<Option<ProviderId>>,
    pub goals_waiting: Signal<Vec<String>>,
    pub primary: Signal<Option<String>>,
    pub selected_job: Signal<Option<String>>,
    pub show_job: Signal<Option<(String, i64)>>,
    pub selected_change: Signal<Option<String>>,
    pub settings_nonce: Signal<i64>,
    pub booted: Signal<bool>,
    pub project_request: Signal<u64>,
    pub project_switching: Signal<bool>,
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
        drawer_width: use_signal(|| 420),
        bus: use_signal(make_bus),
        messages: use_signal(Vec::new),
        lead_mode: use_signal(|| None),
        goals_waiting: use_signal(Vec::new),
        primary: use_signal(|| None),
        selected_job: use_signal(|| None),
        show_job: use_signal(|| None),
        selected_change: use_signal(|| None),
        settings_nonce: use_signal(|| 0),
        booted: use_signal(|| false),
        project_request: use_signal(|| 0),
        project_switching: use_signal(|| false),
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
    if let Ok(list) = api::list_project_terminals().await {
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
    if let Ok(list) = api::list_lead_messages().await {
        store.messages.set(list);
    }
    if let Ok(mode) = api::get_lead_mode().await {
        store.lead_mode.set(mode);
    }
    if let Ok(list) = api::list_goals().await {
        store.goals_waiting.set(waiting_ids(&list));
    }
    store.booted.set(true);
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
        if terminals.read().iter().any(|item| item.id == info.id) {
            terminals.with_mut(|list| upsert(list, info, |item| &item.id));
        } else {
            let generation = *store.project_request.read();
            spawn_local(async move {
                if let Ok(list) = api::list_project_terminals().await
                    && *store.project_request.read() == generation
                {
                    terminals.set(list);
                }
            });
        }
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
    let mut messages = store.messages;
    let chats = store.chats;
    let _ = api::on_lead(move |LeadUpdate { message, .. }| {
        if *store.project_switching.read() {
            return;
        }
        messages.with_mut(|list| upsert(list, message, |item| &item.id));
        let chats = chats;
        let generation = *store.project_request.read();
        spawn_local(async move {
            if let Ok(list) = api::list_lead_chats().await {
                let mut chats = chats;
                if *store.project_request.read() == generation {
                    chats.set(list);
                }
            }
        });
    })
    .await;
    let mut waiting = store.goals_waiting;
    let _ = api::on_goals(move |goal: Goal| {
        waiting.with_mut(|ids| {
            let here = goal.status == GoalStatus::Waiting;
            let has = ids.iter().any(|id| id == &goal.id);
            if here && !has {
                ids.push(goal.id);
            } else if !here {
                ids.retain(|id| id != &goal.id);
            }
        });
    })
    .await;
    let _ = api::on_triggers(move |_| {}).await;
}

fn waiting_ids(goals: &[Goal]) -> Vec<String> {
    goals
        .iter()
        .filter(|goal| goal.status == GoalStatus::Waiting)
        .map(|goal| goal.id.clone())
        .collect()
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

/// A component's event subscriptions, ended when the component unmounts. One that resolves
/// after the unmount is ended at once.
#[derive(Clone, Default)]
pub struct Subscriptions(std::rc::Rc<std::cell::RefCell<SubscriptionSlot>>);

#[derive(Default)]
struct SubscriptionSlot {
    closed: bool,
    held: Vec<api::Subscription>,
}

impl Subscriptions {
    pub fn hold(&self, subscription: api::Subscription) {
        let mut slot = self.0.borrow_mut();
        if slot.closed {
            drop(slot);
            drop(subscription);
        } else {
            slot.held.push(subscription);
        }
    }

    fn close(&self) {
        let held = {
            let mut slot = self.0.borrow_mut();
            slot.closed = true;
            std::mem::take(&mut slot.held)
        };
        drop(held);
    }
}

pub fn use_subscriptions() -> Subscriptions {
    let subscriptions = use_hook(Subscriptions::default);
    let closing = subscriptions.clone();
    use_drop(move || closing.close());
    subscriptions
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
