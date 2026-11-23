//! Settings. The element tree matches `SettingsPage.tsx`, including interface scale and theme.

use dioxus::prelude::*;
use legate_core::{
    AppInfo, CursorModels, JobAccess, PROVIDERS, PlanStatus, ProjectEntry, ProviderId, SkillsState,
    UsageShows,
};
use serde_json::json;

use crate::api;
use crate::components::bot_avatar::{FieldSelect, SelectOption};
use crate::components::star::Star;
use crate::components::updates_section::UpdatesSection;
use crate::logic::theme::{THEMES, ThemeId};
use crate::state::use_store;

const SECTIONS: &[(&str, &str, &str)] = &[
    (
        "general",
        "General",
        "Look, jobs at once and notifications.",
    ),
    ("agents", "Agents", "Sign-in, models and the lead."),
    ("skills", "Skills", "Plans the agents can follow."),
    (
        "projects",
        "Projects",
        "Remembered folders, clones and worktree setup.",
    ),
    ("terminal", "Terminal", "Text size of every terminal pane."),
    (
        "updates",
        "Updates",
        "New versions of the CLIs Legate drives.",
    ),
    (
        "shortcuts",
        "Shortcuts",
        "Keys that work across the window.",
    ),
    ("about", "About", "Version and where your data lives."),
];

const SHORTCUTS: &[(&str, &str)] = &[
    ("Ctrl+,", "Open or close Settings"),
    ("Ctrl+K", "Search the sidebar"),
    ("Ctrl+\\", "Show or hide the sidebar"),
    ("Ctrl+B", "Show or hide the panel"),
    (
        "Ctrl+1 … Ctrl+9",
        "Panel tabs (Files … Goals), in the order they are shown",
    ),
    ("Ctrl+Shift+N", "New window"),
    ("Ctrl+Enter", "Send a message to the lead"),
];

const SCALES: [i64; 5] = [80, 90, 100, 110, 120];

#[component]
pub fn SettingsPage(
    plans: Vec<PlanStatus>,
    projects: Vec<ProjectEntry>,
    theme: ThemeId,
    on_theme: Callback<ThemeId>,
    on_remove_project: Callback<String>,
    on_open_setup: Callback,
    on_lead_changed: Callback,
    tab_order_custom: bool,
    on_reset_tabs: Callback,
    on_close: Callback,
    /// How many CLI updates the Updates section found, for the Settings badge.
    on_updates_checked: Callback<usize>,
) -> Element {
    let _ = plans;
    let store = use_store();
    let mut query = use_signal(String::new);
    let mut section = use_signal(|| "general");
    let mut info = use_signal(|| None::<AppInfo>);
    let mut lead_plan = use_signal(|| None::<Option<ProviderId>>);
    let mut lead_access = use_signal(|| None::<JobAccess>);
    let mut skills = use_signal(|| None::<SkillsState>);
    let mut setup = use_signal(Vec::<String>::new);
    let mut error = use_signal(|| None::<String>);
    let mut loaded = use_signal(|| false);
    if !loaded() {
        loaded.set(true);
        spawn(async move {
            match api::get_app_info().await {
                Ok(next) => info.set(Some(next)),
                Err(text) => error.set(Some(text)),
            }
            if let Ok(plan) = api::get_lead_plan().await {
                lead_plan.set(Some(plan));
            }
            if let Ok(access) = api::get_lead_access().await {
                lead_access.set(Some(access));
            }
            if let Ok(state) = api::skills_get().await {
                skills.set(Some(state));
            }
            if let Ok(lines) = api::get_project_setup().await {
                setup.set(lines);
            }
        });
    }
    let needle = query.read().trim().to_lowercase();
    let prefs = store.prefs.read().clone();
    let facts = info.read().clone();
    let skill_state = skills.read().clone();
    let notice = error.read().clone();
    let commands = setup.read().clone();
    let setup_text = if commands.is_empty() {
        "None yet. Set them on the Routines tab.".to_string()
    } else {
        commands.join("\n")
    };

    rsx! {
        div {
            class: "settings-page",
            role: "dialog",
            aria_modal: "true",
            aria_labelledby: "settings-title",
            onkeydown: move |event| {
                if event.key().to_string() == "Escape" && query.read().is_empty() {
                    event.prevent_default();
                    on_close.call(());
                } else if event.key().to_string() == "Escape" {
                    event.prevent_default();
                    query.set(String::new());
                }
            },
            div { class: "settings-head",
                div { class: "panel-title",
                    Star { size: 12.0 }
                    h2 { id: "settings-title", "Settings" }
                }
                button {
                    class: "btn btn-quiet btn-compact",
                    r#type: "button",
                    onclick: move |_| on_close.call(()),
                    "Close"
                }
            }
            div { class: "settings-body",
                nav { class: "settings-nav", aria_label: "Settings sections",
                    input {
                        class: "project-search settings-search",
                        r#type: "search",
                        value: "{query}",
                        placeholder: "Search settings",
                        aria_label: "Search settings",
                        oninput: move |event| query.set(event.value()),
                    }
                    ul { class: "settings-nav-list",
                        for (id, label, _) in SECTIONS {
                            {
                                let selected = needle.is_empty() && section() == *id;
                                let id = *id;
                                rsx! {
                                    li { key: "{id}",
                                        button {
                                            class: if selected { "settings-nav-item is-selected" } else { "settings-nav-item" },
                                            r#type: "button",
                                            aria_current: if selected { "page" } else { "false" },
                                            onclick: move |_| {
                                                query.set(String::new());
                                                section.set(id);
                                            },
                                            "{label}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "settings-content",
                    if let Some(text) = notice {
                        p { class: "error-text", role: "alert", "{text}" }
                    }
                    for (id, label, blurb) in shown_sections(&needle, section()) {
                        section { class: "settings-section", aria_label: "{label}", key: "{id}",
                            h3 { class: "settings-section-title", "{label}" }
                            p { class: "settings-section-blurb", "{blurb}" }
                            if id == "general" || needle_hits(&needle, "Interface size", "scale zoom ui") {
                                Row { label: "Interface size".to_string(), help: "Scales the whole window.".to_string(),
                                    FieldSelect {
                                        class: "settings-select".to_string(),
                                        aria_label: "Interface size".to_string(),
                                        value: prefs.ui_scale.to_string(),
                                        options: SCALES.iter().map(|scale| SelectOption::new(scale.to_string(), format!("{scale}%"))).collect::<Vec<_>>(),
                                        on_change: Callback::new(move |value: String| {
                                            if let Ok(scale) = value.parse::<i64>() {
                                                save_pref(json!({ "uiScale": scale }));
                                            }
                                        }),
                                    }
                                }
                            }
                            if id == "general" || needle_hits(&needle, "Usage shows", "left used percent") {
                                Row { label: "Usage shows".to_string(), help: "What the meters and the status bar say.".to_string(),
                                    FieldSelect {
                                        class: "settings-select".to_string(),
                                        aria_label: "Usage shows".to_string(),
                                        value: if prefs.usage_shows == UsageShows::Used { "used".to_string() } else { "left".to_string() },
                                        options: vec![SelectOption::new("left", "What is left"), SelectOption::new("used", "What is used")],
                                        on_change: Callback::new(move |value: String| {
                                            let shows = if value == "used" { "used" } else { "left" };
                                            save_pref(json!({ "usageShows": shows }));
                                        }),
                                    }
                                }
                            }
                            if id == "general" || needle_hits(&needle, "Cursor models", "own all") {
                                Row { label: "Cursor models".to_string(), help: "Cursor's own pool, or every model it offers.".to_string(),
                                    FieldSelect {
                                        class: "settings-select".to_string(),
                                        aria_label: "Cursor models".to_string(),
                                        value: if prefs.cursor_models == CursorModels::All { "all".to_string() } else { "own".to_string() },
                                        options: vec![SelectOption::new("own", "Cursor's own"), SelectOption::new("all", "Every model")],
                                        on_change: Callback::new(move |value: String| {
                                            let value = if value == "all" { "all" } else { "own" };
                                            save_pref(json!({ "cursorModels": value }));
                                        }),
                                    }
                                }
                            }
                            if id == "general" || needle_hits(&needle, "Look", "theme dark light") {
                                Row { label: "Look".to_string(), help: "System follows the computer.".to_string(),
                                    FieldSelect {
                                        class: "settings-select".to_string(),
                                        aria_label: "Look".to_string(),
                                        value: theme.as_str().to_string(),
                                        options: THEMES.iter().map(|(id, name)| SelectOption::new(*id, *name)).collect::<Vec<_>>(),
                                        on_change: Callback::new(move |value: String| {
                                            if let Some(next) = ThemeId::parse(&value) {
                                                on_theme.call(next);
                                            }
                                        }),
                                    }
                                }
                            }
                            if id == "general" || needle_hits(&needle, "Jobs at once per plan", "parallel") {
                                Row { label: "Jobs at once per plan".to_string(), help: "How many jobs one plan may run together.".to_string(),
                                    FieldSelect {
                                        class: "settings-select".to_string(),
                                        aria_label: "Jobs at once per plan".to_string(),
                                        value: prefs.max_parallel.map_or_else(|| "rules".to_string(), |count| count.to_string()),
                                        options: std::iter::once(SelectOption::new("rules", "Rules file"))
                                            .chain((1..=8).map(|count| SelectOption::new(count.to_string(), count.to_string())))
                                            .collect::<Vec<_>>(),
                                        on_change: Callback::new(move |value: String| {
                                            if value == "rules" {
                                                save_pref(json!({ "maxParallel": null }));
                                            } else if let Ok(count) = value.parse::<i64>() {
                                                save_pref(json!({ "maxParallel": count }));
                                            }
                                        }),
                                    }
                                }
                            }
                            if id == "general" || needle_hits(&needle, "Notifications", "notify") {
                                Row { label: "Notifications".to_string(), help: "When a job ends and no window is focused.".to_string(),
                                    label { class: "settings-check",
                                        input {
                                            r#type: "checkbox",
                                            checked: prefs.notify,
                                            onchange: move |event| save_pref(json!({ "notify": event.checked() })),
                                        }
                                        "Desktop notification"
                                    }
                                }
                            }
                            if id == "general" || needle_hits(&needle, "Panel tabs", "order") {
                                Row { label: "Panel tabs".to_string(), help: "Drag tabs to reorder them. Reset puts them back.".to_string(),
                                    span { class: "settings-inline",
                                        span { class: "settings-muted", if tab_order_custom { "Your order" } else { "Built-in order" } }
                                        button {
                                            class: "btn btn-quiet btn-compact",
                                            r#type: "button",
                                            disabled: !tab_order_custom,
                                            onclick: move |_| on_reset_tabs.call(()),
                                            "Reset"
                                        }
                                    }
                                }
                            }
                            if id == "agents" || needle_hits(&needle, "Lead runs on", "lead plan") {
                                Row { label: "Lead runs on".to_string(), help: "The plan that answers in the Lead tab.".to_string(),
                                    FieldSelect {
                                        class: "settings-select".to_string(),
                                        aria_label: "Lead runs on".to_string(),
                                        value: lead_plan.read().as_ref().and_then(|plan| *plan).map(|id| id.as_str().to_string()).unwrap_or_default(),
                                        options: std::iter::once(SelectOption::new("", "Auto"))
                                            .chain(PROVIDERS.iter().map(|id| SelectOption::new(id.as_str(), id.as_str())))
                                            .collect::<Vec<_>>(),
                                        on_change: Callback::new(move |value: String| {
                                            let picked = ProviderId::parse(&value);
                                            spawn(async move {
                                                if api::set_lead_plan(picked).await.is_ok() {
                                                    lead_plan.set(Some(picked));
                                                    on_lead_changed.call(());
                                                }
                                            });
                                        }),
                                    }
                                }
                            }
                            if id == "agents" || needle_hits(&needle, "Most the lead may hand out", "access") {
                                Row { label: "Most the lead may hand out".to_string(), help: "Jobs the lead starts cannot exceed this.".to_string(),
                                    FieldSelect {
                                        class: "settings-select".to_string(),
                                        aria_label: "Most the lead may hand out".to_string(),
                                        value: match lead_access() { Some(JobAccess::Read) => "read", Some(JobAccess::Edit) => "edit", Some(JobAccess::Full) => "full", None => "" }.to_string(),
                                        options: vec![
                                            SelectOption::new("read", "Read only"),
                                            SelectOption::new("edit", "Edit files"),
                                            SelectOption::new("full", "Full access"),
                                        ],
                                        on_change: Callback::new(move |value: String| {
                                            let Some(access) = JobAccess::parse(&value) else { return; };
                                            spawn(async move {
                                                if api::set_lead_access(access).await.is_ok() {
                                                    lead_access.set(Some(access));
                                                    on_lead_changed.call(());
                                                }
                                            });
                                        }),
                                    }
                                }
                            }
                            if id == "agents" || needle_hits(&needle, "Sign in", "setup claude cursor") {
                                Row { label: "Sign in".to_string(), help: "Install a CLI or sign in again.".to_string(),
                                    button {
                                        class: "btn btn-quiet btn-compact",
                                        r#type: "button",
                                        onclick: move |_| on_open_setup.call(()),
                                        "Open setup"
                                    }
                                }
                            }
                            if id == "skills" {
                                Row { label: "Your skills".to_string(), help: "Folders of instructions the agents can read.".to_string(),
                                    span { class: "settings-skills",
                                        if let Some(state) = skill_state.clone() {
                                            if state.folders.is_empty() {
                                                span { class: "settings-muted", "None yet." }
                                            } else {
                                                ul { class: "settings-projects",
                                                    for folder in state.folders {
                                                        li { class: "settings-project", key: "{folder.folder}",
                                                            span { class: "settings-project-text",
                                                                span { class: "settings-project-name", "{folder.skills.len()} skills" }
                                                                span { class: "settings-path", "{folder.folder}" }
                                                                if let Some(problem) = folder.problem.clone() {
                                                                    span { class: "settings-skill-problem", "{problem}" }
                                                                }
                                                            }
                                                            button {
                                                                class: "btn btn-quiet btn-compact",
                                                                r#type: "button",
                                                                onclick: move |_| {
                                                                    let folder = folder.folder.clone();
                                                                    spawn(async move {
                                                                        if let Ok(next) = api::skills_remove(&folder).await {
                                                                            skills.set(Some(next));
                                                                        }
                                                                    });
                                                                },
                                                                "Remove"
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            ul { class: "settings-projects",
                                                for skill in state.built_in {
                                                    li { class: "settings-project", key: "{skill.name}",
                                                        span { class: "settings-project-name", "{skill.name}" }
                                                        span { class: "settings-tag", "{given_to(skill.given_to)}" }
                                                    }
                                                }
                                            }
                                        } else {
                                            span { class: "settings-muted", "…" }
                                        }
                                        span { class: "settings-inline",
                                            button {
                                                class: "btn btn-quiet btn-compact",
                                                r#type: "button",
                                                onclick: move |_| {
                                                    spawn(async move {
                                                        if let Ok(next) = api::skills_add().await {
                                                            skills.set(Some(next));
                                                        }
                                                    });
                                                },
                                                "Add folder"
                                            }
                                        }
                                    }
                                }
                            }
                            if id == "projects" || needle_hits(&needle, "Projects", "remove folder") {
                                Row { label: "Projects".to_string(), help: "Removing a project leaves the folder on disk.".to_string(),
                                    ul { class: "settings-projects",
                                        for entry in projects.iter().cloned() {
                                            {
                                                let path = entry.path.clone();
                                                let name = folder_name(&path);
                                                rsx! {
                                                    li { class: "settings-project", key: "{path}",
                                                        span { class: "settings-project-text",
                                                            span { class: "settings-project-name",
                                                                "{name}"
                                                                if entry.active {
                                                                    span { class: "settings-tag", "open" }
                                                                }
                                                            }
                                                            span { class: "settings-path", "{path}" }
                                                        }
                                                        button {
                                                            class: "btn btn-quiet btn-compact",
                                                            r#type: "button",
                                                            disabled: entry.active,
                                                            onclick: move |_| on_remove_project.call(path.clone()),
                                                            "Remove"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            if id == "projects" || needle_hits(&needle, "Clone folder", "github") {
                                Row { label: "Clone folder".to_string(), help: "Where GitHub clones go. Empty asks every time.".to_string(),
                                    span { class: "settings-inline",
                                        span { class: "settings-path", "{prefs.clone_dir.clone().unwrap_or_else(|| \"Ask every time\".into())}" }
                                        button {
                                            class: "btn btn-quiet btn-compact",
                                            r#type: "button",
                                            onclick: move |_| {
                                                spawn(async move {
                                                    let _ = api::choose_clone_dir().await;
                                                });
                                            },
                                            "Choose"
                                        }
                                    }
                                }
                            }
                            if id == "projects" || needle_hits(&needle, "Worktree setup", "commands") {
                                Row { label: "Worktree setup".to_string(), help: "Commands that run in a worktree you create by hand.".to_string(),
                                    span { class: "settings-path", "{setup_text}" }
                                }
                            }
                            if id == "projects" {
                                super::workflow_setup::WorkflowSetup {}
                                if let Some(facts) = facts.clone() {
                                    Row { label: "Worktrees folder".to_string(), help: "Where job worktrees are created.".to_string(),
                                        span { class: "settings-path", "{facts.worktrees_dir}" }
                                    }
                                }
                            }
                            if id == "terminal" || needle_hits(&needle, "Text size", "font") {
                                Row { label: "Text size".to_string(), help: "Applies to every open terminal straight away.".to_string(),
                                    span { class: "settings-stepper", role: "group", aria_label: "Terminal text size",
                                        button {
                                            class: "btn btn-quiet btn-compact",
                                            r#type: "button",
                                            disabled: prefs.terminal_font_size <= 10,
                                            onclick: move |_| save_pref(json!({ "terminalFontSize": (prefs.terminal_font_size - 1).max(10) })),
                                            "−"
                                        }
                                        span { class: "settings-stepper-value", aria_live: "polite", "{prefs.terminal_font_size}" }
                                        button {
                                            class: "btn btn-quiet btn-compact",
                                            r#type: "button",
                                            disabled: prefs.terminal_font_size >= 22,
                                            onclick: move |_| save_pref(json!({ "terminalFontSize": (prefs.terminal_font_size + 1).min(22) })),
                                            "+"
                                        }
                                    }
                                }
                            }
                            if id == "updates" {
                                UpdatesSection { on_checked: on_updates_checked }
                            }
                            if id == "shortcuts" {
                                for (keys, what) in SHORTCUTS {
                                    if needle.is_empty() || needle_hits(&needle, what, keys) {
                                        Row { label: what.to_string(), help: String::new(),
                                            kbd { class: "settings-kbd", "{keys}" }
                                        }
                                    }
                                }
                            }
                            if id == "about" || needle_hits(&needle, "Version", "release") {
                                Row { label: "Version".to_string(), help: String::new(),
                                    span { class: "settings-path", "{facts.as_ref().map(|item| item.version.clone()).unwrap_or_else(|| \"…\".into())}" }
                                }
                            }
                            if id == "about" || needle_hits(&needle, "Data folder", "storage") {
                                Row { label: "Data folder".to_string(), help: "Settings and saved chats. Nothing here leaves this computer.".to_string(),
                                    span { class: "settings-inline",
                                        span { class: "settings-path", "{facts.as_ref().map(|item| item.data_dir.clone()).unwrap_or_else(|| \"…\".into())}" }
                                        button {
                                            class: "btn btn-quiet btn-compact",
                                            r#type: "button",
                                            onclick: move |_| {
                                                spawn(async move { let _ = api::open_data_dir().await; });
                                            },
                                            "Open folder"
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

#[component]
fn Row(label: String, help: String, children: Element) -> Element {
    rsx! {
        div { class: "settings-row",
            div { class: "settings-row-text",
                span { class: "settings-row-label", "{label}" }
                if !help.is_empty() {
                    span { class: "settings-row-help", "{help}" }
                }
            }
            div { class: "settings-row-control", {children} }
        }
    }
}

fn shown_sections<'a>(needle: &str, current: &'a str) -> Vec<(&'a str, &'a str, &'a str)> {
    SECTIONS
        .iter()
        .copied()
        .filter(|(id, label, blurb)| {
            if needle.is_empty() {
                *id == current
            } else {
                format!("{label} {blurb}").to_lowercase().contains(needle)
                    || section_text(id).to_lowercase().contains(needle)
            }
        })
        .collect()
}

fn section_text(id: &str) -> &'static str {
    match id {
        "general" => {
            "interface size usage shows cursor models look theme jobs notifications panel tabs"
        }
        "agents" => "lead runs access sign in setup",
        "skills" => "skills folder",
        "projects" => "projects clone worktree",
        "terminal" => "text size font",
        "updates" => "update upgrade cli version powershell",
        "shortcuts" => "ctrl shortcut",
        "about" => "version data folder",
        _ => "",
    }
}

fn needle_hits(needle: &str, label: &str, extra: &str) -> bool {
    !needle.is_empty() && format!("{label} {extra}").to_lowercase().contains(needle)
}

fn save_pref(patch: serde_json::Value) {
    spawn(async move {
        let _ = api::set_prefs(&patch).await;
    });
}

fn folder_name(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_string()
}

fn given_to(value: legate_core::GivenTo) -> &'static str {
    match value {
        legate_core::GivenTo::EveryAgent => "every agent",
        legate_core::GivenTo::Workers => "workers",
        legate_core::GivenTo::Lead => "lead",
    }
}
