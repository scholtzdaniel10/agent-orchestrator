//! The Crabd theme's body parts: a brow under the header with two see-through eyes, four legs
//! under the status bar that are usage meters, and the loop that cuts the window to the crab
//! (body, the two side panels as arms, the legs; minus the eyes). A shaped window has no
//! system borders, so the crab's outer edges are its own resize grips.

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
use crate::logic::crabd::{self, Area, LEG_COUNT, LEG_LEFTS, LEG_WIDTH, LEGS_KEY};

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

/// A rectangle in physical pixels, as the shell takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

fn to_rect(area: &Area, scale: f64) -> Rect {
    Rect {
        x: (area.x * scale).round() as i32,
        y: (area.y * scale).round() as i32,
        w: (area.w * scale).round() as i32,
        h: (area.h * scale).round() as i32,
    }
}

fn areas_of(selector: &str) -> Vec<Area> {
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
        .map(|bounds| Area {
            x: bounds.left(),
            y: bounds.top(),
            w: bounds.width(),
            h: bounds.height(),
        })
        .collect()
}

fn area_of(selector: &str) -> Option<Area> {
    areas_of(selector).into_iter().next()
}

fn union(parts: &[Option<Area>]) -> Option<Area> {
    let parts: Vec<Area> = parts.iter().flatten().copied().collect();
    let left = parts.iter().map(|area| area.x).reduce(f64::min)?;
    let top = parts.iter().map(|area| area.y).reduce(f64::min)?;
    let right = parts.iter().map(|area| area.x + area.w).reduce(f64::max)?;
    let bottom = parts.iter().map(|area| area.y + area.h).reduce(f64::max)?;
    Some(Area {
        x: left,
        y: top,
        w: right - left,
        h: bottom - top,
    })
}

/// The crab as it is laid out right now, in page pixels: the parts to keep and the eyes.
fn parts() -> (Vec<Area>, Vec<Area>) {
    let mut keep = Vec::new();
    // The body: header to status bar, as wide as the header.
    if let (Some(head), Some(foot)) = (
        area_of(".roman-workspace > .app-header"),
        area_of(".roman-workspace > .status-bar"),
    ) {
        keep.push(Area {
            h: foot.y + foot.h - head.y,
            ..head
        });
    }
    // The arms: each side panel with its resize handle.
    keep.extend(union(&[
        area_of(".app-body > .sidebar-slot"),
        area_of(".app-body > .layout-handle-sidebar"),
    ]));
    keep.extend(union(&[
        area_of(".app-body > .drawer"),
        area_of(".app-body > .layout-handle-drawer"),
    ]));
    keep.extend(areas_of(".crabd-leg"));
    (keep, areas_of(".crabd-eye"))
}

/// The shape to send, in physical pixels; `blink` narrows the eyes to a slit.
fn shape(keep: &[Area], eyes: &[Area], blink: bool) -> (Vec<Rect>, Vec<Rect>) {
    let scale = web_sys::window()
        .map(|window| window.device_pixel_ratio())
        .unwrap_or(1.0);
    let keep = keep.iter().map(|area| to_rect(area, scale)).collect();
    let cut = eyes
        .iter()
        .map(|eye| {
            let eye = to_rect(eye, scale);
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

/// How often the eyes blink while agents work, and for how long.
const BLINK_EVERY_MS: f64 = 3_600.0;
const BLINK_MS: i32 = 150;

const CURSORS: [&str; 8] = ["n", "s", "e", "w", "ne", "nw", "se", "sw"];

/// Shows the resize cursor for a grip (or none) on the whole page.
fn set_grip_cursor(edge: Option<&str>) {
    let Some(root) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.document_element())
    else {
        return;
    };
    let list = root.class_list();
    for name in CURSORS {
        let class = format!("crabd-grip-{name}");
        if Some(name) == edge {
            let _ = list.add_1(&class);
        } else {
            let _ = list.remove_1(&class);
        }
    }
}

struct ShapeState {
    active: bool,
    busy: bool,
    stopped: bool,
    /// The last shape sent; set to an empty one at the start so the first frame out of Crabd
    /// clears a shape a page reload left behind (the shell keeps it across reloads).
    last: Option<(Vec<Rect>, Vec<Rect>)>,
    /// The parts as last measured, for the grips.
    keep: Vec<Area>,
    /// Whether the crab's minimum window size is set; true at the start for the same reason.
    floored: bool,
    last_blink: f64,
    blinking: bool,
}

type Frame = Rc<RefCell<Option<Closure<dyn FnMut()>>>>;

/// One frame of the loop: re-measure, send the shape when it changed, blink when due.
fn frame(state: &Rc<RefCell<ShapeState>>) {
    let now = js_sys::Date::now();
    let mut current = state.borrow_mut();
    if !current.active {
        if current.last.take().is_some() {
            spawn_local(async { send_shape(&[], &[]).await });
        }
        if current.floored {
            current.floored = false;
            spawn_local(async {
                let _ = api::window_control("min_none").await;
            });
        }
        current.keep.clear();
        return;
    }
    if !current.floored {
        current.floored = true;
        spawn_local(async {
            let _ = api::window_control("min_crab").await;
        });
    }
    if current.blinking {
        return;
    }
    let (keep, eyes) = parts();
    let open = shape(&keep, &eyes, false);
    current.keep = keep.clone();
    if current.last.as_ref() != Some(&open) {
        current.last = Some(open.clone());
        let (keep, cut) = open;
        spawn_local(async move { send_shape(&keep, &cut).await });
    }
    if current.busy && now - current.last_blink >= BLINK_EVERY_MS {
        current.last_blink = now;
        current.blinking = true;
        let (shut_keep, shut_cut) = shape(&keep, &eyes, true);
        let state = state.clone();
        drop(current);
        spawn_local(async move {
            send_shape(&shut_keep, &shut_cut).await;
            sleep(BLINK_MS).await;
            let (keep, eyes) = parts();
            let (keep, cut) = shape(&keep, &eyes, false);
            send_shape(&keep, &cut).await;
            let mut current = state.borrow_mut();
            current.blinking = false;
            current.last = Some((keep, cut));
        });
    }
}

struct Listeners {
    state: Rc<RefCell<ShapeState>>,
    move_listener: Closure<dyn FnMut(web_sys::MouseEvent)>,
    down_listener: Closure<dyn FnMut(web_sys::MouseEvent)>,
}

/// Keeps the window cut to the crab while `active`, following every frame, and gives back the
/// plain rectangle once it is not. `busy` makes the eyes blink.
fn use_crab_shape(active: bool, busy: bool) {
    let state = use_hook(|| {
        Rc::new(RefCell::new(ShapeState {
            active: false,
            busy: false,
            stopped: false,
            last: Some((Vec::new(), Vec::new())),
            keep: Vec::new(),
            floored: true,
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
                // The loop: one measure per animation frame, so resizes and panel drags are
                // followed at once instead of leaving parts of the window cut away.
                let next: Frame = Rc::new(RefCell::new(None));
                let again = next.clone();
                let looping = state.clone();
                *next.borrow_mut() = Some(Closure::wrap(Box::new(move || {
                    if looping.borrow().stopped {
                        again.borrow_mut().take();
                        return;
                    }
                    frame(&looping);
                    if let (Some(window), Some(callback)) =
                        (web_sys::window(), again.borrow().as_ref())
                    {
                        let _ = window.request_animation_frame(callback.as_ref().unchecked_ref());
                    }
                }) as Box<dyn FnMut()>));
                if let (Some(window), Some(callback)) = (web_sys::window(), next.borrow().as_ref())
                {
                    let _ = window.request_animation_frame(callback.as_ref().unchecked_ref());
                }
                // The grips: a resize cursor over the crab's outer edges, and a press there
                // starts a system resize from that edge.
                let hovering = state.clone();
                let move_listener = Closure::wrap(Box::new(move |event: web_sys::MouseEvent| {
                    let current = hovering.borrow();
                    let edge = if current.active && event.buttons() == 0 {
                        crabd::grip(
                            &current.keep,
                            event.client_x() as f64,
                            event.client_y() as f64,
                        )
                    } else {
                        None
                    };
                    set_grip_cursor(edge);
                }) as Box<dyn FnMut(_)>);
                let pressing = state.clone();
                let down_listener = Closure::wrap(Box::new(move |event: web_sys::MouseEvent| {
                    let current = pressing.borrow();
                    if !current.active || event.button() != 0 {
                        return;
                    }
                    let Some(edge) = crabd::grip(
                        &current.keep,
                        event.client_x() as f64,
                        event.client_y() as f64,
                    ) else {
                        return;
                    };
                    event.prevent_default();
                    event.stop_propagation();
                    let action = format!("resize_{edge}");
                    spawn_local(async move {
                        let _ = api::window_control(&action).await;
                    });
                }) as Box<dyn FnMut(_)>);
                if let Some(window) = web_sys::window() {
                    let _ = window.add_event_listener_with_callback(
                        "mousemove",
                        move_listener.as_ref().unchecked_ref(),
                    );
                    let _ = window.add_event_listener_with_callback_and_bool(
                        "mousedown",
                        down_listener.as_ref().unchecked_ref(),
                        true,
                    );
                }
                Rc::new(Listeners {
                    state,
                    move_listener,
                    down_listener,
                })
            }
        },
        |listeners| {
            listeners.state.borrow_mut().stopped = true;
            if let Some(window) = web_sys::window() {
                let _ = window.remove_event_listener_with_callback(
                    "mousemove",
                    listeners.move_listener.as_ref().unchecked_ref(),
                );
                let _ = window.remove_event_listener_with_callback_and_bool(
                    "mousedown",
                    listeners.down_listener.as_ref().unchecked_ref(),
                    true,
                );
            }
            set_grip_cursor(None);
            spawn_local(async {
                send_shape(&[], &[]).await;
                let _ = api::window_control("min_none").await;
            });
        },
    );
}

async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// The brow under the header: a strip of shell with the two eyes in it. It moves the
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
    use super::{Area, union};

    #[test]
    fn an_arm_is_its_panel_and_handle_together() {
        let panel = Area {
            x: 0.0,
            y: 100.0,
            w: 240.0,
            h: 400.0,
        };
        let handle = Area {
            x: 240.0,
            y: 100.0,
            w: 10.0,
            h: 400.0,
        };
        assert_eq!(
            union(&[Some(panel), Some(handle)]),
            Some(Area {
                x: 0.0,
                y: 100.0,
                w: 250.0,
                h: 400.0
            })
        );
        assert_eq!(union(&[None, Some(handle)]), Some(handle));
        assert_eq!(union(&[None, None]), None);
    }
}
