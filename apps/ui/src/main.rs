fn main() {
    #[cfg(target_arch = "wasm32")]
    {
        std::panic::set_hook(Box::new(|info| {
            web_sys::console::error_1(&info.to_string().into());
        }));
        dioxus::launch(legate_ui::app::App);
    }
}
