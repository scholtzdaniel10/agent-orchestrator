//! Browser smoke test for Setup. It mounts the real `SetupScreen` against a stand-in for the
//! Tauri bridge, closes it with "Skip for now", then sends a terminal update the way the shell
//! does. It catches the two ways Setup broke the whole UI:
//!
//! - its "Last check" clock read the system clock, which panics in wasm (13b9098);
//! - its terminal listener outlived it and read a dropped signal on the next update (836a913).
//!
//! Runs in a headless browser: see "Browser tests" in CONTRIBUTING.md.
#![cfg(target_arch = "wasm32")]

use dioxus::prelude::*;
use gloo_timers::future::TimeoutFuture;
use legate_ui::components::SetupScreen;
use legate_ui::logic::terminal_bus::create_terminal_bus;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_browser);

/// What the UI uses of `window.__TAURI__`: `core.invoke` and `event.listen`, which resolves to
/// an unlisten function. `window.__legateTest` lets the test count listeners and emit events.
const BRIDGE: &str = r#"
const listeners = {};
window.__legateTest = {
  listening(name) { return (listeners[name] || []).length; },
  emit(name, payload) {
    for (const handler of [...(listeners[name] || [])]) handler({ event: name, id: 0, payload });
  },
};
window.__TAURI__ = {
  core: {
    invoke(cmd) {
      if (cmd === 'setup_status') return Promise.resolve([]);
      return Promise.reject('The test bridge does not answer ' + cmd + '.');
    },
  },
  event: {
    listen(name, handler) {
      (listeners[name] = listeners[name] || []).push(handler);
      return Promise.resolve(() => {
        const list = listeners[name] || [];
        const at = list.indexOf(handler);
        if (at >= 0) list.splice(at, 1);
      });
    },
  },
};
"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__legateTest"], js_name = listening)]
    fn listening(name: &str) -> u32;

    #[wasm_bindgen(js_namespace = ["window", "__legateTest"], js_name = emit, catch)]
    fn emit(name: &str, payload: JsValue) -> Result<(), JsValue>;
}

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

fn document() -> web_sys::Document {
    web_sys::window()
        .and_then(|window| window.document())
        .expect("a document")
}

const ROOT_ID: &str = "setup-smoke-root";

/// What the mounted app shows (not the runner's own page).
fn page_text() -> String {
    document()
        .get_element_by_id(ROOT_ID)
        .and_then(|root| root.text_content())
        .unwrap_or_default()
}

fn button(label: &str) -> Option<web_sys::HtmlElement> {
    let buttons = document().query_selector_all("button").ok()?;
    (0..buttons.length())
        .filter_map(|at| buttons.item(at))
        .filter_map(|node| node.dyn_into::<web_sys::HtmlElement>().ok())
        .find(|element| element.text_content().as_deref() == Some(label))
}

/// Waits up to five seconds for `ready`, then fails with `what`.
async fn wait_for(what: &str, ready: impl Fn() -> bool) {
    for _ in 0..250 {
        if ready() {
            return;
        }
        TimeoutFuture::new(20).await;
    }
    panic!(
        "Timed out waiting for {what}. The page shows: {}",
        page_text()
    );
}

#[wasm_bindgen_test]
async fn setup_opens_closes_and_ignores_later_terminal_updates() {
    js_sys::Function::new_no_args(BRIDGE)
        .call0(&JsValue::NULL)
        .expect("the test bridge installs");
    let root = document().create_element("div").expect("a root element");
    root.set_id(ROOT_ID);
    document()
        .body()
        .expect("a body")
        .append_child(&root)
        .expect("the root mounts");
    dioxus::web::launch::launch_cfg(Host, dioxus::web::Config::new().rootelement(root));

    wait_for("Setup to show when it last checked", || {
        page_text().contains("Last check ")
    })
    .await;
    wait_for("Setup to listen for terminal updates", || {
        listening("terminals:update") == 1
    })
    .await;

    button("Skip for now")
        .expect("a Skip for now button")
        .click();
    wait_for("Setup to close", || {
        document().get_element_by_id("setup-closed").is_some()
    })
    .await;
    wait_for("Setup to stop listening once closed", || {
        listening("terminals:update") == 0
    })
    .await;

    let update = serde_json::json!({
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
    TimeoutFuture::new(50).await;
    assert!(page_text().contains("Setup closed"));
}
