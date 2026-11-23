//! Settings → Updates: each CLI Legate drives, its installed and latest version, and one button
//! that runs the chosen tools' own updaters in PowerShell.

use dioxus::prelude::*;

use crate::api;
use crate::wire::CliUpdate;

#[derive(Clone, PartialEq)]
enum Phase {
    Checking,
    Ready,
    /// The PowerShell window is open; the list is checked again when it closes.
    Updating,
}

#[component]
pub fn UpdatesSection(on_checked: Callback<usize>) -> Element {
    let mut list = use_signal(Vec::<CliUpdate>::new);
    let mut chosen = use_signal(Vec::<String>::new);
    let mut phase = use_signal(|| Phase::Checking);
    let mut error = use_signal(|| None::<String>);
    let mut note = use_signal(|| None::<String>);

    let check = use_callback(move |()| {
        phase.set(Phase::Checking);
        error.set(None);
        spawn(async move {
            match api::cli_updates().await {
                Ok(found) => {
                    let newer: Vec<String> = found
                        .iter()
                        .filter(|row| row.newer && row.can_update)
                        .map(|row| row.id.as_str().to_string())
                        .collect();
                    on_checked.call(newer.len());
                    chosen.set(newer);
                    list.set(found);
                }
                Err(text) => error.set(Some(text)),
            }
            phase.set(Phase::Ready);
        });
    });
    use_hook(move || check(()));

    let rows = list.read().clone();
    let picked = chosen.read().clone();
    let busy = *phase.read() != Phase::Ready;
    let newer = rows.iter().filter(|row| row.newer).count();

    rsx! {
        div { class: "updates",
            div { class: "updates-head",
                p { class: "updates-summary", role: "status",
                    match *phase.read() {
                        Phase::Checking => "Checking for new versions…".to_string(),
                        Phase::Updating => "Updating in PowerShell. Legate checks again when that window closes.".to_string(),
                        Phase::Ready if newer == 0 => "Everything is up to date.".to_string(),
                        Phase::Ready if newer == 1 => "1 update available.".to_string(),
                        Phase::Ready => format!("{newer} updates available."),
                    }
                }
                button {
                    class: "btn btn-quiet btn-compact",
                    r#type: "button",
                    disabled: busy,
                    onclick: move |_| check(()),
                    "Check again"
                }
            }
            if let Some(text) = error() {
                p { class: "error-text", role: "alert", "{text}" }
            }
            ul { class: "updates-list",
                for row in rows {
                    {
                        let id = row.id.as_str().to_string();
                        let on = picked.contains(&id);
                        let checkbox_id = format!("update-{id}");
                        let status = status_of(&row);
                        rsx! {
                            li { key: "{id}", class: if row.newer { "updates-row is-newer" } else { "updates-row" },
                                input {
                                    id: "{checkbox_id}",
                                    r#type: "checkbox",
                                    checked: on,
                                    disabled: !row.can_update || busy,
                                    onchange: {
                                        let id = id.clone();
                                        move |event: FormEvent| {
                                            let id = id.clone();
                                            chosen.with_mut(|list| {
                                                list.retain(|item| *item != id);
                                                if event.checked() {
                                                    list.push(id);
                                                }
                                            });
                                        }
                                    },
                                }
                                label { r#for: "{checkbox_id}", class: "updates-name", "{row.label}" }
                                span { class: "updates-versions",
                                    span { class: "updates-installed", "{row.installed.clone().unwrap_or_else(|| \"not installed\".into())}" }
                                    if let Some(latest) = row.latest.clone() {
                                        span { class: "updates-arrow", aria_hidden: "true", "→" }
                                        span { class: "updates-latest", "{latest}" }
                                    }
                                }
                                span { class: "updates-status is-{status.0}", "{status.1}" }
                            }
                        }
                    }
                }
            }
            div { class: "updates-actions",
                button {
                    class: "btn btn-primary",
                    r#type: "button",
                    disabled: busy || picked.is_empty(),
                    onclick: move |_| {
                        let ids = chosen.read().clone();
                        phase.set(Phase::Updating);
                        note.set(None);
                        spawn(async move {
                            match api::run_cli_updates(&ids).await {
                                Ok(Some(_terminal)) => note.set(Some("The updates are running in a new terminal tab.".into())),
                                Ok(None) => {}
                                Err(text) => error.set(Some(text)),
                            }
                            check(());
                        });
                    },
                    "Update in PowerShell"
                }
                p { class: "hint",
                    "Runs each tool's own updater in a PowerShell window: claude update, cursor-agent update, npm for Codex, winget for GitHub CLI and Git."
                }
            }
            if let Some(text) = note() {
                p { class: "hint", "{text}" }
            }
        }
    }
}

/// A status class and its words for one row.
fn status_of(row: &CliUpdate) -> (&'static str, &'static str) {
    if !row.can_update {
        ("missing", "Not installed")
    } else if row.newer {
        ("newer", "Update available")
    } else if row.latest.is_some() {
        ("current", "Up to date")
    } else {
        ("unknown", "Its updater checks")
    }
}
