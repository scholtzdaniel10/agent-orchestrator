//! Legate's web UI. Logic modules compile on the host so their tests run with
//! `cargo test --workspace`. The Dioxus screens compile for `wasm32` only.

pub mod logic;

#[cfg(target_arch = "wasm32")]
pub mod api;
#[cfg(target_arch = "wasm32")]
pub mod app;
#[cfg(target_arch = "wasm32")]
pub mod components;
#[cfg(target_arch = "wasm32")]
pub mod state;
#[cfg(target_arch = "wasm32")]
pub mod wire;
#[cfg(target_arch = "wasm32")]
pub mod xterm;
