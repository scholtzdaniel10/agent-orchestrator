use dioxus::html::Key;
use dioxus::prelude::*;
use gloo_timers::future::TimeoutFuture;
use legate_core::SearchHit;
use wasm_bindgen::JsCast;

use crate::api;

const INPUT: &str = "history-search-input";
const CLOSE: &str = "history-search-close";

#[component]
pub fn SearchDialog(
    initial_query: String,
    on_close: Callback,
    on_open: Callback<SearchHit>,
) -> Element {
    let mut query = use_signal(|| initial_query);
    let mut selected = use_signal(|| 0usize);
    let previous_focus = use_hook(|| document()?.active_element());
    use_drop(move || {
        if let Some(node) =
            previous_focus.and_then(|node| node.dyn_into::<web_sys::HtmlElement>().ok())
        {
            let _ = node.focus();
        }
    });
    let search = use_resource(move || {
        let text = query();
        async move {
            let result = if text.trim().is_empty() {
                Ok(Vec::new())
            } else {
                TimeoutFuture::new(150).await;
                api::search_history(&text, None).await
            };
            (text, result)
        }
    });

    let text = query();
    let current = search.read();
    let result = current
        .as_ref()
        .filter(|(query, _)| query == &text)
        .map(|(_, result)| result);
    let hits = result
        .and_then(|result| result.as_ref().ok())
        .cloned()
        .unwrap_or_default();
    let index = selected();
    let message = if text.trim().is_empty() {
        Some("Search saved chats and jobs in this project.")
    } else {
        match result {
            None => Some("Searching…"),
            Some(Err(_)) => Some("Could not search saved history."),
            Some(Ok(_)) if hits.is_empty() => Some("No matches."),
            Some(Ok(_)) => None,
        }
    };
    let has_hits = !hits.is_empty();
    let active_id = has_hits.then(|| format!("history-search-hit-{index}"));
    let key_hits = hits.clone();

    rsx! {
        div { class: "new-task-overlay history-search-overlay",
            role: "dialog", aria_modal: "true", aria_label: "Search saved history",
            onmousedown: move |_| on_close.call(()),
            onkeydown: move |event| {
                match event.key() {
                    Key::Escape => on_close.call(()),
                    Key::Tab => {
                        let on_input = document().and_then(|doc| doc.active_element())
                            .is_some_and(|element| element.id() == INPUT);
                        focus(if on_input { CLOSE } else { INPUT });
                    }
                    Key::ArrowUp | Key::ArrowDown if !key_hits.is_empty() => {
                        let next = if event.key() == Key::ArrowUp {
                            (selected() + key_hits.len() - 1) % key_hits.len()
                        } else {
                            (selected() + 1) % key_hits.len()
                        };
                        selected.set(next);
                        focus(INPUT);
                        reveal_selection(next);
                    }
                    Key::Enter => {
                        let on_close_button = document().and_then(|doc| doc.active_element())
                            .is_some_and(|element| element.id() == CLOSE);
                        if on_close_button {
                            on_close.call(());
                        } else if let Some(hit) = key_hits.get(selected()) {
                            on_open.call(hit.clone());
                        }
                    }
                    _ => return,
                }
                event.prevent_default();
                event.stop_propagation();
            },
            div { class: "new-task history-search",
                onmousedown: move |event| event.stop_propagation(),
                div { class: "new-task-field history-search-field",
                    input {
                        id: INPUT, r#type: "search", value: "{text}", placeholder: "Search saved history",
                        role: "combobox", aria_label: "Search saved chats and jobs", aria_autocomplete: "list",
                        aria_controls: "history-search-results", aria_expanded: has_hits,
                        aria_activedescendant: active_id,
                        onmounted: move |_| focus(INPUT),
                        oninput: move |event| {
                            selected.set(0);
                            query.set(event.value());
                        },
                    }
                    button {
                        id: CLOSE, class: "btn", r#type: "button",
                        onclick: move |_| on_close.call(()),
                        "Close"
                    }
                }
                if let Some(message) = message {
                    p { class: "new-task-kicker", role: "status", "{message}" }
                }
                div { id: "history-search-results", class: "job-list history-search-results",
                    role: "listbox", aria_label: "Search results", hidden: !has_hits,
                    for (i, hit) in hits.into_iter().enumerate() {
                        button {
                            key: "{hit.kind.as_str()}:{hit.id}",
                            id: "history-search-hit-{i}", class: if i == index { "history-search-hit is-selected" } else { "history-search-hit" },
                            r#type: "button", role: "option", tabindex: "-1", aria_selected: i == index,
                            onclick: {
                                let hit = hit.clone();
                                move |_| on_open.call(hit.clone())
                            },
                            span { class: "history-search-kind", if hit.kind == legate_core::SearchKind::Chat { "Chat" } else { "Job" } }
                            span { class: "history-search-title", "{hit.title}" }
                            span { class: "history-search-snippet",
                                for part in hit.snippet {
                                    if part.marked {
                                        mark { "{part.text}" }
                                    } else {
                                        "{part.text}"
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

fn document() -> Option<web_sys::Document> {
    web_sys::window()?.document()
}

fn focus(id: &str) {
    if let Some(node) = document()
        .and_then(|doc| doc.get_element_by_id(id))
        .and_then(|node| node.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = node.focus();
    }
}

fn reveal_selection(index: usize) {
    let Some(row) =
        document().and_then(|doc| doc.get_element_by_id(&format!("history-search-hit-{index}")))
    else {
        return;
    };
    if let Some(list) = row.parent_element() {
        let list = list.get_bounding_client_rect();
        let bounds = row.get_bounding_client_rect();
        if bounds.top() < list.top() {
            row.scroll_into_view_with_bool(true);
        } else if bounds.bottom() > list.bottom() {
            row.scroll_into_view_with_bool(false);
        }
    }
}

/// Called after Lead has rendered the saved messages.
pub(super) fn reveal_message(id: &str) -> bool {
    let Some(row) = document().and_then(|doc| doc.get_element_by_id(&format!("lead-message-{id}")))
    else {
        return false;
    };
    row.scroll_into_view_with_bool(false);
    const FLASH: &str = "history-search-flash";
    const STAMP: &str = "data-search-flash";
    let stamp = crate::state::now_ms().to_string();
    let _ = row.set_attribute(STAMP, &stamp);
    let original = row.class_name();
    if !original.split_whitespace().any(|class| class == FLASH) {
        row.set_class_name(&format!("{original} {FLASH}"));
    }
    wasm_bindgen_futures::spawn_local(async move {
        TimeoutFuture::new(1500).await;
        if row.get_attribute(STAMP).as_deref() == Some(stamp.as_str()) {
            let _ = row.remove_attribute(STAMP);
            let classes = row.class_name();
            let classes = classes
                .split_whitespace()
                .filter(|class| *class != FLASH)
                .collect::<Vec<_>>()
                .join(" ");
            row.set_class_name(&classes);
        }
    });
    true
}
