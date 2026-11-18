//! Browser smoke test for Setup. It mounts the real `SetupScreen` against a stand-in for the
//! Tauri bridge, closes it with "Skip for now", then sends a terminal update the way the shell
//! does. It catches the two ways Setup broke the whole UI:
//!
//! - its "Last check" clock read the system clock, which panics in wasm (13b9098);
//! - its terminal listener outlived it and read a dropped signal on the next update (836a913).
//!
//! Runs in a headless browser: see "Browser tests" in CONTRIBUTING.md.
#![cfg(target_arch = "wasm32")]

mod common;

use common::{
    assert_no_app_panic, click, document, emit, install_bridge, listening, mount, settle, text_of,
    wait_for,
};
use dioxus::prelude::*;
use legate_ui::components::SetupScreen;
use legate_ui::logic::terminal_bus::create_terminal_bus;
use serde_json::{Map, json};
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_browser);

const ROOT: &str = "setup-smoke-root";

#[component]
fn Host() -> Element {
    let mut open = use_signal(|| true);
    let bus = use_hook(|| create_terminal_bus(|_| Box::new(|| {}), None));
    rsx! {
        if open() {
            SetupScreen {
                bus: bus.clone(),
                on_close: move |_| open.set(false),
                on_skip: move |_| open.set(false),
            }
        } else {
            p { id: "setup-closed", "Setup closed" }
        }
    }
}

#[wasm_bindgen_test]
async fn setup_opens_closes_and_ignores_later_terminal_updates() {
    let mut answers = Map::new();
    answers.insert("setup_status".into(), json!([]));
    install_bridge(answers);
    mount(ROOT, Host);

    wait_for(ROOT, "Setup to show when it last checked", || {
        text_of(ROOT).contains("Last check ")
    })
    .await;
    wait_for(ROOT, "Setup to listen for terminal updates", || {
        listening("terminals:update") == 1
    })
    .await;

    click(ROOT, ".setup-footer button", Some("Skip for now")).await;
    wait_for(ROOT, "Setup to close", || {
        document().get_element_by_id("setup-closed").is_some()
    })
    .await;
    wait_for(ROOT, "Setup to stop listening once closed", || {
        listening("terminals:update") == 0
    })
    .await;

    let update = json!({
        "info": {
            "id": "term-1",
            "provider": "claude",
            "title": "Claude sign-in",
            "status": "exited",
            "exitCode": 0,
            "model": null,
            "startedAt": 0
        },
        "removed": false
    });
    let payload = serde_wasm_bindgen::to_value(&update).expect("the update converts");
    emit("terminals:update", payload).expect("a terminal update after Setup closed is harmless");
    settle().await;
    assert_no_app_panic("the terminal update after Setup closed");
    assert!(text_of(ROOT).contains("Setup closed"));
}
