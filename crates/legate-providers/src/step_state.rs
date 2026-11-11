use std::collections::HashMap;

use legate_core::{FileEdit, StepTool};

#[derive(Debug, Clone)]
pub(crate) struct PendingStep {
    pub tool: StepTool,
    pub title: String,
    pub edit: Option<FileEdit>,
}

/// Per-adapter memory so start and end pair, and paths can be relative to the session cwd.
#[derive(Debug, Default)]
pub(crate) struct StepParseState {
    cwd_by_session: HashMap<String, String>,
    pending: HashMap<String, PendingStep>,
    seq: u64,
}

impl StepParseState {
    pub(crate) fn cwd(&self, session_id: Option<&str>) -> Option<String> {
        session_id.and_then(|id| self.cwd_by_session.get(id).cloned())
    }

    pub(crate) fn remember_cwd(&mut self, session_id: impl Into<String>, cwd: impl Into<String>) {
        self.cwd_by_session.insert(session_id.into(), cwd.into());
    }

    pub(crate) fn remember(&mut self, id: impl Into<String>, info: PendingStep) {
        self.pending.insert(id.into(), info);
    }

    pub(crate) fn tool(&self, id: &str) -> Option<PendingStep> {
        self.pending.get(id).cloned()
    }

    pub(crate) fn next_id(&mut self, prefix: &str) -> String {
        self.seq += 1;
        format!("{prefix}-{}", self.seq)
    }
}
