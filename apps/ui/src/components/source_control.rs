use dioxus::prelude::*;

use super::files_panel::{DiffBlock, bridge_error};
use crate::api;
use crate::logic::file_tree::{status_mark, status_name};
use crate::state::use_store;

enum Action {
    Commit(String),
    Push,
    OpenPr,
}

#[component]
pub fn SourceControlPanel(r#where: Option<String>, label: String, refresh_key: String) -> Element {
    let store = use_store();
    let mut message = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let mut action_error = use_signal(|| None::<String>);
    let mut pr_url = use_signal(|| None::<String>);
    let mut open_path = use_signal(|| None::<String>);
    let mut status = use_resource(use_reactive(
        (&r#where, &refresh_key),
        |(place, _)| async move { api::branch_status(place.as_deref()).await },
    ));
    // An editor save clears `editor_dirty`; re-read the status so the saved file shows up.
    let mut last_dirty = use_signal(|| None::<String>);
    use_effect(move || {
        let now = store.editor_dirty.read().clone();
        if last_dirty.peek().is_some() && now.is_none() {
            status.restart();
        }
        last_dirty.set(now);
    });

    let action_place = r#where.clone();
    let run = use_callback(move |action: Action| {
        if *busy.peek() || *status.state().peek() != UseResourceState::Ready {
            return;
        }
        busy.set(true);
        action_error.set(None);
        let place = action_place.clone();
        spawn(async move {
            let result = match action {
                Action::Commit(text) => api::commit_all(place.as_deref(), &text).await.map(|()| {
                    message.set(String::new());
                    open_path.set(None);
                }),
                Action::Push => api::push_branch(place.as_deref()).await,
                Action::OpenPr => api::open_branch_pr(place.as_deref())
                    .await
                    .map(|url| pr_url.set(Some(url))),
            };
            if let Err(err) = result {
                action_error.set(Some(bridge_error(&err)));
            }
            status.restart();
            busy.set(false);
        });
    });

    let current = status.read();
    let working = busy() || *status.state().read() != UseResourceState::Ready;
    let feedback = rsx! {
        if let Some(err) = action_error.read().as_ref() {
            p { class: "error-text", role: "alert", "{err}" }
        }
        if let Some(url) = pr_url.read().as_ref() {
            p { class: "change-ok", role: "status",
                "Pull request opened "
                a { href: "{url}", target: "_blank", rel: "noreferrer", "{url}" }
            }
        }
    };
    rsx! {
        section { class: "files-panel", aria_label: "{label}", style: "overflow: auto;",
            match current.as_ref() {
                Some(Ok(status)) => {
                    let push_label = if status.upstream.is_some() { "Push" } else { "Publish branch" };
                    let has_changes = !status.uncommitted.is_empty();
                    rsx! {
                        p { class: "files-status", "{status.branch} ↑{status.ahead} ↓{status.behind}" }
                        if let Some(base) = &status.base {
                            p { class: "files-status", "{status.commits} commits vs {base} · +{status.insertions} −{status.deletions}" }
                        }
                        form {
                            onsubmit: move |event| {
                                event.prevent_default();
                                let text = message();
                                if !text.trim().is_empty() && has_changes {
                                    run.call(Action::Commit(text));
                                }
                            },
                            label { class: "change-pr-field",
                                textarea {
                                    aria_label: "Commit message",
                                    placeholder: "Commit message",
                                    disabled: busy(),
                                    value: "{message}",
                                    oninput: move |event| message.set(event.value()),
                                }
                            }
                            button { class: "btn btn-primary", r#type: "submit",
                                disabled: working || message.read().trim().is_empty() || !has_changes,
                                "Commit"
                            }
                        }
                        div { class: "files-tools",
                            button { class: "btn", r#type: "button", disabled: working,
                                onclick: move |_| run.call(Action::Push), "{push_label}"
                            }
                            if pr_url.read().is_none() && status.upstream.is_some() && status.commits > 0 && *store.github_available.read() {
                                button { class: "btn", r#type: "button", disabled: working,
                                    onclick: move |_| run.call(Action::OpenPr), "Open PR"
                                }
                            }
                        }
                        {feedback}
                        h3 { "Changes" }
                        for file in &status.uncommitted {
                            {
                                let path = file.path.clone();
                                let expanded = open_path.read().as_ref() == Some(&path);
                                let mark = status_mark(file.status).unwrap_or_default();
                                let name = status_name(file.status);
                                let pick = path.clone();
                                rsx! {
                                    div { key: "{path}",
                                        button {
                                            class: "btn btn-quiet files-row",
                                            r#type: "button",
                                            aria_expanded: expanded,
                                            onclick: move |_| open_path.set(if expanded { None } else { Some(pick.clone()) }),
                                            span { class: "files-mark is-{name}", title: "{name}", "{mark}" }
                                            span { class: "files-name", title: "{path}", "{path}" }
                                        }
                                        if expanded {
                                            SourceDiff { r#where: r#where.clone(), path: path, refresh_key: refresh_key.clone() }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Some(Err(err)) => rsx! {
                    p { class: "error-text", role: "alert", "{bridge_error(err)}" }
                    {feedback}
                },
                None => rsx! { p { class: "files-status", "Loading…" } },
            }
        }
    }
}

#[component]
fn SourceDiff(r#where: Option<String>, path: String, refresh_key: String) -> Element {
    let diff = use_resource(use_reactive(
        (&r#where, &path, &refresh_key),
        |(place, path, _)| async move { api::file_diff(place.as_deref(), &path).await },
    ));
    match diff.read().as_ref() {
        Some(Err(err)) => rsx! { p { class: "error-text", role: "alert", "{bridge_error(err)}" } },
        result => {
            rsx! { DiffBlock { text: result.and_then(|value| value.as_ref().ok()).cloned() } }
        }
    }
}
