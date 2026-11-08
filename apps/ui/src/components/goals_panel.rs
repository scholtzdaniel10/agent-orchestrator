use std::cell::Cell;
use std::rc::Rc;

use dioxus::prelude::*;
use legate_core::{JobAccess, JobRecord, JobStatus, JobType};
use wasm_bindgen_futures::spawn_local;

use super::bot_avatar::{FieldSelect, SelectOption, error_text};
use super::star::Star;
use crate::api;
use crate::wire::{
    Goal, GoalInput, GoalRound, GoalStatus, PlannedJob, Trigger, TriggerInput, TriggerKind,
};

#[component]
pub fn GoalsPanel(
    jobs: Vec<JobRecord>,
    on_view_change: Callback<String>,
    on_show_job: Callback<String>,
) -> Element {
    let mut goals = use_signal(|| None::<Vec<Goal>>);
    let mut selected = use_signal(|| None::<String>);
    let mut creating = use_signal(|| false);
    let mut text = use_signal(String::new);
    let mut check = use_signal(String::new);
    let mut rounds = use_signal(|| 5i64);
    let draft = use_signal(|| None::<JobDraft>);
    let note = use_signal(String::new);
    let busy = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let busy_cell = use_hook(|| Rc::new(Cell::new(false)));

    use_effect(move || {
        spawn_local(async move {
            match api::list_goals().await {
                Ok(list) => goals.set(Some(list)),
                Err(err) => error.set(Some(error_text(&err))),
            }
        });
        spawn_local(async move {
            let _ = api::on_goals(move |goal| {
                goals.with_mut(|slot| {
                    let current = slot.clone().unwrap_or_default();
                    *slot = Some(upsert_goal(current, goal));
                });
            })
            .await;
        });
    });

    let listed = goals.read().clone();
    let chosen = listed.as_ref().and_then(|list| {
        let picked = selected.read().clone();
        list.iter()
            .find(|entry| Some(&entry.id) == picked.as_ref())
            .cloned()
            .or_else(|| list.first().cloned())
    });
    let text_empty = text.read().trim().is_empty();
    let show_error = error.read().clone();

    rsx! {
        div { class: "goals",
            div { class: "panel-head",
                div { class: "panel-title",
                    Star { size: 12.0 }
                    h2 { "Goals" }
                }
                button {
                    r#type: "button",
                    class: "btn btn-compact",
                    onclick: move |_| creating.set(!creating()),
                    if creating() { "Close" } else { "New goal" }
                }
            }
            if creating() {
                form {
                    class: "goal-form",
                    onsubmit: {
                        let busy_cell = busy_cell.clone();
                        move |event| {
                            event.prevent_default();
                            let input = GoalInput {
                                text: text(),
                                check: Some(check()),
                                max_rounds: Some(rounds() as f64),
                                quick_type: None,
                                quick_access: None,
                            };
                            run_act(busy_cell.clone(), busy, error, async move {
                                let made = api::create_goal(&input).await?;
                                let id = made.id.clone();
                                goals.with_mut(|slot| {
                                    let current = slot.clone().unwrap_or_default();
                                    *slot = Some(upsert_goal(current, made));
                                });
                                selected.set(Some(id));
                                text.set(String::new());
                                check.set(String::new());
                                creating.set(false);
                                Ok(())
                            });
                        }
                    },
                    textarea {
                        class: "field",
                        rows: "3",
                        value: "{text}",
                        placeholder: "What should be true when this is done? e.g. Login keeps the session after a token refresh.",
                        aria_label: "Goal",
                        oninput: move |event| text.set(event.value()),
                    }
                    input {
                        class: "field",
                        value: "{check}",
                        placeholder: "Check, optional: a command that passes when it is done, e.g. pnpm test",
                        aria_label: "Check command",
                        oninput: move |event| check.set(event.value()),
                    }
                    div { class: "goal-form-foot",
                        label { class: "goal-rounds",
                            "At most"
                            input {
                                class: "field",
                                r#type: "number",
                                min: "1",
                                max: "20",
                                value: "{rounds}",
                                oninput: move |event| rounds.set(parse_i64(&event.value(), 1)),
                            }
                            "rounds"
                        }
                        button {
                            r#type: "submit",
                            class: "btn btn-compact",
                            disabled: busy() || text_empty,
                            "Start"
                        }
                    }
                    p { class: "hint",
                        "The lead plans each round and waits for your OK. Changes collect in one worktree for you to review in Changes. "
                        button {
                            r#type: "button",
                            class: "link-button",
                            onclick: move |_| {
                                text.set("Add a short CONTRIBUTING.md that explains how to install, run and test this project, based on what the code actually uses.".into());
                                rounds.set(2);
                            },
                            "Fill in an example"
                        }
                    }
                }
            }
            if let Some(message) = show_error {
                p { class: "error-text", "{message}" }
            }
            if listed.as_ref().is_some_and(|list| list.is_empty()) && !creating() {
                p { class: "hint", "No goals yet. Start one here, or add a check to a job in Jobs to fix until it passes." }
            }
            if let Some(list) = listed.clone() {
                if !list.is_empty() {
                    ul { class: "goal-list",
                        for entry in list {
                            {
                                let id = entry.id.clone();
                                let status = status_text(entry.status);
                                let status_class = status_class(entry.status);
                                let line = first_line(&entry.text, 90);
                                let round_label = if entry.quick {
                                    "task".to_string()
                                } else {
                                    format!("{}/{}", entry.rounds.len(), entry.max_rounds)
                                };
                                let selected_here = chosen.as_ref().is_some_and(|goal| goal.id == id);
                                let row_class = if selected_here { "goal-row is-selected" } else { "goal-row" };
                                rsx! {
                                    li { key: "{id}",
                                        button {
                                            r#type: "button",
                                            class: "{row_class}",
                                            onclick: move |_| selected.set(Some(id.clone())),
                                            span { class: "goal-status {status_class}", "{status}" }
                                            span { class: "goal-text", "{line}" }
                                            span { class: "goal-round", "{round_label}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if let Some(goal) = chosen {
                {goal_detail(goal, jobs.clone(), draft, note, busy, error, busy_cell.clone(), on_view_change, on_show_job, goals, selected)}
            }
            TriggersSection {}
        }
    }
}

struct JobDraft {
    goal: String,
    round: i64,
    jobs: Vec<PlannedJob>,
}

#[allow(clippy::too_many_arguments)]
fn goal_detail(
    goal: Goal,
    jobs: Vec<JobRecord>,
    draft: Signal<Option<JobDraft>>,
    mut note: Signal<String>,
    busy: Signal<bool>,
    error: Signal<Option<String>>,
    busy_cell: Rc<Cell<bool>>,
    on_view_change: Callback<String>,
    on_show_job: Callback<String>,
    mut goals: Signal<Option<Vec<Goal>>>,
    mut selected: Signal<Option<String>>,
) -> Element {
    let proposal = if goal.status == GoalStatus::Waiting {
        goal.proposal.clone()
    } else {
        None
    };
    let round_n = goal.rounds.len() as i64;
    let edited = match (proposal.as_ref(), draft.read().as_ref()) {
        (Some(_), Some(saved)) if saved.goal == goal.id && saved.round == round_n => {
            saved.jobs.clone()
        }
        (Some(proposal), _) => proposal.jobs.clone(),
        _ => Vec::new(),
    };
    let goal_id = goal.id.clone();
    let worktree = goal.worktree.clone();
    let note_value = note();
    let can_answer = matches!(
        goal.status,
        GoalStatus::Waiting | GoalStatus::Stopped | GoalStatus::Failed
    );
    let can_remove = matches!(
        goal.status,
        GoalStatus::Done | GoalStatus::Stopped | GoalStatus::Failed
    );
    let answer_placeholder = if proposal.is_some() {
        "Or tell the lead what to change, and it plans the round again."
    } else {
        "Tell the lead what to do next."
    };
    let answer_label = if proposal.is_some() {
        "Plan again"
    } else {
        "Continue"
    };

    rsx! {
        section { class: "goal-detail", aria_label: "Goal",
            p { class: "goal-title", "{goal.text}" }
            if let Some(source) = goal.source.clone() {
                p { class: "goal-check", "Started by: {source.label}" }
            }
            if let Some(check) = goal.check.clone() {
                p { class: "goal-check",
                    "Check: "
                    code { "{check}" }
                }
            }
            if goal.status == GoalStatus::Planning {
                p { class: "goal-state", "The lead is reading the code and planning round {round_n + 1}…" }
            }
            if goal.status == GoalStatus::Checking {
                p { class: "goal-state", "Running the check…" }
            }
            if let Some(proposal) = proposal.clone() {
                div { class: "goal-proposal",
                    p { class: "goal-state", "Round {round_n + 1}: {proposal.summary}" }
                    for (index, job) in edited.clone().into_iter().enumerate() {
                        {proposal_job(index, job, goal_id.clone(), round_n, edited.clone(), draft)}
                    }
                    div { class: "goal-actions",
                        button {
                            r#type: "button",
                            class: "btn btn-compact",
                            disabled: busy() || edited.is_empty(),
                            onclick: {
                                let busy_cell = busy_cell.clone();
                                let goal_id = goal_id.clone();
                                let edited = edited.clone();
                                move |_| {
                                    let goal_id = goal_id.clone();
                                    let edited = edited.clone();
                                    run_act(busy_cell.clone(), busy, error, async move {
                                        let _ = api::approve_goal(&goal_id, Some(&edited)).await?;
                                        Ok(())
                                    });
                                }
                            },
                            "Approve round"
                        }
                        button {
                            r#type: "button",
                            class: "btn btn-quiet btn-compact",
                            disabled: busy(),
                            onclick: {
                                let busy_cell = busy_cell.clone();
                                let goal_id = goal_id.clone();
                                move |_| {
                                    let goal_id = goal_id.clone();
                                    run_act(busy_cell.clone(), busy, error, async move {
                                        let _ = api::stop_goal(&goal_id).await?;
                                        Ok(())
                                    });
                                }
                            },
                            "Stop"
                        }
                    }
                }
            }
            if goal.status == GoalStatus::Waiting {
                if let Some(question) = goal.question.clone() {
                    p { class: "goal-question", "{question}" }
                }
            }
            if let Some(outcome) = goal.outcome.clone() {
                p { class: "goal-outcome", "{outcome}" }
            }
            if can_answer {
                div { class: "goal-answer",
                    textarea {
                        rows: "2",
                        value: "{note_value}",
                        placeholder: "{answer_placeholder}",
                        aria_label: "Note for the lead",
                        oninput: move |event| note.set(event.value()),
                    }
                    button {
                        r#type: "button",
                        class: "btn btn-quiet btn-compact",
                        disabled: busy(),
                        onclick: {
                            let busy_cell = busy_cell.clone();
                            let goal_id = goal_id.clone();
                            move |_| {
                                let goal_id = goal_id.clone();
                                let text = note();
                                run_act(busy_cell.clone(), busy, error, async move {
                                    let _ = api::answer_goal(&goal_id, &text).await?;
                                    note.set(String::new());
                                    Ok(())
                                });
                            }
                        },
                        "{answer_label}"
                    }
                }
            }
            if goal.status == GoalStatus::Running {
                div { class: "goal-actions",
                    button {
                        r#type: "button",
                        class: "btn btn-quiet btn-compact",
                        disabled: busy(),
                        onclick: {
                            let busy_cell = busy_cell.clone();
                            let goal_id = goal_id.clone();
                            move |_| {
                                let goal_id = goal_id.clone();
                                run_act(busy_cell.clone(), busy, error, async move {
                                    let _ = api::stop_goal(&goal_id).await?;
                                    Ok(())
                                });
                            }
                        },
                        "Stop"
                    }
                }
            }
            if can_remove {
                div { class: "goal-actions",
                    button {
                        r#type: "button",
                        class: "btn btn-compact",
                        onclick: {
                            let worktree = worktree.clone();
                            move |_| on_view_change.call(worktree.clone())
                        },
                        "Review in Changes"
                    }
                    button {
                        r#type: "button",
                        class: "btn btn-quiet btn-compact",
                        disabled: busy(),
                        onclick: {
                            let busy_cell = busy_cell.clone();
                            let goal_id = goal_id.clone();
                            move |_| {
                                let goal_id = goal_id.clone();
                                run_act(busy_cell.clone(), busy, error, async move {
                                    api::remove_goal(&goal_id).await?;
                                    goals.with_mut(|slot| {
                                        if let Some(list) = slot.as_mut() {
                                            list.retain(|entry| entry.id != goal_id);
                                        }
                                    });
                                    selected.set(None);
                                    Ok(())
                                });
                            }
                        },
                        "Remove"
                    }
                }
            }
            if !goal.rounds.is_empty() {
                ol { class: "goal-rounds-list",
                    for round in goal.rounds.iter().rev().cloned() {
                        {round_item(round, round_n, jobs.clone(), on_show_job)}
                    }
                }
            }
        }
    }
}

fn proposal_job(
    index: usize,
    job: PlannedJob,
    goal_id: String,
    round: i64,
    edited: Vec<PlannedJob>,
    mut draft: Signal<Option<JobDraft>>,
) -> Element {
    let prompt = job.prompt.clone();
    let job_type = job.job_type.as_str().to_string();
    let access = job.access.as_str().to_string();
    rsx! {
        div { key: "{index}", class: "goal-job",
            div { class: "goal-job-head",
                FieldSelect {
                    aria_label: "Job type".to_string(),
                    value: job_type,
                    options: job_type_options(),
                    on_change: {
                        let goal_id = goal_id.clone();
                        let edited = edited.clone();
                        Callback::new(move |next: String| {
                            let Some(job_type) = JobType::parse(&next) else { return };
                            write_job(draft, &goal_id, round, &edited, index, |job| job.job_type = job_type);
                        })
                    },
                }
                FieldSelect {
                    aria_label: "Access".to_string(),
                    value: access,
                    options: vec![
                        SelectOption::new("edit", "edit files"),
                        SelectOption::new("full", "edit + run commands"),
                    ],
                    on_change: {
                        let goal_id = goal_id.clone();
                        let edited = edited.clone();
                        Callback::new(move |next: String| {
                            let Some(access) = JobAccess::parse(&next) else { return };
                            write_job(draft, &goal_id, round, &edited, index, |job| job.access = access);
                        })
                    },
                }
                button {
                    r#type: "button",
                    class: "btn btn-quiet btn-compact",
                    aria_label: "Leave this job out",
                    onclick: {
                        let goal_id = goal_id.clone();
                        let edited = edited.clone();
                        move |_| {
                            let jobs = edited.iter().enumerate().filter(|(at, _)| *at != index).map(|(_, job)| job.clone()).collect();
                            draft.set(Some(JobDraft { goal: goal_id.clone(), round, jobs }));
                        }
                    },
                    "×"
                }
            }
            textarea {
                rows: "4",
                value: "{prompt}",
                aria_label: "Job prompt",
                oninput: {
                    let goal_id = goal_id.clone();
                    let edited = edited.clone();
                    move |event| {
                        let next = event.value();
                        write_job(draft, &goal_id, round, &edited, index, |job| job.prompt = next.clone());
                    }
                },
            }
        }
    }
}

fn round_item(
    round: GoalRound,
    last: i64,
    jobs: Vec<JobRecord>,
    on_show_job: Callback<String>,
) -> Element {
    let n = round.n;
    let summary = round.summary.clone();
    let open = n == last;
    rsx! {
        li { key: "{n}", class: "goal-round-item",
            p { class: "goal-round-head", "Round {n}: {summary}" }
            ul { class: "goal-round-jobs",
                for (index, id) in round.job_ids.iter().cloned().enumerate() {
                    {
                        let prompt = round.jobs.get(index).map(|job| first_line(&job.prompt, 70)).unwrap_or_else(|| first_line(&id, 70));
                        let status = jobs.iter().find(|job| job.id == id).map(|job| job_status_name(job.status)).unwrap_or("ended");
                        let id_for_click = id.clone();
                        rsx! {
                            li { key: "{id}",
                                button {
                                    r#type: "button",
                                    class: "goal-round-job",
                                    onclick: move |_| on_show_job.call(id_for_click.clone()),
                                    span { class: "goal-job-status is-{status}", "{status}" }
                                    "{prompt}"
                                }
                            }
                        }
                    }
                }
            }
            for problem in round.problems.clone() {
                p { key: "{problem}", class: "goal-problem", "{problem}" }
            }
            if let Some(check) = round.check.clone() {
                {
                    let summary_class = if check.code == Some(0) { "is-pass" } else { "is-fail" };
                    let summary_text = if check.code == Some(0) {
                        "Check passed".to_string()
                    } else {
                        format!("Check failed (exit {})", check.code.map(|code| code.to_string()).unwrap_or_else(|| "?".into()))
                    };
                    let output = last_chars(&check.output, 3000);
                    rsx! {
                        details { class: "goal-check-result", open: open,
                            summary { class: "{summary_class}", "{summary_text}" }
                            pre { "{output}" }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn TriggersSection() -> Element {
    let mut triggers = use_signal(Vec::<Trigger>::new);
    let mut adding = use_signal(|| false);
    let mut kind = use_signal(|| TriggerKind::Daily);
    let mut at = use_signal(|| "07:30".to_string());
    let mut hours = use_signal(|| 6i64);
    let mut text = use_signal(String::new);
    let mut check = use_signal(String::new);
    let busy = use_signal(|| false);
    let error = use_signal(|| None::<String>);
    let busy_cell = use_hook(|| Rc::new(Cell::new(false)));

    use_effect(move || {
        spawn_local(async move {
            if let Ok(list) = api::list_triggers().await {
                triggers.set(list);
            }
        });
        spawn_local(async move {
            let _ = api::on_triggers(move |list| triggers.set(list)).await;
        });
    });

    let scheduled = matches!(kind(), TriggerKind::Daily | TriggerKind::Hourly);
    let kind_now = kind();
    let text_empty = text.read().trim().is_empty();
    let listed = triggers.read().clone();
    let show_error = error.read().clone();

    rsx! {
        section { class: "goal-triggers", aria_label: "Triggers",
            div { class: "goal-triggers-head",
                h3 { "Triggers" }
                button {
                    r#type: "button",
                    class: "btn btn-quiet btn-compact",
                    onclick: move |_| adding.set(!adding()),
                    if adding() { "Close" } else { "Add trigger" }
                }
            }
            p { class: "hint", "Start a goal on a schedule or when GitHub shows a problem. It still waits for your OK each round." }
            if adding() {
                form {
                    class: "goal-form",
                    onsubmit: {
                        let busy_cell = busy_cell.clone();
                        move |event| {
                            event.prevent_default();
                            let chosen = kind();
                            let input = TriggerInput {
                                kind: kind_name(chosen).into(),
                                at: if chosen == TriggerKind::Daily { Some(at()) } else { None },
                                hours: if chosen == TriggerKind::Hourly { Some(hours() as f64) } else { None },
                                text: Some(text()),
                                check: Some(check()),
                                max_rounds: None,
                            };
                            run_act(busy_cell.clone(), busy, error, async move {
                                let _ = api::add_trigger(&input).await?;
                                if let Ok(list) = api::list_triggers().await {
                                    triggers.set(list);
                                }
                                text.set(String::new());
                                check.set(String::new());
                                adding.set(false);
                                Ok(())
                            });
                        }
                    },
                    div { class: "goal-form-foot",
                        FieldSelect {
                            class: "field".to_string(),
                            aria_label: "When".to_string(),
                            value: kind_name(kind_now).to_string(),
                            options: trigger_options(),
                            on_change: Callback::new(move |next: String| {
                                if let Some(parsed) = parse_kind(&next) {
                                    kind.set(parsed);
                                }
                            }),
                        }
                        if kind_now == TriggerKind::Daily {
                            input {
                                class: "field",
                                r#type: "time",
                                value: "{at}",
                                aria_label: "Time",
                                oninput: move |event| at.set(event.value()),
                            }
                        }
                        if kind_now == TriggerKind::Hourly {
                            label { class: "goal-rounds",
                                "every"
                                input {
                                    class: "field",
                                    r#type: "number",
                                    min: "1",
                                    max: "168",
                                    value: "{hours}",
                                    oninput: move |event| hours.set(parse_i64(&event.value(), 1)),
                                }
                                "hours"
                            }
                        }
                    }
                    if kind_now == TriggerKind::CiFailed || kind_now == TriggerKind::MainBroken {
                        p { class: "hint",
                            if kind_now == TriggerKind::CiFailed {
                                "Watches your open pull requests from the branch you have checked out; the failing log goes with the goal."
                            } else {
                                "Watches the newest run on the default branch while it is checked out; the failing log goes with the goal."
                            }
                        }
                    }
                    textarea {
                        class: "field",
                        rows: "2",
                        value: "{text}",
                        placeholder: if scheduled {
                            "The goal, e.g. Update dependencies, run the tests, and fix what breaks."
                        } else {
                            "Extra instructions, optional."
                        },
                        aria_label: "Goal",
                        oninput: move |event| text.set(event.value()),
                    }
                    input {
                        class: "field",
                        value: "{check}",
                        placeholder: "Check, optional: e.g. pnpm test",
                        aria_label: "Check command",
                        oninput: move |event| check.set(event.value()),
                    }
                    div { class: "goal-actions",
                        button {
                            r#type: "submit",
                            class: "btn btn-compact",
                            disabled: busy() || (scheduled && text_empty),
                            "Add"
                        }
                    }
                }
            }
            if let Some(message) = show_error {
                p { class: "error-text", "{message}" }
            }
            if !listed.is_empty() {
                ul { class: "goal-list",
                    for trigger in listed {
                        {trigger_row(trigger, busy, error, busy_cell.clone(), triggers)}
                    }
                }
            }
        }
    }
}

fn trigger_row(
    trigger: Trigger,
    busy: Signal<bool>,
    error: Signal<Option<String>>,
    busy_cell: Rc<Cell<bool>>,
    mut triggers: Signal<Vec<Trigger>>,
) -> Element {
    let id = trigger.id.clone();
    let title = trigger_title(&trigger);
    let sub = if trigger.text.is_empty() {
        "no extra instructions".to_string()
    } else {
        first_line(&trigger.text, 60)
    };
    let fired = trigger.last_fired.map(locale_time);
    let last_error = trigger.last_error.clone();
    let enabled = trigger.enabled;
    let scheduled = matches!(trigger.kind, TriggerKind::Daily | TriggerKind::Hourly);
    let run_label = if scheduled { "Run now" } else { "Look now" };
    let run_title = if scheduled {
        "Start its goal now"
    } else {
        "Look at GitHub now"
    };
    let remove_label = format!("Remove {title}");
    let toggle_label = format!("{title} on");
    rsx! {
        li { key: "{id}", class: "trigger-row",
            label { class: "trigger-on",
                input {
                    r#type: "checkbox",
                    checked: enabled,
                    aria_label: "{toggle_label}",
                    onchange: {
                        let busy_cell = busy_cell.clone();
                        let id = id.clone();
                        move |event| {
                            let on = event.checked();
                            let id = id.clone();
                            run_act(busy_cell.clone(), busy, error, async move {
                                let _ = api::toggle_trigger(&id, on).await?;
                                if let Ok(list) = api::list_triggers().await {
                                    triggers.set(list);
                                }
                                Ok(())
                            });
                        }
                    },
                }
            }
            span { class: "trigger-text",
                span { class: "trigger-title", "{title}" }
                span { class: "trigger-sub",
                    "{sub}"
                    if let Some(fired) = fired {
                        " · last {fired}"
                    }
                }
                if let Some(last_error) = last_error {
                    span { class: "goal-problem", "{last_error}" }
                }
            }
            button {
                r#type: "button",
                class: "btn btn-quiet btn-compact",
                disabled: busy(),
                title: "{run_title}",
                onclick: {
                    let busy_cell = busy_cell.clone();
                    let id = id.clone();
                    move |_| {
                        let id = id.clone();
                        run_act(busy_cell.clone(), busy, error, async move {
                            api::run_trigger(&id).await?;
                            if let Ok(list) = api::list_triggers().await {
                                triggers.set(list);
                            }
                            Ok(())
                        });
                    }
                },
                "{run_label}"
            }
            button {
                r#type: "button",
                class: "btn btn-quiet btn-compact",
                disabled: busy(),
                aria_label: "{remove_label}",
                onclick: {
                    let busy_cell = busy_cell.clone();
                    let id = id.clone();
                    move |_| {
                        let id = id.clone();
                        run_act(busy_cell.clone(), busy, error, async move {
                            api::remove_trigger(&id).await?;
                            if let Ok(list) = api::list_triggers().await {
                                triggers.set(list);
                            }
                            Ok(())
                        });
                    }
                },
                "×"
            }
        }
    }
}

fn run_act(
    cell: Rc<Cell<bool>>,
    mut busy: Signal<bool>,
    mut error: Signal<Option<String>>,
    task: impl std::future::Future<Output = Result<(), String>> + 'static,
) {
    if cell.get() {
        return;
    }
    cell.set(true);
    busy.set(true);
    error.set(None);
    spawn_local(async move {
        if let Err(err) = task.await {
            error.set(Some(error_text(&err)));
        }
        cell.set(false);
        busy.set(false);
    });
}

fn write_job(
    mut draft: Signal<Option<JobDraft>>,
    goal: &str,
    round: i64,
    edited: &[PlannedJob],
    index: usize,
    patch: impl FnOnce(&mut PlannedJob),
) {
    let mut jobs = edited.to_vec();
    if let Some(job) = jobs.get_mut(index) {
        patch(job);
    }
    draft.set(Some(JobDraft {
        goal: goal.to_string(),
        round,
        jobs,
    }));
}

fn upsert_goal(mut list: Vec<Goal>, goal: Goal) -> Vec<Goal> {
    list.retain(|entry| entry.id != goal.id);
    list.push(goal);
    list.sort_by_key(|entry| std::cmp::Reverse(entry.created_at));
    list
}

fn job_type_options() -> Vec<SelectOption> {
    JobType::ALL
        .into_iter()
        .map(|kind| SelectOption::new(kind.as_str(), kind.as_str()))
        .collect()
}

fn trigger_options() -> Vec<SelectOption> {
    [
        TriggerKind::Daily,
        TriggerKind::Hourly,
        TriggerKind::CiFailed,
        TriggerKind::NewIssue,
        TriggerKind::MainBroken,
    ]
    .into_iter()
    .map(|kind| SelectOption::new(kind_name(kind), trigger_label(kind)))
    .collect()
}

fn status_text(status: GoalStatus) -> &'static str {
    match status {
        GoalStatus::Planning => "planning",
        GoalStatus::Waiting => "needs you",
        GoalStatus::Running => "working",
        GoalStatus::Checking => "checking",
        GoalStatus::Done => "done",
        GoalStatus::Stopped => "stopped",
        GoalStatus::Failed => "failed",
    }
}

fn status_class(status: GoalStatus) -> &'static str {
    match status {
        GoalStatus::Planning => "is-planning",
        GoalStatus::Waiting => "is-waiting",
        GoalStatus::Running => "is-running",
        GoalStatus::Checking => "is-checking",
        GoalStatus::Done => "is-done",
        GoalStatus::Stopped => "is-stopped",
        GoalStatus::Failed => "is-failed",
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

fn kind_name(kind: TriggerKind) -> &'static str {
    match kind {
        TriggerKind::Daily => "daily",
        TriggerKind::Hourly => "hourly",
        TriggerKind::CiFailed => "ci-failed",
        TriggerKind::NewIssue => "new-issue",
        TriggerKind::MainBroken => "main-broken",
    }
}

fn trigger_label(kind: TriggerKind) -> &'static str {
    match kind {
        TriggerKind::Daily => "Every day at a time",
        TriggerKind::Hourly => "Every few hours",
        TriggerKind::CiFailed => "CI fails on my pull request",
        TriggerKind::NewIssue => "A new GitHub issue",
        TriggerKind::MainBroken => "Tests break on the default branch",
    }
}

fn parse_kind(value: &str) -> Option<TriggerKind> {
    Some(match value {
        "daily" => TriggerKind::Daily,
        "hourly" => TriggerKind::Hourly,
        "ci-failed" => TriggerKind::CiFailed,
        "new-issue" => TriggerKind::NewIssue,
        "main-broken" => TriggerKind::MainBroken,
        _ => return None,
    })
}

fn trigger_title(trigger: &Trigger) -> String {
    match trigger.kind {
        TriggerKind::Daily => format!("Every day at {}", trigger.at.as_deref().unwrap_or("?")),
        TriggerKind::Hourly => format!("Every {} hours", trigger.hours.unwrap_or(24)),
        other => trigger_label(other).to_string(),
    }
}

fn first_line(text: &str, max: usize) -> String {
    let line = text
        .trim()
        .split('\n')
        .next()
        .unwrap_or("")
        .trim_end_matches('\r');
    let count = line.chars().count();
    if count > max {
        let mut cut: String = line.chars().take(max.saturating_sub(1)).collect();
        cut.push('…');
        cut
    } else {
        line.to_string()
    }
}

fn last_chars(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        text.to_string()
    } else {
        text.chars().skip(count - max).collect()
    }
}

fn parse_i64(value: &str, fallback: i64) -> i64 {
    match value.parse::<i64>() {
        Ok(number) if number != 0 => number,
        _ => fallback,
    }
}

fn locale_time(ms: i64) -> String {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms as f64));
    String::from(date.to_locale_string("default", &wasm_bindgen::JsValue::UNDEFINED))
}
