//! Legate's own window frame: the windows have no system title bar, so the app header (and a
//! slim bar on popped-out windows) moves the window, and the minimize, maximize and close
//! buttons sit at its right end, as in the Claude and Teams desktop apps.

use dioxus::core::use_hook_with_cleanup;
use dioxus::prelude::*;
use dioxus::web::WebEventExt;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::api;

/// What a press on these does not start a window drag from.
const INTERACTIVE: &str = "button, a, input, textarea, select, summary, details, label, [role='button'], [role='menu'], [contenteditable='true'], .no-drag";

/// Starts a window move from a press on an empty part of a title bar; a double press maximizes
/// or restores, as on a system title bar. Presses on buttons, menus and fields are left alone.
pub fn drag_window(event: &MouseEvent) {
    let Some(mouse) = event.try_as_web_event() else {
        return;
    };
    if mouse.button() != 0 || !api::in_desktop() {
        return;
    }
    let on_control = mouse
        .target()
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
        .and_then(|element| element.closest(INTERACTIVE).ok().flatten())
        .is_some();
    if on_control {
        return;
    }
    // Keeps the press from selecting header text while the window moves.
    mouse.prevent_default();
    let action = if mouse.detail() >= 2 {
        "toggle_maximize"
    } else {
        "drag"
    };
    spawn_local(async move {
        let _ = api::window_control(action).await;
    });
}

/// Minimize, maximize or restore, and close. Renders nothing outside the desktop shell.
#[component]
pub fn WindowControls() -> Element {
    let mut maximized = use_signal(|| false);
    use_hook_with_cleanup(
        move || {
            let refresh = move || {
                spawn_local(async move {
                    if let Ok(next) = api::window_control("state").await {
                        maximized.set(next);
                    }
                });
            };
            refresh();
            let closure =
                wasm_bindgen::closure::Closure::wrap(
                    Box::new(move |_: web_sys::Event| refresh()) as Box<dyn FnMut(_)>
                );
            if let Some(window) = web_sys::window() {
                let _ = window
                    .add_event_listener_with_callback("resize", closure.as_ref().unchecked_ref());
            }
            std::rc::Rc::new(closure)
        },
        |closure| {
            if let Some(window) = web_sys::window() {
                let _ = window.remove_event_listener_with_callback(
                    "resize",
                    closure.as_ref().as_ref().unchecked_ref(),
                );
            }
        },
    );
    if !api::in_desktop() {
        return rsx! {};
    }
    let is_max = maximized();
    rsx! {
        div { class: "window-controls", role: "group", aria_label: "Window",
            button {
                class: "window-control is-minimize",
                r#type: "button",
                title: "Minimize",
                aria_label: "Minimize",
                onclick: move |_| { spawn(async move { let _ = api::window_control("minimize").await; }); },
                svg { view_box: "0 0 10 10", width: "10", height: "10", "aria-hidden": "true",
                    path { d: "M0 5.5H10", stroke: "currentColor", stroke_width: "1" }
                }
            }
            button {
                class: "window-control is-maximize",
                r#type: "button",
                title: if is_max { "Restore down" } else { "Maximize" },
                aria_label: if is_max { "Restore down" } else { "Maximize" },
                onclick: move |_| {
                    spawn(async move {
                        if let Ok(next) = api::window_control("toggle_maximize").await {
                            maximized.set(next);
                        }
                    });
                },
                if is_max {
                    svg { view_box: "0 0 10 10", width: "10", height: "10", "aria-hidden": "true", fill: "none",
                        path { d: "M2.5 2.5V0.5H9.5V7.5H7.5", stroke: "currentColor", stroke_width: "1" }
                        rect { x: "0.5", y: "2.5", width: "7", height: "7", stroke: "currentColor", stroke_width: "1" }
                    }
                } else {
                    svg { view_box: "0 0 10 10", width: "10", height: "10", "aria-hidden": "true", fill: "none",
                        rect { x: "0.5", y: "0.5", width: "9", height: "9", stroke: "currentColor", stroke_width: "1" }
                    }
                }
            }
            button {
                class: "window-control is-close",
                r#type: "button",
                title: "Close",
                aria_label: "Close",
                onclick: move |_| { spawn(async move { let _ = api::window_control("close").await; }); },
                svg { view_box: "0 0 10 10", width: "10", height: "10", "aria-hidden": "true",
                    path { d: "M0.5 0.5L9.5 9.5M9.5 0.5L0.5 9.5", stroke: "currentColor", stroke_width: "1.1" }
                }
            }
        }
    }
}

/// The slim title bar of a popped-out terminal or panel window.
#[component]
pub fn WindowBar(title: String) -> Element {
    if !api::in_desktop() {
        return rsx! {};
    }
    rsx! {
        header { class: "window-bar", onmousedown: move |event| drag_window(&event),
            span { class: "window-bar-title", "{title}" }
            WindowControls {}
        }
    }
}
