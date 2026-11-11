use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use dioxus::prelude::*;
use legate_core::{JobRecord, JobStatus, JobStep, JobStepStatus, StepTool};
use wasm_bindgen_futures::spawn_local;

use crate::api;
use crate::logic::agent_tabs::step_word;
use crate::logic::job_steps::{diff_for_file, edited_files, edited_files_summary};

const DIFF_LINE_CAP: usize = 5000;

thread_local! {
    static STEPS_OPEN: RefCell<HashMap<String, bool>> = RefCell::new(HashMap::new());
}

fn tool_word(tool: StepTool) -> &'static str {
    step_word(tool)
}

fn split_path(path: &str) -> (String, String) {
    let norm = path.replace('\\', "/");
    match norm.rfind('/') {
        Some(index) => (norm[index + 1..].to_string(), norm[..index].to_string()),
        None => (norm, String::new()),
    }
}

fn format_duration(ms: i64) -> String {
    if ms < 1000 {
        return format!("{ms}ms");
    }
    let seconds = ms as f64 / 1000.0;
    if seconds < 10.0 {
        return format!("{seconds:.1}s");
    }
    format!("{}s", seconds.round() as i64)
}

fn file_label(count: usize) -> String {
    if count == 1 {
        "1 file".into()
    } else {
        format!("{count} files")
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

#[component]
pub fn JobSteps(job: JobRecord) -> Element {
    let steps = job.steps.clone().unwrap_or_default();
    let dropped = job.steps_dropped.unwrap_or(0);
    if steps.is_empty() && dropped == 0 {
        return rsx! {};
    }
    let show_edits = !edited_files(&steps).is_empty();
    rsx! {
        div { class: "job-steps-block",
            if show_edits {
                EditedFilesCard { job: job.clone() }
            }
            StepsList { job: job.clone() }
        }
    }
}

#[component]
fn EditedFilesCard(job: JobRecord) -> Element {
    let files = edited_files(job.steps.as_deref().unwrap_or(&[]));
    let summary = edited_files_summary(&job);
    let mut open_path = use_signal(|| None::<String>);
    let mut diff_text = use_signal(|| None::<String>);
    let mut diff_failed = use_signal(|| false);
    let mut diff_loading = use_signal(|| job.change.is_some());
    let change_id = job.change.clone();
    let mut loaded = use_signal(|| None::<String>);
    let ticket = use_hook(|| Rc::new(Cell::new(0u64)));
    if loaded() != change_id {
        loaded.set(change_id.clone());
        diff_text.set(None);
        diff_failed.set(false);
        diff_loading.set(change_id.is_some());
        if let Some(id) = change_id.clone() {
            let next = ticket.get() + 1;
            ticket.set(next);
            let ticket = ticket.clone();
            spawn_local(async move {
                let result = api::change_diff(&id).await;
                if ticket.get() != next {
                    return;
                }
                match result {
                    Ok(text) => {
                        diff_text.set(Some(text));
                        diff_loading.set(false);
                    }
                    Err(_) => {
                        diff_failed.set(true);
                        diff_loading.set(false);
                    }
                }
            });
        }
    }
    let Some(summary) = summary else {
        return rsx! {};
    };
    let added = summary.added;
    let removed = summary.removed;
    let title = format!("Edited {}", file_label(summary.files));
    let text = diff_text.read().clone();
    let failed = diff_failed();
    let loading = diff_loading();
    let opened = open_path.read().clone();
    let change = job.change.clone();

    rsx! {
        div { class: "job-edits",
            div { class: "job-edits-head",
                span { class: "job-edits-title", "{title}" }
                span { class: "change-stats",
                    span { class: "stat-add", "+{added}" }
                    " "
                    span { class: "stat-del", "−{removed}" }
                }
            }
            ul { class: "job-edits-list",
                for file in files {
                    {
                        let path = file.path.clone();
                        let (name, folder) = split_path(&path);
                        let is_open = opened.as_deref() == Some(path.as_str());
                        let added = file.added;
                        let removed = file.removed;
                        let body_text = text.clone();
                        let body_change = change.clone();
                        rsx! {
                            li { key: "{path}", class: "job-edits-item",
                                button {
                                    r#type: "button",
                                    class: "job-edits-row",
                                    aria_expanded: if is_open { "true" } else { "false" },
                                    onclick: {
                                        let path = path.clone();
                                        move |_| {
                                            open_path.set(if is_open { None } else { Some(path.clone()) });
                                        }
                                    },
                                    span { class: "job-edits-name", "{name}" }
                                    if !folder.is_empty() {
                                        span { class: "job-edits-folder", "{folder}" }
                                    }
                                    span { class: "change-stats",
                                        span { class: "stat-add", "+{added}" }
                                        " "
                                        span { class: "stat-del", "−{removed}" }
                                    }
                                    span { class: if is_open { "job-edits-chevron is-open" } else { "job-edits-chevron" }, ">" }
                                }
                                if is_open {
                                    {file_body(body_change.as_deref(), failed, loading, body_text.as_deref(), &path)}
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn file_body(
    change_id: Option<&str>,
    failed: bool,
    loading: bool,
    text: Option<&str>,
    file_path: &str,
) -> Element {
    if change_id.is_none() || failed {
        return rsx! { p { class: "job-edits-gone", "The diff is no longer available." } };
    }
    if loading || text.is_none() {
        return rsx! { p { class: "diff-status", "Loading diff…" } };
    }
    let text = text.unwrap_or("");
    file_diff(text, file_path)
}

fn file_diff(text: &str, file_path: &str) -> Element {
    let lines = diff_for_file(text, file_path);
    if lines.is_empty() {
        return rsx! { p { class: "job-edits-gone", "The diff is no longer available." } };
    }
    let hidden = lines.len().saturating_sub(DIFF_LINE_CAP);
    let shown: Vec<_> = lines.into_iter().take(DIFF_LINE_CAP).collect();
    rsx! {
        div { class: "diff-block",
            for (index, line) in shown.into_iter().enumerate() {
                {
                    let kind = line.kind.as_str();
                    let text = if line.text.is_empty() { " ".to_string() } else { line.text };
                    rsx! {
                        div { key: "{index}", class: "diff-line is-{kind}", "{text}" }
                    }
                }
            }
            if hidden > 0 {
                div { class: "diff-line is-meta", "… {hidden} more lines not shown" }
            }
        }
    }
}

#[component]
fn StepsList(job: JobRecord) -> Element {
    let steps = job.steps.clone().unwrap_or_default();
    let dropped = job.steps_dropped.unwrap_or(0);
    let running = job.status == JobStatus::Running || job.status == JobStatus::Queued;
    let initial = STEPS_OPEN.with(|map| map.borrow().get(&job.id).copied().unwrap_or(running));
    let mut open = use_signal(|| initial);
    let mut follow = use_signal(|| true);
    let mut for_id = use_signal(String::new);
    if for_id() != job.id {
        let next = STEPS_OPEN.with(|map| map.borrow().get(&job.id).copied().unwrap_or(running));
        open.set(next);
        follow.set(true);
        for_id.set(job.id.clone());
    }
    let mut running_sig = use_signal(|| running);
    if running_sig() != running {
        running_sig.set(running);
    }
    let mut steps_len = use_signal(|| steps.len());
    if steps_len() != steps.len() {
        steps_len.set(steps.len());
    }
    let list_id = format!("job-steps-{}", job.id);
    let list_id_effect = list_id.clone();
    use_effect(move || {
        let open_now = open();
        let follow_now = follow();
        let running_now = running_sig();
        let _ = steps_len();
        if open_now && follow_now && running_now {
            stick(&list_id_effect);
        }
    });
    let count = steps.len();
    let list_id_scroll = list_id.clone();

    rsx! {
        div { class: "job-steps",
            button {
                r#type: "button",
                class: "job-steps-head",
                aria_expanded: if open() { "true" } else { "false" },
                onclick: {
                    let id = job.id.clone();
                    move |_| {
                        let next = !open();
                        STEPS_OPEN.with(|map| {
                            map.borrow_mut().insert(id.clone(), next);
                        });
                        open.set(next);
                    }
                },
                "Steps ({count})"
            }
            if open() {
                div {
                    id: "{list_id}",
                    class: "job-steps-list",
                    onscroll: move |_| follow.set(near_bottom(&list_id_scroll)),
                    if dropped > 0 {
                        p { class: "job-steps-dropped", "{dropped} earlier steps not kept." }
                    }
                    for step in steps {
                        StepRow { key: "{step.id}", step }
                    }
                }
            }
        }
    }
}

#[component]
fn StepRow(step: JobStep) -> Element {
    let mut open = use_signal(|| false);
    let duration = step
        .ended_at
        .map(|ended| format_duration((ended - step.started_at).max(0)))
        .unwrap_or_default();
    let has_detail = step
        .detail
        .as_ref()
        .is_some_and(|detail| !detail.is_empty());
    let detail = step.detail.clone().unwrap_or_default();
    let title = step.title.clone();
    let word = tool_word(step.tool);
    let running = step.status == JobStepStatus::Running;
    let failed = step.status == JobStepStatus::Failed;
    let body = rsx! {
        span { class: "job-step-tool", "{word}" }
        span { class: "job-step-title", title: "{title}", "{title}" }
        if !duration.is_empty() {
            span { class: "job-step-time", "{duration}" }
        }
        if running {
            span { class: "status status-running",
                span { class: "status-dot", aria_hidden: "true" }
            }
        }
        if failed {
            span { class: "job-step-failed", "failed" }
        }
    };
    if !has_detail {
        return rsx! { div { class: "job-step", {body} } };
    }
    rsx! {
        div { class: "job-step-wrap",
            button {
                r#type: "button",
                class: "job-step",
                aria_expanded: if open() { "true" } else { "false" },
                onclick: move |_| open.set(!open()),
                {body}
            }
            if open() {
                pre { class: "job-step-detail", "{detail}" }
            }
        }
    }
}
