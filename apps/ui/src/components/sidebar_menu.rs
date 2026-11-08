use dioxus::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

#[derive(Clone, PartialEq)]
pub struct MenuEntry {
    pub label: String,
    pub on_select: Option<Callback>,
    pub disabled: bool,
    pub danger: bool,
    pub checked: bool,
    /// `true` when the row is a ticked option. A plain action leaves this false.
    pub shows_check: bool,
    pub heading: bool,
    pub separator: bool,
    pub title: String,
}

impl MenuEntry {
    pub fn item(label: impl Into<String>, on_select: Callback) -> Self {
        Self {
            label: label.into(),
            on_select: Some(on_select),
            disabled: false,
            danger: false,
            checked: false,
            shows_check: false,
            heading: false,
            separator: false,
            title: String::new(),
        }
    }

    pub fn heading(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            on_select: None,
            disabled: true,
            danger: false,
            checked: false,
            shows_check: false,
            heading: true,
            separator: false,
            title: String::new(),
        }
    }

    pub fn check(label: impl Into<String>, on: bool, on_select: Callback) -> Self {
        Self {
            label: label.into(),
            on_select: Some(on_select),
            disabled: false,
            danger: false,
            checked: on,
            shows_check: true,
            heading: false,
            separator: false,
            title: String::new(),
        }
    }
}

#[component]
pub fn SidebarMenu(
    label: String,
    at: (f64, f64),
    entries: Vec<MenuEntry>,
    on_close: Callback,
) -> Element {
    let (x, y) = at;
    let id = use_hook(|| format!("side-menu-{}", js_sys::Math::random().to_bits()));
    let menu_id = id.clone();
    use_hook({
        let id = id.clone();
        move || std::rc::Rc::new(install_close(id, on_close))
    });
    use_effect({
        let id = id.clone();
        move || {
            place_menu(&id, x, y);
        }
    });

    rsx! {
        ul {
            id: "{menu_id}",
            class: "side-menu",
            role: "menu",
            aria_label: "{label}",
            style: "left: {x}px; top: {y}px;",
            oncontextmenu: move |event| event.prevent_default(),
            onkeydown: move |event| on_menu_key(&event, on_close),
            for (index, entry) in entries.into_iter().enumerate() {
                {
                    let sep = if entry.separator { " has-separator" } else { "" };
                    let on_select = entry.on_select;
                    let label = entry.label.clone();
                    rsx! {
                        if entry.heading {
                            li {
                                key: "{index}",
                                role: "presentation",
                                class: "side-menu-heading{sep}",
                                "{label}"
                            }
                        } else {
                            li {
                                key: "{index}",
                                role: "none",
                                class: if entry.separator { "has-separator" } else { "" },
                                button {
                                    r#type: "button",
                                    role: if entry.shows_check { "menuitemcheckbox" } else { "menuitem" },
                                    aria_checked: if entry.shows_check { if entry.checked { "true" } else { "false" } } else { "" },
                                    class: if entry.danger { "side-menu-item is-danger" } else { "side-menu-item" },
                                    disabled: entry.disabled,
                                    title: if entry.title.is_empty() { "" } else { "{entry.title}" },
                                    onclick: move |_| {
                                        on_close.call(());
                                        if let Some(select) = on_select {
                                            select.call(());
                                        }
                                    },
                                    if entry.shows_check {
                                        span { class: "side-menu-check", aria_hidden: "true",
                                            if entry.checked { "■" }
                                        }
                                    }
                                    "{label}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

struct MenuListen {
    closure: Closure<dyn FnMut(web_sys::Event)>,
    blur: Closure<dyn FnMut(web_sys::Event)>,
}

fn install_close(id: String, on_close: Callback) -> MenuListen {
    let menu_id = id.clone();
    let closure = Closure::wrap(Box::new(move |event: web_sys::Event| {
        let Some(target) = event.target() else {
            return;
        };
        let Ok(node) = target.dyn_into::<web_sys::Node>() else {
            return;
        };
        if let Some(menu) = document().and_then(|doc| doc.get_element_by_id(&menu_id))
            && menu.contains(Some(&node))
        {
            return;
        }
        on_close.call(());
    }) as Box<dyn FnMut(web_sys::Event)>);
    let blur = Closure::wrap(Box::new(move |_event: web_sys::Event| {
        on_close.call(());
    }) as Box<dyn FnMut(web_sys::Event)>);
    if let Some(doc) = document() {
        let _ = doc.add_event_listener_with_callback("mousedown", closure.as_ref().unchecked_ref());
    }
    if let Some(window) = web_sys::window() {
        let _ = window.add_event_listener_with_callback("blur", blur.as_ref().unchecked_ref());
    }
    MenuListen { closure, blur }
}

impl Drop for MenuListen {
    fn drop(&mut self) {
        if let Some(doc) = document() {
            let _ = doc.remove_event_listener_with_callback(
                "mousedown",
                self.closure.as_ref().unchecked_ref(),
            );
        }
        if let Some(window) = web_sys::window() {
            let _ = window
                .remove_event_listener_with_callback("blur", self.blur.as_ref().unchecked_ref());
        }
    }
}

fn place_menu(id: &str, x: f64, y: f64) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(menu) = document().and_then(|doc| doc.get_element_by_id(id)) else {
        return;
    };
    let rect = menu.get_bounding_client_rect();
    let width = window
        .inner_width()
        .ok()
        .and_then(|value| value.as_f64())
        .unwrap_or(800.0);
    let height = window
        .inner_height()
        .ok()
        .and_then(|value| value.as_f64())
        .unwrap_or(600.0);
    let left = x.max(4.0).min((width - rect.width() - 4.0).max(4.0));
    let top = y.max(4.0).min((height - rect.height() - 4.0).max(4.0));
    let _ = menu.set_attribute("style", &format!("left: {left}px; top: {top}px;"));
    if let Ok(Some(button)) = menu.query_selector("button:not(:disabled)")
        && let Some(element) = button.dyn_ref::<web_sys::HtmlElement>()
    {
        let _ = element.focus();
    }
}

fn on_menu_key(event: &Event<KeyboardData>, on_close: Callback) {
    let key = event.key().to_string();
    if key == "Escape" || key == "Tab" {
        event.prevent_default();
        event.stop_propagation();
        on_close.call(());
        return;
    }
    if key != "ArrowDown" && key != "ArrowUp" {
        return;
    }
    event.prevent_default();
    let Some(doc) = document() else {
        return;
    };
    let Some(active) = doc.active_element() else {
        return;
    };
    let Some(menu) = active.closest(".side-menu").ok().flatten() else {
        return;
    };
    let Ok(list) = menu.query_selector_all("button:not(:disabled)") else {
        return;
    };
    let length = list.length();
    if length == 0 {
        return;
    }
    let active = document().and_then(|doc| doc.active_element());
    let mut index = -1i32;
    for slot in 0..length {
        if list.get(slot).is_some_and(|node| {
            active.as_ref().is_some_and(|element| {
                let current: &web_sys::Node = element.unchecked_ref();
                node.is_same_node(Some(current))
            })
        }) {
            index = slot as i32;
            break;
        }
    }
    let step: i32 = if key == "ArrowDown" { 1 } else { -1 };
    let next = (index + step).rem_euclid(length as i32) as u32;
    if let Some(button) = list.get(next)
        && let Some(element) = button.dyn_ref::<web_sys::HtmlElement>()
    {
        let _ = element.focus();
    }
}

fn document() -> Option<web_sys::Document> {
    web_sys::window()?.document()
}
