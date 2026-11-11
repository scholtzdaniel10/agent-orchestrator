//! Job router and orchestrator.
//!
//! Scoring matches `src/core/router/router.ts`. [`RouterState`] keeps per-plan usage in memory
//! so a pick does not scan run history. `local` is a normal provider; it has no allowance row
//! in the bundled rules, and a missing allowance is unlimited headroom (see [`RouterState`]).

mod clock;
mod ids;
mod orchestrator;
mod ports;
mod state;
mod usage;
mod versions;

pub use clock::{FrozenClock, ManualClock, SystemClock, system_now_ms};
pub use ids::{RunningCounts, jobs_on_plan};
pub use legate_core::{DEFAULT_MAX_PARALLEL, DEFAULT_PACE, plan_of};
pub use orchestrator::{
    CwdSource, JOB_HEARTBEAT_MS, JOB_STALE_MS, JOB_WATCH_MS, Orchestrator, OrchestratorError,
    OrchestratorOpts, SubmitAccess, SubmitExtra, Subscription, WatchGuard, WorkerInfo,
    WorkerWindow,
};
pub use ports::{
    ActiveRun, BoxFuture, Clock, ExitStatus, RunJob, Runner, Workspaces, WorktreeCreated,
};
pub use state::RouterState;
pub use usage::{
    CompiledRules, DASHBOARD_WINDOW, DashboardReading, Pick, PlanUsage, REASON_BEHIND,
    REASON_EXPIRING, REASON_FIRST, REASON_HEADROOM, REASON_ONLY, RouterError, Score, UsageSource,
    WindowState, add_months, dashboard_reading, dashboard_reading_value, headroom, pick_provider,
    pick_with_reason, plan_usage, run_weight, score_providers,
};
pub use versions::{compare_to_tested, stderr_is_limit, tested_version, unknown_format_message};
