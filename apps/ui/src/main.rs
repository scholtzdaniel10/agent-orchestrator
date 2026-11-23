fn main() {
    #[cfg(target_arch = "wasm32")]
    {
        std::panic::set_hook(Box::new(|info| {
            web_sys::console::error_1(&info.to_string().into());
        }));
        hide_browser_context_menu();
        dioxus::launch(legate_ui::app::App);
    }
}

/// WebView2's own right-click menu (Emoji, Cut, Copy, Paste, Inspect) is a browser's, not
/// Legate's. Legate's own menus (projects, chats, flow) are handlers on their elements and still
/// open; keyboard copy and paste keep working. DevTools stay on Ctrl+Shift+I in dev builds.
#[cfg(target_arch = "wasm32")]
fn hide_browser_context_menu() {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;

    let Some(window) = web_sys::window() else {
        return;
    };
    let handler = Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
        event.prevent_default();
    });
    let _ =
        window.add_event_listener_with_callback("contextmenu", handler.as_ref().unchecked_ref());
    handler.forget();
}
