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

## Desktop click-through

The Windows CI smoke test builds Legate and clicks through its screens over CDP, saving screenshots and an error report.
Trigger it with `gh workflow run clickthrough.yml --ref <branch>` on a non-main branch, or push a `clickthrough/*` branch.
Download the `desktop-clickthrough` artifact from the Actions run for `report.md`, `errors.json` and screenshots.

## Browser tests

The UI's screens only compile for wasm, so `cargo test --workspace` never runs them. `apps/ui/tests/` holds `wasm-bindgen-test` smoke tests that mount real screens in a browser against a stand-in for the Tauri bridge (`tests/common`). CI runs them in headless Chrome (the `ui-browser` job).

- `app_smoke`: mounts `App`, skips Setup, then opens and closes every panel in `DrawerTab::ALL`, every Settings section and the New task dialog. A step fails if the app panics (with the app's own panic message) or if closing a screen leaves a Tauri listener live. New panels and sections are picked up without changing the test.
- `setup_smoke`: Setup on its own, including a terminal update after it has closed.

Commands the stand-in does not answer resolve to `null`. If a screen needs real data to render, add its command to `answers()` in `app_smoke.rs`, built from the Rust type so the shape cannot drift.

```bash
cargo install wasm-bindgen-cli --version 0.2.130 --locked   # match wasm-bindgen in Cargo.lock
cargo test -p legate-ui --target wasm32-unknown-unknown --test app_smoke --test setup_smoke --locked
```

`.cargo/config.toml` points the wasm target at `wasm-bindgen-test-runner`. It needs a WebDriver: set `CHROMEDRIVER`, `GECKODRIVER` or `MSEDGEDRIVER`, or put one on PATH. Without one, set `NO_HEADLESS=1` and open the address it prints in any browser.

## Layout

| Path | Role |
| --- | --- |
| `crates/*` | Core, store, router, providers, pty, git, lead, bridge, app, CLI, liteagent |
| `apps/ui` | Dioxus WASM UI |
| `apps/desktop/src-tauri` | Tauri shell |
| `resources/plugins` | Bundled agent skills |
| `docs/` | Rewrite plan, Windows install notes, screenshots |

See [README.md](README.md) and [docs/rust-rewrite.md](docs/rust-rewrite.md).
