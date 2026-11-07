//! Display names and setup-command state for hand-made worktrees.

use std::collections::HashMap;

use legate_core::SetupState;
use serde_json::Value;

use crate::error::GitError;
use crate::gitpath::utf16_len;

const NAME_MESSAGE: &str = "Name must be 1 to 40 characters on one line.";
const COMMAND_MESSAGE: &str = "each command must be 1 to 300 characters on one line";

#[derive(Debug, Default, Clone)]
pub struct SetupTracker {
    by_change: HashMap<String, SetupState>,
}

impl SetupTracker {
    pub fn new() -> Self {
        Self {
            by_change: HashMap::new(),
        }
    }

    pub fn get(&self, id: &str) -> Option<SetupState> {
        self.by_change.get(id).copied()
    }

    pub fn clear(&mut self, id: &str) {
        self.by_change.remove(id);
    }

    /// `spawned` is false when the setup shell could not be opened.
    pub fn begin(&mut self, id: &str, spawned: bool) {
        self.by_change.insert(
            id.to_string(),
            if spawned {
                SetupState::Running
            } else {
                SetupState::Failed
            },
        );
    }

    /// Mirrors the main-process terminal listener for a shell tied to a worktree.
    pub fn on_terminal(
        &mut self,
        change: Option<&str>,
        provider: &str,
        removed: bool,
        status: &str,
        exit_code: Option<i64>,
    ) {
        if provider != "shell" {
            return;
        }
        let Some(change) = change else {
            return;
        };
        if removed {
            if self.by_change.get(change) == Some(&SetupState::Running) {
                self.by_change
                    .insert(change.to_string(), SetupState::Failed);
            }
        } else if status == "exited" {
            let state = if exit_code == Some(0) {
                SetupState::Done
            } else {
                SetupState::Failed
            };
            self.by_change.insert(change.to_string(), state);
        }
    }
}

pub fn parse_worktree_name(
    value: Option<&str>,
    required: bool,
) -> Result<Option<String>, GitError> {
    let Some(value) = value else {
        if required {
            return Err(GitError::new(NAME_MESSAGE));
        }
        return Ok(None);
    };
    if value.contains('\r') || value.contains('\n') {
        return Err(GitError::new(NAME_MESSAGE));
    }
    let trimmed = value.trim();
    if trimmed.is_empty() {
        if required {
            return Err(GitError::new(NAME_MESSAGE));
        }
        return Ok(None);
    }
    if utf16_len(trimmed) > 40 {
        return Err(GitError::new(NAME_MESSAGE));
    }
    Ok(Some(trimmed.to_string()))
}

pub fn parse_worktree_name_value(
    value: &Value,
    required: bool,
) -> Result<Option<String>, GitError> {
    match value {
        Value::Null => parse_worktree_name(None, required),
        Value::String(text) => parse_worktree_name(Some(text), required),
        _ => Err(GitError::new(NAME_MESSAGE)),
    }
}

pub fn parse_setup_commands(value: &Value) -> Result<Vec<String>, GitError> {
    let Some(items) = value.as_array() else {
        return Err(GitError::new("commands must be an array"));
    };
    if items.len() > 5 {
        return Err(GitError::new("at most 5 setup commands"));
    }
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let Some(text) = item.as_str() else {
            return Err(GitError::new(COMMAND_MESSAGE));
        };
        if text.is_empty() || utf16_len(text) > 300 || text.contains('\r') || text.contains('\n') {
            return Err(GitError::new(COMMAND_MESSAGE));
        }
        out.push(text.to_string());
    }
    Ok(out)
}
