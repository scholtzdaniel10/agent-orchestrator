use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use dioxus::html::{Key, Modifiers};
use dioxus::prelude::*;
use legate_core::{
    ChangeSet, JobRecord, JobStatus, LeadMessage, LeadRole, LeadStatus, PlanProblem, PlanStatus,
    ProviderId, TerminalInfo, TerminalProvider, TerminalStatus, plan_of,
};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use super::bot_avatar::{BotAvatar, BotName, BotState};
use super::sidebar_menu::{MenuEntry, SidebarMenu};
use super::star::Star;
use crate::api::{self, LeadUpdate};
use crate::logic::flow_events::{
    FlowEvent, FlowSnapshot, format_hours_minutes, format_log_time, observe_flow,
};
use crate::state;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = ResizeObserver)]
    type JsResizeObserver;

    #[wasm_bindgen(constructor)]
    fn new(callback: &js_sys::Function) -> JsResizeObserver;

    #[wasm_bindgen(method, js_name = observe)]
    fn observe(this: &JsResizeObserver, target: &web_sys::Element);
}

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
    let mut lead_local = use_signal(Vec::<LeadMessage>::new);
    let mut lead_ready = use_signal(|| false);
    let mut local_events = use_signal(Vec::<FlowEvent>::new);
    let mut local_starts = use_signal(HashMap::<String, i64>::new);
    let mut local_flashes = use_signal(HashMap::<String, i64>::new);
    let across = use_signal(|| false);
    let mut tick = use_signal(|| 0i64);
    let prev = use_hook(|| Rc::new(RefCell::new(None::<FlowSnapshot>)));
    let next_id = use_hook(|| Rc::new(Cell::new(1i64)));
    let prev_jobs = use_hook(|| Rc::new(RefCell::new(Vec::<JobRecord>::new())));
    let watch = use_hook(|| Rc::new(Cell::new(false)));
    let clock = use_hook(|| Rc::new(Cell::new(0u64)));

    use_effect(move || {
        spawn_local(async move {
            if let Ok(list) = api::list_lead_messages().await {
                lead_local.set(list);
            }
            lead_ready.set(true);
        });
        spawn_local(async move {
            let _ = api::on_lead(move |LeadUpdate { message, .. }| {
                lead_local.with_mut(|list| upsert_message(list, message));
            })
            .await;
        });
    });

    let running = jobs.iter().any(|job| job.status == JobStatus::Running);
    let mut running_flag = use_signal(|| running);
    if running_flag() != running {
        running_flag.set(running);
    }
    use_effect(move || {
        let ticket = clock.get() + 1;
        clock.set(ticket);
        if !running_flag() {
            return;
        }
        let clock = clock.clone();
        spawn_local(async move {
            while clock.get() == ticket {
                gloo_timers::future::TimeoutFuture::new(1_000).await;
                if clock.get() != ticket {
                    break;
                }
                tick.set(state::now_ms());
            }
        });
    });

    let using_local_lead = lead.is_empty();
    let lead_view = if using_local_lead {
        lead_local()
    } else {
        lead.clone()
    };
    let ready = !using_local_lead || lead_ready();
    if ready {
        let snap = FlowSnapshot {
            jobs: jobs.clone(),
            plans: plans.clone(),
            lead: lead_view.clone(),
            terminals: terminals.clone(),
            changes: changes.clone(),
        };
        let diff = {
            let mut slot = prev.borrow_mut();
            if slot.as_ref() == Some(&snap) {
                Vec::new()
            } else {
                let diff = observe_flow(slot.as_ref(), &snap, state::now_ms());
                *slot = Some(snap);
                diff
            }
        };
        if !diff.is_empty() {
            local_events.with_mut(|current| {
                for change in diff {
                    let id = next_id.get();
                    next_id.set(id + 1);
                    current.push(change.with_id(id));
                }
                if current.len() > 200 {
                    let drop_count = current.len() - 200;
                    current.drain(0..drop_count);
                }
            });
        }
    }

    {
        let tracked = {
            let previous = prev_jobs.borrow();
            track_runs(
                &local_starts.read(),
                &local_flashes.read(),
                &previous,
                &jobs,
                state::now_ms(),
            )
        };
        *prev_jobs.borrow_mut() = jobs.clone();
        if tracked.2 {
            local_starts.set(tracked.0);
            local_flashes.set(tracked.1);
        }
    }

    let elapsed_now = if tick() == 0 { now } else { tick() };
    let shown_events = if events.is_empty() {
        local_events()
    } else {
        events
    };
    let shown_starts = if starts.is_empty() {
        local_starts()
    } else {
        starts
    };
    let shown_flashes = if flashes.is_empty() {
        local_flashes()
    } else {
        flashes
    };
    let across_now = across();
    let mut layout_stamp = use_signal(String::new);
    let layout_mark = format!("{}:{}:{}", jobs.len(), plans.len(), across_now);
    if layout_stamp() != layout_mark {
        layout_stamp.set(layout_mark);
    }
    use_effect(move || {
        let _ = layout_stamp();
        measure_across(across);
        let Some(root) = element("flow-root") else {
            return;
        };
        if watch.get() {
            return;
        }
        let callback = Closure::wrap(Box::new(move || {
            measure_across(across);
        }) as Box<dyn FnMut()>);
        let observer = JsResizeObserver::new(callback.as_ref().unchecked_ref());
        observer.observe(&root);
        callback.forget();
        watch.set(true);
    });

    let flow_class = if across_now { "flow is-across" } else { "flow" };
    rsx! {
        div { id: "flow-root", class: "{flow_class}",
            div { class: "panel-head",
                div { class: "panel-title",
                    Star { size: 12.0 }
                    h2 { id: "flow-heading", "Flow" }
                    span { class: "flow-live",
                        span { class: "flow-live-dot", aria_hidden: "true" }
                        "live"
                    }
                }
            }
            div { class: "flow-body",
                div { class: "flow-topology",
                    Topology {
                        jobs: jobs.clone(),
                        plans: plans.clone(),
                        lead: lead_view,
                        terminals: terminals.clone(),
                        changes: changes.clone(),
                        now,
                        elapsed_now,
                        starts: shown_starts,
                        flashes: shown_flashes,
                        on_show_terminal,
                        across: across_now,
                    }
                }
                EventLog { events: shown_events }
            }
        }
    }
}

#[component]
fn Topology(
    jobs: Vec<JobRecord>,
    plans: Vec<PlanStatus>,
    lead: Vec<LeadMessage>,
    terminals: Vec<TerminalInfo>,
    changes: Vec<ChangeSet>,
    now: i64,
    elapsed_now: i64,
    starts: HashMap<String, i64>,
    flashes: HashMap<String, i64>,
    on_show_terminal: Callback<String>,
    across: bool,
) -> Element {
    let mut menu = use_signal(|| None::<(ProviderId, f64, f64)>);
    let wired = use_hook(|| Rc::new(Cell::new(false)));
    let mut stamp = use_signal(String::new);
    let mark = topo_stamp(
        &jobs,
        &plans,
        &lead,
        &terminals,
        &changes,
        now,
        elapsed_now,
        across,
    );
    if stamp() != mark {
        stamp.set(mark);
    }
    use_effect(move || {
        let _ = stamp();
        if let Some(root) = element("flow-topo") {
            draw_wires(&root);
            if !wired.get() {
                let callback = Closure::wrap(Box::new(|| {
                    if let Some(root) = element("flow-topo") {
                        draw_wires(&root);
                    }
                }) as Box<dyn FnMut()>);
                let observer = JsResizeObserver::new(callback.as_ref().unchecked_ref());
                observer.observe(&root);
                callback.forget();
                wired.set(true);
            }
        }
    });

    let meters = meter_plans(&plans);
    let streaming = lead
        .iter()
        .any(|message| message.role == LeadRole::Lead && message.status == LeadStatus::Streaming);
    let state = lead_avatar(&lead);
    let detail = lead_detail(&lead);
    let topo_class = if across { "topo is-across" } else { "topo" };
    let lead_box = format!(
        "topo-lead {}",
        box_class(
            if state == BotState::Idle {
                None
            } else {
                Some("lead")
            },
            false
        )
    );
    let you_lead_class = if streaming {
        "flow-wire is-lead"
    } else {
        "flow-wire"
    };
    let lead_router_class = if streaming {
        "flow-wire is-router"
    } else {
        "flow-wire"
    };
    let menu_at = menu();
    let menu_entries =
        menu_at.map(|(plan, _, _)| terminal_entries(plan, &terminals, on_show_terminal));

    rsx! {
        div { id: "flow-topo", class: "{topo_class}",
            div { class: "topo-overlay",
                svg { class: "flow-wires", "aria-hidden": "true",
                    path { id: "flow-path-you-lead", "data-flow-path": "you-lead", class: "{you_lead_class}" }
                    path { id: "flow-path-lead-router", "data-flow-path": "lead-router", class: "{lead_router_class}" }
                    for plan in plans.iter().cloned() {
                        {
                            let id = plan.id.as_str();
                            let feeding = plan_feeding(&jobs, plan.id);
                            let class = if feeding { format!("flow-wire is-{id}") } else { "flow-wire".into() };
                            rsx! {
                                path {
                                    key: "{id}",
                                    id: "flow-path-worker-{id}",
                                    "data-flow-path": "worker-{id}",
                                    class: "{class}",
                                }
                            }
                        }
                    }
                }
                if streaming {
                    span { id: "flow-dot-you-lead", class: "flow-dot is-lead", "data-flow-dot": "you-lead", aria_hidden: "true" }
                    span { id: "flow-dot-lead-router", class: "flow-dot is-router", "data-flow-dot": "lead-router", aria_hidden: "true" }
                }
                for plan in plans.iter().cloned() {
                    if plan_feeding(&jobs, plan.id) {
                        {
                            let id = plan.id.as_str();
                            rsx! {
                                span {
                                    key: "{id}",
                                    id: "flow-dot-worker-{id}",
                                    class: "flow-dot is-{id}",
                                    "data-flow-dot": "worker-{id}",
                                    aria_hidden: "true",
                                }
                            }
                        }
                    }
                }
            }
            div { id: "flow-you", class: "flow-box topo-you", "data-flow": "you",
                {you_glyph()}
                span { class: "flow-name", "you" }
            }
            div { id: "flow-lead", class: "{lead_box}", "data-flow": "lead",
                div { class: "flow-line",
                    BotAvatar { bot: BotName::Lead, state: state, size: 20.0, title: None }
                    span { class: "flow-line-text",
                        span { class: "flow-name is-lead", "lead" }
                        if let Some(provider) = detail.0 {
                            span { class: "flow-keep", " · {provider}" }
                        }
                        if let Some(model) = detail.1.clone() {
                            span { class: "flow-model", " · {model}" }
                        }
                    }
                }
                div { class: "flow-sub",
                    span { class: "flow-sub-text", "{lead_line(state)}" }
                }
            }
            div { id: "flow-router", class: "flow-box topo-router", "data-flow": "router",
                div { class: "flow-router-label", "router" }
                for plan in meters {
                    {
                        let left = headroom_percent(plan.used);
                        let id = plan.id.as_str();
                        let fill = fill_class(plan.id, left);
                        rsx! {
                            div { key: "{id}", class: "flow-plan-row",
                                span { class: "flow-plan-name is-{id}", "{id}" }
                                div {
                                    class: "meter meter-thin",
                                    role: "meter",
                                    aria_valuemin: "0",
                                    aria_valuemax: "100",
                                    aria_valuenow: "{left}",
                                    aria_label: "{id} allowance left",
                                    div { class: "{fill}", style: "width: {left}%;" }
                                }
                                span { class: "flow-pct", "{left}%" }
                                span { class: "flow-plan-trail",
                                    if plan.running > 1 {
                                        span { class: "flow-queued", "{plan.running} running" }
                                    }
                                    if plan.queued > 0 {
                                        span { class: "flow-queued", "{plan.queued} queued" }
                                    }
                                    if plan.at_risk { {warn_mark()} }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "topo-workers",
                for plan in plans.iter().cloned() {
                    {worker_box(plan, &jobs, &terminals, &changes, now, elapsed_now, &starts, &flashes, menu)}
                }
            }
            if let Some((plan, x, y)) = menu_at {
                if let Some(entries) = menu_entries {
                    SidebarMenu {
                        label: format!("{} terminals", plan.as_str()),
                        at: (x, y),
                        entries,
                        on_close: Callback::new(move |_| menu.set(None)),
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn worker_box(
    plan: PlanStatus,
    jobs: &[JobRecord],
    terminals: &[TerminalInfo],
    changes: &[ChangeSet],
    now: i64,
    elapsed_now: i64,
    starts: &HashMap<String, i64>,
    flashes: &HashMap<String, i64>,
    mut menu: Signal<Option<(ProviderId, f64, f64)>>,
) -> Element {
    let avatar = worker_avatar(&plan, jobs, now);
    let running = running_job(jobs, plan.id);
    let parallel = plan.running.max(running_count(jobs, plan.id) as i64);
    let suffix = terminal_suffix(terminals, plan.id);
    let waiting = change_suffix(changes, jobs, plan.id);
    let extra = format!("{suffix}{waiting}");
    let tone = if avatar == BotState::Working || avatar == BotState::Error {
        Some(plan.id.as_str())
    } else {
        None
    };
    let flash = flashes.contains_key(plan.id.as_str());
    let class = box_class(tone, flash);
    let id = plan.id;
    let model = plan.model.clone().unwrap_or_else(|| "CLI default".into());
    let (text, elapsed) = worker_text(
        &plan,
        running.as_ref(),
        parallel,
        &extra,
        now,
        elapsed_now,
        starts,
    );
    let box_id = format!("flow-worker-{}", id.as_str());
    rsx! {
        div {
            key: "{id.as_str()}",
            id: "{box_id}",
            "data-flow": "worker-{id.as_str()}",
            class: "{class}",
            tabindex: "0",
            title: "Right-click to show its terminals",
            oncontextmenu: move |event| {
                event.prevent_default();
                let point = event.client_coordinates();
                menu.set(Some((id, point.x, point.y)));
            },
            onkeydown: move |event| {
                let key = event.key();
                let menu_key = key == Key::ContextMenu || (key == Key::F10 && event.modifiers().contains(Modifiers::SHIFT));
                if !menu_key {
                    return;
                }
                event.prevent_default();
                if let Some(rect) = element(&box_id).map(|el| el.get_bounding_client_rect()) {
                    menu.set(Some((id, rect.left(), rect.bottom())));
                }
            },
            div { class: "flow-line",
                BotAvatar { bot: BotName::Plan(id), state: avatar, size: 20.0, title: None }
                span { class: "flow-line-text",
                    span { class: "flow-name is-{id.as_str()}", "{id.as_str()}" }
                    span { class: "flow-model", " · {model}" }
                }
            }
            div { class: "flow-sub",
                span { class: "flow-sub-text", "{text}" }
                if let Some(elapsed) = elapsed.clone() {
                    span { class: "flow-elapsed", "{elapsed}" }
                }
                if elapsed.is_some() && !extra.is_empty() {
                    span { class: "flow-sub-text", "{extra}" }
                }
            }
        }
    }
}

#[component]
fn EventLog(events: Vec<FlowEvent>) -> Element {
    let stick = use_hook(|| Rc::new(Cell::new(true)));
    let stick_effect = stick.clone();
    let mut stamp = use_signal(String::new);
    let mark = format!(
        "{}:{}",
        events.len(),
        events.last().map(|event| event.id).unwrap_or(0)
    );
    if stamp() != mark {
        stamp.set(mark);
    }
    use_effect(move || {
        let _ = stamp();
        if !stick_effect.get() {
            return;
        }
        if let Some(body) = element("flow-log-body") {
            body.set_scroll_top(body.scroll_height());
        }
    });
    rsx! {
        div { class: "flow-log",
            div { class: "flow-log-head", "Activity" }
            div {
                id: "flow-log-body",
                class: "flow-log-body",
                role: "log",
                aria_live: "off",
                onscroll: {
                    let stick = stick.clone();
                    move |_| {
                        let Some(body) = element("flow-log-body") else { return };
                        stick.set(body.scroll_height() - body.scroll_top() - body.client_height() <= 20);
                    }
                },
                if events.is_empty() {
                    p { class: "flow-empty", "No activity yet. Events appear here as work happens." }
                } else {
                    for event in events {
                        {
                            let id = event.id;
                            let actor = event.actor.as_str();
                            let result_class = match event.tone {
                                None => "flow-result".to_string(),
                                Some(tone) => format!("flow-result is-{}", tone.as_str()),
                            };
                            let result = event.result.clone().unwrap_or_default();
                            rsx! {
                                div { key: "{id}", class: "flow-event",
                                    span { class: "flow-time", "{format_log_time(event.at)}" }
                                    span { class: "flow-actor flow-actor-{actor}", "{actor}" }
                                    span { class: "flow-text", "{event.text}" }
                                    span { class: "{result_class}", "{result}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn terminal_entries(
    plan: ProviderId,
    terminals: &[TerminalInfo],
    on_show: Callback<String>,
) -> Vec<MenuEntry> {
    let mine: Vec<&TerminalInfo> = terminals
        .iter()
        .filter(|terminal| same_plan(terminal.provider, plan))
        .collect();
    if mine.is_empty() {
        return vec![MenuEntry {
            label: format!("No {} terminals open", plan.as_str()),
            on_select: None,
            disabled: true,
            danger: false,
            checked: false,
            shows_check: false,
            heading: false,
            separator: false,
            title: String::new(),
        }];
    }
    let mut entries = vec![MenuEntry::heading(format!("{} terminals", plan.as_str()))];
    for terminal in mine {
        let id = terminal.id.clone();
        let title = if terminal.status == TerminalStatus::Running {
            "Show this terminal".to_string()
        } else {
            format!("Show this terminal ({})", terminal_status(terminal.status))
        };
        entries.push(MenuEntry {
            label: terminal.title.clone(),
            on_select: Some(Callback::new(move |_| on_show.call(id.clone()))),
            disabled: false,
            danger: false,
            checked: false,
            shows_check: false,
            heading: false,
            separator: false,
            title,
        });
    }
    entries
}

fn track_runs(
    starts: &HashMap<String, i64>,
    flashes: &HashMap<String, i64>,
    prev_jobs: &[JobRecord],
    next_jobs: &[JobRecord],
    now: i64,
) -> (HashMap<String, i64>, HashMap<String, i64>, bool) {
    let mut next_starts = HashMap::new();
    let mut changed = false;
    for job in next_jobs {
        if job.status != JobStatus::Running {
            continue;
        }
        match starts.get(&job.id) {
            None => {
                next_starts.insert(job.id.clone(), now);
                changed = true;
            }
            Some(existing) => {
                next_starts.insert(job.id.clone(), *existing);
            }
        }
    }
    for id in starts.keys() {
        if !next_starts.contains_key(id) {
            changed = true;
        }
    }
    let mut next_flashes = flashes.clone();
    for job in next_jobs {
        let Some(prev) = prev_jobs.iter().find(|item| item.id == job.id) else {
            continue;
        };
        if job.failed_over.len() <= prev.failed_over.len() {
            continue;
        }
        let until = now + 2000;
        for provider in job.failed_over.iter().skip(prev.failed_over.len()) {
            next_flashes.insert(provider.as_str().to_string(), until);
            changed = true;
        }
    }
    if !changed {
        return (starts.clone(), flashes.clone(), false);
    }
    (next_starts, next_flashes, true)
}

fn meter_plans(plans: &[PlanStatus]) -> Vec<PlanStatus> {
    let mut running = HashMap::<ProviderId, i64>::new();
    let mut queued = HashMap::<ProviderId, i64>::new();
    let mut busy = Vec::new();
    for plan in plans {
        let spent = plan_of(plan.id);
        if spent == plan.id {
            continue;
        }
        *running.entry(spent).or_insert(0) += plan.running;
        *queued.entry(spent).or_insert(0) += plan.queued;
        if plan.busy {
            busy.push(spent);
        }
    }
    plans
        .iter()
        .filter(|plan| plan_of(plan.id) == plan.id)
        .map(|plan| {
            let more_running = running.get(&plan.id).copied().unwrap_or(0);
            let more_queued = queued.get(&plan.id).copied().unwrap_or(0);
            if more_running == 0 && more_queued == 0 && !busy.contains(&plan.id) {
                return plan.clone();
            }
            let mut next = plan.clone();
            next.running += more_running;
            next.queued += more_queued;
            next.busy = plan.busy || busy.contains(&plan.id);
            next
        })
        .collect()
}

fn headroom_percent(used: f64) -> i64 {
    let raw = ((1.0 - used) * 100.0).round();
    if !raw.is_finite() {
        return 0;
    }
    raw.clamp(0.0, 100.0) as i64
}

fn fill_class(id: ProviderId, left: i64) -> String {
    if left < 15 {
        "meter-fill meter-bad".into()
    } else if left < 30 {
        "meter-fill meter-warn".into()
    } else {
        format!("meter-fill meter-{}", id.as_str())
    }
}

fn format_elapsed(ms: i64) -> String {
    let total = (ms / 1000).max(0);
    let minutes = total / 60;
    let seconds = total % 60;
    if minutes == 0 {
        format!("{seconds}s")
    } else {
        format!("{minutes}m {seconds}s")
    }
}

fn lead_avatar(messages: &[LeadMessage]) -> BotState {
    if messages
        .iter()
        .any(|message| message.role == LeadRole::Lead && message.status == LeadStatus::Streaming)
    {
        return BotState::Working;
    }
    for message in messages.iter().rev() {
        if message.role != LeadRole::Lead {
            continue;
        }
        return if message.status == LeadStatus::Error {
            BotState::Error
        } else {
            BotState::Idle
        };
    }
    BotState::Idle
}

fn lead_detail(messages: &[LeadMessage]) -> (Option<String>, Option<String>) {
    for message in messages.iter().rev() {
        if message.role != LeadRole::Lead {
            continue;
        }
        if message.provider.is_some() || message.model.is_some() {
            return (
                message.provider.map(|id| id.as_str().to_string()),
                message.model.clone(),
            );
        }
    }
    (None, None)
}

fn lead_line(state: BotState) -> &'static str {
    match state {
        BotState::Working => "working…",
        BotState::Error => "error",
        _ => "idle",
    }
}

fn running_job(jobs: &[JobRecord], id: ProviderId) -> Option<JobRecord> {
    jobs.iter()
        .rev()
        .find(|job| job.provider == Some(id) && job.status == JobStatus::Running)
        .cloned()
}

fn running_count(jobs: &[JobRecord], id: ProviderId) -> usize {
    jobs.iter()
        .filter(|job| job.provider == Some(id) && job.status == JobStatus::Running)
        .count()
}

fn worker_avatar(plan: &PlanStatus, jobs: &[JobRecord], now: i64) -> BotState {
    let resting = !plan.available || plan.resting_until.is_some_and(|until| until > now);
    if resting {
        return BotState::Resting;
    }
    if plan.busy || running_job(jobs, plan.id).is_some() {
        return BotState::Working;
    }
    if jobs
        .iter()
        .rev()
        .find(|job| job.provider == Some(plan.id))
        .is_some_and(|job| job.status == JobStatus::Failed)
    {
        return BotState::Error;
    }
    BotState::Idle
}

fn terminal_suffix(terminals: &[TerminalInfo], id: ProviderId) -> String {
    let count = terminals
        .iter()
        .filter(|terminal| same_plan(terminal.provider, id))
        .count();
    if count == 0 {
        String::new()
    } else if count == 1 {
        " · 1 terminal".into()
    } else {
        format!(" · {count} terminals")
    }
}

fn change_suffix(changes: &[ChangeSet], jobs: &[JobRecord], id: ProviderId) -> String {
    let ids: Vec<&str> = jobs
        .iter()
        .filter(|job| job.provider == Some(id))
        .filter_map(|job| job.change.as_deref())
        .collect();
    let count = changes
        .iter()
        .filter(|item| ids.iter().any(|id| *id == item.id))
        .count();
    if count == 0 {
        String::new()
    } else if count == 1 {
        " · 1 change".into()
    } else {
        format!(" · {count} changes")
    }
}

fn plan_feeding(jobs: &[JobRecord], id: ProviderId) -> bool {
    jobs.iter().any(|job| {
        job.provider == Some(id)
            && (job.status == JobStatus::Running || job.status == JobStatus::Queued)
    })
}

fn worker_text(
    plan: &PlanStatus,
    running: Option<&JobRecord>,
    parallel: i64,
    extra: &str,
    now: i64,
    elapsed_now: i64,
    starts: &HashMap<String, i64>,
) -> (String, Option<String>) {
    if plan.problem == Some(PlanProblem::NotInstalled) {
        return (format!("not installed{extra}"), None);
    }
    if plan.problem == Some(PlanProblem::SignedOut) || !plan.available {
        return (format!("signed out{extra}"), None);
    }
    if let Some(until) = plan.resting_until.filter(|until| *until > now) {
        return (
            format!("resting until {}{extra}", format_hours_minutes(until)),
            None,
        );
    }
    if let Some(running) = running {
        let started = starts.get(&running.id).copied().unwrap_or(elapsed_now);
        let more = parallel - 1;
        let text = if more > 0 {
            format!("{} · +{more} more · ", running.job_type.as_str())
        } else {
            format!("{} · running · ", running.job_type.as_str())
        };
        return (text, Some(format_elapsed(elapsed_now - started)));
    }
    (format!("idle{extra}"), None)
}

fn box_class(tone: Option<&str>, flash: bool) -> String {
    let mut class = match tone {
        Some(tone) => format!("flow-box is-{tone}"),
        None => "flow-box".to_string(),
    };
    if flash {
        class.push_str(" is-flash");
    }
    class
}

fn same_plan(provider: TerminalProvider, id: ProviderId) -> bool {
    matches!(
        (provider, id),
        (TerminalProvider::Claude, ProviderId::Claude)
            | (TerminalProvider::Cursor, ProviderId::Cursor)
            | (TerminalProvider::Grok, ProviderId::Grok)
            | (TerminalProvider::Local, ProviderId::Local)
    )
}

fn terminal_status(status: TerminalStatus) -> &'static str {
    match status {
        TerminalStatus::Running => "running",
        TerminalStatus::Exited => "exited",
        TerminalStatus::Sleeping => "sleeping",
    }
}

fn upsert_message(list: &mut Vec<LeadMessage>, message: LeadMessage) {
    if let Some(slot) = list.iter_mut().find(|item| item.id == message.id) {
        *slot = message;
    } else {
        list.push(message);
    }
}

#[allow(clippy::too_many_arguments)]
fn topo_stamp(
    jobs: &[JobRecord],
    plans: &[PlanStatus],
    lead: &[LeadMessage],
    terminals: &[TerminalInfo],
    changes: &[ChangeSet],
    now: i64,
    elapsed: i64,
    across: bool,
) -> String {
    format!(
        "{}:{}:{}:{}:{}:{now}:{elapsed}:{across}",
        jobs.len(),
        plans.len(),
        lead.len(),
        terminals.len(),
        changes.len()
    )
}

fn you_glyph() -> Element {
    rsx! {
        svg {
            class: "flow-you-icon",
            width: "14",
            height: "14",
            view_box: "0 0 16 16",
            "aria-hidden": "true",
            "focusable": "false",
            path {
                fill: "currentColor",
                d: "M8 8a3 3 0 1 0 0-6 3 3 0 0 0 0 6Zm0 1.5c-2.67 0-8 1.34-8 4V15h16v-1.5c0-2.66-5.33-4-8-4Z",
            }
        }
    }
}

fn warn_mark() -> Element {
    rsx! {
        span { class: "flow-risk", title: "Unused allowance expires soon",
            svg {
                width: "12",
                height: "12",
                view_box: "0 0 16 16",
                "aria-hidden": "true",
                "focusable": "false",
                path {
                    fill: "currentColor",
                    fill_rule: "evenodd",
                    d: "M8 1.5 15 14H1L8 1.5ZM7.25 6h1.5v4.2h-1.5V6Zm0 5.4h1.5V13h-1.5v-1.6Z",
                }
            }
        }
    }
}

struct Point {
    x: f64,
    y: f64,
}

fn draw_wires(root: &web_sys::Element) {
    let base = root.get_bounding_client_rect();
    if base.width() == 0.0 || base.height() == 0.0 {
        return;
    }
    let across = root
        .get_attribute("class")
        .is_some_and(|class| class.contains("is-across"));
    let out = if across { "right" } else { "bottom" };
    let into = if across { "left" } else { "top" };
    let you = find_box(root, "you");
    let lead = find_box(root, "lead");
    let router = find_box(root, "router");
    if let (Some(you), Some(lead)) = (you.as_ref(), lead.as_ref()) {
        paint(
            root,
            "you-lead",
            &straight(&edge_point(&base, you, out), &edge_point(&base, lead, into)),
        );
    }
    if let (Some(lead), Some(router)) = (lead.as_ref(), router.as_ref()) {
        paint(
            root,
            "lead-router",
            &straight(
                &edge_point(&base, lead, out),
                &edge_point(&base, router, into),
            ),
        );
    }
    let Some(router) = router else { return };
    let from = edge_point(&base, &router, out);
    let mut targets = Vec::new();
    if let Ok(nodes) = root.query_selector_all("[data-flow]") {
        for index in 0..nodes.length() {
            let Some(node) = nodes.item(index) else {
                continue;
            };
            let Ok(element) = node.dyn_into::<web_sys::Element>() else {
                continue;
            };
            let Some(name) = element.get_attribute("data-flow") else {
                continue;
            };
            let Some(id) = name.strip_prefix("worker-") else {
                continue;
            };
            targets.push((id.to_string(), edge_point(&base, &element, into)));
        }
    }
    if across {
        let bus_x = if targets.is_empty() {
            from.x
        } else {
            let min_x = targets
                .iter()
                .map(|(_, point)| point.x)
                .fold(f64::INFINITY, f64::min);
            (from.x + min_x) / 2.0
        };
        for (id, point) in &targets {
            let aligned = targets.len() == 1 && (from.y - point.y).abs() <= 1.0;
            let path = if aligned {
                straight(&from, point)
            } else {
                tee_across(&from, point, bus_x)
            };
            paint(root, &format!("worker-{id}"), &path);
        }
        return;
    }
    let bus_y = if targets.is_empty() {
        from.y
    } else {
        let sum: f64 = targets.iter().map(|(_, point)| point.y).sum();
        (from.y + sum / targets.len() as f64) / 2.0
    };
    for (id, point) in &targets {
        let aligned = targets.len() == 1 && (from.x - point.x).abs() <= 1.0;
        let path = if aligned {
            straight(&from, point)
        } else {
            tee(&from, point, bus_y)
        };
        paint(root, &format!("worker-{id}"), &path);
    }
}

fn find_box(root: &web_sys::Element, name: &str) -> Option<web_sys::Element> {
    root.query_selector(&format!("[data-flow=\"{name}\"]"))
        .ok()
        .flatten()
}

fn edge_point(root: &web_sys::DomRect, element: &web_sys::Element, edge: &str) -> Point {
    let rect = element.get_bounding_client_rect();
    if edge == "left" || edge == "right" {
        Point {
            x: (if edge == "left" {
                rect.left()
            } else {
                rect.right()
            }) - root.left(),
            y: rect.top() + rect.height() / 2.0 - root.top(),
        }
    } else {
        Point {
            x: rect.left() + rect.width() / 2.0 - root.left(),
            y: (if edge == "top" {
                rect.top()
            } else {
                rect.bottom()
            }) - root.top(),
        }
    }
}

fn round_tenths(value: f64) -> String {
    format!("{value:.1}")
}

fn straight(from: &Point, to: &Point) -> String {
    format!(
        "M {} {} L {} {}",
        round_tenths(from.x),
        round_tenths(from.y),
        round_tenths(to.x),
        round_tenths(to.y)
    )
}

fn tee(from: &Point, to: &Point, bus_y: f64) -> String {
    format!(
        "M {} {} V {} H {} V {}",
        round_tenths(from.x),
        round_tenths(from.y),
        round_tenths(bus_y),
        round_tenths(to.x),
        round_tenths(to.y)
    )
}

fn tee_across(from: &Point, to: &Point, bus_x: f64) -> String {
    format!(
        "M {} {} H {} V {} H {}",
        round_tenths(from.x),
        round_tenths(from.y),
        round_tenths(bus_x),
        round_tenths(to.y),
        round_tenths(to.x)
    )
}

fn paint(root: &web_sys::Element, name: &str, d: &str) {
    if let Some(path) = find_path(root, name) {
        let _ = path.set_attribute("d", d);
    }
    if let Some(dot) = find_dot(root, name)
        && let Ok(html) = dot.dyn_into::<web_sys::HtmlElement>()
    {
        let _ = html
            .style()
            .set_property("offset-path", &format!("path('{d}')"));
    }
}

fn find_path(root: &web_sys::Element, name: &str) -> Option<web_sys::Element> {
    root.query_selector(&format!("[data-flow-path=\"{name}\"]"))
        .ok()
        .flatten()
}

fn find_dot(root: &web_sys::Element, name: &str) -> Option<web_sys::Element> {
    root.query_selector(&format!("[data-flow-dot=\"{name}\"]"))
        .ok()
        .flatten()
}

fn element(id: &str) -> Option<web_sys::Element> {
    web_sys::window()?.document()?.get_element_by_id(id)
}

fn measure_across(mut across: Signal<bool>) {
    let Some(root) = element("flow-root") else {
        return;
    };
    let rect = root.get_bounding_client_rect();
    if rect.width() == 0.0 {
        return;
    }
    let next = rect.width() >= 720.0 && rect.width() >= rect.height() * 1.6;
    if across() != next {
        across.set(next);
    }
}
