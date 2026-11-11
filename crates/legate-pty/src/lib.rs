//! Interactive terminal sessions.
//!
//! This is the Rust port of `src/core/pty/host.ts`. A [`PtyHost`] opens a provider CLI
//! or a plain shell in a PTY, restores saved sessions from [`legate_store::Store`], and
//! publishes coalesced output for the UI.
//!
//! # Command lines
//!
//! Provider argv is **not** built here and this crate does not depend on `legate-providers`.
//! [`PtyHost::open`] decides which [`LaunchOpts`] fields are set, then calls the `launch`
//! closure the app supplies (the TypeScript `PtyHostOptions.launch`):
//!
//! - Claude's first start sets `session_id` (a new UUID). The Claude adapter turns that
//!   into `claude --session-id <uuid>`.
//! - Taking over a job, restoring, and waking set `resume`. Adapters turn that into
//!   `--resume <session>` (`claude --resume`, `agent --resume`, …).
//! - Cursor and Grok, with no prompt, set `resume` to the id from `create_chat`
//!   (`agent --resume <chat>`). With a prompt they set `session_id` instead, because
//!   Cursor drops the prompt when `--resume` is present; the adapter maps `sessionId`
//!   back to `--resume` and keeps the prompt. That mapping lives in the provider, same
//!   as `host.ts`.
//!
//! The only command line this crate builds itself is [`setup_launch`]: the shell script
//! `open_shell` runs (`powershell.exe -NoLogo -NoProfile -Command …` on Windows,
//! `sh -c "… && …"` elsewhere).
//!
//! # Output
//!
//! The real PTY is read on a dedicated thread. Bytes are decoded to UTF-8 (a split
//! codepoint is held until the next read) and coalesced: a chunk is published when it
//! reaches 32 KB or when 6 ms have passed since the first byte, whichever comes first.
//! The host then applies the TypeScript 12 ms flush before listeners and the broadcast
//! see it, so a pane never gets one event per byte. Late panes call [`PtyHost::snapshot`]
//! for the rolling scrollback (256_000 characters in memory, 200_000 in the store).
//!
//! # Resize claims
//!
//! [`PtyHost::resize`] always resizes a running session. [`PtyHost::resize_claimed`]
//! applies the focused-window rule from `claimTerminal`: the window that last wrote or
//! called [`PtyHost::claim_terminal`] owns the size; the first resize claims an unowned
//! terminal; other windows are ignored.

mod claims;
mod env;
mod error;
mod host;
mod launch;
mod process;
mod schedule;
mod syncutil;
mod textutil;

#[cfg(test)]
mod tests;

pub use claims::TerminalClaims;
pub use env::terminal_env;
pub use error::PtyError;
pub use host::{
    CloseWait, ExistsFn, LaunchFn, PtyHost, PtyHostOptions, Subscription, TerminalChunk, fixed_cwd,
};
pub use launch::{ChatFn, Launch, LaunchOpts, OpenOptions, ShellOptions, chat_fn, setup_launch};
pub use process::{KillTree, PtyProcess, QueuedPty, SpawnOpts, SpawnPty, spawn_pty};
pub use schedule::Scheduler;

pub use legate_core::{Platform, ProviderId, TerminalInfo, TerminalProvider, TerminalStatus};
pub use legate_store::{
    RunOutcome, RunRow, Store, StoreError, TERMINAL_SCROLLBACK_CAP, TerminalRow,
};

/// How long output waits so a burst becomes one chunk. Same as `FLUSH_MS` in `host.ts`.
pub const OUTPUT_FLUSH: std::time::Duration = std::time::Duration::from_millis(12);

/// Reader-side coalesce delay. The chunk is flushed at this age or at [`READER_COALESCE_BYTES`].
pub const READER_COALESCE: std::time::Duration = std::time::Duration::from_millis(6);

/// Reader-side coalesce size. A full buffer is published without waiting for the timer.
pub const READER_COALESCE_BYTES: usize = 32 * 1024;

/// In-memory scrollback kept for a pane that attaches late. `SNAPSHOT_MAX` in `host.ts`.
pub const SNAPSHOT_MAX_CHARS: usize = 256_000;

/// Largest write accepted, in Unicode scalar values. `WRITE_MAX` in `host.ts`.
pub const WRITE_MAX_CHARS: usize = 65_536;

/// Default cap on live sessions. Sleeping sessions do not count.
pub const DEFAULT_MAX_TERMINALS: usize = 16;

/// Mark prepended to restored scrollback. Kept as one string so snapshots match the app.
pub const RESTORE_MARK: &str = "\r\n\x1b[2m— restored session —\x1b[0m\r\n";

/// Mark inserted when a resume dies during probation and a new session starts in the same tab.
pub const FRESH_MARK: &str = "\r\n\x1b[2m— could not resume; started a new session —\x1b[0m\r\n";

/// Platform this process is running on, using [`Platform`]'s names.
pub fn host_platform() -> Platform {
    if cfg!(windows) {
        Platform::Win32
    } else if cfg!(target_os = "macos") {
        Platform::Darwin
    } else {
        Platform::Linux
    }
}
