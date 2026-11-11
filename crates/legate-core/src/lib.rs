//! Types, settings, paths, ids, and router rules shared by every Legate crate.
//!
//! No Tokio, and no dependency that needs a non-wasm target. The store and the router
//! build on these types.

pub mod api;
pub mod ids;
pub mod job;
pub mod paths;
pub mod rules;
pub mod settings;
pub mod types;

mod util;

pub use api::{
    BuiltInSkill, FileContent, FileEntry, FileList, FileStatus, FoundSkill, GivenTo, PlanProblem,
    PlanStatus, PlanWindow, SkillFolderReport, SkillsState,
};
pub use ids::{IdHit, IdItem, resolve_id};
pub use job::{JobRecord, JobStatus, JobStep, JobStepStatus};
pub use paths::{EnvVars, Platform, data_dir, legacy_data_dir, resolve_project};
pub use rules::{DEFAULT_MAX_PARALLEL, DEFAULT_PACE, RulesError, load_rules};
pub use settings::{DEFAULT_UI_SCALE, Settings, SettingsError, UI_SCALES, is_valid_model};
pub use types::{
    AgentEvent, AppInfo, BridgeInfo, ChangeSet, ChangedFile, CursorModels, DEFAULT_LOCAL_MODEL,
    DEFAULT_LOCAL_NUM_CTX, DEFAULT_LOCAL_URL, FileEdit, GROK_DEFAULT_MODEL, GithubRepo, JobAccess,
    JobRules, JobType, LOCAL_NUM_CTX_MAX, LOCAL_NUM_CTX_MIN, LeadChat, LeadMessage,
    LeadMessageKind, LeadProposal, LeadRole, LeadStatus, ModelOption, PROVIDERS, PaceRules,
    PlanAllowance, Prefs, ProjectEntry, ProjectInfo, ProjectWorktree, ProposalStatus, ProposedJob,
    ProviderId, RouterRules, RunOptions, SetupState, StepPhase, StepTool, TerminalInfo,
    TerminalProvider, TerminalStatus, UsageShows, UsageWindow, VersionStatus, access_of,
    is_job_access, is_job_type, is_provider_id, plan_of,
};
