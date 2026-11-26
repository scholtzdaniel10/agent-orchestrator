//! The Crabd theme's body parts: a forehead with two see-through eyes above the header, four
//! legs under the status bar that are usage meters, and the loop that cuts the window to the
//! crab (body, the two side panels as arms, the legs; minus the eyes).

use std::cell::RefCell;
use std::rc::Rc;

use dioxus::core::use_hook_with_cleanup;
use dioxus::prelude::*;
use legate_core::PlanStatus;
use serde::Serialize;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::api;
use crate::components::window_frame::drag_window;
use crate::logic::crabd::{self, LEG_COUNT, LEG_LEFTS, LEG_WIDTH, LEGS_KEY};

/// Told when Settings changes a leg, so the legs here follow at once.
pub const LEGS_EVENT: &str = "crabdlegs";

pub fn read_saved_legs() -> Option<String> {
    web_sys::window()?
        .local_storage()
        .ok()??
        .get_item(LEGS_KEY)
        .ok()?
}

pub fn write_saved_legs(raw: &str) {
    if let Some(storage) =
        web_sys::window().and_then(|window| window.local_storage().ok().flatten())
    {
        let _ = storage.set_item(LEGS_KEY, raw);
    }
    if let (Some(window), Ok(event)) = (web_sys::window(), web_sys::Event::new(LEGS_EVENT)) {
        let _ = window.dispatch_event(&event);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

/// One element's box in physical pixels, or nothing when it is not on screen.
fn rect_of(selector: &str, scale: f64) -> Option<Rect> {
    let element = web_sys::window()?
        .document()?
        .query_selector(selector)
        .ok()??;
    let bounds = element.get_bounding_client_rect();
    (bounds.width() > 0.0 && bounds.height() > 0.0).then(|| Rect {
        x: (bounds.left() * scale).round() as i32,
        y: (bounds.top() * scale).round() as i32,
        w: (bounds.width() * scale).round() as i32,
        h: (bounds.height() * scale).round() as i32,
    })
}

fn rects_of(selector: &str, scale: f64) -> Vec<Rect> {
    let Some(list) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.query_selector_all(selector).ok())
    else {
        return Vec::new();
    };
    (0..list.length())
        .filter_map(|index| list.item(index))
        .filter_map(|node| node.dyn_into::<web_sys::Element>().ok())
        .map(|element| element.get_bounding_client_rect())
        .filter(|bounds| bounds.width() > 0.0 && bounds.height() > 0.0)
        .map(|bounds| Rect {
            x: (bounds.left() * scale).round() as i32,
            y: (bounds.top() * scale).round() as i32,
            w: (bounds.width() * scale).round() as i32,
            h: (bounds.height() * scale).round() as i32,
        })
        .collect()
}

fn union(parts: &[Option<Rect>]) -> Option<Rect> {
    let parts: Vec<Rect> = parts.iter().flatten().copied().collect();
    let left = parts.iter().map(|rect| rect.x).min()?;
    let top = parts.iter().map(|rect| rect.y).min()?;
    let right = parts.iter().map(|rect| rect.x + rect.w).max()?;
    let bottom = parts.iter().map(|rect| rect.y + rect.h).max()?;
    Some(Rect {
        x: left,
        y: top,
        w: right - left,
        h: bottom - top,
    })
}

/// The crab as it is laid out right now: what to keep and the eyes to cut. `blink` closes the
/// eyes to a slit.
fn measure(blink: bool) -> (Vec<Rect>, Vec<Rect>) {
    let scale = web_sys::window()
        .map(|window| window.device_pixel_ratio())
        .unwrap_or(1.0);
    let mut keep = Vec::new();
    // The body: forehead to status bar, as wide as the forehead.
    if let (Some(head), Some(foot)) = (
        rect_of(".crabd-forehead", scale),
        rect_of(".app > .status-bar", scale),
    ) {
        keep.push(Rect {
            x: head.x,
            y: head.y,
            w: head.w,
            h: foot.y + foot.h - head.y,
        });
    }
    // The arms: each side panel with its resize handle.
    keep.extend(union(&[
        rect_of(".app-body > .sidebar-slot", scale),
        rect_of(".app-body > .layout-handle-sidebar", scale),
    ]));
    keep.extend(union(&[
        rect_of(".app-body > .drawer", scale),
        rect_of(".app-body > .layout-handle-drawer", scale),
    ]));
    keep.extend(rects_of(".crabd-leg", scale));
    let cut = rects_of(".crabd-eye", scale)
        .into_iter()
        .map(|eye| {
            if blink {
                let slit = (eye.h / 8).max(2);
                Rect {
                    y: eye.y + (eye.h - slit) / 2,
                    h: slit,
                    ..eye
                }
            } else {
                eye
            }
        })
        .collect();
    (keep, cut)
}

async fn send_shape(keep: &[Rect], cut: &[Rect]) {
    let _ = api::set_window_shape(keep, cut).await;
}

/// How often the shape is re-measured (panels resize, docks open and close), and how often
/// the eyes blink while agents work.
const MEASURE_MS: i32 = 400;
const BLINK_EVERY_MS: f64 = 3_600.0;
const BLINK_MS: i32 = 150;

/// Keeps the window cut to the crab while `active`, and gives back the plain rectangle once
/// it is not. `busy` makes the eyes blink.
fn use_crab_shape(active: bool, busy: bool) {
    let state = use_hook(|| {
        Rc::new(RefCell::new(ShapeState {
            active: false,
            busy: false,
            // As if a shape were set, so the first tick out of Crabd clears one left behind by
            // a page reload (the shell keeps a window's shape across reloads).
            last: Some((Vec::new(), Vec::new())),
            last_blink: 0.0,
            blinking: false,
        }))
    });
    {
        let mut current = state.borrow_mut();
        current.active = active;
        current.busy = busy;
    }
    use_hook_with_cleanup(
        {
            let state = state.clone();
            move || {
                let tick = Closure::wrap(Box::new(move || {
                    let now = js_sys::Date::now();
                    let mut current = state.borrow_mut();
                    if !current.active {
                        if current.last.take().is_some() {
                            spawn_local(async { send_shape(&[], &[]).await });
                        }
                        return;
                    }
                    if current.blinking {
                        return;
                    }
                    let blink_now = current.busy && now - current.last_blink >= BLINK_EVERY_MS;
                    let shape = measure(false);
                    if current.last.as_ref() != Some(&shape) {
                        current.last = Some(shape.clone());
                        let (keep, cut) = shape;
                        spawn_local(async move { send_shape(&keep, &cut).await });
                    }
                    if blink_now {
                        current.last_blink = now;
                        current.blinking = true;
                        let (keep, cut) = measure(true);
                        let state = state.clone();
                        drop(current);
                        spawn_local(async move {
                            send_shape(&keep, &cut).await;
                            gloo_sleep(BLINK_MS).await;
                            let (keep, cut) = measure(false);
                            send_shape(&keep, &cut).await;
                            let mut current = state.borrow_mut();
                            current.blinking = false;
                            current.last = Some((keep, cut));
                        });
                    }
                }) as Box<dyn FnMut()>);
                let id = web_sys::window()
                    .and_then(|window| {
                        window
                            .set_interval_with_callback_and_timeout_and_arguments_0(
                                tick.as_ref().unchecked_ref(),
                                MEASURE_MS,
                            )
                            .ok()
                    })
                    .unwrap_or(0);
                Rc::new((id, tick))
            }
        },
        |timer| {
            if let Some(window) = web_sys::window() {
                window.clear_interval_with_handle(timer.0);
            }
            spawn_local(async { send_shape(&[], &[]).await });
        },
    );
}

struct ShapeState {
    active: bool,
    busy: bool,
    last: Option<(Vec<Rect>, Vec<Rect>)>,
    last_blink: f64,
    blinking: bool,
}

async fn gloo_sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// The forehead above the header: a strip of shell with the two eyes in it. It moves the
/// window like the header does.
#[component]
pub fn CrabdForehead(busy: bool) -> Element {
    rsx! {
        div { class: if busy { "crabd-forehead is-busy" } else { "crabd-forehead" }, onmousedown: move |event| drag_window(&event),
            span { class: "crabd-eye is-left", aria_hidden: "true" }
            span { class: "crabd-eye is-right", aria_hidden: "true" }
        }
    }
}

/// The four legs, each a usage meter that fills from the ground up.
#[component]
pub fn CrabdLegs(plans: Vec<PlanStatus>) -> Element {
    let mut nonce = use_signal(|| 0u32);
    use_hook_with_cleanup(
        move || {
            let closure = Closure::wrap(Box::new(move |_: web_sys::Event| {
                nonce.with_mut(|value| *value += 1);
            }) as Box<dyn FnMut(_)>);
            if let Some(window) = web_sys::window() {
                let _ = window
                    .add_event_listener_with_callback(LEGS_EVENT, closure.as_ref().unchecked_ref());
            }
            Rc::new(closure)
        },
        |closure| {
            if let Some(window) = web_sys::window() {
                let _ = window.remove_event_listener_with_callback(
                    LEGS_EVENT,
                    closure.as_ref().as_ref().unchecked_ref(),
                );
            }
        },
    );
    let _ = nonce();
    let options = crabd::leg_options(&plans);
    let legs = crabd::legs(read_saved_legs().as_deref(), &options);
    rsx! {
        div { class: "crabd-legs", role: "group", aria_label: "Usage",
            for index in 0..LEG_COUNT {
                {
                    let reading = legs[index].as_deref().and_then(|key| crabd::reading(&plans, key));
                    let left = LEG_LEFTS[index] * 100.0;
                    let width = LEG_WIDTH * 100.0;
                    let style = format!("left: {left:.1}%; width: {width:.1}%;");
                    match reading {
                        Some(leg) => {
                            let percent = (leg.used * 100.0).round();
                            let level = if leg.used >= 0.9 { "is-high" } else if leg.used >= 0.7 { "is-warm" } else { "" };
                            rsx! {
                                div { key: "{index}", class: "crabd-leg {level}", style: "{style}", title: "{leg.title}",
                                    span { class: "crabd-leg-fill", style: "height: {percent}%;" }
                                    span { class: "crabd-leg-name", span { class: "crabd-leg-plan", "{leg.plan}" } "{leg.short}" }
                                    span { class: "crabd-leg-value", "{percent}%" }
                                }
                            }
                        }
                        None => rsx! {
                            div { key: "{index}", class: "crabd-leg is-empty", style: "{style}", title: "Set this leg in Settings, under Look",
                                span { class: "crabd-leg-name", "–" }
                            }
                        },
                    }
                }
            }
        }
    }
}

/// Drives the shape; renders nothing itself.
#[component]
pub fn CrabdShape(active: bool, busy: bool) -> Element {
    use_crab_shape(active, busy);
    rsx! {}
}

#[cfg(test)]
mod tests {
    use super::{Rect, union};

    #[test]
    fn an_arm_is_its_panel_and_handle_together() {
        let panel = Rect {
            x: 0,
            y: 100,
            w: 240,
            h: 400,
        };
        let handle = Rect {
            x: 240,
            y: 100,
            w: 10,
            h: 400,
        };
        assert_eq!(
            union(&[Some(panel), Some(handle)]),
            Some(Rect {
                x: 0,
                y: 100,
                w: 250,
                h: 400
            })
        );
        assert_eq!(union(&[None, Some(handle)]), Some(handle));
        assert_eq!(union(&[None, None]), None);
    }
}
