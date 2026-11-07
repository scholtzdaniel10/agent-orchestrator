/// Told to every agent Legate starts. Other agent apps install skills and hooks for the
/// whole user account, so an agent here can read them and try to drive an app that is not running.
pub const LEGATE_NOTE: &str = "You are running inside Legate, which hands coding work to agents. \
Skills or instructions for Orca or other agent apps do not apply here: those apps are not running, \
so do not use the `orca` CLI.";

/// The prompt with the note before it, for CLIs that have no system prompt flag (Cursor).
pub fn with_legate_note(prompt: &str) -> String {
    format!("{LEGATE_NOTE}\n\n{prompt}")
}
