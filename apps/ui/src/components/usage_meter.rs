use dioxus::prelude::*;
use legate_core::{PlanProblem, PlanStatus, ProviderId, UsageShows, VersionStatus, plan_of};
use wasm_bindgen_futures::spawn_local;

use crate::api;
use crate::components::bot_avatar::{BotAvatar, BotName, BotState};
use crate::components::model_picker::ModelPicker;
use crate::logic::usage_format::{reset_choices, usage_figure};

#[derive(Clone, Copy, PartialEq, Eq)]
enum PlanWord {
    NotInstalled,
    SignedOut,
    Resting,
    Busy,
    Ready,
}

#[component]
pub fn UsageMeter(
    plans: Option<Vec<PlanStatus>>,
    now: i64,
    on_open_setup: Callback,
    #[props(default = UsageShows::Left)] shows: UsageShows,
) -> Element {
    let mut checking = use_signal(|| false);
    let mut refresh_error = use_signal(|| None::<String>);
    use_effect(move || {
        spawn_local(async move {
            match api::refresh_usage().await {
                Ok(result) if !result.ok => refresh_error.set(result.error),
                Err(text) => refresh_error.set(Some(text)),
                _ => {}
            }
        });
    });
    let notice = refresh_error.read().clone();
    rsx! {
        div { class: "usage",
            div { class: "panel-head",
                div { class: "panel-title",
                    crate::components::star::Star { size: 12.0 }
                    h2 { id: "usage-heading", "Usage" }
                }
                button {
                    class: "btn btn-quiet btn-compact",
                    r#type: "button",
                    onclick: move |_| on_open_setup.call(()),
                    "Setup"
                }
            }
            if let Some(notice) = notice {
                p { class: "hint", "{notice}" }
            }
            if let Some(plans) = plans {
                if plans.is_empty() {
                    p { class: "hint", "No plans found. Sign in to the claude or agent CLI." }
                } else {
                    div { class: "usage-list",
                        for plan in meter_plans(&plans) {
                            {
                                let word = plan_word(&plan, now);
                                let figure = usage_figure(plan.used, shows);
                                let left = headroom(plan.used);
                                let name = plan.id.as_str().to_string();
                                let plan_id = plan.id;
                                rsx! {
                                    div { class: "usage-card-wrap", key: "{name}",
                                        div { class: "usage-card",
                                            div { class: "usage-head",
                                                BotAvatar { bot: BotName::Plan(plan.id), state: avatar(word), size: 32.0, title: None }
                                                span { class: "usage-name", "{name}" }
                                                span { class: "{status_class(word, plan.running)}",
                                                    span { class: "status-dot", aria_hidden: "true" }
                                                    "{status_label(&plan, word)}"
                                                }
                                            }
                                            if let Some(text) = problem_notice(&plan) {
                                                div { class: "usage-problem",
                                                    p { class: "hint", "{text}" }
                                                    button {
                                                        class: "btn btn-compact",
                                                        disabled: checking(),
                                                        onclick: move |_| {
                                                            if checking() { return; }
                                                            checking.set(true);
                                                            spawn_local(async move {
                                                                let _ = api::recheck_plans().await;
                                                                checking.set(false);
                                                            });
                                                        },
                                                        "Check again"
                                                    }
                                                }
                                            } else {
                                                div { class: "usage-readout",
                                                    div { class: "usage-figure",
                                                        span { class: "usage-percent", "{figure}%" }
                                                        span { class: "usage-left-word", "{shows_word(shows)}" }
                                                    }
                                                    div { class: "meter", role: "meter", aria_valuemin: "0", aria_valuemax: "100", aria_valuenow: "{figure}",
                                                        div { class: "{fill_class(plan.id, left)}", style: "width: {figure}%;" }
                                                    }
                                                    for (index, slot) in plan.windows.iter().cloned().enumerate() {
                                                        {
                                                            let slot_figure = usage_figure(slot.used, shows);
                                                            let phrase = slot.resets_at.and_then(|at| reset_phrase(at, now));
                                                            rsx! {
                                                                div { class: "usage-window", key: "{index}",
                                                                    div { class: "usage-window-top",
                                                                        span { class: "usage-window-label", "{slot.name}" }
                                                                        span { "{slot_figure}%" }
                                                                    }
                                                                    if let Some(phrase) = phrase {
                                                                        p { class: "hint", "{phrase}" }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                    p { class: "hint", "{version_line(&plan)}" }
                                                    ModelPicker { plan: plan.clone(), disabled: plan_unusable(word), label_title: None }
                                                    if plan.id == ProviderId::Cursor && plan.windows.is_empty() && plan.usage_at.is_none() {
                                                        CursorReading { plan_id: plan_id, now: now }
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
            } else {
                p { class: "hint", "Checking plans…" }
            }
        }
    }
}

#[component]
fn CursorReading(plan_id: ProviderId, now: i64) -> Element {
    let choices = reset_choices(now, 32);
    let mut used = use_signal(|| 0.0_f64);
    let mut reset = use_signal(|| {
        choices
            .first()
            .map(|item| item.value.clone())
            .unwrap_or_default()
    });
    let _ = plan_id;
    rsx! {
        div { class: "cursor-reading",
            p { class: "hint", "Cursor does not report a number. Enter what the dashboard shows." }
            label {
                "Used %"
                input {
                    r#type: "number",
                    min: "0",
                    max: "100",
                    value: "{used()}",
                    oninput: move |event| {
                        used.set(event.value().parse().unwrap_or(0.0));
                    },
                }
            }
            select {
                onchange: move |event| reset.set(event.value()),
                for choice in choices {
                    option { value: "{choice.value}", "{choice.label}" }
                }
            }
            button {
                class: "btn btn-quiet btn-compact",
                onclick: move |_| {
                    let percent = used();
                    let when = crate::logic::usage_format::date_from_value(&reset()).unwrap_or(now);
                    spawn_local(async move {
                        let _ = api::set_cursor_reading(percent, when as f64).await;
                    });
                },
                "Save reading"
            }
        }
    }
}

fn meter_plans(plans: &[PlanStatus]) -> Vec<PlanStatus> {
    let mut folded = plans.to_vec();
    for plan in plans {
        let spent = plan_of(plan.id);
        if spent == plan.id {
            continue;
        }
        if let Some(slot) = folded.iter_mut().find(|item| item.id == spent) {
            slot.running += plan.running;
            slot.queued += plan.queued;
            slot.busy = slot.busy || plan.busy;
        }
    }
    folded.retain(|plan| plan_of(plan.id) == plan.id);
    folded
}

fn plan_word(plan: &PlanStatus, now: i64) -> PlanWord {
    if plan.problem == Some(PlanProblem::NotInstalled) {
        return PlanWord::NotInstalled;
    }
    if plan.problem == Some(PlanProblem::SignedOut) || !plan.available {
        return PlanWord::SignedOut;
    }
    if plan.resting_until.is_some_and(|until| until > now) {
        return PlanWord::Resting;
    }
    if plan.busy || plan.running > 0 {
        return PlanWord::Busy;
    }
    PlanWord::Ready
}

fn status_label(plan: &PlanStatus, word: PlanWord) -> String {
    if word == PlanWord::Busy && plan.running > 1 {
        format!("{} running", plan.running)
    } else {
        match word {
            PlanWord::NotInstalled => "not installed".into(),
            PlanWord::SignedOut => "signed out".into(),
            PlanWord::Resting => "resting".into(),
            PlanWord::Busy => "busy".into(),
            PlanWord::Ready => "ready".into(),
        }
    }
}

fn problem_notice(plan: &PlanStatus) -> Option<&'static str> {
    match plan.problem {
        Some(PlanProblem::NotInstalled) if plan.id == ProviderId::Claude => Some(
            "Claude Code is not installed. Install it, then run `claude` once in a terminal to sign in.",
        ),
        Some(PlanProblem::NotInstalled) => Some(
            "The Cursor CLI is not installed. Install it, then run `agent login` in a terminal.",
        ),
        Some(PlanProblem::SignedOut) if plan.id == ProviderId::Claude => {
            Some("Claude Code is signed out. Run `claude` in a terminal and sign in.")
        }
        Some(PlanProblem::SignedOut) => {
            Some("The Cursor CLI is signed out. Run `agent login` in a terminal.")
        }
        None => None,
    }
}

fn version_line(plan: &PlanStatus) -> String {
    match (&plan.version, plan.version_status) {
        (None, _) | (_, VersionStatus::Unknown) => "CLI version unknown".into(),
        (Some(version), VersionStatus::Tested) => format!("CLI {version}"),
        (Some(version), VersionStatus::Newer) => format!(
            "CLI {version} · newer than tested ({})",
            plan.tested_version
        ),
        (Some(version), VersionStatus::Older) => {
            format!(
                "CLI {version} · older than tested ({}) — update it",
                plan.tested_version
            )
        }
    }
}

fn avatar(word: PlanWord) -> BotState {
    match word {
        PlanWord::NotInstalled | PlanWord::SignedOut | PlanWord::Resting => BotState::Resting,
        PlanWord::Busy => BotState::Working,
        PlanWord::Ready => BotState::Idle,
    }
}

fn status_class(word: PlanWord, running: i64) -> &'static str {
    match word {
        PlanWord::NotInstalled | PlanWord::SignedOut => "status status-out",
        PlanWord::Busy if running > 1 => "status status-running",
        PlanWord::Busy => "status status-busy",
        PlanWord::Resting => "status status-resting",
        PlanWord::Ready => "status status-ready",
    }
}

fn plan_unusable(word: PlanWord) -> bool {
    matches!(word, PlanWord::NotInstalled | PlanWord::SignedOut)
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

fn shows_word(shows: UsageShows) -> &'static str {
    match shows {
        UsageShows::Left => "left",
        UsageShows::Used => "used",
    }
}

fn headroom(used: f64) -> i64 {
    usage_figure(used, UsageShows::Left)
}

fn reset_phrase(resets_at: i64, now: i64) -> Option<String> {
    let remaining = resets_at - now;
    if remaining <= 0 {
        return None;
    }
    if remaining < 60_000 {
        return Some("resets in under 1m".into());
    }
    let days = remaining / 86_400_000;
    let hours = (remaining / 3_600_000) % 24;
    let minutes = (remaining / 60_000) % 60;
    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{days}d"));
    }
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    if minutes > 0 {
        parts.push(format!("{minutes}m"));
    }
    Some(format!("resets in {}", parts.join(" ")))
}
