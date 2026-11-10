use crate::state::use_store;
use dioxus::prelude::*;
use wasm_bindgen_futures::spawn_local;

#[component]
pub fn WorkflowSetup() -> Element {
    let store = use_store();
    let mut workflow = use_signal(String::new);
    let mut copied = use_signal(|| false);
    let mut error = use_signal(|| false);
    let project = store
        .project
        .read()
        .as_ref()
        .map(|p| p.path.clone())
        .unwrap_or_default();
    let plans = store.plans.read().clone().unwrap_or_default();
    let agents = plans
        .iter()
        .filter(|p| p.available && p.problem.is_none())
        .map(|p| p.id.as_str())
        .collect::<Vec<_>>();
    let prompt = crate::logic::workflow_setup::setup_prompt(&project, &workflow(), &agents);
    let copy_prompt = prompt.clone();
    rsx! {
        section { class: "workflow-setup", aria_label: "Workflow setup prompt",
            h3 { "Set up your workflow" }
            p { class: "hint", "Describe how you work. Give the generated prompt to an agent in this project." }
            label { r#for: "workflow-goal", "Your workflow" }
            textarea { id: "workflow-goal", rows: "2", value: "{workflow}",
                placeholder: "For example: Rust development with tests and a separate code reviewer.",
                oninput: move |event| { workflow.set(event.value()); copied.set(false); error.set(false); }
            }
            button { class: "btn btn-primary", r#type: "button", disabled: project.is_empty(),
                onclick: move |_| {
                    let prompt=copy_prompt.clone();
                    spawn_local(async move {
                        let Some(window)=web_sys::window() else { error.set(true); return; };
                        let result=wasm_bindgen_futures::JsFuture::from(window.navigator().clipboard().write_text(&prompt)).await;
                        copied.set(result.is_ok()); error.set(result.is_err());
                    });
                },
                if copied() { "Prompt copied" } else { "Copy setup prompt" }
            }
            if error() { p { class: "hint", role: "status", "Clipboard unavailable. Select the prompt below and copy it." } }
            details {
                summary { "Preview setup prompt" }
                textarea { class: "workflow-prompt", rows: "10", readonly: true, aria_label: "Generated setup prompt", value: "{prompt}" }
            }
        }
    }
}
