//! SQLite store for runs, chats, jobs, and the rest of Legate's local database.
//!
//! The default connection is rusqlite with the bundled SQLite library, so an existing
//! Legate file opens unchanged and two processes can share it (`PRAGMA busy_timeout = 5000`).

mod migrate;
mod store;

#[cfg(feature = "turso")]
pub mod turso_backend;

#[cfg(test)]
mod tests;

pub use store::{
    ChatRow, NewWindow, RoutineRow, RunOutcome, RunRow, Store, StoreError, StoredJob, StoredWindow,
    TERMINAL_SCROLLBACK_CAP, TerminalRow, WorktreeNameRow, cap_scrollback,
};
