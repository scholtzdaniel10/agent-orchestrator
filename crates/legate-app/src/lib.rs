//! The Legate main process, without Electron.
//!
//! [`AppHandle`] is cloneable, `Send`, and `Sync`. Its methods are the renderer's
//! `OrchestratorApi`. The orchestrator, lead, goals, and triggers live on one
//! thread inside a `LocalSet`. The bridge, terminals, and git commands run on the
//! multi-thread runtime that hosts that set.
//!
//! Window placement helpers ([`refuse_window`], [`cascade_bounds`]) stay here for
//! the Tauri shell. Opening OS windows, folder dialogs, and `shell.openPath` do not.

mod actions;
mod debounce;
mod error;
mod event;
mod handle;
mod options;
mod paths;
mod plugins;
mod session;
mod usage;
mod windows;
mod wire;

pub use debounce::{CHANGE_FALLBACK_MS, CHANGE_MIN_GAP_MS, CHANGE_QUIET_MS, ChangeRefresh, Timers};
pub use error::AppError;
pub use event::AppEvent;
pub use handle::AppHandle;
pub use options::{AppOptions, GoalInput, TerminalKind, TriggerInput, UsageOutcome, WorktreePick};
pub use paths::app_data_dir;
pub use usage::{USAGE_GAP_MS, USAGE_TICK_MS, UsageTrigger, WindowSight, usage_check_due};
pub use windows::{
    Bounds, CASCADE_OFFSET, MAX_WINDOWS, PoppedOutRegistry, Size, WINDOW_CAP_MESSAGE,
    cascade_bounds, refuse_window,
};

pub use legate_core::{
    AppInfo, ChangeSet, FileContent, FileList, JobAccess, JobRecord, JobType, LeadChat,
    LeadMessage, PlanStatus, Prefs, ProjectEntry, ProjectInfo, ProviderId,
};
pub use legate_git::{CommitDetail, CommitPage, MergeOutcome, OpenedPr};
pub use legate_lead::{Goal, Requirement, Routine, Trigger};
