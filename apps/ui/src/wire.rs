//! JSON copies of types that live outside `legate-core`.
//!
//! `legate-lead` and `legate-git` do not build for `wasm32`. Their serde shapes
//! are the contract; these structs read the same JSON.

use legate_core::{JobAccess, JobType};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RequirementId {
    Git,
    Claude,
    Codex,
    Cursor,
    Gh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RequirementNeed {
    Required,
    Plan,
    Optional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RequirementState {
    Ready,
    Missing,
    SignedOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Install {
    pub command: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Requirement {
    pub id: RequirementId,
    pub label: String,
    pub need: RequirementNeed,
    pub state: RequirementState,
    pub version: Option<String>,
    pub install: Install,
    #[serde(default)]
    pub sign_in: Option<String>,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedJob {
    #[serde(rename = "type")]
    pub job_type: JobType,
    pub access: JobAccess,
    pub prompt: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GoalStatus {
    Planning,
    Waiting,
    Running,
    Checking,
    Done,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalCheck {
    pub code: Option<i32>,
    pub output: String,
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalRound {
    pub n: i64,
    pub summary: String,
    pub jobs: Vec<PlannedJob>,
    pub job_ids: Vec<String>,
    pub problems: Vec<String>,
    pub check: Option<GoalCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalProposal {
    pub summary: String,
    pub jobs: Vec<PlannedJob>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalSource {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Goal {
    pub id: String,
    pub project: String,
    pub text: String,
    pub check: Option<String>,
    pub max_rounds: i64,
    pub quick: bool,
    pub worktree: String,
    pub status: GoalStatus,
    pub rounds: Vec<GoalRound>,
    pub proposal: Option<GoalProposal>,
    pub question: Option<String>,
    pub notes: Vec<String>,
    pub outcome: Option<String>,
    #[serde(default)]
    pub source: Option<GoalSource>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TriggerKind {
    Daily,
    Hourly,
    CiFailed,
    NewIssue,
    MainBroken,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Trigger {
    pub id: String,
    pub project: String,
    pub kind: TriggerKind,
    pub at: Option<String>,
    pub hours: Option<i64>,
    pub text: String,
    pub check: Option<String>,
    pub max_rounds: i64,
    pub enabled: bool,
    pub last_fired: Option<i64>,
    pub seen: Vec<String>,
    pub primed: bool,
    pub last_error: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineAfter {
    pub job_type: JobType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Routine {
    pub id: String,
    pub name: String,
    pub prompt: String,
    #[serde(rename = "type")]
    pub job_type: JobType,
    pub worker: String,
    pub access: JobAccess,
    #[serde(default)]
    pub after: Option<RoutineAfter>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    pub hash: String,
    pub short: String,
    pub parents: Vec<String>,
    pub author: String,
    pub time: i64,
    pub subject: String,
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitFile {
    pub path: String,
    pub added: Option<i64>,
    pub removed: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitDetail {
    pub hash: String,
    pub author: String,
    pub email: String,
    pub time: i64,
    pub message: String,
    pub files: Vec<CommitFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitPage {
    pub commits: Vec<Commit>,
    pub more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeOutcome {
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenedPr {
    pub url: String,
    pub branch: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageOutcome {
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalInput {
    pub text: String,
    #[serde(default)]
    pub check: Option<String>,
    #[serde(default)]
    pub max_rounds: Option<f64>,
    #[serde(default)]
    pub quick_type: Option<JobType>,
    #[serde(default)]
    pub quick_access: Option<JobAccess>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerInput {
    pub kind: String,
    #[serde(default)]
    pub at: Option<String>,
    #[serde(default)]
    pub hours: Option<f64>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub check: Option<String>,
    #[serde(default)]
    pub max_rounds: Option<f64>,
}
