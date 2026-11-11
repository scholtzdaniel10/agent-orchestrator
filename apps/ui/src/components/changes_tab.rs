//! Changes waiting for review. Port of `ChangesTab.tsx`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use dioxus::prelude::*;
use gloo_timers::callback::Timeout;
use legate_core::{ChangeSet, ChangedFile, JobRecord, JobStatus, ProviderId, plan_of};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use super::files_panel::{DiffLineList, bridge_error, document_element};
use crate::api;
use crate::components::star::RayBurst;
use crate::logic::worktree_label::{NamedChange, worktree_label};

const WORKERS: [ProviderId; 4] = [
    ProviderId::Claude,
    ProviderId::Codex,
    ProviderId::Cursor,
    ProviderId::Grok,
];

#[derive(Clone, PartialEq, Eq)]
struct Notice {
    id: String,
    ok: bool,
    text: String,
}

fn worker_label(id: ProviderId) -> &'static str {
    match id {
        ProviderId::Claude => "Claude",
        ProviderId::Codex => "Codex",
        ProviderId::Grok => "Grok",
        ProviderId::Cursor | ProviderId::Local => "Cursor",
    }
}

fn job_type_for(jobs: &[JobRecord], id: &str) -> String {
    jobs.iter()
        .find(|job| job.change.as_deref() == Some(id))
        .map(|job| job.job_type.as_str().to_string())
        .unwrap_or_else(|| "earlier session".to_string())
}

fn worktree_job_running(jobs: &[JobRecord], id: &str) -> bool {
    jobs.iter().any(|job| {
        job.change.as_deref() == Some(id)
            && matches!(job.status, JobStatus::Queued | JobStatus::Running)
    })
}

fn file_label(count: usize) -> String {
    if count == 1 {
        "1 file".to_string()
    } else {
        format!("{count} files")
    }
}

fn title_from_prompt(prompt: &str) -> String {
    let line = prompt
        .split_once('\n')
        .map(|(head, _)| head)
        .unwrap_or(prompt);
    let line = line.strip_suffix('\r').unwrap_or(line).trim();
    utf16_prefix(line, 72)
}

fn js_len(text: &str) -> usize {
    text.encode_utf16().count()
}

fn utf16_prefix(text: &str, max: usize) -> String {
    String::from_utf16_lossy(&text.encode_utf16().take(max).collect::<Vec<_>>())
}

fn worker_shown(
    id: ProviderId,
    claude_available: bool,
    codex_available: bool,
    cursor_available: bool,
) -> bool {
    if id == ProviderId::Codex {
        return codex_available;
    }
    if plan_of(id) == ProviderId::Claude {
        claude_available
    } else {
        cursor_available
    }
}

fn label_of(change: &ChangeSet) -> String {
    worktree_label(&NamedChange {
        id: change.id.clone(),
        name: change.name.clone(),
    })
}

#[component]
pub fn ChangesTab(
    changes: Vec<ChangeSet>,
    jobs: Vec<JobRecord>,
    selected_id: Option<String>,
    on_select: Callback<Option<String>>,
    claude_available: bool,
    codex_available: bool,
    cursor_available: bool,
    opening: bool,
    on_open_here: Callback<(ProviderId, String)>,
) -> Element {
    let busy = use_signal(|| false);
    let armed_id = use_signal(|| None::<String>);
    let notice = use_signal(|| None::<Notice>);
    let mut github_on = use_signal(|| false);
    let merged_ok = use_signal(|| false);
    let mut pr_form = use_signal(|| false);
    let mut pr_title = use_signal(String::new);
    let mut pr_body = use_signal(String::new);
    let pr_opening = use_signal(|| false);
    let pr_url = use_signal(|| None::<String>);
    let mut pr_error = use_signal(|| None::<String>);
    let mut list_seen = use_signal(|| changes.clone());
    let mut list_tick = use_signal(|| 0_u32);
    let busy_flag = use_hook(|| Rc::new(Cell::new(false)));
    let armed_timer = use_hook(|| Rc::new(RefCell::new(None::<Timeout>)));
    let notice_timer = use_hook(|| Rc::new(RefCell::new(None::<Timeout>)));

    if list_seen.peek().as_slice() != changes.as_slice() {
        list_seen.set(changes.clone());
        let next_tick = list_tick.peek().saturating_add(1);
        list_tick.set(next_tick);
    }

    let valid = selected_id
        .as_ref()
        .is_some_and(|id| changes.iter().any(|item| &item.id == id));
    let desired = if valid {
        selected_id.clone()
    } else {
        changes.first().map(|item| item.id.clone())
    };
    let resolved = desired.clone();
    let selected = changes
        .iter()
        .find(|item| Some(&item.id) == resolved.as_ref())
        .cloned();
    let mut track_selected = use_signal(|| selected_id.clone());
    if track_selected.peek().as_ref() != selected_id.as_ref() {
        track_selected.set(selected_id.clone());
    }
    let id_list: Vec<String> = changes.iter().map(|item| item.id.clone()).collect();
    let mut track_ids = use_signal(|| id_list.clone());
    if track_ids.peek().as_slice() != id_list.as_slice() {
        track_ids.set(id_list);
    }
    use_effect(move || {
        let current = track_selected();
        let ids = track_ids();
        let present = current
            .as_ref()
            .is_some_and(|id| ids.iter().any(|item| item == id));
        if !present {
            on_select.call(ids.first().cloned());
        }
    });

    let armed_slot = armed_timer.clone();
    use_effect(move || {
        let armed = armed_id.read().clone();
        *armed_slot.borrow_mut() = None;
        if armed.is_none() {
            return;
        }
        let mut clear = armed_id;
        *armed_slot.borrow_mut() = Some(Timeout::new(4000, move || clear.set(None)));
    });

    let notice_slot = notice_timer.clone();
    use_effect(move || {
        let current = notice.read().clone();
        *notice_slot.borrow_mut() = None;
        let Some(current) = current else {
            return;
        };
        if !current.ok || (github_on() && merged_ok()) {
            return;
        }
        let mut clear = notice;
        *notice_slot.borrow_mut() = Some(Timeout::new(5000, move || clear.set(None)));
    });

    use_effect(move || {
        spawn_local(async move {
            let ok = api::github_available().await.unwrap_or(false);
            github_on.set(ok);
        });
    });

    use_effect(move || {
        if !pr_form() {
            return;
        }
        if let Some(el) = document_element("pr-title") {
            let html: &web_sys::HtmlElement = el.unchecked_ref();
            let _ = html.focus();
        }
    });

    let confirming = selected
        .as_ref()
        .is_some_and(|item| armed_id.read().as_ref() == Some(&item.id));
    let job_running = selected
        .as_ref()
        .is_some_and(|item| worktree_job_running(&jobs, &item.id));
    let ok_notice = notice
        .read()
        .as_ref()
        .filter(|item| item.ok)
        .map(|item| item.text.clone());
    let bad_notice = notice.read().as_ref().and_then(|item| {
        if !item.ok && selected.as_ref().is_some_and(|change| change.id == item.id) {
            Some(item.text.clone())
        } else {
            None
        }
    });
    let tick = list_tick();
    let show_pr = github_on() && merged_ok() && pr_url.read().is_none() && !pr_form();
    let title_len = js_len(&pr_title.read());
    let merge_id = selected
        .as_ref()
        .map(|item| item.id.clone())
        .unwrap_or_default();
    let discard_id = merge_id.clone();
    let open_id = merge_id.clone();
    let jobs_for_merge = jobs.clone();
    let merge_flag = busy_flag.clone();
    let discard_flag = busy_flag.clone();
    let workers: Vec<ProviderId> = WORKERS
        .into_iter()
        .filter(|id| worker_shown(*id, claude_available, codex_available, cursor_available))
        .collect();

    rsx! {
        div { class: "changes-pane",
            if let Some(text) = ok_notice {
                p { class: "change-ok", role: "status", "{text}" }
            }
            if let Some(url) = pr_url.read().as_ref() {
                p { class: "change-ok", role: "status",
                    "Pull request opened "
                    a { href: "{url}", target: "_blank", rel: "noreferrer", "{url}" }
                }
            }
            if show_pr {
                div { class: "change-pr",
                    button {
                        r#type: "button",
                        class: "btn btn-primary",
                        disabled: busy() || pr_opening(),
                        onclick: move |_| pr_form.set(true),
                        "Open pull request"
                    }
                }
            }
            if pr_form() && pr_url.read().is_none() {
                form {
                    class: "change-pr-form",
                    onsubmit: move |event| {
                        event.prevent_default();
                        submit_pr(pr_title, pr_body, pr_opening, pr_error, pr_url, pr_form);
                    },
                    onkeydown: move |event| {
                        if event.key() == Key::Escape {
                            event.prevent_default();
                            if !pr_opening() {
                                pr_form.set(false);
                                pr_error.set(None);
                            }
                        }
                    },
                    div { class: "change-pr-field",
                        label { r#for: "pr-title", "Title" }
                        input {
                            id: "pr-title",
                            r#type: "text",
                            value: "{pr_title}",
                            maxlength: 120,
                            disabled: pr_opening(),
                            oninput: move |event| pr_title.set(event.value()),
                        }
                    }
                    div { class: "change-pr-field",
                        label { r#for: "pr-body", "Description" }
                        textarea {
                            id: "pr-body",
                            value: "{pr_body}",
                            maxlength: 4000,
                            rows: 4,
                            disabled: pr_opening(),
                            oninput: move |event| pr_body.set(event.value()),
                        }
                    }
                    div { class: "change-actions",
                        button {
                            r#type: "submit",
                            class: "btn btn-primary",
                            disabled: pr_opening() || !(1..=120).contains(&title_len),
                            "Open pull request"
                        }
                        button {
                            r#type: "button",
                            class: "btn btn-quiet",
                            disabled: pr_opening(),
                            onclick: move |_| {
                                if !pr_opening() {
                                    pr_form.set(false);
                                    pr_error.set(None);
                                }
                            },
                            "Cancel"
                        }
                    }
                    if pr_opening() {
                        p { class: "change-help", "Opening pull request\u{2026}" }
                    }
                    if let Some(message) = pr_error.read().as_ref() {
                        p { class: "field-error", role: "alert", "{message}" }
                    }
                }
            }
            if changes.is_empty() || selected.is_none() {
                div { class: "changes-empty",
                    RayBurst {}
                    p { class: "hint", "No changes waiting. Tick \"Let it edit files\" on a job to get one." }
                }
            } else if let Some(selected) = selected.clone() {
                div { class: "changes",
                    div { role: "listbox", aria_label: "Changes", class: "change-list",
                        for (index, item) in changes.iter().cloned().enumerate() {
                            ChangeOption {
                                key: "{item.id}",
                                id: item.id.clone(),
                                name: label_of(&item),
                                origin: job_type_for(&jobs, &item.id),
                                setup: item.setup,
                                file_count: item.files.len(),
                                insertions: item.insertions,
                                deletions: item.deletions,
                                is_selected: resolved.as_deref() == Some(item.id.as_str()),
                                next_id: changes.get(index + 1).map(|row| row.id.clone()),
                                prev_id: index.checked_sub(1).and_then(|slot| changes.get(slot).map(|row| row.id.clone())),
                                on_select: on_select,
                            }
                        }
                    }
                    div { class: "change-detail",
                        div { class: "change-head",
                            span { class: "change-branch", "{label_of(&selected)}" }
                            if selected.files.is_empty() {
                                span { class: "change-stats", "No changes yet" }
                            } else {
                                ChangeStats { files: selected.files.len(), insertions: selected.insertions, deletions: selected.deletions }
                            }
                        }
                        div { class: "change-actions",
                            if !selected.files.is_empty() {
                                button {
                                    r#type: "button",
                                    class: "btn btn-primary",
                                    disabled: busy() || job_running,
                                    title: if job_running { "job running" } else { "" },
                                    onclick: move |_| merge_selected(merge_id.clone(), jobs_for_merge.clone(), merge_flag.clone(), busy, notice, armed_id, merged_ok, pr_form, pr_url, pr_error, pr_body, pr_title),
                                    "Merge"
                                }
                            }
                            button {
                                r#type: "button",
                                class: if confirming { "btn btn-quiet is-danger-label" } else { "btn btn-quiet" },
                                disabled: busy() || job_running,
                                title: if job_running { "job running" } else { "" },
                                onclick: move |_| discard_selected(discard_id.clone(), discard_flag.clone(), busy, notice, armed_id),
                                if confirming { "Discard for good?" } else { "Discard" }
                            }
                            if job_running {
                                span { class: "change-help", "job running" }
                            }
                            for id in workers {
                                OpenHere {
                                    key: "{id.as_str()}",
                                    provider: id,
                                    change_id: open_id.clone(),
                                    opening: opening,
                                    busy: busy,
                                    on_open_here: on_open_here,
                                }
                            }
                        }
                        if !selected.files.is_empty() {
                            p { class: "change-help", "Merge applies this to your project as staged changes. Nothing is committed." }
                        }
                        if let Some(message) = bad_notice {
                            p { class: "error-text", role: "alert", "{message}" }
                        }
                        if !selected.files.is_empty() {
                            ul { class: "change-files",
                                for file in selected.files.iter().cloned() {
                                    li { key: "{file.path}", class: "change-file",
                                        span { class: "change-path", "{file.path}" }
                                        FileStat { file: file.clone() }
                                    }
                                }
                            }
                            DiffPane { id: selected.id.clone(), tick: tick }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ChangeOption(
    id: String,
    name: String,
    origin: String,
    setup: Option<legate_core::SetupState>,
    file_count: usize,
    insertions: i64,
    deletions: i64,
    is_selected: bool,
    next_id: Option<String>,
    prev_id: Option<String>,
    on_select: Callback<Option<String>>,
) -> Element {
    let click_id = id.clone();
    let key_id = id.clone();
    let row_class = if is_selected {
        "job-row is-selected"
    } else {
        "job-row"
    };
    let tab = if is_selected { "0" } else { "-1" };
    rsx! {
        div {
            id: "change-{id}",
            role: "option",
            aria_selected: if is_selected { "true" } else { "false" },
            tabindex: "{tab}",
            class: "{row_class}",
            onclick: move |_| on_select.call(Some(click_id.clone())),
            onkeydown: move |event| {
                let key_name = event.key();
                if key_name == Key::Enter || key_name.to_string() == " " {
                    event.prevent_default();
                    on_select.call(Some(key_id.clone()));
                    return;
                }
                if key_name != Key::ArrowDown && key_name != Key::ArrowUp {
                    return;
                }
                event.prevent_default();
                let next = if key_name == Key::ArrowDown { next_id.clone() } else { prev_id.clone() };
                if let Some(next) = next {
                    on_select.call(Some(next.clone()));
                    if let Some(el) = document_element(&format!("change-{next}")) {
                        let html: &web_sys::HtmlElement = el.unchecked_ref();
                        let _ = html.focus();
                    }
                }
            },
            span { class: "change-id", "{name}" }
            span { class: "change-origin", "{origin}" }
            if setup == Some(legate_core::SetupState::Running) {
                span { class: "change-origin", "setting up\u{2026}" }
            }
            if setup == Some(legate_core::SetupState::Failed) {
                span { class: "setup-failed", "setup failed" }
            }
            if file_count == 0 {
                span { class: "change-stats", "No changes yet" }
            } else {
                ChangeStats { files: file_count, insertions: insertions, deletions: deletions }
            }
        }
    }
}

#[component]
fn OpenHere(
    provider: ProviderId,
    change_id: String,
    opening: bool,
    busy: Signal<bool>,
    on_open_here: Callback<(ProviderId, String)>,
) -> Element {
    let label = worker_label(provider);
    rsx! {
        button {
            r#type: "button",
            class: "btn btn-quiet btn-compact",
            disabled: opening || busy(),
            onclick: move |_| on_open_here.call((provider, change_id.clone())),
            "{label} here"
        }
    }
}

#[component]
fn ChangeStats(files: usize, insertions: i64, deletions: i64) -> Element {
    let files = file_label(files);
    rsx! {
        span { class: "change-stats",
            "{files} \u{00b7} "
            span { class: "stat-add", "+{insertions}" }
            " "
            span { class: "stat-del", "\u{2212}{deletions}" }
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

#[component]
fn DiffPane(id: String, tick: u32) -> Element {
    let mut text = use_signal(|| None::<String>);
    let mut error = use_signal(|| None::<String>);
    let mut loading = use_signal(|| true);
    let mut seen = use_signal(|| (String::new(), 0_u32));
    let next = (id.clone(), tick);
    if seen.peek().clone() != next {
        seen.set(next);
    }
    let ticket = use_hook(|| Rc::new(Cell::new(0_u64)));
    use_effect(move || {
        let (id, _tick) = seen();
        let id_now = ticket.get().saturating_add(1);
        ticket.set(id_now);
        loading.set(true);
        error.set(None);
        let ticket = ticket.clone();
        spawn_local(async move {
            match api::change_diff(&id).await {
                Ok(next) => {
                    if ticket.get() == id_now {
                        text.set(Some(next));
                        loading.set(false);
                    }
                }
                Err(err) => {
                    if ticket.get() == id_now {
                        error.set(Some(bridge_error(&err)));
                        loading.set(false);
                    }
                }
            }
        });
    });
    let shown = text.read().clone();
    let failed = error.read().clone();
    rsx! {
        div { class: "diff-block", role: "region", aria_label: "Diff", aria_busy: "{loading()}",
            if loading() {
                p { class: "diff-status", "Loading diff\u{2026}" }
            }
            if !loading() {
                if let Some(message) = failed {
                    p { class: "error-text", role: "alert", "{message}" }
                } else if let Some(body) = shown {
                    DiffLineList { text: body, jump_nonce: 0, jump_path: String::new() }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn merge_selected(
    id: String,
    jobs: Vec<JobRecord>,
    busy_flag: Rc<Cell<bool>>,
    mut busy: Signal<bool>,
    mut notice: Signal<Option<Notice>>,
    mut armed_id: Signal<Option<String>>,
    mut merged_ok: Signal<bool>,
    mut pr_form: Signal<bool>,
    mut pr_url: Signal<Option<String>>,
    mut pr_error: Signal<Option<String>>,
    mut pr_body: Signal<String>,
    mut pr_title: Signal<String>,
) {
    if busy_flag.get() {
        return;
    }
    busy_flag.set(true);
    busy.set(true);
    notice.set(None);
    armed_id.set(None);
    let prompt = jobs
        .iter()
        .find(|job| job.change.as_deref() == Some(&id))
        .map(|job| job.prompt.clone())
        .unwrap_or_default();
    spawn_local(async move {
        match api::merge_change(&id).await {
            Ok(result) => {
                notice.set(Some(Notice {
                    id: id.clone(),
                    ok: result.ok,
                    text: result.message,
                }));
                if result.ok {
                    merged_ok.set(true);
                    pr_form.set(false);
                    pr_url.set(None);
                    pr_error.set(None);
                    pr_body.set(String::new());
                    pr_title.set(title_from_prompt(&prompt));
                }
            }
            Err(err) => notice.set(Some(Notice {
                id,
                ok: false,
                text: bridge_error(&err),
            })),
        }
        busy_flag.set(false);
        busy.set(false);
    });
}

fn discard_selected(
    id: String,
    busy_flag: Rc<Cell<bool>>,
    mut busy: Signal<bool>,
    mut notice: Signal<Option<Notice>>,
    mut armed_id: Signal<Option<String>>,
) {
    if busy_flag.get() {
        return;
    }
    if armed_id.peek().as_ref() != Some(&id) {
        armed_id.set(Some(id));
        return;
    }
    armed_id.set(None);
    busy_flag.set(true);
    busy.set(true);
    notice.set(None);
    spawn_local(async move {
        match api::discard_change(&id).await {
            Ok(()) => {}
            Err(err) => notice.set(Some(Notice {
                id,
                ok: false,
                text: bridge_error(&err),
            })),
        }
        busy_flag.set(false);
        busy.set(false);
    });
}

fn submit_pr(
    pr_title: Signal<String>,
    pr_body: Signal<String>,
    mut pr_opening: Signal<bool>,
    mut pr_error: Signal<Option<String>>,
    mut pr_url: Signal<Option<String>>,
    mut pr_form: Signal<bool>,
) {
    if *pr_opening.peek() {
        return;
    }
    let title = pr_title.peek().clone();
    let body = pr_body.peek().clone();
    let len = js_len(&title);
    if !(1..=120).contains(&len) {
        return;
    }
    pr_opening.set(true);
    pr_error.set(None);
    spawn_local(async move {
        match api::github_open_pr(&title, &body).await {
            Ok(opened) => {
                pr_url.set(Some(opened.url));
                pr_form.set(false);
            }
            Err(err) => pr_error.set(Some(bridge_error(&err))),
        }
        pr_opening.set(false);
    });
}
