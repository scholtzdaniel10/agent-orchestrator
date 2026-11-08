# Legate in Rust: rewrite plan and tracker

Branch `rust-rewrite` (worktree `C:\Users\schol\Desktop\Projects\legate-rust`).
Reference implementation: the TypeScript app at `origin/main` (`git show origin/main:<path>`),
plus the Live usage switch from commit `6f5ca70` on `launch-docs`.
liteagent reference: `C:\Users\schol\Desktop\Projects\liteagent` (Python), Rust port goes to its `rust` branch too.

## Done when

1. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` pass.
2. `cargo tauri build` makes a Windows installer, and the installed app starts and shows the project screen.
3. Router benchmark (criterion) beats the TypeScript router on the same inputs; numbers recorded below.
4. Local-models provider runs a job against llama-server or Ollama.
5. TypeScript app removed from the branch; README describes the Rust app; branch pushed; draft PR open.
6. liteagent `rust` branch holds the standalone crate with its tests.

## Stack

| Need | Choice | Why |
| --- | --- | --- |
| Desktop shell | Tauri 2 | Small installer, native webview, multi-window, updater |
| UI | Dioxus 0.7 (web/WASM) loaded by Tauri | Rust UI; calls Tauri commands via `window.__TAURI__` |
| Terminals in the UI | xterm.js via JS interop | No Rust terminal renderer for webviews matches it |
| Storage | rusqlite `bundled` by default; Turso behind feature `turso` | Same schema as `store.ts`. The multi-process check failed for Turso on Windows; see the Log |
| Local HTTP (MCP bridge) | axum + tokio | Bridge is localhost with a bearer token; no user accounts, so no axum_session_auth |
| Dev loop | bacon (`bacon.toml`) | check / clippy / test on save |
| PTY | portable-pty | ConPTY on Windows |
| CLI | clap + ratatui | Replaces ink |
| Not used | OpenProfit, axum_session_auth | No billing dashboard or sign-in in a local desktop app |

## Workspace layout

```
Cargo.toml                 workspace, shared deps
crates/legate-core         types.ts, settings, paths, ids, rules (no tokio; builds for wasm32)
crates/legate-store        store.ts on Turso, same tables and migrations
crates/legate-router       router.ts + orchestrator.ts
crates/legate-providers    claude, cursor, grok, process, edit-size, versions, local
crates/liteagent           Rust port of liteagent: llama-server client, prefix store, tool grammar, cascade, loop guard, Ollama
crates/legate-pty          pty/host.ts
crates/legate-git          worktrees, changes, history, files, github, worktree-guard, fresh-path
crates/legate-lead         lead, goals, goal-runtime, triggers, routines, plugins, setup
crates/legate-bridge       bridge/server.ts + tools.ts on axum
crates/legate-cli          `legate` binary
apps/desktop/src-tauri     Tauri app: OrchestratorApi as commands, on* as events, windows
apps/ui                    Dioxus UI port of src/renderer
```

## Faster router

- Keep decisions identical to `router.ts` (port every case in `router.test.ts` and `orchestrator.test.ts`).
- Per-plan usage cached in memory and updated on each run or usage event, so a pick never scans run history.
- Rolling weight per plan kept as a running sum with a time-ordered queue, evicted as the window slides.
- Rules validated once into fixed arrays indexed by job type and provider; picking allocates nothing.
- The orchestrator queue is event-driven on tokio channels, with no polling timers.
- `local` is a provider with its own plan and unlimited allowance. Rules decide which job types may fall back to it.

Same inputs: one planning pick, 10,000 runs, two windows (`five_hour` at 0.2, `seven_day` at 0.45) on each plan. Medians:

| Path | Providers scored | Median |
| --- | --- | --- |
| TypeScript `pickWithReason` | 3 (claude, cursor, grok; that router has no `local`) | 1.271 µs |
| Rust pure path | 4, including `local` | 2.439 µs |
| Rust `RouterState` | 4, including `local` | 73.0 ns |

`RouterState` is 17.4× the TypeScript baseline (1270.8 ns / 73.0 ns). The pure path rebuilds window snapshots on every call, so a warm TypeScript function beats it. Rust medians are criterion's median point estimates (50 samples, 1s warmup, 2s measurement). The TypeScript median is sample 50 of 101 samples of 2,000 calls after 200 warmup calls, timed with `performance.now()` through vitest.

## Phases

| Phase | Work | Agents | Status |
| --- | --- | --- | --- |
| 0 | Workspace, legate-core, legate-store, bacon.toml, CI, Turso check | 1 | done (core 33 tests, store 19; rusqlite default) |
| 1 | router, providers, liteagent, pty, git | 5 in parallel | done (334 workspace tests) |
| 2 | lead, bridge, local provider adapter | 3 in parallel | done (468 workspace tests) |
| 2b | legate-app: the main-process service (OrchestratorApi in Rust) wiring every crate | 1 | done (493 workspace tests, end-to-end local edit job) |
| 3 | Tauri app; Dioxus UI (3 slices); CLI | 5 | done (663 workspace tests; shell matches Electron layout; full CLI) |
| 4 | Integration, remove TypeScript, installer, README, PR, liteagent branch | 1-2 | pending |

## Rules for every agent

- Only edit the paths your task names. Do not commit; the orchestrator commits after verifying.
- Set `CARGO_TARGET_DIR` to `target/<your-crate>` so parallel builds don't block each other.
- Port the TypeScript tests for your module to Rust tests. Same inputs, same expected outputs.
- No `unwrap()` outside tests; errors via `thiserror` in libraries and `anyhow` in binaries.
- Report: files changed, test counts, any behavior you could not port and why.

## Follow-ups for phase 4

- A `maxParallel` change in Settings does not retune the running router; it needs a rules setter on the orchestrator.
- `JobRecord` has no created-at, so `legate jobs` prints `-` for age.
- `legate plans` shows "CLI version unknown" for local; show the Ollama or llama-server version.
- `Workspaces::remove` / `changes` are stubs (the orchestrator does not call them today).
- The updater is compiled into the shell but left off (`UPDATER_ENABLED` is false, and `bundle.createUpdaterArtifacts` is false). Turn it on when a release endpoint and pubkey exist. Notifications, UI zoom, and the single-instance lock are already wired.

## Log

- Integration note: the orchestrator's ports use `Rc` and non-`Send` futures. The desktop app and CLI run it on one dedicated thread with a tokio `LocalSet`, and talk to it through channels.

- Branches created: Legate `rust-rewrite` from `origin/main` (93612d5); liteagent `rust`.
- Phase 0 storage: the default backend is rusqlite 0.37 with the `bundled` feature. Turso 0.8.3 cannot share one database file between two OS processes on Windows, which the desktop app and the CLI both need (`PRAGMA busy_timeout = 5000` in `store.ts`). A throwaway program kept one connection open and spawned a second process against the same path. Exact failures:
  - Default `Builder::new_local(path).build()` in the child: `Locking error: Failed locking file. File is locked by another process`.
  - Parent open with `.experimental_multiprocess_wal(true)`: `experimental multiprocess WAL is not supported by the active IO backend for 'C:\Users\schol\AppData\Local\Temp\turso-mp-27912.db'`.
  - `Store::open` therefore uses rusqlite. The `turso` feature on `legate-store` still compiles `open_local` / `open_multiprocess` and is not the default. `crates/legate-store` test `multi_process_interleaved_writes` passed on rusqlite: two processes, both connections open, 40 inserts each, then 40 `parent` rows, 40 `child` rows, `resting_until("parent") == 2`, `resting_until("child") == 1`.
- Phase 1 router: `crates/legate-router` ports `router.ts` and `orchestrator.ts`. Router tests 17 (all 16 from `router.test.ts`, plus unlimited headroom when allowance is missing). Orchestrator tests 72/72. `RouterState` pick median 73.0 ns against the TypeScript median 1.271 µs (17.4×) on 10,000 runs and 2 windows.
- Phase 3a shell: Tauri 2 (`apps/desktop/src-tauri`, product Legate, identifier `dev.legate.app`) loads the Dioxus 0.7.10 WASM UI. 100 commands (every `AppHandle` method, plus the window commands `new_window`, `pop_out_terminal`, `popped_out`, `pop_out_panel`, `close_panel`, `popped_panels`, `open_file`, `reveal_project`, `reveal_folder`, `open_data_dir`). 11 events, named like the Electron channels: `jobs:update`, `plans:update`, `changes:update`, `goals:update`, `triggers:update`, `prefs:update`, `lead:update`, `terminals:data`, `terminals:update`, `terminals:popped`, `panels:popped`. `write_terminal`, `claim_terminal`, and `resize_terminal` take the calling window's stable id. `sleep_terminal` returns false for a popped-out terminal. Desktop notifications fire when a job ends and no window is focused, and only when `Prefs.notify` is set. UI zoom follows `Prefs.uiScale`. The updater plugin is not initialized: `UPDATER_ENABLED` is false in the shell and `bundle.createUpdaterArtifacts` is false, so the build does not check for updates. NSIS installer is per-user with the WebView2 download bootstrapper. Fully ported screens: Setup, ProjectBar (projects, GitHub clone, plan cards, UsageMeter), TerminalPane, TerminalTabs, and the pop-out TerminalWindow. The other renderer screens are placeholders with the TypeScript prop signatures. `apps/ui/src/wire.rs` repeats Goal, Trigger, Routine, Commit, and the requirement types as JSON because `legate-lead` and `legate-git` do not build for wasm32. `dx` 0.7.10 asks `cargo +nightly` for a unit-graph progress estimate and continues on stable when that fails; this machine's nightly toolchain has no rustc, and a half-installed nightly makes that probe hang, so the verified build skipped the probe and compiled the UI with stable 1.99. Workspace tests after 3a: 541 passed, 1 ignored. Phase 3 is not finished: the remaining UI slices are still open.
- CLI: `crates/legate-cli` ports `src/cli` (args, help, run, format, and the ink TUI on ratatui). Commands are `ui`, `status` (`plans` is the same command and is not added to the full help text), `job`, `jobs`, `show`, `changes`, `diff`, `merge`, `discard`, `pr`, `worktree`, `routine`, `goal`, and `goals`. The parser is the TypeScript one, so the help sentences and exit codes stay the ones the tests check; clap is only asked for the binary name, and `--help` / `--version` return before a tokio runtime or `AppHandle::start`. The TUI follows `AppHandle::subscribe` and redraws on a state change or a tick of at most 10 Hz. Tests in the crate: args 11/11, format 7/7 (plus the local version line), run 19/19, TUI state 8/8, screen 9/9, session 1/1. `ids.test.ts` (5) and `paths.test.ts` (3) were already in `legate-core`. Release wall time: `legate --help` 13.4 ms, `legate plans` 8.93 s (that is `AppHandle::start` probing the providers). Job age reads the store `created_at` column, then the earliest step `started_at`; `createdAt` was not added to `JobRecord` because the struct is built with literals in `apps/ui` and other crates. A local plan prints `CLI <ollama /api/version>` from `LocalAdapter::version` even though the router still reports local as unknown. Not ported: the Node version check in `src/cli/index.ts`.
