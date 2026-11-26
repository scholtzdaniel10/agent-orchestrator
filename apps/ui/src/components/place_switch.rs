//! The header's "where am I working" switch: the project folder or one of its worktrees, by
//! name. Git's branch names (`orch/3fa9c1d2` for every worktree) are shown small in the list,
//! not as the label.

use dioxus::prelude::*;
use legate_core::ChangeSet;

use crate::logic::worktree_label::{self, NamedChange};

fn named(change: &ChangeSet) -> NamedChange {
    NamedChange {
        id: change.id.clone(),
        name: change.name.clone(),
    }
}

#[component]
pub fn PlaceSwitch(
    branch: Option<String>,
    changes: Vec<ChangeSet>,
    primary: Option<String>,
    on_pick: Callback<Option<String>>,
) -> Element {
    let mut open = use_signal(|| false);
    let names: Vec<NamedChange> = changes.iter().map(named).collect();
    let folder_branch = branch.clone().unwrap_or_else(|| "project folder".into());
    let current = primary
        .as_ref()
        .and_then(|id| changes.iter().find(|change| &change.id == id));
    let (label, kind) = match current {
        Some(change) => (worktree_label::worktree_label(&named(change)), "worktree"),
        None => (
            worktree_label::branch_place_label(&folder_branch, &names),
            "branch",
        ),
    };
    let is_open = open();
    rsx! {
        div { class: "place-switch",
            button {
                class: if is_open { "place-switch-button is-open" } else { "place-switch-button" },
                r#type: "button",
                aria_haspopup: "menu",
                aria_expanded: "{is_open}",
                title: match current {
                    Some(change) => format!("Worktree {label}\nBranch {}", change.branch),
                    None => format!("Project folder\nBranch {folder_branch}"),
                },
                onclick: move |_| open.set(!is_open),
                span { class: "place-switch-kind", "{kind}" }
                span { class: "place-switch-label", "{label}" }
                if !changes.is_empty() {
                    span { class: "place-switch-count", "{changes.len()} worktrees" }
                }
                span { class: "place-switch-caret", aria_hidden: "true", "▾" }
            }
            if is_open {
                div { class: "place-switch-backdrop", onclick: move |_| open.set(false) }
                div { class: "place-switch-menu", role: "menu", aria_label: "Work in",
                    p { class: "place-switch-heading", "Work in" }
                    button {
                        class: if current.is_none() { "place-switch-item is-current" } else { "place-switch-item" },
                        r#type: "button",
                        role: "menuitemradio",
                        aria_checked: "{current.is_none()}",
                        onclick: move |_| {
                            open.set(false);
                            on_pick.call(None);
                        },
                        span { class: "place-switch-item-name", "Project folder" }
                        span { class: "place-switch-item-branch", "{folder_branch}" }
                    }
                    if !changes.is_empty() {
                        p { class: "place-switch-heading", "Worktrees" }
                    }
                    for change in changes.iter().cloned() {
                        {
                            let here = primary.as_deref() == Some(change.id.as_str());
                            let name = worktree_label::worktree_label(&named(&change));
                            let id = change.id.clone();
                            rsx! {
                                button {
                                    key: "{change.id}",
                                    class: if here { "place-switch-item is-current" } else { "place-switch-item" },
                                    r#type: "button",
                                    role: "menuitemradio",
                                    aria_checked: "{here}",
                                    onclick: move |_| {
                                        open.set(false);
                                        on_pick.call(Some(id.clone()));
                                    },
                                    span { class: "place-switch-item-name", "{name}" }
                                    span { class: "place-switch-item-branch", "{change.branch}" }
                                    if !change.files.is_empty() {
                                        span { class: "place-switch-item-stat",
                                            span { class: "stat-add", "+{change.insertions}" }
                                            " "
                                            span { class: "stat-del", "−{change.deletions}" }
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
}
