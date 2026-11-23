//! Model menu for one plan. The element tree matches `ModelPicker.tsx`.

use dioxus::prelude::*;
use legate_core::{CursorModels, ModelOption, PlanStatus, ProviderId};

use crate::api;
use crate::components::bot_avatar::{FieldSelect, SelectOption};
use crate::state::use_store;

const OTHER: &str = "__other__";

#[component]
pub fn ModelPicker(plan: PlanStatus, disabled: bool, label_title: Option<String>) -> Element {
    let store = use_store();
    let show = store.prefs.read().cursor_models;
    let mut models = use_signal(|| None::<Vec<ModelOption>>);
    let id = plan.id;
    let available = plan.available;
    use_effect(move || {
        if !available {
            return;
        }
        spawn(async move {
            if let Ok(list) = api::list_models(id).await {
                models.set(Some(list));
            }
        });
    });
    let listed = models
        .read()
        .clone()
        .map(|list| visible_models(list, id, show));
    rsx! {
        ModelField {
            plan: plan,
            disabled: disabled,
            models: listed,
            label_title: label_title,
        }
    }
}

#[component]
fn ModelField(
    plan: PlanStatus,
    disabled: bool,
    models: Option<Vec<ModelOption>>,
    label_title: Option<String>,
) -> Element {
    let saved = plan.model.clone().unwrap_or_default();
    let typing = use_signal(|| false);
    let typed = use_signal(|| saved.clone());
    let error = use_signal(|| None::<String>);
    let select_id = format!("model-{}", plan.id.as_str());
    let other_id = format!("model-other-{}", plan.id.as_str());
    let error_id = format!("model-error-{}", plan.id.as_str());
    let hint_id = format!("model-hint-{}", plan.id.as_str());
    let listed = models.clone().unwrap_or_default();
    let saved_listed = saved.is_empty() || listed.iter().any(|option| option.id == saved);
    let title = label_title.clone().unwrap_or_default();
    let notice = error.read().clone();
    let notice_text = notice.clone();
    let saved_change = saved.clone();
    let saved_blur = saved.clone();
    let saved_key = saved.clone();
    let plan_id = plan.id;
    let described = match (label_title.is_some(), notice.is_some()) {
        (true, true) => format!("{hint_id} {error_id}"),
        (true, false) => hint_id.clone(),
        (false, true) => error_id.clone(),
        (false, false) => String::new(),
    };

    rsx! {
        div { class: "model-picker",
            label { r#for: "{select_id}", title: "{title}", "Model" }
            if label_title.is_some() {
                span { id: "{hint_id}", class: "visually-hidden", "{title}" }
            }
            div { class: "model-row",
                FieldSelect {
                    id: select_id.clone(),
                    class: "model-input field".to_string(),
                    disabled: disabled,
                    aria_invalid: notice.is_some(),
                    aria_describedby: described.clone(),
                    value: if typing() { OTHER.to_string() } else { saved.clone() },
                    options: model_options(&saved, saved_listed, &listed),
                    on_change: Callback::new(move |picked: String| {
                        let mut typed = typed;
                        let mut typing = typing;
                        let mut error = error;
                        if picked == OTHER {
                            typed.set(saved_change.clone());
                            typing.set(true);
                            return;
                        }
                        typing.set(false);
                        if picked != saved_change {
                            save_model(plan_id, picked, disabled, &mut error);
                        }
                    }),
                }
            }
            if typing() {
                div { class: "model-row",
                    label { r#for: "{other_id}", class: "visually-hidden", "Model name" }
                    input {
                        id: "{other_id}",
                        class: "model-input field",
                        placeholder: "Model name, then Enter",
                        value: "{typed}",
                        disabled: disabled,
                        spellcheck: "false",
                        autocomplete: "off",
                        oninput: move |event| {
                            let mut typed = typed;
                            typed.set(event.value());
                        },
                        onblur: move |_| {
                            let mut typing = typing;
                            let mut error = error;
                            let next = typed.read().trim().to_string();
                            typing.set(false);
                            if next != saved_blur {
                                save_model(plan_id, next, disabled, &mut error);
                            }
                        },
                        onkeydown: move |event| {
                            let mut typing = typing;
                            let mut error = error;
                            if event.key().to_string() == "Escape" {
                                event.prevent_default();
                                typing.set(false);
                            } else if event.key().to_string() == "Enter" {
                                event.prevent_default();
                                let next = typed.read().trim().to_string();
                                typing.set(false);
                                if next != saved_key {
                                    save_model(plan_id, next, disabled, &mut error);
                                }
                            }
                        },
                    }
                }
            }
            if let Some(text) = notice_text {
                p { id: "{error_id}", class: "error-text", role: "alert", "{text}" }
            }
        }
    }
}

fn save_model(id: ProviderId, next: String, disabled: bool, error: &mut Signal<Option<String>>) {
    if disabled {
        return;
    }
    let mut error = *error;
    error.set(None);
    let model = if next.is_empty() { None } else { Some(next) };
    spawn(async move {
        if let Err(text) = api::set_model(id, model.as_deref()).await {
            error.set(Some(text));
        }
    });
}

fn visible_models(
    models: Vec<ModelOption>,
    plan: ProviderId,
    show: CursorModels,
) -> Vec<ModelOption> {
    if plan != ProviderId::Cursor || show == CursorModels::All {
        return models;
    }
    models
        .into_iter()
        .filter(|model| cursor_own(&model.id))
        .collect()
}

fn cursor_own(id: &str) -> bool {
    id == "auto"
        || id.starts_with("composer-")
        || id.starts_with("grok-")
        || id.starts_with("cursor-grok-")
}

/// The model menu: the CLI default, a saved model the CLI no longer lists, every listed model
/// (its id as the tooltip), and Other… to type one.
fn model_options(saved: &str, saved_listed: bool, listed: &[ModelOption]) -> Vec<SelectOption> {
    let mut options = vec![SelectOption::new("", "CLI default")];
    if !saved_listed && !saved.is_empty() {
        options.push(SelectOption::new(saved, saved));
    }
    options.extend(listed.iter().map(|option| {
        SelectOption::new(option.id.clone(), option.label.clone()).titled(option.id.clone())
    }));
    options.push(SelectOption::new(OTHER, "Other…"));
    options
}
