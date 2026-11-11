//! Boundaries the app fills in.
//!
//! [`JobSink`] is how a lead turn, a routine, and a goal start work. [`BridgeSource`] is how a
//! lead turn learns the local MCP bridge. Neither trait mentions `Orchestrator` or the bridge
//! server. [`LeadAdapter`] is the CLI session; [`adapt_provider`] builds one from a
//! `ProviderAdapter`.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use legate_core::{AgentEvent, BridgeInfo, JobAccess, JobRecord, JobType, ProviderId, RunOptions};
use legate_providers::{LimitError, ProcessExit, ProviderAdapter, RunHandle};

use crate::error::LeadError;

/// One headless CLI session the lead (or a goal planner) can read.
#[async_trait]
pub trait LeadSession: Send {
    async fn recv(&mut self) -> Option<AgentEvent>;
    async fn exit_status(&mut self) -> ProcessExit;
    fn kill(&self);
}

/// What [`crate::Lead`] needs from a plan. Real CLIs use [`adapt_provider`].
#[async_trait]
pub trait LeadAdapter: Send + Sync {
    fn id(&self) -> ProviderId;
    async fn is_installed(&self) -> bool;
    async fn is_signed_in(&self) -> bool;
    /// `Err` is the TypeScript `run()` throw (`adapter exploded`).
    fn run(
        &self,
        job_id: &str,
        prompt: &str,
        cwd: &Path,
        opts: &RunOptions,
    ) -> Result<Box<dyn LeadSession>, String>;
    fn is_limit_error(&self, stderr: &str) -> bool;
}

struct ProviderSession {
    handle: RunHandle,
}

#[async_trait]
impl LeadSession for ProviderSession {
    async fn recv(&mut self) -> Option<AgentEvent> {
        self.handle.events.recv().await
    }

    async fn exit_status(&mut self) -> ProcessExit {
        self.handle.exit().await
    }

    fn kill(&self) {
        self.handle.kill();
    }
}

struct ProviderWrap(Arc<dyn ProviderAdapter>);

#[async_trait]
impl LeadAdapter for ProviderWrap {
    fn id(&self) -> ProviderId {
        self.0.id()
    }

    async fn is_installed(&self) -> bool {
        self.0.is_installed().await
    }

    async fn is_signed_in(&self) -> bool {
        self.0.is_signed_in().await
    }

    fn run(
        &self,
        job_id: &str,
        prompt: &str,
        cwd: &Path,
        opts: &RunOptions,
    ) -> Result<Box<dyn LeadSession>, String> {
        Ok(Box::new(ProviderSession {
            handle: self.0.run(job_id, prompt, cwd, opts),
        }))
    }

    fn is_limit_error(&self, stderr: &str) -> bool {
        self.0.is_limit_error(LimitError::Text(stderr))
    }
}

/// Wrap a Claude, Cursor, Grok, or local adapter so the lead can run it.
pub fn adapt_provider(adapter: Arc<dyn ProviderAdapter>) -> Arc<dyn LeadAdapter> {
    Arc::new(ProviderWrap(adapter))
}

/// A job the lead, a routine, or a goal wants started. Unset fields are left unset.
#[derive(Debug, Clone)]
pub struct JobRequest {
    pub job_type: JobType,
    pub prompt: String,
    pub provider: Option<ProviderId>,
    pub access: Option<JobAccess>,
    pub title: Option<String>,
    pub lead_message: Option<String>,
    pub routine: Option<String>,
    pub goal: Option<String>,
    pub base: Option<String>,
    pub group: Option<String>,
    pub project: Option<String>,
}

impl Default for JobRequest {
    fn default() -> Self {
        Self {
            job_type: JobType::Planning,
            prompt: String::new(),
            provider: None,
            access: None,
            title: None,
            lead_message: None,
            routine: None,
            goal: None,
            base: None,
            group: None,
            project: None,
        }
    }
}

/// Submit, list, cancel, and hear about jobs.
///
/// The callback is `Rc` because the orchestrator listener is `!Send`. Implement this on the
/// `LocalSet` thread. `on_update` returns an unsubscribe.
pub trait JobSink {
    fn submit(&self, request: JobRequest) -> Result<JobRecord, LeadError>;
    fn list(&self) -> Vec<JobRecord>;
    fn get(&self, id: &str) -> Option<JobRecord>;
    fn cancel(&self, id: &str) -> Result<(), LeadError>;
    fn on_update(&self, callback: std::rc::Rc<dyn Fn(JobRecord)>) -> Box<dyn FnOnce()>;
}

/// Sink used until [`crate::Lead::attach_jobs`]. Submit fails; nothing is listed.
pub struct EmptyJobs;

impl JobSink for EmptyJobs {
    fn submit(&self, _request: JobRequest) -> Result<JobRecord, LeadError> {
        Err(LeadError::message("jobs cannot start yet"))
    }

    fn list(&self) -> Vec<JobRecord> {
        Vec::new()
    }

    fn get(&self, _id: &str) -> Option<JobRecord> {
        None
    }

    fn cancel(&self, _id: &str) -> Result<(), LeadError> {
        Ok(())
    }

    fn on_update(&self, _callback: std::rc::Rc<dyn Fn(JobRecord)>) -> Box<dyn FnOnce()> {
        Box::new(|| {})
    }
}

/// The local MCP bridge a lead turn attaches to the CLI. Read on each turn.
pub trait BridgeSource: Send + Sync {
    fn info(&self) -> BridgeInfo;
}

impl BridgeSource for BridgeInfo {
    fn info(&self) -> BridgeInfo {
        self.clone()
    }
}

/// One process invocation. `code` is `None` when the process never exited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Run a CLI. `cwd` is the working directory, or the caller's when `None`.
#[async_trait]
pub trait CommandRun: Send + Sync {
    async fn run(
        &self,
        command: &str,
        args: &[String],
        cwd: Option<&str>,
    ) -> Result<CommandOutput, String>;
}

/// Spawn `command` with tokio. Dropping the future kills the process.
pub struct SystemCommand;

#[async_trait]
impl CommandRun for SystemCommand {
    async fn run(
        &self,
        command: &str,
        args: &[String],
        cwd: Option<&str>,
    ) -> Result<CommandOutput, String> {
        let mut cmd = tokio::process::Command::new(command);
        cmd.args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);
        if let Some(cwd) = cwd {
            cmd.current_dir(cwd);
        }
        let output = cmd.output().await.map_err(|err| err.to_string())?;
        Ok(CommandOutput {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}
