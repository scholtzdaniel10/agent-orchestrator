//! Saved jobs and the commands a new worktree runs. Matches `Routines.tsx`.

use dioxus::prelude::*;
use legate_core::{JobAccess, JobRecord, JobType, PROVIDERS};
use serde_json::json;
use wasm_bindgen_futures::spawn_local;

use crate::api;
use crate::wire::Routine;

#[derive(Clone, PartialEq, Eq)]
struct Draft {
    id: Option<String>,
    name: String,
    prompt: String,
    job_type: JobType,
    worker: String,
    access: JobAccess,
    after: String,
}

#[component]
pub fn Routines(project: String, on_run: Callback<JobRecord>) -> Element {
    let mut commands = use_signal(String::new);
    let mut routines = use_signal(Vec::<Routine>::new);
    let mut draft = use_signal(|| None::<Draft>);
    let mut form_error = use_signal(|| None::<String>);
    let mut setup_error = use_signal(|| None::<String>);
    let mut load_error = use_signal(|| None::<String>);
    let mut run_error = use_signal(|| None::<String>);
    let mut loading = use_signal(|| true);
    let mut setup_saving = use_signal(|| false);
    let mut saving = use_signal(|| false);
    let mut running = use_signal(|| None::<String>);
    let mut armed = use_signal(|| None::<String>);
    let mut seen_project = use_signal(String::new);
    if seen_project() != project {
        seen_project.set(project.clone());
        let listed = project.clone();
        spawn_local(async move {
            match api::get_project_setup().await {
                Ok(lines) => commands.set(lines.join("\n")),
                Err(text) => setup_error.set(Some(text)),
            }
            match api::list_routines().await {
                Ok(list) => routines.set(list),
                Err(text) => load_error.set(Some(text)),
            }
            loading.set(false);
            let _ = listed;
        });
    }
    let lines: Vec<String> = commands.read().split('\n').map(str::to_string).collect();
    let too_long = lines.len() > 5;
    let list = routines.read().clone();
    let editing = draft.read().clone();
    let load_notice = load_error.read().clone();
    let setup_notice = setup_error.read().clone();
    let run_notice = run_error.read().clone();
    let form_notice = form_error.read().clone();

    rsx! {
        div { class: "routines-pane",
            section { class: "routines-setup", aria_label: "Worktree setup",
                h3 { class: "routines-heading", "Worktree setup" }
                label { class: "sr-only", r#for: "setup-commands", "Setup commands" }
                textarea {
                    id: "setup-commands",
                    rows: "5",
                    value: "{commands}",
                    placeholder: "pnpm install",
                    oninput: move |event| commands.set(event.value()),
                }
                div { class: "routines-setup-foot",
                    p { class: "hint", "Runs in each worktree you create by hand, in a terminal you can watch." }
                    button {
                        class: "btn btn-primary",
                        r#type: "button",
                        disabled: setup_saving(),
                        onclick: move |_| {
                            if setup_saving() { return; }
                            let lines: Vec<String> = commands
                                .read()
                                .split('\n')
                                .map(str::trim)
                                .filter(|line| !line.is_empty())
                                .take(5)
                                .map(str::to_string)
                                .collect();
                            setup_saving.set(true);
                            spawn_local(async move {
                                match api::set_project_setup(&lines).await {
                                    Ok(()) => {
                                        commands.set(lines.join("\n"));
                                        setup_error.set(None);
                                    }
                                    Err(text) => setup_error.set(Some(text)),
                                }
                                setup_saving.set(false);
                            });
                        },
                        if setup_saving() { "Saving…" } else { "Save" }
                    }
                }
                if too_long {
                    p { class: "error-text", role: "alert", "At most 5 commands, one per line." }
                }
                if let Some(text) = setup_notice {
                    p { class: "field-error", role: "alert", "{text}" }
                }
            }
            section { class: "routines-list", aria_label: "Routines",
                if loading() {
                    p { class: "hint", "Loading routines…" }
                }
                if let Some(text) = load_notice {
                    p { class: "error-text", role: "alert", "{text}" }
                }
                if !loading() && list.is_empty() && editing.is_none() {
                    p { class: "hint", "A routine is a saved job you run with one click, or after a job of a chosen type finishes." }
                }
                for routine in list {
                    {
                        let id = routine.id.clone();
                        let run_id = id.clone();
                        let edit = routine.clone();
                        let remove_id = id.clone();
                        let after = routine.after.as_ref().map(|item| format!(" · after every {} job", item.job_type.as_str())).unwrap_or_default();
                        rsx! {
                            div { class: "job-row", key: "{id}",
                                div { class: "job-main",
                                    span { class: "job-title", "{routine.name}" }
                                }
                                div { class: "job-meta", "{routine.job_type.as_str()} · {routine.worker} · {routine.access.as_str()}{after}" }
                                div { class: "routines-row-actions",
                                    button {
                                        class: "btn btn-quiet btn-compact",
                                        r#type: "button",
                                        disabled: running.read().is_some(),
                                        onclick: move |_| {
                                            let id = run_id.clone();
                                            running.set(Some(id.clone()));
                                            spawn_local(async move {
                                                match api::run_routine(&id).await {
                                                    Ok(job) => {
                                                        run_error.set(None);
                                                        on_run.call(job);
                                                    }
                                                    Err(text) => run_error.set(Some(text)),
                                                }
                                                running.set(None);
                                            });
                                        },
                                        if running.read().as_deref() == Some(id.as_str()) { "Running…" } else { "Run" }
                                    }
                                    button {
                                        class: "btn btn-quiet btn-compact",
                                        r#type: "button",
                                        onclick: move |_| {
                                            draft.set(Some(Draft {
                                                id: Some(edit.id.clone()),
                                                name: edit.name.clone(),
                                                prompt: edit.prompt.clone(),
                                                job_type: edit.job_type,
                                                worker: edit.worker.clone(),
                                                access: edit.access,
                                                after: edit.after.as_ref().map(|item| item.job_type.as_str().to_string()).unwrap_or_default(),
                                            }));
                                            form_error.set(None);
                                        },
                                        "Edit"
                                    }
                                    button {
                                        class: if armed.read().as_deref() == Some(remove_id.as_str()) { "btn btn-quiet btn-compact is-danger-label" } else { "btn btn-quiet btn-compact" },
                                        r#type: "button",
                                        onclick: move |_| {
                                            if armed.read().as_deref() != Some(remove_id.as_str()) {
                                                armed.set(Some(remove_id.clone()));
                                                return;
                                            }
                                            let id = remove_id.clone();
                                            armed.set(None);
                                            spawn_local(async move {
                                                if let Err(text) = api::remove_routine(&id).await {
                                                    load_error.set(Some(text));
                                                    return;
                                                }
                                                if let Ok(list) = api::list_routines().await {
                                                    routines.set(list);
                                                }
                                            });
                                        },
                                        if armed.read().as_deref() == Some(id.as_str()) { "Delete?" } else { "Delete" }
                                    }
                                }
                            }
                        }
                    }
                }
                if let Some(text) = run_notice {
                    p { class: "error-text", role: "alert", "{text}" }
                }
                if editing.is_none() {
                    button {
                        class: "btn btn-quiet",
                        r#type: "button",
                        onclick: move |_| {
                            draft.set(Some(Draft {
                                id: None,
                                name: String::new(),
                                prompt: String::new(),
                                job_type: JobType::Planning,
                                worker: "auto".into(),
                                access: JobAccess::Read,
                                after: String::new(),
                            }));
                            form_error.set(None);
                        },
                        "New routine"
                    }
                } else if let Some(current) = editing {
                    form {
                        class: "routines-form",
                        onsubmit: move |event| {
                            event.prevent_default();
                            if saving() { return; }
                            let Some(current) = draft.read().clone() else { return; };
                            saving.set(true);
                            spawn_local(async move {
                                let id = current.id.clone().unwrap_or_else(fresh_id);
                                let mut body = json!({
                                    "id": id,
                                    "name": current.name,
                                    "prompt": current.prompt,
                                    "type": current.job_type,
                                    "worker": current.worker,
                                    "access": current.access,
                                });
                                if let Some(kind) = JobType::parse(&current.after) {
                                    body["after"] = json!({ "jobType": kind });
                                }
                                match api::save_routine(&body).await {
                                    Ok(()) => {
                                        if let Ok(list) = api::list_routines().await {
                                            routines.set(list);
                                        }
                                        draft.set(None);
                                        form_error.set(None);
                                    }
                                    Err(text) => form_error.set(Some(text)),
                                }
                                saving.set(false);
                            });
                        },
                        label { r#for: "routine-name", "Name" }
                        input {
                            id: "routine-name",
                            r#type: "text",
                            value: "{current.name}",
                            maxlength: "40",
                            oninput: move |event| {
                                let value = event.value();
                                update_draft(draft, |item| item.name = value);
                            },
                        }
                        label { r#for: "routine-prompt", "Prompt" }
                        textarea {
                            id: "routine-prompt",
                            rows: "4",
                            value: "{current.prompt}",
                            maxlength: "8000",
                            oninput: move |event| {
                                let value = event.value();
                                update_draft(draft, |item| item.prompt = value);
                            },
                        }
                        div { class: "composer-tools",
                            label { r#for: "routine-type", "Job type" }
                            select {
                                id: "routine-type",
                                class: "select job-type",
                                onchange: move |event| {
                                    let next = JobType::parse(&event.value()).unwrap_or(JobType::Planning);
                                    update_draft(draft, |item| item.job_type = next);
                                },
                                for kind in JobType::ALL {
                                    option { value: "{kind.as_str()}", selected: current.job_type == kind, "{kind.as_str()}" }
                                }
                            }
                            label { r#for: "routine-worker", "Worker" }
                            select {
                                id: "routine-worker",
                                class: "select plan-choice",
                                onchange: move |event| {
                                    let next = event.value();
                                    update_draft(draft, |item| item.worker = next);
                                },
                                option { value: "auto", selected: current.worker == "auto", "auto" }
                                for id in PROVIDERS {
                                    option { value: "{id.as_str()}", selected: current.worker == id.as_str(), "{id.as_str()}" }
                                }
                            }
                            label { r#for: "routine-access", "Access" }
                            select {
                                id: "routine-access",
                                class: "select job-access",
                                onchange: move |event| {
                                    let next = JobAccess::parse(&event.value()).unwrap_or(JobAccess::Read);
                                    update_draft(draft, |item| item.access = next);
                                },
                                option { value: "read", selected: current.access == JobAccess::Read, "Read only" }
                                option { value: "edit", selected: current.access == JobAccess::Edit, "Edit files" }
                                option { value: "full", selected: current.access == JobAccess::Full, "Full access" }
                            }
                            label { r#for: "routine-after", "Run automatically" }
                            select {
                                id: "routine-after",
                                class: "select",
                                onchange: move |event| {
                                    let next = event.value();
                                    update_draft(draft, |item| item.after = next);
                                },
                                option { value: "", selected: current.after.is_empty(), "only when I click Run" }
                                for kind in JobType::ALL {
                                    option { value: "{kind.as_str()}", selected: current.after == kind.as_str(), "after every {kind.as_str()} job" }
                                }
                            }
                        }
                        div { class: "routines-form-actions",
                            button { r#type: "submit", class: "btn btn-primary", disabled: saving(), if saving() { "Saving…" } else { "Save" } }
                            button {
                                r#type: "button",
                                class: "btn btn-quiet",
                                onclick: move |_| {
                                    draft.set(None);
                                    form_error.set(None);
                                },
                                "Cancel"
                            }
                        }
                        if let Some(text) = form_notice {
                            p { class: "field-error", role: "alert", "{text}" }
                        }
                    }
                }
            }
        }
    }
}

fn update_draft(mut draft: Signal<Option<Draft>>, change: impl FnOnce(&mut Draft)) {
    draft.with_mut(|slot| {
        if let Some(item) = slot {
            change(item);
        }
    });
}

fn fresh_id() -> String {
    format!(
        "routine-{}-{}",
        js_sys::Date::now() as u64,
        (js_sys::Math::random() * 1_000_000_000.0) as u64
    )
}
