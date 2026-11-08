# Contributing

This branch is a **Rust** Cargo workspace. The Electron / TypeScript app is gone from `rust-rewrite`.

## Setup

1. Install stable Rust (MSVC toolchain on Windows) and WebView2.
2. `cargo install tauri-cli --version "^2" --locked`
3. `cargo install dioxus-cli --locked`
4. `rustup target add wasm32-unknown-unknown`
5. Optional: `cargo install bacon`

## Checks before a PR

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI runs those on Windows and Ubuntu (`.github/workflows/rust.yml`).

## Layout

| Path | Role |
| --- | --- |
| `crates/*` | Core, store, router, providers, pty, git, lead, bridge, app, CLI, liteagent |
| `apps/ui` | Dioxus WASM UI |
| `apps/desktop/src-tauri` | Tauri shell |
| `resources/plugins` | Bundled agent skills |
| `docs/` | Rewrite plan, Windows install notes, screenshots |

See [README.md](README.md) and [docs/rust-rewrite.md](docs/rust-rewrite.md).
