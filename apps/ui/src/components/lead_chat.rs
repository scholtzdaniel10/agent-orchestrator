use std::cell::{Cell, RefCell};
use std::rc::Rc;

use dioxus::html::{Key, Modifiers};
use dioxus::prelude::*;
use legate_core::{
    CursorModels, JobAccess, JobRecord, JobStatus, LeadMessage, LeadMessageKind, LeadRole,
    LeadStatus, ModelOption, PROVIDERS, PlanStatus, ProposalStatus, ProposedJob, ProviderId,
    is_provider_id, plan_of,
};
use wasm_bindgen_futures::spawn_local;

use super::bot_avatar::{BotAvatar, BotName, BotState, FieldSelect, SelectOption, error_text};
use super::lead_plan::LeadPlan;
use super::rich_text::RichText;
use super::sidebar_menu::{MenuEntry, SidebarMenu};
use super::star::{RayBurst, Star};
use crate::api::{self, LeadUpdate};
use crate::logic::job_steps::{edited_files_summary, latest_step_title};
use crate::state::{self, use_store};

#[component]
pub fn LeadChat(
    messages: Vec<LeadMessage>,
    jobs: Vec<JobRecord>,
    plans: Option<Vec<PlanStatus>>,
    mode: Option<ProviderId>,
    on_reset: Callback<Option<ProviderId>>,
    on_open_terminal: Callback<ProviderId>,
    on_show_job: Callback<String>,
    on_run_plan: Callback<(String, Vec<ProposedJob>)>,
    settings_nonce: i64,
) -> Element {
    let store = use_store();
    let mut draft = use_signal(String::new);
    let send_error = use_signal(|| None::<String>);
    let mut choice = use_signal(|| "auto".to_string());
    let mut plan_error = use_signal(|| None::<String>);
    let mut worker_access = use_signal(|| JobAccess::Read);
    let mut lead_model = use_signal(|| None::<LeadModels>);
    let mut model_error = use_signal(|| None::<String>);
    let mut new_menu = use_signal(|| None::<(f64, f64)>);
    let mut live = use_signal(|| messages.clone());
    let mut seen_props = use_signal(|| messages.clone());
    let mut mode_live = use_signal(|| mode);
    let action_error = use_signal(|| None::<(String, String)>);
    let access_ref = use_hook(|| Rc::new(Cell::new(JobAccess::Read)));
    let choice_ref = use_hook(|| Rc::new(RefCell::new("auto".to_string())));
    let picked = use_hook(|| Rc::new(Cell::new(false)));
    let sending = use_hook(|| Rc::new(Cell::new(false)));
    let resetting = use_hook(|| Rc::new(Cell::new(false)));
    let load_ticket = use_hook(|| Rc::new(Cell::new(0u64)));

    if messages != seen_props() && !messages.is_empty() {
        seen_props.set(messages.clone());
        live.set(messages.clone());
    }
    let mut nonce = use_signal(|| settings_nonce);
    if nonce() != settings_nonce {
        nonce.set(settings_nonce);
    }

    {
        let load_ticket = load_ticket.clone();
        let picked = picked.clone();
        let choice_ref = choice_ref.clone();
        let access_ref = access_ref.clone();
        use_effect(move || {
            let fresh = nonce() > 0;
            let ticket = load_ticket.get() + 1;
            load_ticket.set(ticket);
            let picked = picked.clone();
            let choice_ref = choice_ref.clone();
            let plan_ticket = load_ticket.clone();
            spawn_local(async move {
                match api::get_lead_plan().await {
                    Ok(plan) => {
                        if plan_ticket.get() != ticket || (picked.get() && !fresh) {
                            return;
                        }
                        let value = plan
                            .map(|id| id.as_str().to_string())
                            .unwrap_or_else(|| "auto".into());
                        *choice_ref.borrow_mut() = value.clone();
                        choice.set(value);
                    }
                    Err(err) => {
                        if plan_ticket.get() == ticket && !picked.get() {
                            plan_error.set(Some(error_text(&err)));
                        }
                    }
                }
            });
            let access_ticket = load_ticket.clone();
            let access_ref = access_ref.clone();
            spawn_local(async move {
                match api::get_lead_access().await {
                    Ok(access) => {
                        if access_ticket.get() != ticket {
                            return;
                        }
                        access_ref.set(access);
                        worker_access.set(access);
                    }
                    Err(err) => {
                        if access_ticket.get() == ticket {
                            plan_error.set(Some(error_text(&err)));
                        }
                    }
                }
            });
        });
    }

    {
        let model_ticket = use_hook(|| Rc::new(Cell::new(0u64)));
        let model_ticket = model_ticket.clone();
        use_effect(move || {
            let selected = choice();
            let _ = nonce();
            if selected == "auto" {
                return;
            }
            let Some(provider) = ProviderId::parse(&selected) else {
                return;
            };
            let ticket = model_ticket.get() + 1;
            model_ticket.set(ticket);
            let model_ticket = model_ticket.clone();
            spawn_local(async move {
                match tokio_join_models(provider).await {
                    Ok((options, value)) => {
                        if model_ticket.get() == ticket {
                            lead_model.set(Some(LeadModels {
                                plan: provider,
                                options,
                                value,
                            }));
                        }
                    }
                    Err(err) => {
                        if model_ticket.get() == ticket {
                            model_error.set(Some(error_text(&err)));
                        }
                    }
                }
            });
        });
    }

    use_effect(move || {
        spawn_local(async move {
            if let Ok(list) = api::list_lead_messages().await {
                live.with_mut(|current| *current = merge_messages(list, current));
            }
            if let Ok(direct) = api::get_lead_mode().await {
                mode_live.set(direct);
            }
        });
        spawn_local(async move {
            let _ = api::on_lead(move |LeadUpdate { message, .. }| {
                live.with_mut(|list| upsert_message(list, message));
            })
            .await;
        });
    });

    let shown = live.read().clone();
    let mut stamp_sig = use_signal(String::new);
    let stamp = message_stamp(&shown);
    if stamp_sig() != stamp {
        stamp_sig.set(stamp);
    }
    use_effect(move || {
        let _ = stamp_sig();
        scroll_bottom("lead-log");
    });

    let direct = mode_live();
    let lead_streaming = shown
        .iter()
        .any(|message| message.role == LeadRole::Lead && message.status == LeadStatus::Streaming);
    let draft_empty = draft.read().trim().is_empty();
    let no_plan_ready = plans.as_ref().is_some_and(|list| match direct {
        None => !list.iter().any(|plan| plan.available),
        Some(id) => !plan_available(Some(list), id),
    });
    let send_disabled = draft_empty || lead_streaming || no_plan_ready;
    let show_models = store.prefs.read().cursor_models;
    let model_state = lead_model.read().clone();
    let model_shown = choice() != "auto"
        && model_state
            .as_ref()
            .is_some_and(|model| model.plan.as_str() == choice());
    let model_options = model_options(model_state.as_ref(), &choice(), show_models);
    let heading = match direct {
        None => "Lead".to_string(),
        Some(id) => format!("Just {}", id.as_str()),
    };
    let avatar = match direct {
        Some(id) => BotName::Plan(id),
        None => BotName::Lead,
    };
    let direct_name = direct.map(|id| id.as_str().to_string()).unwrap_or_default();
    let menu_at = new_menu();
    let plans_for_menu = plans.clone();

    rsx! {
        div { class: "lead-chat",
            div { class: "panel-head",
                div { class: "panel-title",
                    Star { size: 12.0 }
                    BotAvatar {
                        bot: avatar,
                        state: if lead_streaming { BotState::Working } else { BotState::Idle },
                        size: 20.0,
                        title: None,
                    }
                    h2 { id: "lead-heading", "{heading}" }
                }
                div { class: "head-actions",
                    button {
                        id: "lead-new-chat",
                        r#type: "button",
                        class: "btn btn-quiet",
                        aria_haspopup: "menu",
                        aria_expanded: if menu_at.is_some() { "true" } else { "false" },
                        disabled: lead_streaming,
                        onclick: move |_| {
                            if let Some(rect) = element_rect("lead-new-chat") {
                                new_menu.set(Some((rect.left(), rect.bottom() + 4.0)));
                            }
                        },
                        "New chat ▾"
                    }
                    if let Some((x, y)) = menu_at {
                        SidebarMenu {
                            label: "New chat".to_string(),
                            at: (x, y),
                            entries: new_chat_entries(plans_for_menu.as_deref(), on_reset, mode_live, live, resetting, store),
                            on_close: Callback::new(move |_| new_menu.set(None)),
                        }
                    }
                }
            }
            div { class: "lead-options",
                if let Some(id) = direct {
                    span { class: "hint", title: "Reads the project. No edits, no commands, no delegation.", "read only" }
                    button {
                        r#type: "button",
                        class: "btn btn-quiet",
                        title: "Open {direct_name} in a terminal in this project",
                        onclick: move |_| on_open_terminal.call(id),
                        "Open in terminal"
                    }
                } else {
                    span { class: "lead-option",
                        label { r#for: "lead-plan", "Runs on" }
                        FieldSelect {
                            id: "lead-plan".to_string(),
                            class: "plan-choice".to_string(),
                            title: "Switching plans starts a fresh lead session.".to_string(),
                            value: choice(),
                            disabled: lead_streaming,
                            aria_invalid: plan_error.read().is_some(),
                            aria_describedby: if plan_error.read().is_some() { "lead-plan-error".to_string() } else { String::new() },
                            options: plan_options(plans.as_deref()),
                            on_change: {
                                let choice_ref = choice_ref.clone();
                                let picked = picked.clone();
                                Callback::new(move |next: String| {
                                    let value = if is_provider_id(&next) { next } else { "auto".into() };
                                    let previous = choice_ref.borrow().clone();
                                    picked.set(true);
                                    *choice_ref.borrow_mut() = value.clone();
                                    choice.set(value.clone());
                                    plan_error.set(None);
                                    let choice_ref = choice_ref.clone();
                                    spawn_local(async move {
                                        let plan = if value == "auto" { None } else { ProviderId::parse(&value) };
                                        if let Err(err) = api::set_lead_plan(plan).await
                                            && choice_ref.borrow().as_str() == value
                                        {
                                            *choice_ref.borrow_mut() = previous.clone();
                                            choice.set(previous);
                                            plan_error.set(Some(error_text(&err)));
                                        }
                                    });
                                })
                            },
                        }
                    }
                    if model_shown {
                        span { class: "lead-option",
                            label { r#for: "lead-model", "Model" }
                            FieldSelect {
                                id: "lead-model".to_string(),
                                class: "plan-choice lead-model".to_string(),
                                title: "The lead's own model. Workers keep the model set in Settings.".to_string(),
                                value: model_state.as_ref().and_then(|model| model.value.clone()).unwrap_or_default(),
                                disabled: lead_streaming,
                                options: model_options,
                                on_change: Callback::new(move |next: String| {
                                    let Some(model) = lead_model.read().clone() else { return };
                                    let value = if next.is_empty() { None } else { Some(next) };
                                    let plan = model.plan;
                                    lead_model.set(Some(LeadModels {
                                        plan,
                                        options: model.options.clone(),
                                        value: value.clone(),
                                    }));
                                    model_error.set(None);
                                    spawn_local(async move {
                                        if let Err(err) = api::set_lead_model(plan, value.as_deref()).await {
                                            model_error.set(Some(error_text(&err)));
                                        }
                                    });
                                }),
                            }
                        }
                    }
                    span { class: "lead-option",
                        label { r#for: "lead-access", "Workers may" }
                        FieldSelect {
                            id: "lead-access".to_string(),
                            class: "plan-choice lead-access".to_string(),
                            value: worker_access().as_str().to_string(),
                            disabled: lead_streaming,
                            options: vec![
                                SelectOption::new("read", "read only"),
                                SelectOption::new("edit", "edit files"),
                                SelectOption::new("full", "do anything"),
                            ],
                            on_change: {
                                let access_ref = access_ref.clone();
                                Callback::new(move |next: String| {
                                    let value = JobAccess::parse(&next).unwrap_or(JobAccess::Read);
                                    let previous = access_ref.get();
                                    access_ref.set(value);
                                    worker_access.set(value);
                                    plan_error.set(None);
                                    let access_ref = access_ref.clone();
                                    spawn_local(async move {
                                        if let Err(err) = api::set_lead_access(value).await
                                            && access_ref.get() == value
                                        {
                                            access_ref.set(previous);
                                            worker_access.set(previous);
                                            plan_error.set(Some(error_text(&err)));
                                        }
                                    });
                                })
                            },
                        }
                    }
                }
            }
            if let Some(message) = model_error.read().clone() {
                p { class: "error-text", role: "alert", "{message}" }
            }
            if let Some(message) = plan_error.read().clone() {
                p { id: "lead-plan-error", class: "error-text", role: "alert", "{message}" }
            }
            div { id: "lead-log", class: "lead-log", role: "log", aria_live: "polite",
                if shown.is_empty() {
                    div { class: "empty",
                        RayBurst {}
                        BotAvatar { bot: BotName::Lead, state: BotState::Idle, size: 40.0, title: None }
                        p { class: "hint",
                            if no_plan_ready {
                                "No plan is ready. See Usage in the panel."
                            } else if direct.is_none() {
                                "Ask the lead for something and it will split the work across your plans."
                            } else {
                                "Talk straight to {direct_name}. It can read this project but not change files or run commands. For that, open it in a terminal."
                            }
                        }
                    }
                } else {
                    for message in shown {
                        {message_row(message, jobs.clone(), on_show_job, on_run_plan, action_error)}
                    }
                }
            }
            if let Some(message) = send_error.read().clone() {
                pre { id: "lead-error", class: "output-error", role: "alert", "{message}" }
            }
            form {
                class: "composer",
                onsubmit: {
                    let sending = sending.clone();
                    move |event| {
                        event.prevent_default();
                        launch_send(draft, send_error, live, sending.clone(), lead_streaming);
                    }
                },
                label { r#for: "lead-input",
                    if direct.is_none() { "Message to the lead" } else { "Message to {direct_name}" }
                }
                div { class: "composer-box",
                    textarea {
                        id: "lead-input",
                        rows: "3",
                        value: "{draft}",
                        aria_describedby: if send_error.read().is_some() { "lead-error" } else { "" },
                        oninput: move |event| draft.set(event.value()),
                        onkeydown: {
                            let sending = sending.clone();
                            move |event| {
                                if event.key() == Key::Enter && event.modifiers().contains(Modifiers::CONTROL) {
                                    event.prevent_default();
                                    launch_send(draft, send_error, live, sending.clone(), lead_streaming);
                                }
                            }
                        },
                    }
                    div { class: "composer-foot",
                        span { class: "composer-hint", "Ctrl+Enter to send" }
                        button {
                            class: "btn btn-primary",
                            r#type: "submit",
                            disabled: send_disabled,
                            "Send"
                        }
                    }
                }
            }
        }
    }
}

struct LeadModels {
    plan: ProviderId,
    options: Vec<ModelOption>,
    value: Option<String>,
}

impl Clone for LeadModels {
    fn clone(&self) -> Self {
        Self {
            plan: self.plan,
            options: self.options.clone(),
            value: self.value.clone(),
        }
    }
}

fn launch_send(
    mut draft: Signal<String>,
    mut send_error: Signal<Option<String>>,
    mut live: Signal<Vec<LeadMessage>>,
    sending: Rc<Cell<bool>>,
    streaming: bool,
) {
    let text = draft();
    if text.trim().is_empty() || streaming || sending.get() {
        return;
    }
    sending.set(true);
    draft.set(String::new());
    send_error.set(None);
    spawn_local(async move {
        match api::send_lead(&text).await {
            Ok(message) => live.with_mut(|list| upsert_message(list, message)),
            Err(err) => {
                draft.set(text);
                send_error.set(Some(error_text(&err)));
            }
        }
        sending.set(false);
    });
}

fn new_chat_entries(
    plans: Option<&[PlanStatus]>,
    on_reset: Callback<Option<ProviderId>>,
    mode_live: Signal<Option<ProviderId>>,
    live: Signal<Vec<LeadMessage>>,
    resetting: Rc<Cell<bool>>,
    store: state::Store,
) -> Vec<MenuEntry> {
    let lead_resetting = resetting.clone();
    let mut entries = vec![menu_entry(
        "Lead chat",
        "The lead splits the work across your plans",
        false,
        Callback::new(move |_| {
            start_chat(
                None,
                on_reset,
                mode_live,
                live,
                lead_resetting.clone(),
                store,
            );
        }),
    )];
    for id in PROVIDERS {
        let ready = plan_available(plans, plan_of(id));
        let title = format!("Talk straight to {}: read only, no delegation", id.as_str());
        let resetting = resetting.clone();
        entries.push(menu_entry(
            &format!("Just {}", id.as_str()),
            &title,
            !ready,
            Callback::new(move |_| {
                start_chat(
                    Some(id),
                    on_reset,
                    mode_live,
                    live,
                    resetting.clone(),
                    store,
                );
            }),
        ));
    }
    entries
}

fn start_chat(
    direct: Option<ProviderId>,
    on_reset: Callback<Option<ProviderId>>,
    mut mode_live: Signal<Option<ProviderId>>,
    mut live: Signal<Vec<LeadMessage>>,
    resetting: Rc<Cell<bool>>,
    mut store: state::Store,
) {
    if resetting.get() {
        return;
    }
    resetting.set(true);
    spawn_local(async move {
        if let Err(err) = api::reset_lead(direct).await {
            let _ = err;
            resetting.set(false);
            return;
        }
        mode_live.set(direct);
        if let Ok(list) = api::list_lead_messages().await {
            live.set(list);
        } else {
            live.set(Vec::new());
        }
        if let Ok(chats) = api::list_lead_chats().await {
            store.chats.set(chats);
        }
        on_reset.call(direct);
        resetting.set(false);
    });
}

fn menu_entry(label: &str, title: &str, disabled: bool, on_select: Callback) -> MenuEntry {
    MenuEntry {
        label: label.into(),
        on_select: Some(on_select),
        disabled,
        danger: false,
        checked: false,
        shows_check: false,
        heading: false,
        separator: false,
        title: title.into(),
    }
}

fn message_row(
    message: LeadMessage,
    jobs: Vec<JobRecord>,
    on_show_job: Callback<String>,
    on_run_plan: Callback<(String, Vec<ProposedJob>)>,
    mut action_error: Signal<Option<(String, String)>>,
) -> Element {
    let is_user = message.role == LeadRole::User;
    let show_dots = !is_user && message.status == LeadStatus::Streaming && message.text.is_empty();
    let delegated: Vec<JobRecord> = jobs
        .into_iter()
        .filter(|job| job.lead_message.as_deref() == Some(message.id.as_str()))
        .collect();
    let waiting_jobs = delegated
        .iter()
        .filter(|job| job.status == JobStatus::Queued || job.status == JobStatus::Running)
        .count();
    let row_class = if is_user {
        "lead-row lead-row-user"
    } else {
        "lead-row"
    };
    let msg_class = if message.status == LeadStatus::Error {
        "lead-msg lead-msg-error"
    } else if is_user {
        "lead-msg lead-msg-user"
    } else {
        "lead-msg"
    };
    let meta = lead_meta(&message);
    let text = message.text.clone();
    let proposal = message.proposal.clone();
    let show_plan = !is_user
        && proposal
            .as_ref()
            .is_some_and(|item| item.status == ProposalStatus::Waiting)
        && message.status != LeadStatus::Streaming;
    let dismissed = !is_user
        && proposal
            .as_ref()
            .is_some_and(|item| item.status == ProposalStatus::Dismissed);
    let message_id = message.id.clone();
    let problem = action_error
        .read()
        .clone()
        .filter(|(id, _)| id == &message_id)
        .map(|(_, text)| text);

    rsx! {
        div { key: "{message_id}", class: "{row_class}",
            div { class: "{msg_class}",
                span { class: "lead-meta",
                    if is_user { "You" } else { {meta_line(&meta)} }
                }
                div { class: "lead-body",
                    if is_user {
                        "{text}"
                    } else if show_dots {
                        span { class: "dots", aria_hidden: "true",
                            span { class: "dot" }
                            span { class: "dot" }
                            span { class: "dot" }
                        }
                    } else {
                        RichText { text: text.clone() }
                    }
                }
                if show_plan {
                    if let Some(proposal) = proposal.clone() {
                        LeadPlan {
                            key: "{message_id}",
                            proposal,
                            on_run: Callback::new({
                                let message_id = message_id.clone();
                                move |planned: Vec<ProposedJob>| {
                                    let id = message_id.clone();
                                    on_run_plan.call((id.clone(), planned.clone()));
                                    spawn_local(async move {
                                        if let Err(err) = api::run_lead_proposal(&id, Some(&planned)).await {
                                            action_error.set(Some((id, error_text(&err))));
                                        }
                                    });
                                }
                            }),
                            on_dismiss: Callback::new({
                                let message_id = message_id.clone();
                                move |_| {
                                    let id = message_id.clone();
                                    spawn_local(async move {
                                        if let Err(err) = api::dismiss_lead_proposal(&id).await {
                                            action_error.set(Some((id, error_text(&err))));
                                        }
                                    });
                                }
                            }),
                        }
                    }
                }
                if let Some(problem) = problem {
                    p { class: "error-text", role: "alert", "{problem}" }
                }
                if dismissed {
                    p { class: "lead-wait hint", "Plan dismissed. Nothing ran." }
                }
                if !is_user && waiting_jobs > 0 && message.status != LeadStatus::Streaming {
                    p { class: "lead-wait hint",
                        "Waiting for {waiting_jobs} job"
                        if waiting_jobs != 1 { "s" }
                        ". The lead will report when they finish."
                    }
                }
                if !delegated.is_empty() {
                    div { class: "lead-delegated",
                        h3 { class: "lead-delegated-title", "Delegated" }
                        div { class: "lead-delegated-list",
                            for job in delegated {
                                {delegated_card(job, on_show_job)}
                            }
                        }
                    }
                }
            }
        }
    }
}

fn delegated_card(job: JobRecord, on_show_job: Callback<String>) -> Element {
    let label = format!(
        "Show job: {}",
        job.prompt.chars().take(60).collect::<String>()
    );
    let step_title = latest_step_title(&job);
    let edits = edited_files_summary(&job);
    let prompt = job.prompt.clone();
    let provider = job.provider;
    let status = status_name(job.status);
    let id = job.id.clone();
    let meta = delegated_meta(&job);
    let running = job.status == JobStatus::Running;
    rsx! {
        button {
            key: "{id}",
            r#type: "button",
            class: "lead-job-card",
            aria_label: "{label}",
            onclick: move |_| on_show_job.call(id.clone()),
            div { class: "lead-job-card-main",
                if let Some(provider) = provider {
                    BotAvatar {
                        bot: BotName::Plan(provider),
                        state: job_avatar(job.status),
                        size: 20.0,
                        title: Some(provider.as_str().to_string()),
                    }
                }
                span { class: "lead-job-plan",
                    if let Some(provider) = provider { "{provider.as_str()}" } else { "waiting for a worker" }
                }
                span { class: "status status-{status}",
                    span { class: "status-dot", aria_hidden: "true" }
                    "{status}"
                }
            }
            div { class: "lead-job-card-meta", {meta_line(&meta)} }
            if running {
                if let Some(step_title) = step_title {
                    p { class: "lead-job-step", title: "{step_title}", "{step_title}" }
                }
            }
            if let Some(edits) = edits {
                p { class: "lead-job-edits",
                    {file_sentence(edits.files)}
                    " "
                    span { class: "stat-add", "+{edits.added}" }
                    " "
                    span { class: "stat-del", "−{edits.removed}" }
                }
            }
            p { class: "lead-job-prompt", title: "{prompt}", "{prompt}" }
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

fn lead_meta(message: &LeadMessage) -> Vec<String> {
    let mut parts = Vec::new();
    if message.kind == Some(LeadMessageKind::Report) {
        parts.push("Report".into());
    }
    parts.push("Lead".into());
    if let Some(provider) = message.provider {
        parts.push(provider.as_str().into());
    }
    if let Some(model) = &message.model {
        parts.push(model.clone());
    }
    if message.status == LeadStatus::Streaming {
        parts.push("working…".into());
    }
    parts
}

fn delegated_meta(job: &JobRecord) -> Vec<String> {
    let mut parts = vec![job.job_type.as_str().to_string()];
    if let Some(mark) = access_mark(job) {
        parts.push(mark.into());
    }
    if let Some(reason) = job.reason.as_ref().filter(|reason| !reason.is_empty()) {
        parts.push(reason.clone());
    }
    parts
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

fn meta_line(parts: &[String]) -> Element {
    rsx! {
        for (index, part) in parts.iter().cloned().enumerate() {
            span { key: "{index}", style: "display: contents;",
                if index > 0 {
                    span { class: "meta-sep", " / " }
                }
                span { "{part}" }
            }
        }
    }
}

fn job_avatar(status: JobStatus) -> BotState {
    match status {
        JobStatus::Running => BotState::Working,
        JobStatus::Failed => BotState::Error,
        JobStatus::Queued => BotState::Resting,
        JobStatus::Done => BotState::Idle,
    }
}

fn status_name(status: JobStatus) -> &'static str {
    match status {
        JobStatus::Queued => "queued",
        JobStatus::Running => "running",
        JobStatus::Done => "done",
        JobStatus::Failed => "failed",
    }
}

fn plan_available(plans: Option<&[PlanStatus]>, id: ProviderId) -> bool {
    plans.is_some_and(|list| list.iter().any(|plan| plan.id == id && plan.available))
}

fn plan_options(plans: Option<&[PlanStatus]>) -> Vec<SelectOption> {
    let mut options = vec![SelectOption::new("auto", "auto")];
    for id in PROVIDERS {
        options.push(
            SelectOption::new(id.as_str(), id.as_str())
                .disabled(!plan_available(plans, plan_of(id))),
        );
    }
    options
}

fn model_options(
    model: Option<&LeadModels>,
    choice: &str,
    show: CursorModels,
) -> Vec<SelectOption> {
    let Some(model) = model else {
        return Vec::new();
    };
    if model.plan.as_str() != choice {
        return Vec::new();
    }
    let mut listed = visible_models(&model.options, model.plan, show);
    if let Some(value) = &model.value
        && !listed.iter().any(|option| &option.id == value)
    {
        listed.push(ModelOption {
            id: value.clone(),
            label: value.clone(),
        });
    }
    let mut options = vec![SelectOption::new("", "as its jobs")];
    for option in listed {
        options.push(SelectOption::new(option.id.clone(), option.label));
    }
    options
}

fn visible_models(
    models: &[ModelOption],
    plan: ProviderId,
    show: CursorModels,
) -> Vec<ModelOption> {
    if plan != ProviderId::Cursor || show == CursorModels::All {
        return models.to_vec();
    }
    models
        .iter()
        .filter(|model| is_cursor_own(&model.id))
        .cloned()
        .collect()
}

fn is_cursor_own(id: &str) -> bool {
    id == "auto"
        || id.starts_with("composer-")
        || id.starts_with("grok-")
        || id.starts_with("cursor-grok-")
}

async fn tokio_join_models(
    provider: ProviderId,
) -> Result<(Vec<ModelOption>, Option<String>), String> {
    let options = api::list_models(provider).await?;
    let value = api::get_lead_model(provider).await?;
    Ok((options, value))
}

fn upsert_message(list: &mut Vec<LeadMessage>, message: LeadMessage) {
    if let Some(slot) = list.iter_mut().find(|item| item.id == message.id) {
        *slot = message;
    } else {
        list.push(message);
    }
}

fn merge_messages(fetched: Vec<LeadMessage>, current: &[LeadMessage]) -> Vec<LeadMessage> {
    let mut out = Vec::with_capacity(fetched.len() + current.len());
    for message in fetched {
        if let Some(newer) = current.iter().find(|item| item.id == message.id) {
            out.push(newer.clone());
        } else {
            out.push(message);
        }
    }
    for message in current {
        if !out.iter().any(|item| item.id == message.id) {
            out.push(message.clone());
        }
    }
    out
}

fn message_stamp(messages: &[LeadMessage]) -> String {
    let Some(last) = messages.last() else {
        return "0".into();
    };
    format!(
        "{}:{}:{}:{:?}",
        messages.len(),
        last.id,
        last.text.len(),
        last.status
    )
}

fn scroll_bottom(id: &str) {
    let Some(element) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(id))
    else {
        return;
    };
    element.set_scroll_top(element.scroll_height());
}

fn element_rect(id: &str) -> Option<web_sys::DomRect> {
    let element = web_sys::window()?.document()?.get_element_by_id(id)?;
    Some(element.get_bounding_client_rect())
}
