use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use dioxus::html::{Key, Modifiers};
use dioxus::prelude::*;
use legate_core::{
    ChangeSet, JobAccess, JobRecord, JobStatus, JobType, PROVIDERS, PlanStatus, ProviderId,
    is_provider_id, plan_of,
};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use super::bot_avatar::{BotAvatar, BotName, BotState, FieldSelect, SelectOption, error_text};
use super::job_steps::JobSteps;
use super::rich_text::RichText;
use super::star::RayBurst;
use crate::api;

const ACCESS_KEY: &str = "orch.access";

#[component]
pub fn JobsPanel(
    jobs: Vec<JobRecord>,
    plans: Option<Vec<PlanStatus>>,
    changes: Vec<ChangeSet>,
    is_repo: bool,
    selected_job_id: Option<String>,
    show_job: Option<(String, i64)>,
    on_select_job: Callback<Option<String>>,
    on_job: Callback<JobRecord>,
    on_view_change: Callback<String>,
    on_goal: Callback,
) -> Element {
    let mut prompt = use_signal(String::new);
    let mut check = use_signal(String::new);
    let mut job_type = use_signal(|| JobType::Planning);
    let mut worker = use_signal(|| "auto".to_string());
    let mut access = use_signal(load_access);
    let stopping_id = use_signal(|| None::<String>);
    let submitting = use_signal(|| false);
    let submit_error = use_signal(|| None::<String>);
    let sending = use_hook(|| Rc::new(Cell::new(false)));
    let stopping = use_hook(|| Rc::new(Cell::new(false)));
    let mut shown = use_signal(|| show_job.clone());
    if shown() != show_job {
        shown.set(show_job.clone());
    }
    use_effect(move || {
        if let Some((id, _)) = shown() {
            scroll_job(&format!("job-{id}"));
        }
    });

    let newest: Vec<JobRecord> = jobs.iter().rev().cloned().collect();
    let selected_job = jobs
        .iter()
        .find(|job| Some(&job.id) == selected_job_id.as_ref())
        .cloned();
    let prompt_empty = prompt.read().trim().is_empty();
    let claude_ready = plan_available(plans.as_deref(), ProviderId::Claude);
    let cursor_ready = plan_available(plans.as_deref(), ProviderId::Cursor);
    let both_ready = claude_ready && cursor_ready;
    let no_plan_ready = plans.is_some() && !claude_ready && !cursor_ready;
    let worker_now = worker();
    let choice_blocked = match worker_now.as_str() {
        "both" => !both_ready,
        "auto" => false,
        other => {
            ProviderId::parse(other).is_none_or(|id| !plan_available(plans.as_deref(), plan_of(id)))
        }
    };
    let submit_disabled = prompt_empty || no_plan_ready || choice_blocked || submitting();
    let access_now = access();
    let access_value = if is_repo { access_now.as_str() } else { "read" };
    let plans_for_options = plans.clone();

    rsx! {
        div { class: "jobs-panel",
            form {
                class: "composer",
                onsubmit: {
                    let sending = sending.clone();
                    move |event| {
                        event.prevent_default();
                        launch_submit(
                            prompt, check, job_type(), worker(), access(), is_repo, sending.clone(),
                            submitting, submit_error, on_job, on_select_job, on_goal,
                        );
                    }
                },
                label { class: "sr-only", r#for: "prompt", "Prompt" }
                div { class: "composer-box",
                    textarea {
                        id: "prompt",
                        rows: "3",
                        value: "{prompt}",
                        placeholder: "Hand a job straight to a worker.",
                        oninput: move |event| prompt.set(event.value()),
                        onkeydown: {
                            let sending = sending.clone();
                            move |event| {
                                if event.key() == Key::Enter && event.modifiers().contains(Modifiers::CONTROL) && !prompt.read().trim().is_empty() {
                                    event.prevent_default();
                                    launch_submit(
                                        prompt, check, job_type(), worker(), access(), is_repo, sending.clone(),
                                        submitting, submit_error, on_job, on_select_job, on_goal,
                                    );
                                }
                            }
                        },
                    }
                    if is_repo {
                        input {
                            class: "composer-check",
                            value: "{check}",
                            maxlength: "2000",
                            placeholder: "Check, optional: e.g. pnpm test -- auth. Fix rounds until it passes, asking you each time.",
                            aria_label: "Check command",
                            disabled: worker_now == "both",
                            oninput: move |event| check.set(event.value()),
                        }
                    }
                    div { class: "composer-foot",
                        div { class: "composer-tools",
                            label { r#for: "job-type", "Job type" }
                            FieldSelect {
                                id: "job-type".to_string(),
                                class: "job-type".to_string(),
                                value: job_type().as_str().to_string(),
                                options: JobType::ALL.into_iter().map(|kind| SelectOption::new(kind.as_str(), kind.as_str())).collect::<Vec<_>>(),
                                on_change: Callback::new(move |next: String| {
                                    if let Some(kind) = JobType::parse(&next) {
                                        job_type.set(kind);
                                    }
                                }),
                            }
                            label { r#for: "job-worker", "Worker" }
                            FieldSelect {
                                id: "job-worker".to_string(),
                                class: "plan-choice".to_string(),
                                value: worker_now.clone(),
                                options: worker_options(plans_for_options.as_deref(), both_ready),
                                on_change: Callback::new(move |next: String| worker.set(as_plan_choice(&next))),
                            }
                            label { r#for: "job-access", "Access" }
                            FieldSelect {
                                id: "job-access".to_string(),
                                class: "job-access".to_string(),
                                value: access_value.to_string(),
                                title: if is_repo { String::new() } else { "Open a git repository to let jobs edit files.".into() },
                                options: vec![
                                    SelectOption::new("read", "Read only"),
                                    SelectOption::new("edit", "Edit files").disabled(!is_repo),
                                    SelectOption::new("full", "Full access").disabled(!is_repo),
                                ],
                                on_change: Callback::new(move |next: String| {
                                    let value = as_access(&next);
                                    access.set(value);
                                    save_access(value);
                                }),
                            }
                            if access_now == JobAccess::Full && is_repo {
                                span { class: "composer-hint access-hint", "Runs commands on your computer without asking. Its file changes still wait in Changes." }
                            }
                        }
                        button {
                            class: "btn btn-primary",
                            r#type: "submit",
                            disabled: submit_disabled,
                            if submitting() { "Submitting…" } else { "Submit" }
                        }
                    }
                }
                if no_plan_ready {
                    p { class: "hint", "No plan is ready. See Usage in the panel." }
                }
                if let Some(message) = submit_error.read().clone() {
                    p { class: "field-error", role: "alert", "{message}" }
                }
            }
            if let Some(selected_job) = selected_job {
                section { class: "output-pane", aria_label: "Output",
                    button {
                        r#type: "button",
                        class: "btn btn-quiet btn-compact jobs-back",
                        onclick: move |_| on_select_job.call(None),
                        "← Jobs"
                    }
                    if selected_job.group.as_ref().is_some_and(|group| jobs.iter().any(|job| job.group.as_deref() == Some(group.as_str()) && job.id != selected_job.id)) {
                        CompareOutput {
                            group_jobs: jobs.into_iter().filter(|job| job.group == selected_job.group).collect::<Vec<_>>(),
                            changes: changes.clone(),
                            on_view_change,
                            stopping_id: stopping_id(),
                            on_stop: Callback::new({
                                let stopping = stopping.clone();
                                move |id: String| stop_job(id, stopping.clone(), stopping_id, submit_error, on_job)
                            }),
                        }
                    } else {
                        div { class: "output-head",
                            if let Some(provider) = selected_job.provider {
                                BotAvatar {
                                    bot: BotName::Plan(provider),
                                    state: job_avatar(selected_job.status),
                                    size: 20.0,
                                    title: None,
                                }
                            }
                            span { title: "{output_title(&selected_job)}", "{output_title(&selected_job)}" }
                            if can_stop(&selected_job) {
                                button {
                                    r#type: "button",
                                    class: "btn btn-compact",
                                    disabled: stopping_id.read().as_deref() == Some(selected_job.id.as_str()),
                                    onclick: {
                                        let stopping = stopping.clone();
                                        let id = selected_job.id.clone();
                                        move |_| stop_job(id.clone(), stopping.clone(), stopping_id, submit_error, on_job)
                                    },
                                    if stopping_id.read().as_deref() == Some(selected_job.id.as_str()) { "Stopping…" } else { "Stop" }
                                }
                            }
                        }
                        div { class: "output-body",
                            JobSteps { job: selected_job.clone() }
                            RichText { text: selected_job.output.clone() }
                        }
                        if let Some(message) = selected_job.error.clone() {
                            pre { class: "output-error", "{message}" }
                        }
                    }
                }
            } else {
                section { class: "jobs", aria_label: "Jobs",
                    if newest.is_empty() {
                        div { class: "empty-state",
                            RayBurst {}
                            p { class: "empty-state-title", "No jobs yet" }
                            p { class: "empty-state-guidance", "Jobs you submit, and jobs the lead hands out, appear here." }
                        }
                    } else {
                        div { role: "listbox", aria_label: "Jobs", class: "job-list",
                            for (index, job) in newest.into_iter().enumerate() {
                                {job_row(index, job, newest_ids(&jobs), selected_job_id.clone(), changes.clone(), on_select_job, on_view_change)}
                            }
                        }
                    }
                }
            }
        }
    }
}

fn newest_ids(jobs: &[JobRecord]) -> Vec<String> {
    jobs.iter().rev().map(|job| job.id.clone()).collect()
}

fn job_row(
    index: usize,
    job: JobRecord,
    order: Vec<String>,
    selected_job_id: Option<String>,
    changes: Vec<ChangeSet>,
    on_select_job: Callback<Option<String>>,
    on_view_change: Callback<String>,
) -> Element {
    let is_selected = Some(&job.id) == selected_job_id.as_ref();
    let tab = if is_selected || (selected_job_id.is_none() && index == 0) {
        "0"
    } else {
        "-1"
    };
    let row_class = if is_selected {
        "job-row is-selected"
    } else {
        "job-row"
    };
    let status = job_status_name(job.status);
    let prompt = job.prompt.clone();
    let id = job.id.clone();
    let step_title = if job.status == JobStatus::Running {
        crate::logic::job_steps::latest_step_title(&job)
    } else {
        None
    };
    let change_id = visible_change(&job, &changes);
    let failover = failover_chain(&job);
    let parts = meta_parts(&job);
    let row_id = format!("job-{id}");
    rsx! {
        div {
            key: "{id}",
            id: "{row_id}",
            role: "option",
            aria_selected: if is_selected { "true" } else { "false" },
            tabindex: "{tab}",
            class: "{row_class}",
            onclick: {
                let id = id.clone();
                move |_| on_select_job.call(Some(id.clone()))
            },
            onkeydown: {
                let id = id.clone();
                let order = order.clone();
                move |event| {
                    let key = event.key();
                    let name = key.to_string();
                    if key == Key::Enter || name == " " {
                        event.prevent_default();
                        on_select_job.call(Some(id.clone()));
                        return;
                    }
                    if key != Key::ArrowDown && key != Key::ArrowUp {
                        return;
                    }
                    event.prevent_default();
                    let Some(at) = order.iter().position(|item| item == &id) else { return };
                    let next_at = if key == Key::ArrowDown { at + 1 } else { at.wrapping_sub(1) };
                    let Some(next) = order.get(next_at) else { return };
                    on_select_job.call(Some(next.clone()));
                    focus_id(&format!("job-{next}"));
                }
            },
            div { class: "job-main",
                if let Some(provider) = job.provider {
                    BotAvatar {
                        bot: BotName::Plan(provider),
                        state: job_avatar(job.status),
                        size: 20.0,
                        title: Some(provider.as_str().to_string()),
                    }
                } else {
                    span { class: "job-avatar-slot", aria_hidden: "true" }
                }
                span { class: "job-title", title: "{prompt}", "{prompt}" }
                span { class: "status status-{status}",
                    span { class: "status-dot", aria_hidden: "true" }
                    "{status}"
                }
            }
            if let Some(step_title) = step_title.clone() {
                p { class: "job-step-live", title: "{step_title}", "{step_title}" }
            }
            div { class: "job-meta",
                {meta_line(&parts)}
                if let Some(change_id) = change_id.clone() {
                    button {
                        r#type: "button",
                        class: "btn btn-quiet btn-compact",
                        onclick: move |event| {
                            event.stop_propagation();
                            on_view_change.call(change_id.clone());
                        },
                        onkeydown: move |event| event.stop_propagation(),
                        "View change"
                    }
                }
            }
            if !job.failed_over.is_empty() {
                div {
                    class: "failover",
                    aria_label: "Failed over from {failover.first().cloned().unwrap_or_default()} to {failover.last().cloned().unwrap_or_default()}",
                    span { class: "failover-label", "Failover" }
                    for (hop, name) in failover.iter().cloned().enumerate() {
                        span { key: "{id}-failover-{hop}-{name}", class: "failover-hop",
                            if hop > 0 { {failover_arrow()} }
                            span { "{name}" }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn CompareOutput(
    group_jobs: Vec<JobRecord>,
    changes: Vec<ChangeSet>,
    on_view_change: Callback<String>,
    stopping_id: Option<String>,
    on_stop: Callback<String>,
) -> Element {
    let merging = use_signal(|| false);
    let messages = use_signal(HashMap::<String, String>::new);
    let merging_ref = use_hook(|| Rc::new(Cell::new(false)));
    let mut ordered = group_jobs.clone();
    ordered.sort_by_key(|job| plan_rank(job.provider));
    let either_busy = ordered
        .iter()
        .any(|job| job.status == JobStatus::Queued || job.status == JobStatus::Running);
    let message_map = messages.read().clone();

    rsx! {
        div { class: "compare-output",
            for job in ordered.clone() {
                {
                    let change_id = visible_change(&job, &changes);
                    let change = change_id.as_ref().and_then(|id| changes.iter().find(|item| &item.id == id)).cloned();
                    let waiting = job.output.is_empty() && (job.status == JobStatus::Queued || job.status == JobStatus::Running);
                    let message = message_map.get(&job.id).cloned();
                    let status = job_status_name(job.status);
                    let provider_name = job.provider.map(|id| id.as_str().to_string()).unwrap_or_else(|| "—".into());
                    let model = job.model.clone();
                    let head = if let Some(model) = &model {
                        format!("{provider_name} · {model}")
                    } else {
                        provider_name.clone()
                    };
                    let stopping = stopping_id.as_deref() == Some(job.id.as_str());
                    let id = job.id.clone();
                    let files = change.as_ref().map(|item| item.files.len()).unwrap_or(0);
                    let file_label = if files == 1 { "1 file".to_string() } else { format!("{files} files") };
                    let insertions = change.as_ref().map(|item| item.insertions).unwrap_or(0);
                    let deletions = change.as_ref().map(|item| item.deletions).unwrap_or(0);
                    rsx! {
                        div { key: "{id}", class: "compare-col",
                            div { class: "output-head",
                                if let Some(provider) = job.provider {
                                    BotAvatar {
                                        bot: BotName::Plan(provider),
                                        state: job_avatar(job.status),
                                        size: 20.0,
                                        title: Some(provider.as_str().to_string()),
                                    }
                                }
                                span { "{head}" }
                                span { class: "status status-{status}",
                                    span { class: "status-dot", aria_hidden: "true" }
                                    "{status}"
                                }
                                if can_stop(&job) {
                                    button {
                                        r#type: "button",
                                        class: "btn btn-compact",
                                        disabled: stopping,
                                        onclick: {
                                            let id = id.clone();
                                            move |_| on_stop.call(id.clone())
                                        },
                                        if stopping { "Stopping…" } else { "Stop" }
                                    }
                                }
                            }
                            div { class: "output-body",
                                JobSteps { job: job.clone() }
                                if waiting {
                                    p { class: "compare-wait", if job.status == JobStatus::Queued { "Waiting…" } else { "Working…" } }
                                } else {
                                    RichText { text: job.output.clone() }
                                }
                            }
                            if let Some(message) = job.error.clone() {
                                pre { class: "output-error", "{message}" }
                            }
                            if let Some(message) = message {
                                pre { class: "output-error", "{message}" }
                            }
                            if let Some(change) = change {
                                {
                                    let view_id = change.id.clone();
                                    let merge_id = change.id;
                                    let merge_job = job.id.clone();
                                    let merge_ordered = ordered.clone();
                                    let merge_changes = changes.clone();
                                    let merge_flag = merging_ref.clone();
                                    rsx! {
                                        div { class: "compare-foot",
                                            span { class: "change-stats",
                                                "{file_label} · "
                                                span { class: "stat-add", "+{insertions}" }
                                                " "
                                                span { class: "stat-del", "−{deletions}" }
                                            }
                                            button {
                                                r#type: "button",
                                                class: "btn btn-quiet btn-compact",
                                                onclick: move |_| on_view_change.call(view_id.clone()),
                                                "View change"
                                            }
                                            button {
                                                r#type: "button",
                                                class: "btn btn-primary btn-compact",
                                                disabled: merging() || either_busy,
                                                title: if either_busy { "Wait for both to finish" } else { "" },
                                                onclick: move |_| {
                                                    merge_column(
                                                        merge_job.clone(),
                                                        merge_id.clone(),
                                                        merge_ordered.clone(),
                                                        merge_changes.clone(),
                                                        merge_flag.clone(),
                                                        merging,
                                                        messages,
                                                        either_busy,
                                                    );
                                                },
                                                if merging() { "Merging…" } else { "Merge this, discard the other" }
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
}

#[allow(clippy::too_many_arguments)]
fn merge_column(
    job_id: String,
    change_id: String,
    ordered: Vec<JobRecord>,
    changes: Vec<ChangeSet>,
    merging_ref: Rc<Cell<bool>>,
    mut merging: Signal<bool>,
    mut messages: Signal<HashMap<String, String>>,
    either_busy: bool,
) {
    if merging_ref.get() || either_busy {
        return;
    }
    merging_ref.set(true);
    merging.set(true);
    let merging_ref = merging_ref.clone();
    spawn_local(async move {
        match api::merge_change(&change_id).await {
            Ok(result) if !result.ok => {
                messages.with_mut(|map| {
                    map.insert(job_id, result.message);
                });
            }
            Ok(_) => {
                messages.with_mut(|map| {
                    map.remove(&job_id);
                });
                for other in &ordered {
                    if other.id == job_id {
                        continue;
                    }
                    if let Some(other_change) = visible_change(other, &changes) {
                        let _ = api::discard_change(&other_change).await;
                    }
                }
            }
            Err(err) => {
                messages.with_mut(|map| {
                    map.insert(job_id, error_text(&err));
                });
            }
        }
        merging_ref.set(false);
        merging.set(false);
    });
}

#[allow(clippy::too_many_arguments)]
fn launch_submit(
    prompt: Signal<String>,
    check: Signal<String>,
    job_type: JobType,
    worker: String,
    access: JobAccess,
    is_repo: bool,
    sending: Rc<Cell<bool>>,
    mut submitting: Signal<bool>,
    mut submit_error: Signal<Option<String>>,
    on_job: Callback<JobRecord>,
    on_select_job: Callback<Option<String>>,
    on_goal: Callback,
) {
    let text = prompt();
    if text.trim().is_empty() || sending.get() {
        return;
    }
    sending.set(true);
    submitting.set(true);
    submit_error.set(None);
    let check_text = check();
    spawn_local(async move {
        let outcome = submit_body(
            &text,
            &check_text,
            job_type,
            &worker,
            access,
            is_repo,
            on_job,
            on_select_job,
            on_goal,
            prompt,
            check,
        )
        .await;
        if let Err(err) = outcome {
            submit_error.set(Some(error_text(&err)));
        }
        sending.set(false);
        submitting.set(false);
    });
}

#[allow(clippy::too_many_arguments)]
async fn submit_body(
    text: &str,
    check_text: &str,
    job_type: JobType,
    worker: &str,
    access: JobAccess,
    is_repo: bool,
    on_job: Callback<JobRecord>,
    on_select_job: Callback<Option<String>>,
    on_goal: Callback,
    mut prompt: Signal<String>,
    mut check: Signal<String>,
) -> Result<(), String> {
    if worker == "both" {
        let group = random_id();
        let job_access = if is_repo { access } else { JobAccess::Read };
        let mut results = Vec::new();
        let mut problem: Option<String> = None;
        for provider in [ProviderId::Claude, ProviderId::Cursor] {
            match api::submit_job(
                job_type,
                text,
                Some(provider),
                Some(job_access),
                Some(&group),
            )
            .await
            {
                Ok(job) => {
                    on_job.call(job.clone());
                    results.push(job);
                }
                Err(err) => problem = Some(error_text(&err)),
            }
        }
        if let Some(first) = results.first() {
            prompt.set(String::new());
            on_select_job.call(Some(first.id.clone()));
        }
        if let Some(problem) = problem {
            return Err(problem);
        }
        return Ok(());
    }
    if !check_text.trim().is_empty() && is_repo {
        let input = crate::wire::GoalInput {
            text: text.to_string(),
            check: Some(check_text.to_string()),
            max_rounds: None,
            quick_type: Some(job_type),
            quick_access: Some(if access == JobAccess::Read {
                JobAccess::Edit
            } else {
                access
            }),
        };
        api::create_goal(&input).await?;
        prompt.set(String::new());
        check.set(String::new());
        on_goal.call(());
        return Ok(());
    }
    let provider = if worker == "auto" {
        None
    } else {
        ProviderId::parse(worker)
    };
    let job_access = if is_repo { access } else { JobAccess::Read };
    let job = api::submit_job(job_type, text, provider, Some(job_access), None).await?;
    prompt.set(String::new());
    on_job.call(job.clone());
    on_select_job.call(Some(job.id));
    Ok(())
}

fn stop_job(
    id: String,
    stopping: Rc<Cell<bool>>,
    mut stopping_id: Signal<Option<String>>,
    mut submit_error: Signal<Option<String>>,
    on_job: Callback<JobRecord>,
) {
    if stopping.get() {
        return;
    }
    stopping.set(true);
    stopping_id.set(Some(id.clone()));
    spawn_local(async move {
        match api::cancel_job(&id).await {
            Ok(job) => {
                on_job.call(job);
                submit_error.set(None);
            }
            Err(err) => submit_error.set(Some(error_text(&err))),
        }
        stopping.set(false);
        stopping_id.set(None);
    });
}

fn worker_options(plans: Option<&[PlanStatus]>, both_ready: bool) -> Vec<SelectOption> {
    let mut options = vec![SelectOption::new("auto", "auto")];
    for id in PROVIDERS {
        options.push(
            SelectOption::new(id.as_str(), id.as_str())
                .disabled(!plan_available(plans, plan_of(id))),
        );
    }
    let both = SelectOption::new("both", "both (compare)").disabled(!both_ready);
    options.push(if both_ready {
        both
    } else {
        both.titled("Both Claude and Cursor need to be ready to compare.")
    });
    options
}

fn as_plan_choice(value: &str) -> String {
    if value == "both" || value == "auto" || is_provider_id(value) {
        value.to_string()
    } else {
        "auto".into()
    }
}

fn as_access(value: &str) -> JobAccess {
    if value == "edit" || value == "full" {
        JobAccess::parse(value).unwrap_or(JobAccess::Read)
    } else {
        JobAccess::Read
    }
}

fn plan_available(plans: Option<&[PlanStatus]>, id: ProviderId) -> bool {
    plans.is_some_and(|list| list.iter().any(|plan| plan.id == id && plan.available))
}

fn plan_rank(provider: Option<ProviderId>) -> usize {
    provider
        .and_then(|id| PROVIDERS.iter().position(|item| *item == id))
        .unwrap_or(PROVIDERS.len())
}

fn job_avatar(status: JobStatus) -> BotState {
    match status {
        JobStatus::Running => BotState::Working,
        JobStatus::Failed => BotState::Error,
        JobStatus::Queued => BotState::Resting,
        JobStatus::Done => BotState::Idle,
    }
}

fn job_status_name(status: JobStatus) -> &'static str {
    match status {
        JobStatus::Queued => "queued",
        JobStatus::Running => "running",
        JobStatus::Done => "done",
        JobStatus::Failed => "failed",
    }
}

fn can_stop(job: &JobRecord) -> bool {
    job.status == JobStatus::Queued || job.status == JobStatus::Running
}

fn access_mark(job: &JobRecord) -> Option<&'static str> {
    if job.access == Some(JobAccess::Full) {
        Some("FULL")
    } else if job.edit == Some(true) || job.access == Some(JobAccess::Edit) {
        Some("EDIT")
    } else {
        None
    }
}

fn output_title(job: &JobRecord) -> String {
    let mut parts = vec![job.job_type.as_str().to_string()];
    if let Some(provider) = job.provider {
        parts.push(provider.as_str().into());
    }
    if let Some(model) = job.model.as_ref().filter(|model| !model.is_empty()) {
        parts.push(model.clone());
    }
    if let Some(reason) = job.reason.as_ref().filter(|reason| !reason.is_empty()) {
        parts.push(reason.clone());
    }
    if let Some(mark) = access_mark(job) {
        parts.push(mark.into());
    }
    parts.join(" · ")
}

fn visible_change(job: &JobRecord, changes: &[ChangeSet]) -> Option<String> {
    let id = job.change.clone()?;
    changes.iter().any(|item| item.id == id).then_some(id)
}

enum MetaPart {
    Text(&'static str),
    Owned(String),
    Reason(String),
    Model(String),
    Id(String),
}

fn meta_parts(job: &JobRecord) -> Vec<MetaPart> {
    let mut parts = vec![MetaPart::Owned(job.job_type.as_str().to_string())];
    if job.group.is_some() {
        parts.push(MetaPart::Text("compare"));
    }
    if job.lead_message.is_some() {
        parts.push(MetaPart::Text("from lead"));
    }
    if job.routine.is_some() {
        parts.push(MetaPart::Text("routine"));
    }
    if let Some(mark) = access_mark(job) {
        parts.push(MetaPart::Text(mark));
    }
    if let Some(reason) = job.reason.as_ref().filter(|reason| !reason.is_empty()) {
        parts.push(MetaPart::Reason(reason.clone()));
    }
    if let Some(model) = job.model.as_ref().filter(|model| !model.is_empty()) {
        parts.push(MetaPart::Model(model.clone()));
    }
    parts.push(MetaPart::Id(job.id.chars().take(8).collect()));
    parts
}

fn meta_line(parts: &[MetaPart]) -> Element {
    rsx! {
        for (index, part) in parts.iter().enumerate() {
            if index > 0 {
                span { key: "sep-{index}", class: "meta-sep", " / " }
            }
            match part {
                MetaPart::Text(text) => rsx! { span { key: "part-{index}", "{text}" } },
                MetaPart::Owned(text) => rsx! { span { key: "part-{index}", "{text}" } },
                MetaPart::Reason(text) => rsx! { span { key: "part-{index}", class: "job-reason-mono", "{text}" } },
                MetaPart::Model(text) => rsx! { span { key: "part-{index}", class: "job-model", "{text}" } },
                MetaPart::Id(text) => rsx! { span { key: "part-{index}", class: "job-id", "{text}" } },
            }
        }
    }
}

fn failover_chain(job: &JobRecord) -> Vec<String> {
    let mut chain: Vec<String> = job
        .failed_over
        .iter()
        .map(|id| id.as_str().to_string())
        .collect();
    chain.push(
        job.provider
            .map(|id| id.as_str().to_string())
            .unwrap_or_else(|| "—".into()),
    );
    chain
}

fn failover_arrow() -> Element {
    rsx! {
        svg {
            class: "failover-arrow",
            width: "10",
            height: "10",
            view_box: "0 0 10 10",
            "aria-hidden": "true",
            "focusable": "false",
            path {
                d: "M1.5 5h5.5M5 2.5 7.5 5 5 7.5",
                fill: "none",
                stroke: "currentColor",
                stroke_width: "1.25",
                stroke_linecap: "round",
                stroke_linejoin: "round",
            }
        }
    }
}

fn load_access() -> JobAccess {
    let Some(storage) = local_storage() else {
        return JobAccess::Read;
    };
    let Ok(Some(value)) = storage.get_item(ACCESS_KEY) else {
        return JobAccess::Read;
    };
    as_access(&value)
}

fn save_access(value: JobAccess) {
    let Some(storage) = local_storage() else {
        return;
    };
    let _ = storage.set_item(ACCESS_KEY, value.as_str());
}

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

fn random_id() -> String {
    let Some(window) = web_sys::window() else {
        return format!("group-{}", js_sys::Date::now() as i64);
    };
    let Ok(crypto) = js_sys::Reflect::get(&window, &wasm_bindgen::JsValue::from_str("crypto"))
    else {
        return format!("group-{}", js_sys::Date::now() as i64);
    };
    let Ok(method) = js_sys::Reflect::get(&crypto, &wasm_bindgen::JsValue::from_str("randomUUID"))
    else {
        return format!("group-{}", js_sys::Date::now() as i64);
    };
    let Some(function) = method.dyn_ref::<js_sys::Function>() else {
        return format!("group-{}", js_sys::Date::now() as i64);
    };
    function
        .call0(&crypto)
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_else(|| format!("group-{}", js_sys::Date::now() as i64))
}

fn scroll_job(id: &str) {
    let Some(element) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(id))
    else {
        return;
    };
    let Ok(method) =
        js_sys::Reflect::get(&element, &wasm_bindgen::JsValue::from_str("scrollIntoView"))
    else {
        return;
    };
    let Some(function) = method.dyn_ref::<js_sys::Function>() else {
        return;
    };
    let options = js_sys::Object::new();
    let _ = js_sys::Reflect::set(
        &options,
        &wasm_bindgen::JsValue::from_str("block"),
        &wasm_bindgen::JsValue::from_str("nearest"),
    );
    let _ = function.call1(&element, &options);
}

fn focus_id(id: &str) {
    let Some(element) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(id))
    else {
        return;
    };
    let Ok(html) = element.dyn_into::<web_sys::HtmlElement>() else {
        return;
    };
    let _ = html.focus();
}
