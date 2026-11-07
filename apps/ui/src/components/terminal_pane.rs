//! One xterm session. Output arrives on the terminal bus; keystrokes go back through Tauri.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use dioxus::prelude::*;
use gloo_timers::callback::Interval;
use legate_core::{TerminalInfo, TerminalStatus};
use wasm_bindgen_futures::spawn_local;

use crate::api;
use crate::logic::terminal_bus::{Detach, TerminalBus};
use crate::state::use_store;
use crate::xterm;

#[derive(Clone, PartialEq)]
pub enum PanePlacement {
    Slot(i64),
    Hidden,
}

struct Session {
    id: String,
    detach: Option<Detach>,
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(detach) = self.detach.take() {
            detach.call();
        }
        let _ = xterm::dispose(&self.id);
    }
}

#[component]
pub fn TerminalPane(
    info: TerminalInfo,
    heading: Option<String>,
    bus: TerminalBus,
    restore: bool,
    placement: PanePlacement,
    focused: bool,
    focus_nonce: i64,
    on_activate: Callback,
    on_close: Callback,
    #[props(default = false)] own_window: bool,
) -> Element {
    let store = use_store();
    let id = info.id.clone();
    let visible = !matches!(placement, PanePlacement::Hidden);
    let popped = !own_window && store.popped.read().iter().any(|item| item == &id);
    let label = heading.clone().unwrap_or_else(|| info.title.clone());
    let host = format!("term-host-{id}");
    let font = store.prefs.read().terminal_font_size as f64;
    let mut session: Signal<Option<Session>> = use_signal(|| None);
    let ticket = use_hook(|| Rc::new(Cell::new(0u64)));
    let seen_focus = use_hook(|| Rc::new(Cell::new(0i64)));
    let claim_key = use_hook(|| Rc::new(RefCell::new(String::new())));
    let waking = use_hook(|| Rc::new(Cell::new(false)));
    let mut booted = use_signal(String::new);
    let boot_key = format!("{id}:{restore}:{popped}:{visible}");
    if booted() != boot_key {
        booted.set(boot_key);
    }

    use_effect(move || {
        let key = booted();
        let mut parts = key.split(':');
        let Some(term_id) = parts.next() else { return };
        let restore = parts.next() == Some("true");
        let popped = parts.next() == Some("true");
        let visible = parts.next() == Some("true");
        let term_id = term_id.to_string();
        let host = format!("term-host-{term_id}");
        let next = Cell::get(&ticket) + 1;
        ticket.set(next);
        let ticket = ticket.clone();
        let bus = bus.clone();
        session.set(None);
        if !visible || popped {
            return;
        }
        spawn_local(async move {
            let Some(element) = web_sys::window()
                .and_then(|window| window.document())
                .and_then(|document| document.get_element_by_id(&host))
            else {
                return;
            };
            if xterm::open(&term_id, &element, font).await.is_err() || Cell::get(&ticket) != next {
                let _ = xterm::dispose(&term_id);
                return;
            }
            let replaying = Rc::new(Cell::new(false));
            let flag = replaying.clone();
            let write_id = term_id.clone();
            xterm::on_data(&term_id, move |data| {
                if Cell::get(&flag) || data.is_empty() {
                    return;
                }
                let write_id = write_id.clone();
                spawn_local(async move {
                    let _ = api::write_terminal(&write_id, &data).await;
                });
            });
            if restore && let Ok(snapshot) = api::terminal_snapshot(&term_id).await {
                replaying.set(true);
                let _ = xterm::write(&term_id, &snapshot);
                replaying.set(false);
                bus.forget(&term_id);
            }
            if Cell::get(&ticket) != next {
                let _ = xterm::dispose(&term_id);
                return;
            }
            let sink_id = term_id.clone();
            let detach = bus.attach(&term_id, move |chunk| {
                let _ = xterm::write(&sink_id, &chunk);
            });
            let _ = xterm::disable_stdin(&term_id, false);
            session.set(Some(Session {
                id: term_id,
                detach: Some(detach),
            }));
        });
    });

    if info.status != TerminalStatus::Running {
        let _ = xterm::disable_stdin(&id, true);
    }
    if !visible || popped {
        let _ = xterm::drop_webgl(&id);
    }
    if visible && focused && focus_nonce != Cell::get(&seen_focus) {
        seen_focus.set(focus_nonce);
        let _ = xterm::focus(&id);
    }
    let watch = format!("{visible}:{focused}:{popped}:{id}");
    if claim_key.borrow().as_str() != watch {
        *claim_key.borrow_mut() = watch;
        if visible && focused && !popped {
            let claim = id.clone();
            spawn_local(async move {
                let _ = api::claim_terminal(&claim).await;
                if let Ok(cells) = xterm::fit(&claim) {
                    let _ = api::resize_terminal(&claim, cells.cols, cells.rows).await;
                }
            });
        }
    }
    if info.status == TerminalStatus::Sleeping && visible && !popped {
        if !Cell::get(&waking) {
            waking.set(true);
            let wake = id.clone();
            spawn_local(async move {
                let _ = api::wake_terminal(&wake, 80.0, 24.0).await;
            });
        }
    } else {
        waking.set(false);
    }

    let fit_id = id.clone();
    let prefs = store.prefs;
    let fit_font = use_hook(|| Rc::new(Cell::new(font)));
    use_hook(move || {
        let fit_id = fit_id.clone();
        let fit_font = fit_font.clone();
        Rc::new(Interval::new(700, move || {
            let wanted = prefs.read().terminal_font_size as f64;
            if (Cell::get(&fit_font) - wanted).abs() > 0.1
                && let Ok(cells) = xterm::font(&fit_id, wanted)
            {
                fit_font.set(wanted);
                let fit_id = fit_id.clone();
                spawn_local(async move {
                    let _ = api::resize_terminal(&fit_id, cells.cols, cells.rows).await;
                });
                return;
            }
            if let Ok(cells) = xterm::fit(&fit_id) {
                let fit_id = fit_id.clone();
                spawn_local(async move {
                    let _ = api::resize_terminal(&fit_id, cells.cols, cells.rows).await;
                });
            }
        }))
    });

    let class = match (&placement, focused) {
        (PanePlacement::Hidden, _) => "terminal-pane is-hidden".to_string(),
        (PanePlacement::Slot(slot), true) => format!("terminal-pane is-slot-{slot} is-focused"),
        (PanePlacement::Slot(slot), false) => format!("terminal-pane is-slot-{slot}"),
    };
    let exit = info
        .exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "—".into());
    let pop_id = id.clone();
    let close_label = label.clone();

    rsx! {
        div {
            class: "{class}",
            onmousedown: move |_| on_activate.call(()),
            if visible {
                div { class: "terminal-pane-head",
                    span { class: "terminal-pane-title", "{label}" }
                    if !own_window {
                        button {
                            class: "tab-close",
                            title: "Open in its own window",
                            aria_label: "Open in its own window",
                            onclick: move |event| {
                                event.stop_propagation();
                                let pop_id = pop_id.clone();
                                spawn_local(async move {
                                    let _ = api::pop_out_terminal(&pop_id).await;
                                });
                            },
                            "↗"
                        }
                    }
                    button {
                        class: "tab-close",
                        aria_label: "Close {close_label}",
                        onclick: move |event| {
                            event.stop_propagation();
                            on_close.call(());
                        },
                        "×"
                    }
                }
            }
            if popped {
                div { class: "terminal-popped",
                    p { "Open in its own window" }
                    button {
                        class: "btn btn-quiet btn-compact",
                        onclick: move |event| {
                            event.stop_propagation();
                            let pop_id = id.clone();
                            spawn_local(async move {
                                let _ = api::pop_out_terminal(&pop_id).await;
                            });
                        },
                        "Bring back"
                    }
                }
            } else {
                if info.status == TerminalStatus::Exited {
                    p { class: "terminal-banner", "Session ended (exit code {exit}). Close the tab or open a new one." }
                } else if info.status == TerminalStatus::Sleeping {
                    p { class: "terminal-banner", "Asleep to save memory. Waking it…" }
                }
                div { id: "{host}", class: "terminal-host" }
            }
        }
    }
}
