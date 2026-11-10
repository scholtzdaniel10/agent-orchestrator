use dioxus::prelude::*;
use legate_core::{JobAccess, LeadProposal, PROVIDERS, ProposedJob, ProviderId};

use super::bot_avatar::{BotAvatar, BotName, BotState, FieldSelect, SelectOption};

fn worker_name(id: ProviderId) -> &'static str {
    match id {
        ProviderId::Claude => "Claude",
        ProviderId::Codex => "Codex",
        ProviderId::Grok => "Grok",
        ProviderId::Cursor => "Cursor",
        ProviderId::Local => "local",
    }
}

fn access_label(access: JobAccess) -> &'static str {
    match access {
        JobAccess::Read => "Read only",
        JobAccess::Edit => "Edit files",
        JobAccess::Full => "Full access",
    }
}

#[component]
pub fn LeadPlan(
    proposal: LeadProposal,
    on_run: Callback<Vec<ProposedJob>>,
    on_dismiss: Callback,
) -> Element {
    let mut jobs = use_signal(|| proposal.jobs.clone());
    let mut open = use_signal(|| None::<usize>);
    let mut busy = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let current = jobs.read().clone();
    let runnable: Vec<ProposedJob> = current
        .iter()
        .filter(|job| !job.prompt.trim().is_empty())
        .cloned()
        .collect();
    let run_label = if runnable.len() == 1 {
        "Run 1 agent".to_string()
    } else {
        format!("Run {} agents", runnable.len())
    };
    let expanded = open();

    rsx! {
        div { class: "lead-plan", aria_label: "The lead's plan",
            h3 { class: "lead-delegated-title", "Plan" }
            p { class: "lead-plan-summary", "{proposal.summary}" }
            ol { class: "lead-plan-list",
                for (index, job) in current.iter().cloned().enumerate() {
                    {
                        let title = job.title.clone();
                        let prompt = job.prompt.clone();
                        let is_open = expanded == Some(index);
                        let worker_value = job.worker.map(|id| id.as_str().to_string()).unwrap_or_else(|| "auto".into());
                        let access_value = job.access.as_str().to_string();
                        let mut worker_options = vec![SelectOption::new("auto", "Router picks")];
                        for id in PROVIDERS {
                            worker_options.push(SelectOption::new(id.as_str(), worker_name(id)));
                        }
                        let access_options = [JobAccess::Read, JobAccess::Edit, JobAccess::Full]
                            .into_iter()
                            .map(|access| SelectOption::new(access.as_str(), access_label(access)))
                            .collect::<Vec<_>>();
                        let remove_label = format!("Remove {title}");
                        let title_label = format!("Title of job {}", index + 1);
                        let worker_label = format!("Worker for {title}");
                        let access_label_text = format!("Access for {title}");
                        let prompt_label = format!("Prompt for {title}");
                        rsx! {
                            li { key: "{index}", class: "lead-plan-card",
                                div { class: "lead-plan-row",
                                    if let Some(worker) = job.worker {
                                        BotAvatar { bot: BotName::Plan(worker), state: BotState::Idle, size: 18.0, title: None }
                                    } else {
                                        BotAvatar { bot: BotName::Lead, state: BotState::Idle, size: 18.0, title: None }
                                    }
                                    input {
                                        class: "lead-plan-title",
                                        aria_label: "{title_label}",
                                        value: "{title}",
                                        maxlength: 80,
                                        oninput: move |event| {
                                            let next = event.value();
                                            jobs.with_mut(|list| {
                                                if let Some(item) = list.get_mut(index) {
                                                    item.title = next;
                                                }
                                            });
                                        },
                                    }
                                    button {
                                        r#type: "button",
                                        class: "btn btn-quiet btn-compact",
                                        aria_expanded: if is_open { "true" } else { "false" },
                                        onclick: move |_| open.set(if is_open { None } else { Some(index) }),
                                        if is_open { "Less" } else { "Edit" }
                                    }
                                    button {
                                        r#type: "button",
                                        class: "tab-close",
                                        aria_label: "{remove_label}",
                                        onclick: move |_| {
                                            jobs.with_mut(|list| {
                                                if index < list.len() {
                                                    list.remove(index);
                                                }
                                            });
                                            open.set(None);
                                        },
                                        "×"
                                    }
                                }
                                div { class: "lead-plan-meta",
                                    FieldSelect {
                                        aria_label: worker_label,
                                        value: worker_value,
                                        options: worker_options,
                                        on_change: Callback::new(move |next: String| {
                                            let worker = ProviderId::parse(&next);
                                            jobs.with_mut(|list| {
                                                if let Some(item) = list.get_mut(index) {
                                                    item.worker = worker;
                                                }
                                            });
                                        }),
                                    }
                                    FieldSelect {
                                        aria_label: access_label_text,
                                        value: access_value,
                                        options: access_options,
                                        on_change: Callback::new(move |next: String| {
                                            let access = if next == "edit" || next == "full" {
                                                JobAccess::parse(&next).unwrap_or(JobAccess::Read)
                                            } else {
                                                JobAccess::Read
                                            };
                                            jobs.with_mut(|list| {
                                                if let Some(item) = list.get_mut(index) {
                                                    item.access = access;
                                                }
                                            });
                                        }),
                                    }
                                    span { class: "lead-plan-type", "{job.job_type.as_str()}" }
                                }
                                if is_open {
                                    textarea {
                                        class: "lead-plan-prompt",
                                        aria_label: "{prompt_label}",
                                        rows: "6",
                                        value: "{prompt}",
                                        oninput: move |event| {
                                            let next = event.value();
                                            jobs.with_mut(|list| {
                                                if let Some(item) = list.get_mut(index) {
                                                    item.prompt = next;
                                                }
                                            });
                                        },
                                    }
                                } else {
                                    p { class: "lead-job-prompt", title: "{prompt}", "{prompt}" }
                                }
                            }
                        }
                    }
                }
            }
            if let Some(message) = error.read().clone() {
                p { class: "error-text", role: "alert", "{message}" }
            }
            div { class: "lead-plan-actions",
                button {
                    r#type: "button",
                    class: "btn btn-primary",
                    disabled: busy() || runnable.is_empty(),
                    onclick: {
                        let runnable = runnable.clone();
                        let on_run = on_run;
                        move |_| {
                            if busy() || runnable.is_empty() {
                                return;
                            }
                            busy.set(true);
                            error.set(None);
                            on_run.call(runnable.clone());
                            busy.set(false);
                        }
                    },
                    "{run_label}"
                }
                button {
                    r#type: "button",
                    class: "btn btn-quiet",
                    disabled: busy(),
                    onclick: move |_| {
                        if busy() {
                            return;
                        }
                        busy.set(true);
                        error.set(None);
                        on_dismiss.call(());
                        busy.set(false);
                    },
                    "Dismiss"
                }
            }
        }
    }
}
