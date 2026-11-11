# Contributing

Legate is a **Rust** Cargo workspace. The Electron / TypeScript app has been replaced.

## Setup

1. Install stable Rust (MSVC toolchain on Windows) and WebView2.
2. `cargo install tauri-cli --version "^2" --locked`
3. `cargo install dioxus-cli --locked`
4. `rustup target add wasm32-unknown-unknown`
5. Optional: `cargo install bacon`

## Checks before a PR

```bash
cargo fmt --all --check
cargo clippy --workspace --exclude legate-desktop --all-targets --locked -- -D warnings
cargo test --workspace --exclude legate-desktop --locked
```

CI runs those on Windows and Ubuntu (`.github/workflows/rust.yml`). On Windows, also build the frontend and check the desktop shell:

```bash
cd apps/ui
dx build --release --platform web
cd ../..
cargo clippy -p legate-desktop --all-targets --locked -- -D warnings
cargo test -p legate-desktop --locked
```

## Layout

| Path | Role |
| --- | --- |
| `crates/*` | Core, store, router, providers, pty, git, lead, bridge, app, CLI, liteagent |
| `apps/ui` | Dioxus WASM UI |
| `apps/desktop/src-tauri` | Tauri shell |
| `resources/plugins` | Bundled agent skills |
| `docs/` | Rewrite plan, Windows install notes, screenshots |

See [README.md](README.md) and [docs/rust-rewrite.md](docs/rust-rewrite.md).
