//! The vendored CodeMirror module, loaded when the first editor mounts.

use std::cell::{Cell, RefCell};

use js_sys::{Function, Promise};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Element, HtmlScriptElement};

const CODEMIRROR_URL: manganis::Asset = manganis::asset!("/assets/vendor/codemirror.mjs");
const LOAD_ERROR: &str = "The editor could not load.";

thread_local! {
    static LOAD: RefCell<Option<Promise>> = const { RefCell::new(None) };
    static LOADED: Cell<bool> = const { Cell::new(false) };
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "legateEditor"], js_name = open, catch)]
    fn open_js(
        id: &str,
        element: &Element,
        text: &str,
        path: &str,
        on_change: &Function,
        on_save: &Function,
    ) -> Result<f64, JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "legateEditor"], js_name = getText, catch)]
    fn get_text_js(id: &str) -> Result<String, JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "legateEditor"], js_name = setText, catch)]
    fn set_text_js(id: &str, text: &str) -> Result<(), JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "legateEditor"], js_name = markClean, catch)]
    fn mark_clean_js(id: &str) -> Result<(), JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "legateEditor"], js_name = focus, catch)]
    fn focus_js(id: &str) -> Result<(), JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "legateEditor"], js_name = dispose, catch)]
    fn dispose_js(id: &str, token: f64) -> Result<(), JsValue>;
}

pub async fn ensure_loaded() -> Result<(), String> {
    if LOADED.get() {
        return Ok(());
    }
    let promise = LOAD.with(|slot| -> Result<Promise, &'static str> {
        let mut slot = slot.borrow_mut();
        if let Some(promise) = slot.as_ref() {
            return Ok(promise.clone());
        }
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or(LOAD_ERROR)?;
        let head = document.head().ok_or(LOAD_ERROR)?;
        let script: HtmlScriptElement = document
            .create_element("script")
            .map_err(|_| LOAD_ERROR)?
            .unchecked_into();
        script.set_type("module");
        script.set_src(&CODEMIRROR_URL.to_string());
        let promise = Promise::new(&mut |resolve, reject| {
            script.set_onload(Some(&resolve));
            script.set_onerror(Some(&reject));
            if let Err(error) = head.append_child(&script) {
                let _ = reject.call1(&JsValue::UNDEFINED, &error);
            }
        });
        *slot = Some(promise.clone());
        Ok(promise)
    })?;
    JsFuture::from(promise).await.map_err(|_| LOAD_ERROR)?;
    LOADED.set(true);
    Ok(())
}

pub async fn open(
    id: &str,
    element: &Element,
    text: &str,
    path: &str,
    on_change: &Function,
    on_save: &Function,
) -> Result<f64, String> {
    ensure_loaded().await?;
    open_js(id, element, text, path, on_change, on_save).map_err(js_error)
}

pub fn get_text(id: &str) -> Result<String, String> {
    get_text_js(id).map_err(js_error)
}

pub fn set_text(id: &str, text: &str) -> Result<(), String> {
    set_text_js(id, text).map_err(js_error)
}

pub fn mark_clean(id: &str) -> Result<(), String> {
    mark_clean_js(id).map_err(js_error)
}

pub fn focus(id: &str) {
    let _ = focus_js(id);
}

pub fn dispose(id: &str, token: f64) {
    let _ = dispose_js(id, token);
}

fn js_error(value: JsValue) -> String {
    value
        .as_string()
        .or_else(|| {
            value
                .dyn_ref::<js_sys::Error>()
                .map(|error| error.message().into())
        })
        .unwrap_or_else(|| "The editor view failed.".into())
}
