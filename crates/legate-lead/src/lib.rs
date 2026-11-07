//! Lead chat, goals, routines, triggers, plugins, and the setup checklist.
//!
//! # Threads
//!
//! The orchestrator in `legate-router` uses `Rc` and futures that are not `Send`. It runs on
//! one thread inside a tokio `LocalSet`. This crate does **not** call `Orchestrator` (and does
//! not depend on `legate-bridge`). [`Lead`] and [`Goals`] are `!Send` as well: they keep `Rc`
//! state and start follow-up work with `spawn_local`, so they belong on that same `LocalSet`.
//!
//! Jobs cross the boundary through [`JobSink`]. The desktop app implements the trait on the
//! orchestrator thread (direct method calls) or as the receiving end of a channel. The lead
//! session itself is a [`LeadAdapter`]. [`adapt_provider`] wraps `legate-providers`'
//! `ProviderAdapter`, which is what runs the lead CLI. `RunHandle` cannot be built outside that
//! crate, so tests supply their own `LeadAdapter`.
//!
//! [`TriggerService`] also runs on the `LocalSet` (`spawn_local`). A scheduled trigger sleeps
//! with `sleep_until` the next fire time. GitHub watches sleep until the next look (five
//! minutes after the previous one), not on a one-second poll. Call [`TriggerService::poke`]
//! after a trigger is saved so the sleep recomputes.
//!
//! # Plugins
//!
//! [`AgentPlugins`] and [`plugin_args`] are the types from `legate-providers`. This crate adds
//! discovery of the bundled skill folders, the person's skill folders, and the copied
//! `legate-your-skills` plugin. It does not replace that small struct.

mod error;
mod goal_runtime;
mod goals;
mod lead;
mod plugins;
mod ports;
mod prompts;
mod routines;
mod setup;
mod text;
mod triggers;
mod watch;

pub use error::LeadError;
pub use goal_runtime::{
    CheckOutput, PlanAdapter, PlanGoalDeps, PlanWorker, plan_goal, run_goal_check,
};
pub use goals::{
    ApplyResult, CreatedTree, Goal, GoalCheck, GoalChecks, GoalDeps, GoalExtra, GoalPlanner,
    GoalProposal, GoalRound, GoalSource, GoalStatus, GoalStore, GoalWorktrees, Goals,
    MAX_JOBS_PER_ROUND, NewGoal, ParsedPlan, PlannedJob, QuickJob, failure_key, parse_plan,
    plan_prompt,
};
pub use lead::{Lead, LeadOpts, NO_PLAN, project_line};
pub use plugins::{
    UserPluginBuild, build_user_plugin, bundled_plugins, bundled_skills, existing_user_plugin,
    skills_in, with_plugins,
};
pub use ports::{
    BridgeSource, CommandOutput, CommandRun, EmptyJobs, JobRequest, JobSink, LeadAdapter,
    LeadSession, SystemCommand, adapt_provider,
};
pub use prompts::{LEAD_DELEGATE_SKILL, LEAD_INSTRUCTIONS};
pub use routines::{Routine, RoutineAfter, RoutineStore, RoutineWorker, Routines, parse_routine};
pub use setup::{
    Platform, ProviderSetup, Requirement, RequirementId, RequirementNeed, RequirementState, Setup,
    SetupAdapter,
};
pub use triggers::{
    Firing, GithubFacts, IssueFact, NewTrigger, PullFact, RunFact, TRIGGER_KINDS, Trigger,
    TriggerKind, fresh_firings, github_firings, is_due, is_scheduled, is_trigger_kind, new_trigger,
    next_due, parse_issues, parse_pulls, parse_runs,
};
pub use watch::{
    GoalStarter, TriggerOpts, TriggerService, TriggerStore, github_facts, next_deadline_ms,
};

pub use legate_providers::{AgentPlugins, plugin_args};
