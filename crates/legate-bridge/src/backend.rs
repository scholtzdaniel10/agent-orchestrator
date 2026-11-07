//! What the lead's tools ask the running app to do.
//!
//! Validation, access lowering, and the JSON text live in this crate. The
//! implementor only talks to the orchestrator.

use std::time::Duration;

use async_trait::async_trait;
use legate_core::{JobAccess, JobRecord, JobType, LeadProposal, ProviderId};

use crate::error::ToolError;

/// Failure talking to the orchestrator thread.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("{message}")]
pub struct BackendError {
    message: String,
}

impl BackendError {
    /// An error whose display text is `message`.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The text the tool will show the model.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl From<BackendError> for ToolError {
    fn from(err: BackendError) -> Self {
        ToolError::new(err.message)
    }
}

/// One worker, in the shape `list_workers` formats.
///
/// Copy these fields from `legate_router::WorkerInfo`: `id`, `available`,
/// `headroom`, `resting_until`, `busy`, and `queued`. That `resting_until` is
/// already `None` when the deadline is not still in the future. `headroom` is
/// the raw fraction; the bridge rounds it to two decimal places.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkerSnapshot {
    /// Plan id (`claude`, `cursor`, `grok`, or `local`).
    pub id: ProviderId,
    /// The worker can take a job right now.
    pub available: bool,
    /// Remaining allowance, before rounding.
    pub headroom: f64,
    /// Epoch milliseconds the plan rests until, when it is still resting.
    pub resting_until: Option<i64>,
    /// At least one job is running on this worker.
    pub busy: bool,
    /// Jobs waiting on this worker.
    pub queued: u64,
}

/// A job the bridge has already checked.
///
/// `access` is the level after the person's ceiling. On the orchestrator
/// thread, submit it with `SubmitAccess::Level(access)`, no chosen provider
/// (the router picks), and `SubmitExtra { lead_message, ..Default::default() }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewJob {
    /// `planning`, `debugging`, `review`, `refactor`, or `boilerplate`.
    pub job_type: JobType,
    /// The self-contained prompt. Not trimmed.
    pub prompt: String,
    /// Granted access. A request above the ceiling is already lowered.
    pub access: JobAccess,
    /// Lead message id for this turn, when there is one.
    pub lead_message: Option<String>,
}

/// Side effects of the lead tools.
///
/// Object-safe and [`Send`] so the HTTP server can hold it on the multi-thread
/// runtime. Each method should hop to the orchestrator's `LocalSet` thread and
/// wait for the reply.
///
/// | Tool | Methods |
/// | --- | --- |
/// | `list_workers` | [`max_access`](Self::max_access), [`workers`](Self::workers) |
/// | `send_job` | [`max_access`](Self::max_access), [`lead_turn`](Self::lead_turn), [`send_job`](Self::send_job) |
/// | `get_status` | [`list_jobs`](Self::list_jobs), [`get_job`](Self::get_job) |
/// | `get_result` | [`get_job`](Self::get_job), [`wait_for_job`](Self::wait_for_job) |
/// | `propose_jobs` | [`max_access`](Self::max_access), [`lead_turn`](Self::lead_turn), [`propose_jobs`](Self::propose_jobs) |
#[async_trait]
pub trait BridgeBackend: Send + Sync {
    /// Access ceiling for this call. The default is `read`.
    ///
    /// Called on every `list_workers`, `send_job`, and `propose_jobs`, not cached.
    #[allow(clippy::unused_async)]
    async fn max_access(&self) -> JobAccess {
        JobAccess::Read
    }

    /// Lead message id for this turn.
    ///
    /// `None` covers both "no callback" and a callback that returned null.
    /// `Some` is stored on the job as `leadMessage`.
    #[allow(clippy::unused_async)]
    async fn lead_turn(&self) -> Option<String> {
        None
    }

    /// Workers in display order. See [`WorkerSnapshot`].
    async fn workers(&self) -> Result<Vec<WorkerSnapshot>, BackendError>;

    /// Hand `job` to the router and return the record immediately.
    ///
    /// The record's `status` is whatever the orchestrator assigned (`running`,
    /// `queued`, `done`, or `failed`). The bridge copies `id`, `provider`, and
    /// `status` into the tool result.
    async fn send_job(&self, job: NewJob) -> Result<JobRecord, BackendError>;

    /// Every job on the board, oldest first, the same order as `Orchestrator::list`.
    async fn list_jobs(&self) -> Result<Vec<JobRecord>, BackendError>;

    /// One job, or `Ok(None)` when the id is unknown.
    async fn get_job(&self, id: &str) -> Result<Option<JobRecord>, BackendError>;

    /// Wait until `id` is done or failed, or until `wait` elapses.
    ///
    /// Return the latest record, including one that is still queued or running
    /// when the wait ends. Return `Ok(None)` if the id disappears.
    ///
    /// On the orchestrator thread this is `get` plus `on_update` until a
    /// terminal status, with a timeout. Subscribe before the second read so a
    /// completion between the two is not missed. The bridge calls this only
    /// when the job was still open and the wait is greater than zero.
    async fn wait_for_job(
        &self,
        id: &str,
        wait: Duration,
    ) -> Result<Option<JobRecord>, BackendError>;

    /// Store a plan for the person to run.
    ///
    /// Called only after the plan checks out and access above the ceiling has
    /// been lowered. `proposal.status` is `waiting`. Nothing should start.
    async fn propose_jobs(&self, turn: String, proposal: LeadProposal) -> Result<(), BackendError>;
}
