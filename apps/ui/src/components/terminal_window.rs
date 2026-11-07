use dioxus::prelude::*;
use legate_core::TerminalInfo;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::api::{self, TerminalUpdate};
use crate::components::terminal_pane::{PanePlacement, TerminalPane};
use crate::state::use_store;

#[component]
pub fn TerminalWindow(id: String) -> Element {
    let store = use_store();
    let mut info = use_signal(|| None::<TerminalInfo>);
    let mut missing = use_signal(|| false);
    let term_id = id.clone();
    let close_id = id.clone();
    use_effect(move || {
        let listed = term_id.clone();
        spawn_local(async move {
            if let Ok(list) = api::list_terminals().await {
                info.set(list.into_iter().find(|item| item.id == listed));
                if info.read().is_none() {
                    missing.set(true);
                }
            }
        });
        let watched = term_id.clone();
        spawn_local(async move {
            let _ = api::on_terminal_update(
                move |TerminalUpdate {
                          info: next,
                          removed,
                      }| {
                    if next.id != watched {
                        return;
                    }
                    if removed {
                        info.set(None);
                        missing.set(true);
                    } else {
                        missing.set(false);
                        info.set(Some(next));
                    }
                },
            )
            .await;
        });
        let closure =
            wasm_bindgen::closure::Closure::wrap(Box::new(|event: web_sys::KeyboardEvent| {
                if event.ctrl_key()
                    && event.shift_key()
                    && !event.alt_key()
                    && (event.key() == "N" || event.key() == "n")
                {
                    event.prevent_default();
                    spawn_local(async {
                        let _ = api::new_window().await;
                    });
                }
            }) as Box<dyn FnMut(_)>);
        if let Some(window) = web_sys::window() {
            let _ = window
                .add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref());
        }
        closure.forget();
    });
    let current = info.read().clone();
    let gone = missing();
    let bus = store.bus.read().clone();
    rsx! {
        div { class: "terminal-window",
            if let Some(term) = current {
                TerminalPane {
                    info: term,
                    heading: None,
                    bus: bus,
                    restore: true,
                    placement: PanePlacement::Slot(0),
                    focused: true,
                    focus_nonce: 1,
                    on_activate: Callback::new(|_| {}),
                    on_close: Callback::new(move |_| {
                        let close_id = close_id.clone();
                        spawn_local(async move { let _ = api::close_terminal(&close_id).await; });
                    }),
                    own_window: true,
                }
            } else if gone {
                p { class: "hint", "That terminal has closed." }
            } else {
                p { class: "hint", "Opening the terminal…" }
            }
        }
    }
}
