use dioxus::prelude::*;

#[derive(Clone, PartialEq)]
pub struct MenuEntry {
    pub label: String,
    pub on_select: Option<Callback>,
    pub disabled: bool,
    pub danger: bool,
    pub checked: bool,
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
            heading: true,
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
    rsx! {
        ul {
            class: "sidebar-menu",
            role: "menu",
            aria_label: "{label}",
            style: "position: fixed; left: {x}px; top: {y}px; z-index: 40;",
            onkeydown: move |event| {
                if event.key().to_string() == "Escape" {
                    on_close.call(());
                }
            },
            for entry in entries {
                li { class: if entry.separator { "menu-separator" } else { "" },
                    if entry.heading {
                        span { class: "menu-heading", "{entry.label}" }
                    } else {
                        button {
                            class: if entry.danger { "menu-item is-danger" } else { "menu-item" },
                            role: "menuitem",
                            disabled: entry.disabled,
                            title: "{entry.title}",
                            onclick: move |_| {
                                if let Some(select) = entry.on_select {
                                    select.call(());
                                }
                                on_close.call(());
                            },
                            if entry.checked { "✓ " }
                            "{entry.label}"
                        }
                    }
                }
            }
        }
        div {
            class: "sidebar-menu-backdrop",
            style: "position: fixed; inset: 0; z-index: 30;",
            onclick: move |_| on_close.call(()),
        }
    }
}
