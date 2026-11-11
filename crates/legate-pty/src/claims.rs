use std::collections::HashMap;
use std::sync::Mutex;

use crate::syncutil::lock;

/// Which window last typed in or claimed each terminal. That window owns the pty size.
///
/// Port of `TerminalClaims` in `src/main/windows.ts`. The main process calls
/// [`TerminalClaims::claim`] on input and on `claimTerminal`, then
/// [`TerminalClaims::allow_resize`] before changing the pty size.
#[derive(Debug, Default)]
pub struct TerminalClaims {
    owners: Mutex<HashMap<String, u64>>,
}

impl TerminalClaims {
    pub fn new() -> Self {
        Self::default()
    }

    /// Remember that `window_id` is the focused owner of `terminal_id`.
    pub fn claim(&self, terminal_id: &str, window_id: u64) {
        lock(&self.owners).insert(terminal_id.to_string(), window_id);
    }

    /// True when this window owns the terminal, or nobody does (and then this window takes it).
    pub fn allow_resize(&self, terminal_id: &str, window_id: u64) -> bool {
        let mut owners = lock(&self.owners);
        match owners.get(terminal_id).copied() {
            None => {
                owners.insert(terminal_id.to_string(), window_id);
                true
            }
            Some(owner) => owner == window_id,
        }
    }

    /// Forget every terminal owned by a window that closed.
    pub fn release_window(&self, window_id: u64) {
        lock(&self.owners).retain(|_, owner| *owner != window_id);
    }
}
