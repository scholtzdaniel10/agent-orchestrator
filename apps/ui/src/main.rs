fn main() {
    #[cfg(target_arch = "wasm32")]
    dioxus::launch(legate_ui::app::App);
}
