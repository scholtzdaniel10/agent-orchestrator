//! xterm.js, fit, and WebGL, reached through a small page script.

use serde::Deserialize;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::Element;

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Cells {
    pub cols: f64,
    pub rows: f64,
}

const XTERM_URL: manganis::Asset = manganis::asset!("/assets/vendor/xterm.mjs");
const FIT_URL: manganis::Asset = manganis::asset!("/assets/vendor/addon-fit.mjs");
const WEBGL_URL: manganis::Asset = manganis::asset!("/assets/vendor/addon-webgl.mjs");

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "legateXterm"], js_name = open, catch)]
    async fn open_js(id: &str, element: &Element, font_size: f64) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "legateXterm"], js_name = write, catch)]
    fn write_js(id: &str, data: &str) -> Result<(), JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "legateXterm"], js_name = onData, catch)]
    fn on_data_js(id: &str, callback: &js_sys::Function) -> Result<(), JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "legateXterm"], js_name = fit, catch)]
    fn fit_js(id: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "legateXterm"], js_name = resize, catch)]
    fn resize_js(id: &str, cols: f64, rows: f64) -> Result<(), JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "legateXterm"], js_name = font, catch)]
    fn font_js(id: &str, size: f64) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "legateXterm"], js_name = focus, catch)]
    fn focus_js(id: &str) -> Result<(), JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "legateXterm"], js_name = disableStdin, catch)]
    fn disable_stdin_js(id: &str, disabled: bool) -> Result<(), JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "legateXterm"], js_name = dropWebgl, catch)]
    fn drop_webgl_js(id: &str) -> Result<(), JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "legateXterm"], js_name = dispose, catch)]
    fn dispose_js(id: &str) -> Result<(), JsValue>;
}

fn js_error(value: JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| "The terminal view failed.".into())
}

fn cells(value: JsValue) -> Result<Cells, String> {
    serde_wasm_bindgen::from_value(value).map_err(|err| err.to_string())
}

pub fn install_assets() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let urls = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&urls, &"xterm".into(), &XTERM_URL.to_string().into());
    let _ = js_sys::Reflect::set(&urls, &"fit".into(), &FIT_URL.to_string().into());
    let _ = js_sys::Reflect::set(&urls, &"webgl".into(), &WEBGL_URL.to_string().into());
    let _ = js_sys::Reflect::set(&window, &"legateAsset".into(), &urls);
}

pub async fn ready() -> bool {
    for _ in 0..40 {
        if let Some(window) = web_sys::window()
            && js_sys::Reflect::has(&window, &"legateXterm".into()).unwrap_or(false)
        {
            return true;
        }
        gloo_timers::future::TimeoutFuture::new(50).await;
    }
    false
}

pub async fn open(id: &str, element: &Element, font_size: f64) -> Result<Cells, String> {
    install_assets();
    if !ready().await {
        return Err("The terminal view has not loaded.".into());
    }
    cells(open_js(id, element, font_size).await.map_err(js_error)?)
}

pub fn write(id: &str, data: &str) -> Result<(), String> {
    write_js(id, data).map_err(js_error)
}

pub fn on_data(id: &str, callback: impl FnMut(String) + 'static) {
    let closure = Closure::wrap(Box::new(callback) as Box<dyn FnMut(String)>);
    let function: &js_sys::Function = closure.as_ref().unchecked_ref();
    let _ = on_data_js(id, function);
    closure.forget();
}

pub fn fit(id: &str) -> Result<Cells, String> {
    cells(fit_js(id).map_err(js_error)?)
}

pub fn resize(id: &str, cols: f64, rows: f64) -> Result<(), String> {
    resize_js(id, cols, rows).map_err(js_error)
}

pub fn font(id: &str, size: f64) -> Result<Cells, String> {
    cells(font_js(id, size).map_err(js_error)?)
}

pub fn focus(id: &str) -> Result<(), String> {
    focus_js(id).map_err(js_error)
}

pub fn disable_stdin(id: &str, disabled: bool) -> Result<(), String> {
    disable_stdin_js(id, disabled).map_err(js_error)
}

pub fn drop_webgl(id: &str) -> Result<(), String> {
    drop_webgl_js(id).map_err(js_error)
}

pub fn dispose(id: &str) -> Result<(), String> {
    dispose_js(id).map_err(js_error)
}
