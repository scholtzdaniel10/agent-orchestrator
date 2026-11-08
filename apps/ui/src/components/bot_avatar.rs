use std::sync::atomic::{AtomicU64, Ordering};

use dioxus::html::{Key, Modifiers};
use dioxus::prelude::*;
use legate_core::ProviderId;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BotName {
    Lead,
    Plan(ProviderId),
}

impl BotName {
    pub fn class(self) -> &'static str {
        match self {
            Self::Lead => "lead",
            Self::Plan(id) => id.as_str(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BotState {
    Idle,
    Working,
    Resting,
    Error,
}

const HEAD: &str =
    "M7.5 11C7.5 7 9.4 5.2 12 5.2S16.5 7 16.5 11C16.5 15.4 14.6 19 12 19.6 9.4 19 7.5 15.4 7.5 11Z";
const NECK: &str = "M9.8 19V24M14.2 19V24";
const BROW_AND_NOSE: &str = "M9 10.8H11.3V14.6H12.7M13.1 10.8H15";
const MOUTH: &str = "M10.8 16.7H13.2";
const CHEEK_HATCH: &str = "M14.3 14L16 12.5M14.2 15.6L16.1 13.9M13.9 17.3L15.7 15.7";
const GROK_ORBIT: &str = "M19.41 8.01L19.02 8.53L18.15 8.90L16.87 9.09L15.25 9.10L13.42 8.91L11.48 8.54L9.58 8.02L7.85 7.39L6.40 6.68L5.33 5.94L4.71 5.23L4.59 4.59L4.98 4.07L5.85 3.70L7.13 3.51L8.75 3.50L10.58 3.69L12.52 4.06L14.42 4.58L16.15 5.21L17.60 5.92L18.67 6.66L19.29 7.37L19.41 8.01Z";
const GROK_SLASH: &str = "M5.57 10.00L4.83 8.80L18.43 0.40L19.17 1.60Z";

fn sun_rays() -> String {
    let mut path = String::new();
    for angle in [-80, -60, -40, -20, 0, 20, 40, 60, 80] {
        let turn = f64::from(angle - 90) * std::f64::consts::PI / 180.0;
        let at = |distance: f64| {
            format!(
                "{:.2} {:.2}",
                12.0 + turn.cos() * distance,
                10.5 + turn.sin() * distance
            )
        };
        let outer = if angle % 40 == 0 { 10.4 } else { 9.0 };
        path.push('M');
        path.push_str(&at(6.6));
        path.push('L');
        path.push_str(&at(outer));
    }
    path
}

fn headgear(bot: BotName) -> Element {
    match bot {
        BotName::Lead => rsx! {
            path { class: "bot-solid", d: "M7.3 9.2L1.2 3.2 3.6 6.3 1 5.8 3.8 8.3 1.6 8.3 7 11.2Z" }
            path { class: "bot-solid", d: "M16.7 9.2L22.8 3.2 20.4 6.3 23 5.8 20.2 8.3 22.4 8.3 17 11.2Z" }
            path { class: "bot-line", d: "M7.4 9.3Q12 6.6 16.6 9.3" }
        },
        BotName::Plan(ProviderId::Claude) => rsx! {
            path { class: "bot-line", d: "{sun_rays()}" }
        },
        BotName::Plan(ProviderId::Cursor) => rsx! {
            path { class: "bot-solid", d: "M4.6 10.5C4.6 1.4 19.4 1.4 19.4 10.5H17.6C17.6 4.2 6.4 4.2 6.4 10.5Z" }
            path { class: "bot-line bot-line-thin", d: "M7 9.6H17" }
            path { class: "bot-line", d: "M7.5 10V15M16.5 10V15" }
        },
        BotName::Plan(ProviderId::Grok | ProviderId::Local) => rsx! {
            path { class: "bot-line", d: "{GROK_ORBIT}" }
            path { class: "bot-solid", d: "{GROK_SLASH}" }
        },
    }
}

#[component]
pub fn BotAvatar(bot: BotName, state: BotState, size: f64, title: Option<String>) -> Element {
    let resting = state == BotState::Resting;
    let eye_top = if resting { "12.1" } else { "11.7" };
    let eye_height = if resting { "0.4" } else { "1" };
    let label = title.unwrap_or_default();
    let labeled = !label.is_empty();
    let mut class = format!("bot-avatar bot-{}", bot.class());
    if state == BotState::Working {
        class.push_str(" is-working");
    }
    if resting {
        class.push_str(" is-resting");
    }
    rsx! {
        svg {
            class: "{class}",
            width: "{size}",
            height: "{size}",
            view_box: "0 0 24 24",
            role: if labeled { "img" } else { "presentation" },
            "aria-label": if labeled { label } else { String::new() },
            "aria-hidden": if labeled { "false" } else { "true" },
            rect { class: "bot-body", x: "0", y: "0", width: "24", height: "24" }
            path { class: "bot-line", d: "{HEAD}" }
            path { class: "bot-line", d: "{NECK}" }
            path { class: "bot-line", d: "{BROW_AND_NOSE}" }
            rect { class: "bot-eye", x: "9.5", y: "{eye_top}", width: "1.4", height: "{eye_height}" }
            rect { class: "bot-eye", x: "13.3", y: "{eye_top}", width: "1.4", height: "{eye_height}" }
            path { class: "bot-line bot-line-thin", d: "{MOUTH}" }
            path { class: "bot-line bot-line-thin", d: "{CHEEK_HATCH}" }
            {headgear(bot)}
            if state == BotState::Error {
                rect { class: "bot-error", x: "19", y: "0", width: "5", height: "5" }
            }
        }
    }
}

/// One row of [`FieldSelect`], the Legate dropdown that stands in for `<select>`.
#[derive(Clone, PartialEq)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
    pub disabled: bool,
    pub title: String,
}

impl SelectOption {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            disabled: false,
            title: String::new(),
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn titled(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }
}

/// Electron prefixes main-process errors; show only the message itself.
pub fn error_text(err: &str) -> String {
    let Some(rest) = err.strip_prefix("Error invoking remote method '") else {
        if err.is_empty() {
            return "Request failed".into();
        }
        return err.to_string();
    };
    let Some(split) = rest.find("': ") else {
        return err.to_string();
    };
    let mut message = &rest[split + 3..];
    if let Some(stripped) = message.strip_prefix("Error: ") {
        message = stripped;
    }
    message.to_string()
}

fn select_uid() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn next_enabled(options: &[SelectOption], from: i64, step: i64) -> i64 {
    let mut index = from + step;
    while index >= 0 && (index as usize) < options.len() {
        if !options[index as usize].disabled {
            return index;
        }
        index += step;
    }
    from
}

fn edge_enabled(options: &[SelectOption], last: bool) -> i64 {
    if last {
        let found = next_enabled(options, options.len() as i64, -1);
        if found == options.len() as i64 {
            -1
        } else {
            found
        }
    } else {
        let found = next_enabled(options, -1, 1);
        if found == -1 { -1 } else { found }
    }
}

fn match_typed(options: &[SelectOption], from: i64, typed: &str) -> i64 {
    let wanted = typed.to_lowercase();
    if wanted.is_empty() || options.is_empty() {
        return -1;
    }
    let len = options.len() as i64;
    for step in 1..=len {
        let index = (from + step).rem_euclid(len);
        let option = &options[index as usize];
        if !option.disabled && option.label.to_lowercase().starts_with(&wanted) {
            return index;
        }
    }
    -1
}

struct Placement {
    top: f64,
    left: f64,
    width: f64,
    max_height: f64,
}

fn place_list(rect: &web_sys::DomRect, rows: usize) -> Placement {
    let (view_w, view_h) = web_sys::window()
        .map(|window| {
            (
                window
                    .inner_width()
                    .ok()
                    .and_then(|value| value.as_f64())
                    .unwrap_or(800.0),
                window
                    .inner_height()
                    .ok()
                    .and_then(|value| value.as_f64())
                    .unwrap_or(600.0),
            )
        })
        .unwrap_or((800.0, 600.0));
    let gap = 4.0;
    let edge = 8.0;
    let list_max: f64 = 320.0;
    let wanted = list_max.min((rows as f64) * 28.0 + 8.0);
    let width = rect
        .width()
        .max(120.0)
        .min((view_w - 2.0 * edge).max(120.0));
    let left = rect.left().clamp(edge, (view_w - edge - width).max(edge));
    let below = view_h - rect.bottom() - gap - edge;
    let above = rect.top() - gap - edge;
    let go_up = wanted > below && above > below;
    let max_height = list_max.min(if go_up { above } else { below }).max(0.0);
    let height = wanted.min(max_height);
    let top = if go_up {
        rect.top() - gap - height
    } else {
        rect.bottom() + gap
    };
    Placement {
        top,
        left,
        width,
        max_height,
    }
}

fn button_rect(id: &str) -> Option<web_sys::DomRect> {
    let element = web_sys::window()?.document()?.get_element_by_id(id)?;
    Some(element.get_bounding_client_rect())
}

fn open_list(
    mut open: Signal<bool>,
    mut active: Signal<i64>,
    mut place: Signal<Option<(f64, f64, f64, f64)>>,
    options: &[SelectOption],
    selected: i64,
    button_id: &str,
) {
    if options.is_empty() {
        return;
    }
    let at = if selected >= 0
        && options
            .get(selected as usize)
            .is_some_and(|option| !option.disabled)
    {
        selected
    } else {
        edge_enabled(options, false)
    };
    active.set(at);
    if let Some(rect) = button_rect(button_id) {
        let placed = place_list(&rect, options.len());
        place.set(Some((
            placed.top,
            placed.left,
            placed.width,
            placed.max_height,
        )));
    }
    open.set(true);
}

fn remember_type(typed: &std::rc::Rc<std::cell::RefCell<(String, i64)>>, key: &str) -> String {
    let now = crate::state::now_ms();
    let mut slot = typed.borrow_mut();
    if now - slot.1 >= 500 {
        slot.0.clear();
    }
    slot.0.push_str(key);
    slot.1 = now;
    slot.0.clone()
}

/// Legate's dropdown. The closed control is a button; the open list uses `.select-list`.
#[component]
pub fn FieldSelect(
    value: String,
    options: Vec<SelectOption>,
    on_change: Callback<String>,
    #[props(default)] id: String,
    #[props(default)] class: String,
    #[props(default)] disabled: bool,
    #[props(default)] title: String,
    #[props(default)] aria_label: String,
    #[props(default)] aria_invalid: bool,
    #[props(default)] aria_describedby: String,
) -> Element {
    let uid = use_hook(select_uid);
    let mut open = use_signal(|| false);
    let mut active = use_signal(|| -1i64);
    let place = use_signal(|| None::<(f64, f64, f64, f64)>);
    let typed = use_hook(|| std::rc::Rc::new(std::cell::RefCell::new((String::new(), 0i64))));
    let button_id = if id.is_empty() {
        format!("legate-select-{uid}")
    } else {
        id
    };
    let list_id = format!("{button_id}-list");
    let selected = options
        .iter()
        .position(|option| option.value == value)
        .map_or(-1, |index| index as i64);
    let current_label = options
        .iter()
        .find(|option| option.value == value)
        .map(|option| option.label.clone())
        .unwrap_or_default();
    let button_class = if class.is_empty() {
        "select".to_string()
    } else {
        format!("select {class}")
    };
    let listed = options.clone();
    let controls = if open() {
        list_id.clone()
    } else {
        String::new()
    };
    let list_style = place().map_or_else(
        || "top: 0; left: 0; visibility: hidden;".to_string(),
        |(top, left, width, max_height)| {
            format!(
                "top: {top}px; left: {left}px; min-width: {width}px; max-height: {max_height}px;"
            )
        },
    );

    rsx! {
        button {
            id: "{button_id}",
            r#type: "button",
            class: "{button_class}",
            title: "{title}",
            disabled: disabled,
            aria_haspopup: "listbox",
            aria_expanded: if open() { "true" } else { "false" },
            aria_controls: "{controls}",
            aria_label: "{aria_label}",
            aria_invalid: if aria_invalid { "true" } else { "false" },
            aria_describedby: "{aria_describedby}",
            onclick: {
                let options = options.clone();
                let button_id = button_id.clone();
                move |_| {
                    if disabled || options.is_empty() {
                        return;
                    }
                    if open() {
                        open.set(false);
                        return;
                    }
                    open_list(open, active, place, &options, selected, &button_id);
                }
            },
            onkeydown: {
                let options = options.clone();
                let button_id = button_id.clone();
                let value = value.clone();
                let on_change = on_change;
                let typed = typed.clone();
                move |event| {
                    let mods = event.modifiers();
                    if mods.contains(Modifiers::ALT) || mods.contains(Modifiers::CONTROL) || mods.contains(Modifiers::META) {
                        return;
                    }
                    let key = event.key();
                    let name = key.to_string();
                    if matches!(key, Key::ArrowDown | Key::ArrowUp | Key::Enter) || name == " " {
                        event.prevent_default();
                        if !disabled && !options.is_empty() && !open() {
                            open_list(open, active, place, &options, selected, &button_id);
                        }
                        return;
                    }
                    if let Key::Character(text) = key
                        && text.chars().count() == 1
                        && !open()
                    {
                            let buffer = remember_type(&typed, &text);
                            let start = if buffer.chars().count() > 1 { selected - 1 } else { selected };
                            let found = match_typed(&options, start, &buffer);
                            if found >= 0 {
                                let next = options[found as usize].value.clone();
                                if next != value {
                                    on_change.call(next);
                                }
                            }
                    }
                }
            },
            span { class: "select-value", "{current_label}" }
            svg {
                class: "select-chevron",
                width: "10",
                height: "6",
                view_box: "0 0 10 6",
                "aria-hidden": "true",
                "focusable": "false",
                path {
                    d: "M1 1l4 4 4-4",
                    fill: "none",
                    stroke: "currentColor",
                    stroke_width: "1.5",
                    stroke_linecap: "square",
                }
            }
        }
        if open() {
            div {
                style: "position: fixed; inset: 0; z-index: 50;",
                onclick: move |_| open.set(false),
            }
            ul {
                id: "{list_id}",
                class: "select-list",
                role: "listbox",
                tabindex: "0",
                aria_label: "{aria_label}",
                style: "{list_style}",
                onkeydown: {
                    let options = options.clone();
                    let value = value.clone();
                    let on_change = on_change;
                    let typed = typed.clone();
                    move |event| {
                        let key = event.key();
                        let name = key.to_string();
                        let step = if key == Key::ArrowDown {
                            Some(next_enabled(&options, active(), 1))
                        } else if key == Key::ArrowUp {
                            Some(next_enabled(&options, active(), -1))
                        } else if key == Key::Home {
                            Some(edge_enabled(&options, false))
                        } else if key == Key::End {
                            Some(edge_enabled(&options, true))
                        } else {
                            None
                        };
                        if let Some(index) = step {
                            event.prevent_default();
                            if index >= 0 {
                                active.set(index);
                            }
                        } else if key == Key::Enter || name == " " {
                            event.prevent_default();
                            let index = active();
                            if let Some(option) = usize::try_from(index)
                                .ok()
                                .and_then(|index| options.get(index))
                                && !option.disabled
                            {
                                let next = option.value.clone();
                                open.set(false);
                                if next != value {
                                    on_change.call(next);
                                }
                            }
                        } else if key == Key::Escape {
                            event.prevent_default();
                            event.stop_propagation();
                            open.set(false);
                        } else if key == Key::Tab {
                            open.set(false);
                        } else if let Key::Character(text) = key {
                            let mods = event.modifiers();
                            if text.chars().count() == 1
                                && !mods.intersects(Modifiers::CONTROL | Modifiers::META | Modifiers::ALT)
                            {
                                let buffer = remember_type(&typed, &text);
                                let found = match_typed(&options, active(), &buffer);
                                if found >= 0 {
                                    active.set(found);
                                }
                            }
                        }
                    }
                },
                for (index, option) in listed.into_iter().enumerate() {
                    {
                        let option_value = option.value.clone();
                        let option_label = option.label;
                        let option_title = option.title;
                        let option_disabled = option.disabled;
                        let picked = index as i64 == selected;
                        let mut classes = "select-option".to_string();
                        if index as i64 == active() {
                            classes.push_str(" is-active");
                        }
                        if picked {
                            classes.push_str(" is-selected");
                        }
                        let pick_value = option_value.clone();
                        let value_now = value.clone();
                        let on_change = on_change;
                        rsx! {
                            li {
                                key: "{option_value}-{index}",
                                class: "{classes}",
                                role: "option",
                                aria_selected: if picked { "true" } else { "false" },
                                aria_disabled: if option_disabled { "true" } else { "false" },
                                title: "{option_title}",
                                onmousemove: move |_| {
                                    if !option_disabled && active() != index as i64 {
                                        active.set(index as i64);
                                    }
                                },
                                onmousedown: move |event| event.prevent_default(),
                                onclick: move |_| {
                                    if option_disabled {
                                        return;
                                    }
                                    open.set(false);
                                    if pick_value != value_now {
                                        on_change.call(pick_value.clone());
                                    }
                                },
                                span { class: "select-mark",
                                    if picked {
                                        svg {
                                            class: "select-check",
                                            width: "10",
                                            height: "8",
                                            view_box: "0 0 10 8",
                                            "aria-hidden": "true",
                                            "focusable": "false",
                                            path {
                                                d: "M1 4l3 3 5-6",
                                                fill: "none",
                                                stroke: "currentColor",
                                                stroke_width: "1.5",
                                            }
                                        }
                                    }
                                }
                                span { class: "select-label", "{option_label}" }
                            }
                        }
                    }
                }
            }
        }
    }
}
