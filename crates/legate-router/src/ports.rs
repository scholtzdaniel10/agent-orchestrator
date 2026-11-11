//! Ports the orchestrator needs from crates that are still being rewritten.
//!
//! [`Runner`] starts one job and streams [`AgentEvent`]s. [`Workspaces`] creates the job's
//! folder. [`Clock`] is the time source tests advance by hand. Nothing here polls.

use std::future::Future;
use std::pin::Pin;

use legate_core::{AgentEvent, ChangedFile, ProviderId, RunOptions};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// One headless run, as `ProviderAdapter.run` returns it.
pub trait ActiveRun {
    fn next_event(&mut self) -> BoxFuture<'_, Option<AgentEvent>>;
    fn exit(&mut self) -> BoxFuture<'_, Result<ExitStatus, String>>;
    fn kill(&self);
    /// A kill handle that does not borrow this run, so cancel can fire while `next_event` is pending.
    fn killer(&self) -> std::rc::Rc<dyn Fn()>;
}

#[derive(Clone, Debug)]
pub struct ExitStatus {
    pub code: Option<i32>,
    pub stderr: String,
    pub unreadable: bool,
}

/// What the adapter receives. The TypeScript `Job` passed to `run` is id plus prompt.
#[derive(Clone, Debug)]
pub struct RunJob {
    pub id: String,
    pub prompt: String,
}

/// Starts a job for one provider. Implementations are the Claude, Cursor, Grok, and local runners.
pub trait Runner {
    fn id(&self) -> ProviderId;
    fn is_installed(&self) -> BoxFuture<'_, bool>;
    fn is_signed_in(&self) -> BoxFuture<'_, bool>;
    fn version(&self) -> BoxFuture<'_, Option<String>>;
    /// `true` when a non-zero exit's stderr means the plan hit its limit.
    fn is_limit_error(&self, stderr: &str) -> bool;
    fn start(
        &self,
        job: &RunJob,
        cwd: &str,
        opts: &RunOptions,
    ) -> Result<Box<dyn ActiveRun>, String>;
}

#[derive(Clone, Debug)]
pub struct WorktreeCreated {
    pub id: String,
    pub path: String,
    pub branch: String,
}

/// Job worktrees. Creating, removing, and listing changes belong to the git crate.
pub trait Workspaces {
    fn create(
        &self,
        project: &str,
        job_id: &str,
        base: Option<&str>,
    ) -> BoxFuture<'_, Result<WorktreeCreated, String>>;
    fn remove(&self, id: &str) -> BoxFuture<'_, Result<(), String>>;
    fn changes(&self, id: &str) -> BoxFuture<'_, Result<Vec<ChangedFile>, String>>;
}

/// Time and the orchestrator's heartbeat, watch, and step-flush callbacks.
///
/// Callbacks run on the orchestrator's thread. [`ManualClock`] fires them from [`ManualClock::tick`].
/// [`SystemClock`] sleeps until the deadline and then runs the callback. Neither polls the job queue.
pub trait Clock {
    fn now_ms(&self) -> i64;
    fn set_interval(&self, period_ms: i64, cb: std::rc::Rc<dyn Fn()>) -> u64;
    fn clear_interval(&self, id: u64);
    fn set_timeout(&self, wait_ms: i64, cb: std::rc::Rc<dyn Fn()>) -> u64;
    fn clear_timeout(&self, id: u64);
}
