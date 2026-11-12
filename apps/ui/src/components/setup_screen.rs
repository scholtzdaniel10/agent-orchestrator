use std::cell::Cell;
use std::rc::Rc;

use dioxus::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::api::{self, TerminalUpdate};
use crate::components::terminal_pane::{PanePlacement, TerminalPane};
use crate::logic::terminal_bus::TerminalBus;
use crate::state::now_ms;
use crate::wire::{Requirement, RequirementId, RequirementState};

#[component]
pub fn SetupScreen(bus: TerminalBus, on_close: Callback, on_skip: Callback) -> Element {
    let rows = use_signal(|| None::<Vec<Requirement>>);
    let checked_at = use_signal(|| None::<i64>);
    let checking = use_signal(|| true);
    let mut copied = use_signal(|| None::<RequirementId>);
    let mut sign_ins = use_signal(Vec::<(String, legate_core::TerminalInfo)>::new);
    let mut error = use_signal(|| None::<String>);

    use_effect(move || {
        spawn_local(async move {
            load(rows, checked_at, checking, error).await;
        });
        spawn_local(async move {
            let _ = api::on_terminal_update(move |TerminalUpdate { info, removed }| {
                let found = sign_ins.read().iter().any(|(_, term)| term.id == info.id);
                if !found {
                    return;
                }
                if removed {
                    sign_ins.with_mut(|list| list.retain(|(_, term)| term.id != info.id));
                    spawn_local(async move {
                        load(rows, checked_at, checking, error).await;
                    });
                    return;
                }
                let exited = info.status == legate_core::TerminalStatus::Exited;
                sign_ins.with_mut(|list| {
                    if let Some(slot) = list.iter_mut().find(|(_, term)| term.id == info.id) {
                        slot.1 = info;
                    }
                });
                if exited {
                    spawn_local(async move {
                        load(rows, checked_at, checking, error).await;
                    });
                }
            })
            .await;
        });
    });

    let ready = rows.read().as_ref().is_some_and(|list| can_work(list));
    let ready_flag = use_hook(|| Rc::new(Cell::new(false)));
    ready_flag.set(ready);
    use_hook(move || {
        let on_close = on_close;
        let ready_flag = ready_flag.clone();
        let closure =
            wasm_bindgen::closure::Closure::wrap(Box::new(move |event: web_sys::KeyboardEvent| {
                if event.key() == "Escape" && ready_flag.get() {
                    event.prevent_default();
                    on_close.call(());
                }
            }) as Box<dyn FnMut(_)>);
        if let Some(window) = web_sys::window() {
            let _ = window
                .add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref());
        }
        closure.forget();
    });

    let list = rows.read().clone();
    let sign = sign_ins.read().clone();
    let message = error.read().clone();
    let when = checked_at();
    let busy = checking();
    let copied_id = copied();

    rsx! {
        div { class: "setup-screen", role: "dialog", aria_modal: "true", aria_labelledby: "setup-heading", tabindex: "-1",
            div { class: "setup-inner",
                h1 { id: "setup-heading", "Set up Legate" }
                p { "Legate runs your own Claude and Cursor plans through their official command-line tools." }
                p { "Sign in to each tool here once — Legate never sees your password or token." }
                if let Some(message) = message {
                    p { class: "setup-error", "{message}" }
                }
                if let Some(list) = list {
                    ul { class: "setup-list",
                        for row in list.iter().cloned() {
                            {
                                let sign_id = sign_in_name(row.id);
                                let term = sign_id.and_then(|name| sign.iter().find(|(id, _)| id == name).map(|(_, term)| term.clone()));
                                let row_id = row.id;
                                let command = row.install.command.clone();
                                rsx! {
                                    li { class: "setup-row", key: "{row_id:?}",
                                        div { class: "setup-row-top",
                                            div { class: "setup-row-name",
                                                span { class: "setup-label", "{row.label}" }
                                                if row.need == crate::wire::RequirementNeed::Plan {
                                                    span { class: "setup-tag", "one of these two is needed" }
                                                }
                                                if row.need == crate::wire::RequirementNeed::Optional {
                                                    span { class: "setup-tag", "optional" }
                                                }
                                            }
                                            span { class: "setup-mark is-{state_class(row.state)}", "{mark_label(row.state)}" }
                                        }
                                        if let Some(version) = row.version.clone() {
                                            p { class: "setup-version", "{version}" }
                                        }
                                        p { class: "setup-note", "{row.note}" }
                                        if row.state == RequirementState::Missing {
                                            div { class: "setup-install",
                                                div { class: "setup-command",
                                                    code { "{command}" }
                                                    button {
                                                        class: "btn btn-quiet btn-compact",
                                                        onclick: move |_| {
                                                            let command = command.clone();
                                                            let row_id = row_id;
                                                            spawn_local(async move {
                                                                if copy_text(&command).await {
                                                                    copied.set(Some(row_id));
                                                                }
                                                            });
                                                        },
                                                        if copied_id == Some(row_id) { "Copied" } else { "Copy" }
                                                    }
                                                }
                                                a { class: "setup-link", href: "{row.install.url}", target: "_blank", rel: "noreferrer", "Official page" }
                                                p { class: "hint", "Run this in a terminal, then press Re-check." }
                                            }
                                        }
                                        if row.state == RequirementState::SignedOut && let Some(name) = sign_id {
                                            div { class: "setup-signin",
                                                button {
                                                    class: "btn btn-primary",
                                                    onclick: move |_| {
                                                        let name = name.to_string();
                                                        spawn_local(async move {
                                                            match api::setup_sign_in(&name).await {
                                                                Ok(info) => {
                                                                    sign_ins.with_mut(|list| list.push((name, info)));
                                                                    error.set(None);
                                                                }
                                                                Err(text) => error.set(Some(text)),
                                                            }
                                                        });
                                                    },
                                                    "Sign in"
                                                }
                                                if let Some(term) = term {
                                                    div { class: "setup-term",
                                                        TerminalPane {
                                                            info: term.clone(),
                                                            heading: None,
                                                            bus: bus.clone(),
                                                            restore: false,
                                                            placement: PanePlacement::Slot(0),
                                                            focused: true,
                                                            focus_nonce: 1,
                                                            own_window: true,
                                                            on_activate: Callback::new(|_| {}),
                                                            on_close: Callback::new(move |_| {
                                                                let id = term.id.clone();
                                                                sign_ins.with_mut(|list| list.retain(|(_, item)| item.id != id));
                                                                spawn_local(async move { let _ = api::close_terminal(&id).await; });
                                                            }),
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
                    div { class: "setup-check",
                        button {
                            class: "btn btn-quiet",
                            disabled: busy,
                            onclick: move |_| {
                                spawn_local(async move { load(rows, checked_at, checking, error).await; });
                            },
                            if busy { "Checking…" } else { "Re-check" }
                        }
                        if let Some(when) = when {
                            span { class: "hint", "Last check {clock(when)}" }
                        }
                    }
                    div { class: "setup-footer",
                        button { class: "btn btn-primary", disabled: !ready, onclick: move |_| on_close.call(()), "Start using Legate" }
                        button { class: "btn btn-quiet", onclick: move |_| on_skip.call(()), "Skip for now" }
                        if !ready {
                            p { class: "hint", "{missing_line(&list)}" }
                        }
                    }
                } else {
                    p { class: "hint", "Checking…" }
                }
            }
        }
    }
}

async fn load(
    mut rows: Signal<Option<Vec<Requirement>>>,
    mut checked_at: Signal<Option<i64>>,
    mut checking: Signal<bool>,
    mut error: Signal<Option<String>>,
) {
    checking.set(true);
    match api::setup_status().await {
        Ok(list) => {
            rows.set(Some(list));
            checked_at.set(Some(now_ms()));
            error.set(None);
        }
        Err(text) => error.set(Some(text)),
    }
    checking.set(false);
}

fn can_work(list: &[Requirement]) -> bool {
    let git = list
        .iter()
        .any(|row| row.id == RequirementId::Git && row.state == RequirementState::Ready);
    let plan = list.iter().any(|row| {
        matches!(
            row.id,
            RequirementId::Claude | RequirementId::Codex | RequirementId::Cursor
        ) && row.state == RequirementState::Ready
    });
    git && plan
}

fn missing_line(list: &[Requirement]) -> &'static str {
    let git = list
        .iter()
        .any(|row| row.id == RequirementId::Git && row.state == RequirementState::Ready);
    let plan = list.iter().any(|row| {
        matches!(
            row.id,
            RequirementId::Claude | RequirementId::Codex | RequirementId::Cursor
        ) && row.state == RequirementState::Ready
    });
    if !git && !plan {
        "Git and at least one plan are needed."
    } else if !git {
        "Git is needed."
    } else {
        "At least one plan is needed."
    }
}

fn mark_label(state: RequirementState) -> &'static str {
    match state {
        RequirementState::Ready => "Ready",
        RequirementState::SignedOut => "Signed out",
        RequirementState::Missing => "Not installed",
    }
}

fn state_class(state: RequirementState) -> &'static str {
    match state {
        RequirementState::Ready => "ready",
        RequirementState::SignedOut => "signed-out",
        RequirementState::Missing => "missing",
    }
}

fn sign_in_name(id: RequirementId) -> Option<&'static str> {
    match id {
        RequirementId::Claude => Some("claude"),
        RequirementId::Codex => Some("codex"),
        RequirementId::Cursor => Some("cursor"),
        RequirementId::Gh => Some("gh"),
        RequirementId::Git => None,
    }
}

/// Local `HH:MM`. `time::UtcOffset::current_local_offset` reads the system clock, which panics
/// in the browser build, so this goes through the same `js_sys::Date` path as the activity log.
fn clock(ms: i64) -> String {
    crate::logic::flow_events::format_hours_minutes(ms)
}

async fn copy_text(text: &str) -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let clipboard = js_sys::Reflect::get(window.navigator().as_ref(), &"clipboard".into()).ok();
    let Some(clipboard) = clipboard else {
        return false;
    };
    let write = js_sys::Reflect::get(&clipboard, &"writeText".into()).ok();
    let Some(write) = write else { return false };
    let Ok(function) = write.dyn_into::<js_sys::Function>() else {
        return false;
    };
    function.call1(&clipboard, &text.into()).is_ok()
}
