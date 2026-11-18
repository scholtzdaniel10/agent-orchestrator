//! Shared harness for the UI's browser smoke tests: a stand-in for the Tauri bridge and a few
//! DOM helpers. Each test file is its own page, so each installs the bridge once.
#![allow(dead_code)]

use dioxus::prelude::*;
use gloo_timers::future::TimeoutFuture;
use serde_json::{Map, Value};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// What the UI uses of `window.__TAURI__`: `core.invoke`, answered from `answers` (anything
/// else resolves to `null`), and `event.listen`, which resolves to an unlisten function.
/// `window.__legateTest` lets a test count listeners, emit events and see which commands ran.
const BRIDGE: &str = r#"
const listeners = {};
const calls = [];
window.__legateTest = {
  listening(name) { return (listeners[name] || []).length; },
  counts() {
    const live = Object.keys(listeners).filter((name) => listeners[name].length).sort();
    return JSON.stringify(Object.fromEntries(live.map((name) => [name, listeners[name].length])));
  },
  calls() { return calls.join(', '); },
  emit(name, payload) {
    for (const handler of [...(listeners[name] || [])]) handler({ event: name, id: 0, payload });
  },
};
window.__TAURI__ = {
  core: {
    invoke(cmd) {
      calls.push(cmd);
      return Promise.resolve(Object.prototype.hasOwnProperty.call(answers, cmd) ? answers[cmd] : null);
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
    pub fn listening(name: &str) -> u32;

    /// Live listeners as JSON, `{"event": count}`, sorted by event name.
    #[wasm_bindgen(js_namespace = ["window", "__legateTest"], js_name = counts)]
    pub fn listener_counts() -> String;

    #[wasm_bindgen(js_namespace = ["window", "__legateTest"], js_name = calls)]
    pub fn calls() -> String;

    #[wasm_bindgen(js_namespace = ["window", "__legateTest"], js_name = emit, catch)]
    pub fn emit(name: &str, payload: JsValue) -> Result<(), JsValue>;
}

thread_local! {
    static APP_PANIC: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Records the first panic anywhere in the page (the app's tasks included), then hands it on
/// to the hook it replaces. dioxus-web sets its own hook when it starts, so `mount` calls this
/// from the root component, after that and before any screen renders.
fn record_panics() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        APP_PANIC.with(|slot| {
            slot.borrow_mut().get_or_insert_with(|| info.to_string());
        });
        previous(info);
    }));
}

/// Fails the test if anything in the page has panicked.
pub fn assert_no_app_panic(during: &str) {
    if let Some(message) = APP_PANIC.with(|slot| slot.borrow().clone()) {
        panic!("The app panicked during {during}: {message}");
    }
}

/// Installs the stand-in bridge. `answers` maps a command name to what it resolves to.
pub fn install_bridge(answers: Map<String, Value>) {
    let answers = js_sys::JSON::parse(&Value::Object(answers).to_string()).expect("answers parse");
    js_sys::Function::new_with_args("answers", BRIDGE)
        .call1(&JsValue::NULL, &answers)
        .expect("the test bridge installs");
}

pub fn document() -> web_sys::Document {
    web_sys::window()
        .and_then(|window| window.document())
        .expect("a document")
}

thread_local! {
    static MOUNTED: std::cell::Cell<Option<fn() -> Element>> = const { std::cell::Cell::new(None) };
}

/// The page's root: installs the panic recorder first, then renders the mounted app in a
/// child scope of its own.
fn recorded_root() -> Element {
    use_hook(record_panics);
    rsx! { Mounted {} }
}

#[component]
fn Mounted() -> Element {
    let app = MOUNTED.with(std::cell::Cell::get).expect("an app to mount");
    app()
}

/// Mounts `app` into a fresh element with id `root_id`, starts it, and records its panics.
pub fn mount(root_id: &str, app: fn() -> Element) {
    MOUNTED.with(|slot| slot.set(Some(app)));
    let root = document().create_element("div").expect("a root element");
    root.set_id(root_id);
    document()
        .body()
        .expect("a body")
        .append_child(&root)
        .expect("the root mounts");
    dioxus::web::launch::launch_cfg(recorded_root, dioxus::web::Config::new().rootelement(root));
}

/// Text of the element with id `root_id`: what the app shows, not the runner's own page.
pub fn text_of(root_id: &str) -> String {
    document()
        .get_element_by_id(root_id)
        .and_then(|root| root.text_content())
        .unwrap_or_default()
}

pub fn exists(selector: &str) -> bool {
    document().query_selector(selector).ok().flatten().is_some()
}

/// The first element matching `selector` whose text is `label` (or the first match, without one).
pub fn find(selector: &str, label: Option<&str>) -> Option<web_sys::HtmlElement> {
    let found = document().query_selector_all(selector).ok()?;
    (0..found.length())
        .filter_map(|at| found.item(at))
        .filter_map(|node| node.dyn_into::<web_sys::HtmlElement>().ok())
        .find(|element| {
            label
                .is_none_or(|label| element.text_content().as_deref().map(str::trim) == Some(label))
        })
}

/// Waits up to five seconds for `ready`, then fails naming `what` and showing `root_id`'s text.
/// Fails at once if the app panics meanwhile.
pub async fn wait_for(root_id: &str, what: &str, ready: impl Fn() -> bool) {
    for _ in 0..250 {
        assert_no_app_panic(&format!("the wait for {what}"));
        if ready() {
            return;
        }
        TimeoutFuture::new(20).await;
    }
    panic!(
        "Timed out waiting for {what}.\nThe app shows: {}\nCommands called: {}",
        text_of(root_id),
        calls()
    );
}

/// Waits for a clickable match of `selector` (with text `label`, if given) and clicks it.
pub async fn click(root_id: &str, selector: &str, label: Option<&str>) {
    let what = format!("{selector} {}", label.unwrap_or_default());
    wait_for(root_id, &format!("{} to be clickable", what.trim()), || {
        find(selector, label).is_some_and(|element| !element.has_attribute("disabled"))
    })
    .await;
    find(selector, label)
        .expect("the element is still there")
        .click();
}

/// Lets queued tasks and renders run.
pub async fn settle() {
    TimeoutFuture::new(50).await;
}
