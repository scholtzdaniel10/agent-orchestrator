use dioxus::prelude::*;
use legate_core::{ModelOption, PlanStatus};
use wasm_bindgen_futures::spawn_local;

use crate::api;

/// A working model list. The full picker menu arrives with the settings slice.
#[component]
pub fn ModelPicker(plan: PlanStatus, disabled: bool, label_title: Option<String>) -> Element {
    let title = label_title.unwrap_or_else(|| format!("{} model", plan.id.as_str()));
    let mut models = use_signal(Vec::<ModelOption>::new);
    let id = plan.id;
    let available = plan.available;
    let current = plan.model.clone().unwrap_or_default();
    use_effect(move || {
        if !available {
            return;
        }
        spawn_local(async move {
            if let Ok(list) = api::list_models(id).await {
                models.set(list);
            }
        });
    });
    let shown = models.read().clone();
    rsx! {
        label { class: "model-picker", title: "{title}",
            select {
                disabled: disabled || !available,
                onchange: move |event| {
                    let value = event.value();
                    let model = if value.is_empty() { None } else { Some(value) };
                    spawn_local(async move {
                        let _ = api::set_model(id, model.as_deref()).await;
                    });
                },
                option { value: "", "CLI default" }
                for model in shown {
                    option { value: "{model.id}", selected: model.id == current, "{model.label}" }
                }
            }
        }
    }
}
