//! Pushes the renderer used to receive on `on*` IPC channels.

use legate_core::{ChangeSet, JobRecord, LeadMessage, PlanStatus, Prefs, TerminalInfo};
use legate_lead::{Goal, Trigger};
use serde::{Deserialize, Serialize};

/// One broadcast the UI subscribes to.
///
/// Window pop-out lists stay in the Tauri shell. Plan pushes go out immediately,
/// and once more after a yield, matching `setTimeout(publishPlans, 0)`. Change
/// pushes that come from terminal output wait out the quiet period in `debounce`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppEvent {
    /// `jobs:update`
    Job(JobRecord),
    /// `plans:update`
    Plans(Vec<PlanStatus>),
    /// `changes:update`
    Changes(Vec<ChangeSet>),
    /// `goals:update`
    Goal(Goal),
    /// `triggers:update`
    Triggers(Vec<Trigger>),
    /// `prefs:update`
    Prefs(Prefs),
    /// `lead:update` — `chat_id` is null when the push did not name a chat.
    Lead {
        message: LeadMessage,
        #[serde(default)]
        chat_id: Option<String>,
    },
    /// `terminals:data`
    TerminalData { id: String, data: String },
    /// `terminals:update`
    TerminalUpdate { info: TerminalInfo, removed: bool },
}
