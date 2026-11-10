//! New task dialog. The element tree matches `NewTaskDialog.tsx`.

use dioxus::prelude::*;
use legate_core::ProviderId;
use wasm_bindgen_futures::spawn_local;

use crate::api;
use crate::components::bot_avatar::{BotAvatar, BotName, BotState};
use crate::components::star::Star;
use crate::logic::new_task::{
    self, HOW_OPTIONS, How, JOB_KINDS, NewTaskContext, NewTaskDraft, NewTaskPlan, WHO_OPTIONS,
};
use crate::wire::GoalInput;

#[component]
pub fn NewTaskDialog(
    open: bool,
    on_close: Callback,
    is_repo: bool,
    claude_available: bool,
    codex_available: bool,
    cursor_available: bool,
    on_start_terminal: Callback<(String, ProviderId, i64)>,
    on_created: Callback<String>,
) -> Element {
    let mut text = use_signal(String::new);
    let mut who = use_signal(|| new_task::DEFAULT_WHO);
    let mut how = use_signal(|| new_task::DEFAULT_HOW);
    let mut count = use_signal(|| new_task::DEFAULT_COUNT as f64);
    let mut kind = use_signal(|| new_task::DEFAULT_KIND);
    let mut check = use_signal(String::new);
    let mut rounds = use_signal(|| new_task::DEFAULT_ROUNDS as f64);
    let mut error = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    let mut was_open = use_signal(|| false);
    if !open {
        was_open.set(false);
        return rsx! {};
    }
    let ctx = NewTaskContext {
        is_repo,
        claude_available,
        codex_available,
        cursor_available,
    };
    if !was_open() {
        was_open.set(true);
        if !new_task::provider_ready(who(), ctx) {
            if let Some(ready) = WHO_OPTIONS
                .iter()
                .find(|choice| new_task::provider_ready(choice.id, ctx))
            {
                who.set(ready.id);
            }
        }
    }
    let chosen = new_task::active_how(how(), is_repo);
    let outside = new_task::repo_hint(is_repo);
    let notice = error.read().clone();

    rsx! {
        div {
            class: "new-task-overlay",
            role: "dialog",
            aria_modal: "true",
            aria_labelledby: "new-task-title",
            onmousedown: move |_| on_close.call(()),
            onkeydown: move |event| {
                if event.key().to_string() == "Escape" {
                    event.prevent_default();
                    on_close.call(());
                }
            },
            form {
                class: "new-task",
                onmousedown: move |event| event.stop_propagation(),
                onsubmit: move |event| {
                    event.prevent_default();
                    if busy() {
                        return;
                    }
                    let draft = NewTaskDraft {
                        text: text(),
                        who: who(),
                        how: how(),
                        count: count(),
                        kind: kind(),
                        check: check(),
                        max_rounds: rounds(),
                    };
                    match new_task::plan_new_task(&draft, ctx) {
                        NewTaskPlan::Invalid { error: text } => error.set(Some(text)),
                        NewTaskPlan::Terminal { text: prompt, who, count } => {
                            busy.set(true);
                            on_start_terminal.call((prompt, who, count));
                            busy.set(false);
                            on_close.call(());
                        }
                        NewTaskPlan::Job { job_type, text: prompt, who, access } => {
                            busy.set(true);
                            spawn_local(async move {
                                match api::submit_job(job_type, &prompt, Some(who), Some(access), None).await {
                                    Ok(_) => {
                                        busy.set(false);
                                        on_created.call("job".into());
                                        on_close.call(());
                                    }
                                    Err(text) => {
                                        error.set(Some(text));
                                        busy.set(false);
                                    }
                                }
                            });
                        }
                        NewTaskPlan::Goal { input, created } => {
                            busy.set(true);
                            let goal = GoalInput {
                                text: input.text,
                                check: if input.check.is_empty() { None } else { Some(input.check) },
                                max_rounds: input.max_rounds.map(|value| value as f64),
                                quick_type: input.quick_type,
                                quick_access: input.quick_access,
                            };
                            spawn_local(async move {
                                match api::create_goal(&goal).await {
                                    Ok(_) => {
                                        busy.set(false);
                                        on_created.call(created.into());
                                        on_close.call(());
                                    }
                                    Err(text) => {
                                        error.set(Some(text));
                                        busy.set(false);
                                    }
                                }
                            });
                        }
                    }
                },
                div { class: "panel-head",
                    div { class: "panel-title",
                        Star { size: 12.0 }
                        h2 { id: "new-task-title", "New task" }
                    }
                }
                label { class: "new-task-field", r#for: "new-task-text",
                    "What should be done?"
                    textarea {
                        id: "new-task-text",
                        rows: "4",
                        value: "{text}",
                        placeholder: "Fix the off-by-one in the pager and add a test",
                        oninput: move |event| {
                            text.set(event.value());
                            error.set(None);
                        },
                    }
                }
                div { class: "new-task-section",
                    p { id: "new-task-who", class: "new-task-kicker", "Who" }
                    div { class: "new-task-who", role: "radiogroup", aria_labelledby: "new-task-who",
                        for option in WHO_OPTIONS {
                            {
                                let ready = new_task::provider_ready(option.id, ctx);
                                let hint = new_task::unavailable_hint(ready);
                                let selected = who() == option.id;
                                rsx! {
                                    button {
                                        key: "{option.id.as_str()}",
                                        r#type: "button",
                                        role: "radio",
                                        aria_checked: "{selected}",
                                        class: if selected { "new-task-choice is-selected" } else { "new-task-choice" },
                                        disabled: !ready,
                                        title: hint.unwrap_or(""),
                                        onclick: move |_| {
                                            who.set(option.id);
                                            error.set(None);
                                        },
                                        BotAvatar {
                                            bot: BotName::Plan(option.id),
                                            state: if ready { BotState::Idle } else { BotState::Error },
                                            size: 18.0,
                                            title: None,
                                        }
                                        span { class: "new-task-choice-body",
                                            span { class: "new-task-choice-name", "{option.label}" }
                                            if let Some(note) = option.note {
                                                span { class: "new-task-choice-detail", "{note}" }
                                            }
                                            if let Some(hint) = hint {
                                                span { class: "new-task-choice-hint", "{hint}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "new-task-section",
                    p { id: "new-task-how", class: "new-task-kicker", "How" }
                    div { class: "new-task-how", role: "radiogroup", aria_labelledby: "new-task-how",
                        for option in HOW_OPTIONS {
                            {
                                let enabled = new_task::how_enabled(option.id, is_repo);
                                let selected = chosen == option.id;
                                let hint = if enabled { String::new() } else { outside.unwrap_or("").to_string() };
                                rsx! {
                                    button {
                                        key: "{option.id.as_str()}",
                                        r#type: "button",
                                        role: "radio",
                                        aria_checked: "{selected}",
                                        class: if selected { "new-task-choice is-selected" } else { "new-task-choice" },
                                        disabled: !enabled,
                                        title: "{hint}",
                                        onclick: move |_| {
                                            how.set(option.id);
                                            error.set(None);
                                        },
                                        span { class: "new-task-choice-body",
                                            span { class: "new-task-choice-name", "{option.label}" }
                                            span { class: "new-task-choice-detail", "{option.detail}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if let Some(hint) = outside {
                        p { class: "hint", "{hint}" }
                    }
                    if chosen == How::Terminal {
                        label { class: "new-task-count",
                            "How many"
                            input {
                                r#type: "number",
                                min: "{new_task::COUNT_MIN}",
                                max: "{new_task::COUNT_MAX}",
                                value: "{count}",
                                oninput: move |event| {
                                    count.set(new_task::clamp_count(event.value().parse().unwrap_or(f64::NAN)) as f64);
                                },
                            }
                        }
                    }
                    if chosen == How::Background {
                        label { class: "new-task-field",
                            "Kind"
                            select {
                                class: "select",
                                onchange: move |event| kind.set(new_task::as_kind(&event.value())),
                                for item in JOB_KINDS {
                                    option { value: "{item.as_str()}", selected: kind() == item, "{item.as_str()}" }
                                }
                            }
                        }
                    }
                    if is_repo && (chosen == How::Background || chosen == How::Goal) {
                        label { class: "new-task-field",
                            "Check"
                            input {
                                class: "new-task-check",
                                value: "{check}",
                                maxlength: "2000",
                                placeholder: "Optional, e.g. pnpm test",
                                oninput: move |event| {
                                    check.set(event.value());
                                    error.set(None);
                                },
                            }
                        }
                    }
                    if chosen == How::Goal {
                        label { class: "new-task-rounds",
                            "At most"
                            input {
                                r#type: "number",
                                min: "{new_task::ROUNDS_MIN}",
                                max: "{new_task::ROUNDS_MAX}",
                                value: "{rounds}",
                                oninput: move |event| {
                                    rounds.set(new_task::clamp_rounds(event.value().parse().unwrap_or(f64::NAN)) as f64);
                                },
                            }
                            "rounds"
                        }
                    }
                }
                if let Some(text) = notice {
                    p { class: "field-error", role: "alert", "{text}" }
                }
                div { class: "new-task-foot",
                    p { class: "hint", "{new_task::landing_line(is_repo)}" }
                    if who() == ProviderId::Codex && chosen != How::Terminal {
                        p { class: "hint", "Codex uses its native sandbox. Workspace access allows commands inside the worktree; read-only prevents file changes." }
                    }
                    div { class: "new-task-actions",
                        button {
                            r#type: "button",
                            class: "btn btn-quiet",
                            onclick: move |_| on_close.call(()),
                            "Cancel"
                        }
                        button {
                            r#type: "submit",
                            class: "btn btn-primary",
                            disabled: busy(),
                            if busy() { "Starting…" } else { {new_task::submit_label(chosen)} }
                        }
                    }
                }
            }
        }
    }
}
