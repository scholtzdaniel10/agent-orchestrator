//! What the Tauri shell (or a test) passes in, and the arguments that are not
//! a single `ProviderId`.

use std::path::PathBuf;
use std::sync::Arc;

use legate_core::{JobAccess, JobType, ProviderId};
use legate_providers::{ClaudeAdapter, CursorAdapter, LocalAdapter};
use serde::{Deserialize, Serialize};

/// Overrides for one process. Unset fields use the Electron app's locations.
#[derive(Default)]
pub struct AppOptions {
    /// Database and settings folder. `None` is `%APPDATA%\Legate` (or `LEGATE_DATA`).
    pub data_dir: Option<PathBuf>,
    /// Project to open, saved into settings when it is a folder.
    pub project: Option<PathBuf>,
    /// Worktree root. `None` is `ORCH_WORKTREES` or `~/.orchestrator/wt`.
    pub worktrees_dir: Option<PathBuf>,
    /// Replaces the Claude CLI adapter. Tests point this at a missing executable.
    pub claude: Option<Arc<ClaudeAdapter>>,
    /// Replaces the Cursor CLI adapter.
    pub cursor: Option<Arc<CursorAdapter>>,
    /// Replaces the Grok adapter (the Cursor CLI with a Grok model).
    pub grok: Option<Arc<CursorAdapter>>,
    /// Replaces the local adapter built from preferences.
    pub local: Option<Arc<LocalAdapter>>,
}

impl AppOptions {
    /// Default locations and the real CLIs.
    pub fn new() -> Self {
        Self::default()
    }
}

/// An interactive session: a plan's CLI, or a plain shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalKind {
    /// Claude, Cursor, Grok, or local.
    Plan(ProviderId),
    /// `powershell` on Windows, `sh` elsewhere. A missing prompt prints a marker.
    Shell,
}

/// `'new'` or an existing 8-character worktree id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreePick {
    /// Create a worktree, then open the terminal there.
    New,
    /// An existing change id.
    Existing(String),
}

/// A goal the person typed. Mirrors `createGoal`'s input object.
#[derive(Debug, Clone, PartialEq)]
pub struct GoalInput {
    pub text: String,
    pub check: Option<String>,
    pub max_rounds: Option<f64>,
    pub quick_type: Option<JobType>,
    pub quick_access: Option<JobAccess>,
}

/// A schedule or a GitHub watch.
#[derive(Debug, Clone, PartialEq)]
pub struct TriggerInput {
    pub kind: String,
    pub at: Option<String>,
    pub hours: Option<f64>,
    pub text: Option<String>,
    pub check: Option<String>,
    pub max_rounds: Option<f64>,
}

/// `{ ok, error? }` from a usage refresh or a dashboard reading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageOutcome {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl UsageOutcome {
    pub fn success() -> Self {
        Self {
            ok: true,
            error: None,
        }
    }

    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(message.into()),
        }
    }
}
