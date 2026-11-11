//! Full-screen review of one worktree. Port of `ReviewPane.tsx`.

use std::cell::Cell;
use std::rc::Rc;

use dioxus::prelude::*;
use legate_core::{ChangeSet, ChangedFile};
use wasm_bindgen_futures::spawn_local;

use super::files_panel::{DiffBlock, bridge_error};
use crate::api;
use crate::logic::worktree_label::{NamedChange, worktree_label};

#[derive(Clone, PartialEq, Eq)]
struct Payload {
    id: String,
    text: Option<String>,
    error: Option<String>,
}

fn label_of(change: &ChangeSet) -> String {
    worktree_label(&NamedChange {
        id: change.id.clone(),
        name: change.name.clone(),
    })
}

#[component]
pub fn ReviewPane(change: ChangeSet, on_close: Callback, on_done: Callback) -> Element {
    let mut payload = use_signal(|| None::<Payload>);
    let action_error = use_signal(|| None::<String>);
    let merge_message = use_signal(|| None::<String>);
    let busy = use_signal(|| false);
    let mut confirming = use_signal(|| false);
    let jump_nonce = use_signal(|| 0_u32);
    let jump_path = use_signal(String::new);
    let busy_flag = use_hook(|| Rc::new(Cell::new(false)));
    let mut seen_id = use_signal(|| change.id.clone());
    if seen_id.peek().as_str() != change.id {
        seen_id.set(change.id.clone());
    }
    let ticket = use_hook(|| Rc::new(Cell::new(0_u64)));
    use_effect(move || {
        let id = seen_id();
        let n = ticket.get().saturating_add(1);
        ticket.set(n);
        let ticket = ticket.clone();
        spawn_local(async move {
            let next = match api::change_diff(&id).await {
                Ok(text) => Payload {
                    id: id.clone(),
                    text: Some(text),
                    error: None,
                },
                Err(err) => Payload {
                    id,
                    text: None,
                    error: Some(bridge_error(&err)),
                },
            };
            if ticket.get() == n {
                payload.set(Some(next));
            }
        });
    });

    let shown = payload
        .read()
        .as_ref()
        .filter(|item| item.id == change.id)
        .cloned();
    let diff_text = shown.as_ref().and_then(|item| item.text.clone());
    let diff_error = shown.as_ref().and_then(|item| item.error.clone());
    let loaded = shown.is_some();
    let merge_id = change.id.clone();
    let discard_id = change.id.clone();
    let jump_files = change.files.clone();
    let merge_flag = busy_flag.clone();
    let discard_flag = busy_flag.clone();

    rsx! {
        div { class: "review-pane", role: "region", aria_label: "Review",
            header { class: "review-head",
                div { class: "review-summary",
                    div { class: "review-title",
                        span { class: "change-branch", "{label_of(&change)}" }
                        span { class: "review-branch", "{change.branch}" }
                        span { class: "change-stats",
                            span { class: "stat-add", "+{change.insertions}" }
                            " "
                            span { class: "stat-del", "\u{2212}{change.deletions}" }
                        }
                    }
                    ul { class: "change-files",
                        for file in jump_files {
                            ReviewFile { key: "{file.path}", file: file, jump_path: jump_path, jump_nonce: jump_nonce }
                        }
                    }
                }
                div { class: "review-actions",
                    button {
                        r#type: "button",
                        class: "btn btn-primary",
                        disabled: busy(),
                        onclick: move |_| merge_change(merge_id.clone(), merge_flag.clone(), busy, action_error, merge_message, on_done),
                        "Merge"
                    }
                    if confirming() {
                        div { class: "review-confirm",
                            span { class: "review-confirm-copy", "Discard this worktree and its changes?" }
                            button {
                                r#type: "button",
                                class: "btn btn-quiet is-danger-label",
                                disabled: busy(),
                                onclick: move |_| discard_change(discard_id.clone(), discard_flag.clone(), busy, action_error, on_done),
                                "Yes"
                            }
                            button {
                                r#type: "button",
                                class: "btn btn-quiet",
                                disabled: busy(),
                                onclick: move |_| confirming.set(false),
                                "No"
                            }
                        }
                    } else {
                        button {
                            r#type: "button",
                            class: "btn btn-quiet",
                            disabled: busy(),
                            onclick: move |_| confirming.set(true),
                            "Discard"
                        }
                    }
                    button {
                        r#type: "button",
                        class: "btn btn-quiet",
                        disabled: busy(),
                        onclick: move |_| on_close.call(()),
                        "Back to terminals"
                    }
                }
            }
            if let Some(message) = merge_message.read().as_ref() {
                p { class: "change-ok", role: "status", "{message}" }
            }
            if let Some(message) = action_error.read().as_ref() {
                p { class: "error-text", role: "alert", "{message}" }
            }
            div { class: "review-diff", aria_busy: "{!loaded}",
                if !loaded {
                    p { class: "diff-status", "Loading diff\u{2026}" }
                }
                if let Some(message) = diff_error.clone() {
                    p { class: "error-text", role: "alert", "{message}" }
                }
                if loaded && diff_error.is_none() {
                    DiffBlock { text: diff_text.clone(), jump_nonce: jump_nonce(), jump_path: jump_path() }
                }
            }
        }
    }
}

#[component]
fn ReviewFile(
    file: ChangedFile,
    mut jump_path: Signal<String>,
    mut jump_nonce: Signal<u32>,
) -> Element {
    let path = file.path.clone();
    let click_path = path.clone();
    rsx! {
        li { class: "change-file",
            button {
                r#type: "button",
                class: "review-file",
                onclick: move |_| {
                    jump_path.set(click_path.clone());
                    jump_nonce.set(jump_nonce() + 1);
                },
                span { class: "change-path", "{path}" }
                FileStat { file: file }
            }
        }
    }
}

#[component]
fn FileStat(file: ChangedFile) -> Element {
    if file.insertions.is_none() || file.deletions.is_none() {
        rsx! { span { class: "diff-binary", "binary" } }
    } else {
        let insertions = file.insertions.unwrap_or(0);
        let deletions = file.deletions.unwrap_or(0);
        rsx! {
            span { class: "change-stats",
                span { class: "stat-add", "+{insertions}" }
                " "
                span { class: "stat-del", "\u{2212}{deletions}" }
            }
        }
    }
}

fn merge_change(
    id: String,
    busy_flag: Rc<Cell<bool>>,
    mut busy: Signal<bool>,
    mut action_error: Signal<Option<String>>,
    mut merge_message: Signal<Option<String>>,
    on_done: Callback,
) {
    if busy_flag.get() {
        return;
    }
    busy_flag.set(true);
    busy.set(true);
    action_error.set(None);
    merge_message.set(None);
    spawn_local(async move {
        match api::merge_change(&id).await {
            Ok(result) => {
                let message = if result.message.is_empty() {
                    "Could not merge this worktree. Try again.".to_string()
                } else {
                    result.message
                };
                if result.ok {
                    merge_message.set(Some(message));
                    on_done.call(());
                } else {
                    action_error.set(Some(message));
                }
            }
            Err(err) => action_error.set(Some(bridge_error(&err))),
        }
        busy_flag.set(false);
        busy.set(false);
    });
}

fn discard_change(
    id: String,
    busy_flag: Rc<Cell<bool>>,
    mut busy: Signal<bool>,
    mut action_error: Signal<Option<String>>,
    on_done: Callback,
) {
    if busy_flag.get() {
        return;
    }
    busy_flag.set(true);
    busy.set(true);
    action_error.set(None);
    spawn_local(async move {
        match api::discard_change(&id).await {
            Ok(()) => on_done.call(()),
            Err(err) => action_error.set(Some(bridge_error(&err))),
        }
        busy_flag.set(false);
        busy.set(false);
    });
}
