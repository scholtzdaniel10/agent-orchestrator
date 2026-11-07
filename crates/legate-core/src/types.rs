//! Domain types from `src/core/types.ts`, plus the Live usage preference.
//!
//! Serde matches the TypeScript JSON: camelCase fields, string enums as the TS literals,
//! `T | null` written as null, and `field?: T` omitted when absent. Tagged unions use `kind`.

use std::collections::BTreeMap;

#[cfg(test)]
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A worker Legate can run. `local` is the on-device model and spends its own plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    Claude,
    Cursor,
    Grok,
    Local,
}

impl ProviderId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Cursor => "cursor",
            Self::Grok => "grok",
            Self::Local => "local",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "claude" => Some(Self::Claude),
            "cursor" => Some(Self::Cursor),
            "grok" => Some(Self::Grok),
            "local" => Some(Self::Local),
            _ => None,
        }
    }
}

/// Every worker, in display order.
pub const PROVIDERS: [ProviderId; 4] = [
    ProviderId::Claude,
    ProviderId::Cursor,
    ProviderId::Grok,
    ProviderId::Local,
];

/// Model passed to the Cursor CLI when a Grok worker has no Grok model set.
pub const GROK_DEFAULT_MODEL: &str = "grok-4.7-high";

/// Which plan a worker spends. A worker with its own plan maps to itself.
/// Grok spends Cursor's plan. Local spends its own.
pub fn plan_of(id: ProviderId) -> ProviderId {
    match id {
        ProviderId::Grok => ProviderId::Cursor,
        other => other,
    }
}

pub fn is_provider_id(value: &str) -> bool {
    ProviderId::parse(value).is_some()
}

/// Interactive session: a plan, or a plain shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TerminalProvider {
    Claude,
    Cursor,
    Grok,
    Local,
    Shell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobType {
    Planning,
    Debugging,
    Review,
    Refactor,
    Boilerplate,
}

impl JobType {
    pub const ALL: [Self; 5] = [
        Self::Planning,
        Self::Debugging,
        Self::Review,
        Self::Refactor,
        Self::Boilerplate,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Planning => "planning",
            Self::Debugging => "debugging",
            Self::Review => "review",
            Self::Refactor => "refactor",
            Self::Boilerplate => "boilerplate",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "planning" => Some(Self::Planning),
            "debugging" => Some(Self::Debugging),
            "review" => Some(Self::Review),
            "refactor" => Some(Self::Refactor),
            "boilerplate" => Some(Self::Boilerplate),
            _ => None,
        }
    }
}

pub fn is_job_type(value: &str) -> bool {
    JobType::parse(value).is_some()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobAccess {
    Read,
    Edit,
    Full,
}

impl JobAccess {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Edit => "edit",
            Self::Full => "full",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "read" => Some(Self::Read),
            "edit" => Some(Self::Edit),
            "full" => Some(Self::Full),
            _ => None,
        }
    }
}

pub fn is_job_access(value: &str) -> bool {
    JobAccess::parse(value).is_some()
}

/// A record with `edit: true` and no `access` is `edit`; neither is `read`.
pub fn access_of(access: Option<&Value>, edit: Option<&Value>) -> JobAccess {
    if let Some(Value::String(text)) = access
        && let Some(parsed) = JobAccess::parse(text)
    {
        return parsed;
    }
    if matches!(edit, Some(Value::Bool(true))) {
        JobAccess::Edit
    } else {
        JobAccess::Read
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VersionStatus {
    Tested,
    Newer,
    Older,
    Unknown,
}

/// One allowance window of a plan, as its CLI reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    pub name: String,
    /// Fraction used, 0..1.
    pub utilization: f64,
    /// Epoch seconds when the window resets, when the CLI says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepTool {
    Say,
    Think,
    Read,
    Edit,
    Shell,
    Search,
    Web,
    Mcp,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepPhase {
    Start,
    End,
}

/// Lines added and removed by one edit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEdit {
    pub path: String,
    pub added: i64,
    pub removed: i64,
}

/// Provider-neutral view of one stream-json line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum AgentEvent {
    #[serde(rename = "init", rename_all = "camelCase")]
    Init {
        session_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "step")]
    Step {
        /// Stable per tool call, so `start` and `end` pair up.
        id: String,
        phase: StepPhase,
        tool: StepTool,
        /// Short and human: "Read src/app.ts", "Edited src/app.ts".
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        /// On `end`: false when the tool reported a failure.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ok: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        edit: Option<FileEdit>,
    },
    /// Fraction of the plan already used (0..1), the worst of its windows.
    #[serde(rename = "usage", rename_all = "camelCase")]
    Usage {
        utilization: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resets_at: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        windows: Option<Vec<UsageWindow>>,
    },
    /// The plan is spent. `resets_at` is epoch seconds when the CLI said so.
    #[serde(rename = "limit", rename_all = "camelCase")]
    Limit {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resets_at: Option<i64>,
        message: String,
    },
    #[serde(rename = "result", rename_all = "camelCase")]
    Result {
        ok: bool,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_ms: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cost_usd: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tokens: Option<i64>,
    },
}

/// How a CLI session reaches the app's local MCP bridge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeInfo {
    /// For example `http://127.0.0.1:53124/mcp`.
    pub url: String,
    /// Bearer token; a new one every app start.
    pub token: String,
    /// Tool names the bridge serves.
    pub tools: Vec<String>,
}

/// Options for one headless CLI run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct RunOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bridge: Option<BridgeInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edit: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access: Option<JobAccess>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_dirs: Option<Vec<String>>,
}

/// A model a CLI can run, for the model picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelOption {
    /// What is passed to the CLI's `--model` flag.
    pub id: String,
    pub label: String,
}

/// Provider order for one job type. The first entry is the first choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRules {
    pub planning: Vec<ProviderId>,
    pub debugging: Vec<ProviderId>,
    pub review: Vec<ProviderId>,
    pub refactor: Vec<ProviderId>,
    pub boilerplate: Vec<ProviderId>,
}

impl JobRules {
    pub fn get(&self, job: JobType) -> &[ProviderId] {
        match job {
            JobType::Planning => &self.planning,
            JobType::Debugging => &self.debugging,
            JobType::Review => &self.review,
            JobType::Refactor => &self.refactor,
            JobType::Boilerplate => &self.boilerplate,
        }
    }
}

/// Estimated allowance per plan, in the units of a run's weight, per rolling window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanAllowance {
    pub window_hours: f64,
    pub units: f64,
}

/// How routing leans toward a plan that is behind an even-usage pace.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaceRules {
    /// Score multiplier is `1 + weight * slack`.
    pub weight: f64,
    /// Lower bound of that multiplier.
    pub min: f64,
    /// Upper bound of that multiplier.
    pub max: f64,
    /// A plan is at risk of wasting allowance when slack is at least this.
    pub at_risk_slack: f64,
    /// And no more than this share of its window is left.
    pub at_risk_left: f64,
}

/// The settings file: router `rules.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterRules {
    pub rules: JobRules,
    pub fallback_fit: f64,
    /// Keyed by the plan a worker spends. Grok has no row; it spends Cursor's.
    pub allowance: BTreeMap<ProviderId, PlanAllowance>,
    /// Hours to rest a provider after a limit error that gave no reset time.
    pub default_rest_hours: f64,
    /// Absent when the file has no pace block. Callers then use [`crate::DEFAULT_PACE`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pace: Option<PaceRules>,
    /// How many jobs one plan may run at once. Clamped to 1..=8.
    pub max_parallel: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LeadRole {
    User,
    Lead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LeadStatus {
    Streaming,
    Done,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LeadMessageKind {
    Report,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProposalStatus {
    Waiting,
    Running,
    Dismissed,
}

/// One job in a lead's plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposedJob {
    pub title: String,
    #[serde(rename = "type")]
    pub job_type: JobType,
    pub access: JobAccess,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker: Option<ProviderId>,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeadProposal {
    pub summary: String,
    pub jobs: Vec<ProposedJob>,
    pub status: ProposalStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_ids: Option<Vec<String>>,
}

/// One message in the lead chat. A lead message grows while its turn streams.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeadMessage {
    pub id: String,
    pub role: LeadRole,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub status: LeadStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<LeadMessageKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<LeadProposal>,
    /// Keys the TypeScript object had that this struct does not name.
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeadChat {
    pub id: String,
    pub title: String,
    /// Epoch ms of the last message.
    pub updated_at: i64,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direct: Option<ProviderId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TerminalStatus {
    Running,
    Exited,
    Sleeping,
}

/// One interactive CLI session, shown as a terminal tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfo {
    pub id: String,
    pub provider: TerminalProvider,
    pub title: String,
    pub status: TerminalStatus,
    pub exit_code: Option<i64>,
    pub model: Option<String>,
    /// Epoch ms.
    pub started_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<String>,
}

/// One file touched by a job, as git reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFile {
    pub path: String,
    /// Lines added; null for a binary file.
    pub insertions: Option<i64>,
    /// Lines removed; null for a binary file.
    pub deletions: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SetupState {
    Running,
    Failed,
    Done,
}

/// A job's pending change: the worktree it edited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeSet {
    /// First 8 characters of the job id.
    pub id: String,
    /// Always `orch/<id>`.
    pub branch: String,
    pub path: String,
    pub files: Vec<ChangedFile>,
    pub insertions: i64,
    pub deletions: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<SetupState>,
}

/// The folder jobs and terminals work in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    pub path: String,
    pub is_repo: bool,
    pub branch: Option<String>,
}

/// `left` shows what remains of a plan. `used` matches Claude's own usage page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageShows {
    Left,
    Used,
}

/// Which Cursor models the pickers list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorModels {
    Own,
    All,
}

/// Preferences set on the Settings page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prefs {
    /// Jobs one plan may run at once, 1..=8. Null uses the rules file.
    pub max_parallel: Option<i64>,
    /// Terminal text size in pixels, 10..=22.
    pub terminal_font_size: i64,
    /// Desktop notification when a job ends while no app window is focused.
    pub notify: bool,
    /// Folder GitHub clones go into. Null asks every time.
    pub clone_dir: Option<String>,
    /// Whole-window scale in percent: 80, 90, 100, 110, or 120.
    pub ui_scale: i64,
    pub usage_shows: UsageShows,
    pub cursor_models: CursorModels,
    /// Read Claude's usage with a tiny turn every 10 minutes while a window is open.
    pub usage_check: bool,
}

/// Read-only facts for the Settings page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub data_dir: String,
    pub worktrees_dir: String,
    pub rules_max_parallel: i64,
}

/// One worktree of a project shown in the sidebar while another project is open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectWorktree {
    pub id: String,
    pub name: String,
    pub branch: String,
    pub files: i64,
    pub insertions: i64,
    pub deletions: i64,
    pub working: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectEntry {
    pub path: String,
    pub is_repo: bool,
    pub branch: Option<String>,
    pub active: bool,
    pub changes: i64,
    pub insertions: i64,
    pub deletions: i64,
    pub dirty: i64,
    pub ahead: i64,
    pub behind: i64,
    pub chats: i64,
    pub running: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktrees: Option<Vec<ProjectWorktree>>,
}

/// One repository from `gh repo list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubRepo {
    pub name_with_owner: String,
    pub description: String,
    pub is_private: bool,
    pub updated_at: String,
}

#[cfg(test)]
fn round_trip<T>(value: &T) -> Result<T, serde_json::Error>
where
    T: Serialize + DeserializeOwned,
{
    serde_json::from_str(&serde_json::to_string(value)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn providers_keep_display_order_and_local_spends_itself() {
        assert_eq!(
            PROVIDERS.map(ProviderId::as_str),
            ["claude", "cursor", "grok", "local"]
        );
        assert_eq!(plan_of(ProviderId::Grok), ProviderId::Cursor);
        assert_eq!(plan_of(ProviderId::Local), ProviderId::Local);
        assert_eq!(plan_of(ProviderId::Claude), ProviderId::Claude);
        assert!(is_provider_id("local"));
        assert!(!is_provider_id("codex"));
    }

    #[test]
    fn access_of_matches_the_typescript_record_rule() {
        let edit = Value::Bool(true);
        let read = Value::String("read".to_string());
        assert_eq!(access_of(Some(&read), Some(&edit)), JobAccess::Read);
        assert_eq!(access_of(None, Some(&edit)), JobAccess::Edit);
        assert_eq!(access_of(None, None), JobAccess::Read);
        assert_eq!(
            access_of(Some(&Value::Bool(true)), Some(&edit)),
            JobAccess::Edit
        );
    }

    #[test]
    fn agent_event_step_json_matches_typescript() {
        let event = AgentEvent::Step {
            id: "t1".to_string(),
            phase: StepPhase::End,
            tool: StepTool::Edit,
            title: "Edited src/app.ts".to_string(),
            detail: Some("updated".to_string()),
            ok: Some(true),
            edit: Some(FileEdit {
                path: "src/app.ts".to_string(),
                added: 3,
                removed: 1,
            }),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(
            json,
            concat!(
                r#"{"kind":"step","id":"t1","phase":"end","tool":"edit","#,
                r#""title":"Edited src/app.ts","detail":"updated","ok":true,"#,
                r#""edit":{"path":"src/app.ts","added":3,"removed":1}}"#
            )
        );
        assert_eq!(round_trip(&event).unwrap(), event);

        let bare = AgentEvent::Step {
            id: "t1".to_string(),
            phase: StepPhase::Start,
            tool: StepTool::Read,
            title: "Read src/app.ts".to_string(),
            detail: None,
            ok: None,
            edit: None,
        };
        assert_eq!(
            serde_json::to_string(&bare).unwrap(),
            r#"{"kind":"step","id":"t1","phase":"start","tool":"read","title":"Read src/app.ts"}"#
        );
    }
}
