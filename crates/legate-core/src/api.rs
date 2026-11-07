//! Renderer-facing types from `src/preload/api-types.ts`.

use serde::{Deserialize, Serialize};

use crate::types::{ProviderId, VersionStatus};

/// Why a plan cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PlanProblem {
    #[serde(rename = "not-installed")]
    NotInstalled,
    #[serde(rename = "signed-out")]
    SignedOut,
}

/// One allowance window of a plan, for example Claude's five-hour and weekly windows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanWindow {
    pub name: String,
    /// Fraction used, 0..1.
    pub used: f64,
    /// Epoch ms it resets, or null when unknown.
    #[serde(default)]
    pub resets_at: Option<i64>,
}

/// One subscription plan as the sidebar shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStatus {
    pub id: ProviderId,
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<PlanProblem>,
    pub used: f64,
    #[serde(default)]
    pub resting_until: Option<i64>,
    #[serde(default)]
    pub resets_at: Option<i64>,
    pub at_risk: bool,
    pub busy: bool,
    pub running: i64,
    pub queued: i64,
    #[serde(default)]
    pub model: Option<String>,
    pub windows: Vec<PlanWindow>,
    #[serde(default)]
    pub usage_at: Option<i64>,
    #[serde(default)]
    pub version: Option<String>,
    pub version_status: VersionStatus,
    pub tested_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileStatus {
    Clean,
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub status: FileStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileList {
    pub entries: Vec<FileEntry>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum FileContent {
    #[serde(rename = "text")]
    Text { text: String, bytes: i64 },
    #[serde(rename = "binary")]
    Binary { bytes: i64 },
    #[serde(rename = "too-large")]
    TooLarge { bytes: i64 },
    #[serde(rename = "missing")]
    Missing,
}

/// Who a built-in skill is given to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GivenTo {
    #[serde(rename = "every agent")]
    EveryAgent,
    #[serde(rename = "workers")]
    Workers,
    #[serde(rename = "lead")]
    Lead,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuiltInSkill {
    pub name: String,
    #[serde(rename = "givenTo")]
    pub given_to: GivenTo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoundSkill {
    pub name: String,
    pub folder: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillFolderReport {
    pub folder: String,
    pub skills: Vec<FoundSkill>,
    pub problem: Option<String>,
}

/// Skills loaded into the agents Legate starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsState {
    pub built_in: Vec<BuiltInSkill>,
    pub folders: Vec<SkillFolderReport>,
}
