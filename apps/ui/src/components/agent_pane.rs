use std::cell::Cell;
use std::rc::Rc;

use dioxus::prelude::*;
use legate_core::{JobAccess, JobRecord, JobStatus};
use wasm_bindgen_futures::spawn_local;

use super::bot_avatar::{BotAvatar, BotName, BotState, error_text};
use crate::api;
use crate::logic::agent_tabs::{agent_status_label, step_word};
use crate::logic::job_steps::edited_files_summary;

fn avatar_state(status: JobStatus) -> BotState {
    match status {
        JobStatus::Running => BotState::Working,
        JobStatus::Failed => BotState::Error,
        JobStatus::Queued => BotState::Resting,
        JobStatus::Done => BotState::Idle,
    }
}

fn stick(id: &str) {
    let Some(element) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(id))
    else {
        return;
    };
    element.set_scroll_top(element.scroll_height());
}

fn near_bottom(id: &str) -> bool {
    let Some(element) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(id))
    else {
        return true;
    };
    element.scroll_height() - element.scroll_top() - element.client_height() < 24
}

fn access_name(access: Option<JobAccess>) -> &'static str {
    access.map(JobAccess::as_str).unwrap_or("read")
}

#[component]
pub fn AgentPane(job: JobRecord, on_take_over: Callback<String>, on_close: Callback) -> Element {
    let mut busy = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let stick_to = use_hook(|| Rc::new(Cell::new(true)));
    let steps = job.steps.clone().unwrap_or_default();
    let edits = edited_files_summary(&job);
    let live = job.status == JobStatus::Running || job.status == JobStatus::Queued;
    let mut steps_len = use_signal(|| steps.len());
    if steps_len() != steps.len() {
        steps_len.set(steps.len());
    }
    let mut output_len = use_signal(|| job.output.len());
    if output_len() != job.output.len() {
        output_len.set(job.output.len());
    }
    let log_id = format!("agent-log-{}", job.id);
    let log_effect = log_id.clone();
    let stick_effect = stick_to.clone();
    use_effect(move || {
        let _ = (steps_len(), output_len());
        if stick_effect.get() {
            stick(&log_effect);
        }
    });

    let started = job.session_id.is_some();
    let take_title = if !started {
        "Take over is possible once the agent has started"
    } else if live {
        "Stop the job and continue its session in a terminal"
    } else {
        "Continue this session in a terminal"
    };
    let mut meta = vec![
        job.provider
            .map(|id| id.as_str().to_string())
            .unwrap_or_else(|| "waiting for a worker".into()),
        job.job_type.as_str().to_string(),
        access_name(job.access).to_string(),
    ];
    if let Some(model) = job.model.clone() {
        meta.push(model);
    }
    let meta_line = meta.join(" · ");
    let heading = job
        .title
        .clone()
        .unwrap_or_else(|| job.prompt.chars().take(60).collect());
    let label = format!(
        "Agent: {}",
        job.title
            .clone()
            .unwrap_or_else(|| job.job_type.as_str().into())
    );
    let status = job.status;
    let status_name = match status {
        JobStatus::Queued => "queued",
        JobStatus::Running => "running",
        JobStatus::Done => "done",
        JobStatus::Failed => "failed",
    };
    let status_label = agent_status_label(&job);
    let provider = job.provider;
    let prompt = job.prompt.clone();
    let output = job.output.clone();
    let job_error = job.error.clone();
    let change = job.change.clone();
    let job_id = job.id.clone();
    let log_scroll = log_id.clone();

    rsx! {
        section { class: "agent-pane", aria_label: "{label}",
            header { class: "agent-pane-head",
                div { class: "agent-pane-title",
                    if let Some(provider) = provider {
                        BotAvatar {
                            bot: BotName::Plan(provider),
                            state: avatar_state(status),
                            size: 22.0,
                            title: None,
                        }
                    }
                    div { class: "agent-pane-name",
                        strong { "{heading}" }
                        span { class: "agent-pane-meta", "{meta_line}" }
                    }
                    span { class: "status status-{status_name}",
                        span { class: "status-dot", aria_hidden: "true" }
                        "{status_label}"
                    }
                }
                div { class: "agent-pane-actions",
                    button {
                        r#type: "button",
                        class: "btn",
                        disabled: busy() || !started,
                        title: "{take_title}",
                        onclick: {
                            let job_id = job_id.clone();
                            let on_take_over = on_take_over;
                            move |_| {
                                if busy() {
                                    return;
                                }
                                busy.set(true);
                                error.set(None);
                                let job_id = job_id.clone();
                                let on_take_over = on_take_over;
                                spawn_local(async move {
                                    match api::take_over_job(&job_id, 100.0, 30.0).await {
                                        Ok(_) => on_take_over.call(job_id),
                                        Err(err) => {
                                            error.set(Some(error_text(&err)));
                                            busy.set(false);
                                        }
                                    }
                                });
                            }
                        },
                        if busy() { "Opening…" } else { "Take over" }
                    }
                    if live {
                        button {
                            r#type: "button",
                            class: "btn btn-quiet",
                            onclick: {
                                let job_id = job_id.clone();
                                move |_| {
                                    error.set(None);
                                    let job_id = job_id.clone();
                                    spawn_local(async move {
                                        if let Err(err) = api::cancel_job(&job_id).await {
                                            error.set(Some(error_text(&err)));
                                        }
                                    });
                                }
                            },
                            "Stop"
                        }
                    }
                    button {
                        r#type: "button",
                        class: "btn btn-quiet",
                        onclick: move |_| on_close.call(()),
                        "Back to terminals"
                    }
                }
            }
            if let Some(message) = error.read().clone() {
                p { class: "error-text", role: "alert", "{message}" }
            }
            div {
                id: "{log_id}",
                class: "agent-pane-log",
                onscroll: {
                    let stick_to = stick_to.clone();
                    move |_| stick_to.set(near_bottom(&log_scroll))
                },
                p { class: "agent-pane-prompt", "> {prompt}" }
                for step in steps {
                    {
                        let word = step_word(step.tool);
                        let title = step.title.clone();
                        let failed = step.status == legate_core::JobStepStatus::Failed;
                        let status_name = match step.status {
                            legate_core::JobStepStatus::Running => "running",
                            legate_core::JobStepStatus::Done => "done",
                            legate_core::JobStepStatus::Failed => "failed",
                        };
                        let edit = step.edit.clone();
                        rsx! {
                            p { key: "{step.id}", class: "agent-pane-step is-{status_name}",
                                span { class: "agent-pane-word", "{word}" }
                                span { "{title}" }
                                if let Some(edit) = edit {
                                    span { class: "agent-pane-edit",
                                        span { class: "stat-add", "+{edit.added}" }
                                        " "
                                        span { class: "stat-del", "−{edit.removed}" }
                                    }
                                }
                                if failed {
                                    span { class: "agent-pane-failed", "failed" }
                                }
                            }
                        }
                    }
                }
                if !output.is_empty() {
                    div { class: "agent-pane-output", "{output}" }
                }
                if status == JobStatus::Queued {
                    p { class: "agent-pane-wait", "Waiting for a free worker…" }
                }
                if let Some(message) = job_error {
                    p { class: "agent-pane-error", "{message}" }
                }
                if let Some(edits) = edits {
                    if !live {
                        p { class: "agent-pane-done",
                            {file_sentence(edits.files)}
                            " "
                            span { class: "stat-add", "+{edits.added}" }
                            " "
                            span { class: "stat-del", "−{edits.removed}" }
                            if change.is_some() {
                                " · pick its worktree in the sidebar to review it"
                            }
                        }
                    }
                }
            }
        }
    }
}

fn file_sentence(files: usize) -> String {
    if files == 1 {
        "Edited 1 file".into()
    } else {
        format!("Edited {files} files")
    }
}
