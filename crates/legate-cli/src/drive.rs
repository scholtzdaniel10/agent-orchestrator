//! The command layer talks to this, not to `AppHandle` directly.
//! Tests stand in a fake. The binary uses [`crate::app_drive::AppDrive`].

use std::sync::Arc;

use legate_core::{ChangeSet, JobAccess, JobRecord, JobType, ProjectInfo, ProviderId};
use legate_git::{ListedName, MergeOutcome, OpenedPr};
use legate_lead::{Goal, Routine};
use legate_providers::InteractiveCommand;
use tokio::sync::broadcast;

use crate::format::PlanView;

pub type Listener<T> = Arc<dyn Fn(T) + Send + Sync>;
pub type SigHandler = Arc<dyn Fn() + Send + Sync>;

#[derive(Debug)]
pub struct DriveError {
    pub message: String,
}

impl DriveError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for DriveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DriveError {}

impl From<String> for DriveError {
    fn from(message: String) -> Self {
        Self { message }
    }
}

impl From<&str> for DriveError {
    fn from(message: &str) -> Self {
        Self {
            message: message.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct JobRow {
    /// Epoch ms from the store. `None` when the row has no timestamp.
    pub created_at: Option<i64>,
    pub job: JobRecord,
}

#[derive(Debug, Clone)]
pub struct CreatedTree {
    pub id: String,
    pub path: String,
    pub branch: String,
}

#[derive(Debug, Clone)]
pub struct GoalRequest {
    pub project: String,
    pub text: String,
    pub check: Option<String>,
    pub max_rounds: f64,
}

#[derive(Debug, Clone)]
pub struct JobRequest {
    pub job_type: JobType,
    pub prompt: String,
    pub provider: Option<ProviderId>,
    pub access: JobAccess,
}

/// Drops the listener when dropped.
pub struct Unsub {
    close: Option<Box<dyn FnOnce() + Send>>,
}

impl Unsub {
    pub fn new(close: impl FnOnce() + Send + 'static) -> Self {
        Self {
            close: Some(Box::new(close)),
        }
    }
}

impl Drop for Unsub {
    fn drop(&mut self) {
        if let Some(close) = self.close.take() {
            close();
        }
    }
}

pub trait Drive: Send + Sync {
    fn version(&self) -> &str;
    fn project(&self) -> String;
    fn now(&self) -> i64;
    fn color(&self) -> bool;
    fn tty(&self) -> bool;
    fn term_size(&self) -> (u16, u16);
    fn stdout(&self, text: &str);
    fn stderr(&self, text: &str);
    fn stdin_is_tty(&self) -> bool;
    fn read_stdin(&self) -> impl Future<Output = String> + Send;
    fn ask(&self, prompt: &str) -> impl Future<Output = String> + Send;
    fn on_sigint(&self, handler: SigHandler);
    fn clear_sigint(&self) {}
    fn exit_now(&self, code: i32);
    fn init(&self) -> impl Future<Output = Result<(), DriveError>> + Send;
    fn plans(&self) -> impl Future<Output = Result<Vec<PlanView>, DriveError>> + Send;
    fn submit(
        &self,
        request: JobRequest,
    ) -> impl Future<Output = Result<JobRecord, DriveError>> + Send;
    fn cancel(&self, id: &str) -> impl Future<Output = Result<JobRecord, DriveError>> + Send;
    fn on_job(&self, cb: Listener<JobRecord>) -> Unsub;
    fn jobs_newest(&self, limit: i64) -> Vec<JobRow>;
    fn worktree_names(&self) -> Vec<ListedName>;
    fn set_worktree_name(&self, row: ListedName);
    fn delete_worktree_name(&self, id: &str);
    fn project_info(&self) -> impl Future<Output = Result<ProjectInfo, DriveError>> + Send;
    fn changes(&self) -> impl Future<Output = Result<Vec<ChangeSet>, DriveError>> + Send;
    fn change_ids(&self) -> impl Future<Output = Result<Vec<String>, DriveError>> + Send;
    fn folder(&self, id: &str) -> String;
    fn diff(&self, id: &str) -> impl Future<Output = Result<String, DriveError>> + Send;
    fn merge(&self, id: &str) -> impl Future<Output = Result<MergeOutcome, DriveError>> + Send;
    fn discard(&self, id: &str) -> impl Future<Output = Result<(), DriveError>> + Send;
    fn create_tree(
        &self,
        job_id: &str,
    ) -> impl Future<Output = Result<CreatedTree, DriveError>> + Send;
    fn routines(&self) -> impl Future<Output = Result<Vec<Routine>, DriveError>> + Send;
    fn run_routine(&self, id: &str) -> Result<JobRecord, DriveError>;
    fn note_job_finished(&self, _job: &JobRecord) {}
    fn open_pr(
        &self,
        title: &str,
        body: &str,
    ) -> impl Future<Output = Result<OpenedPr, DriveError>> + Send;
    fn goals(&self) -> impl Future<Output = Result<Vec<Goal>, DriveError>> + Send;
    fn create_goal(
        &self,
        input: GoalRequest,
    ) -> impl Future<Output = Result<Goal, DriveError>> + Send;
    fn approve_goal(&self, id: &str) -> Result<Goal, DriveError>;
    fn answer_goal(&self, id: &str, text: &str) -> Result<Goal, DriveError>;
    fn stop_goal(&self, id: &str) -> Result<Goal, DriveError>;
    fn on_goal(&self, cb: Listener<Goal>) -> Unsub;
    fn interactive(&self, provider: ProviderId) -> InteractiveCommand;
    fn spawn_inherited(
        &self,
        spec: &InteractiveCommand,
        cwd: &str,
    ) -> impl Future<Output = Result<(), DriveError>> + Send;
    /// Live pushes. `None` on the fake: the session uses [`Drive::on_job`] instead.
    fn events(&self) -> Option<broadcast::Receiver<LiveEvent>> {
        None
    }
}

/// What the terminal watches. Not a database poll.
#[derive(Debug, Clone)]
pub enum LiveEvent {
    Job(Box<JobRecord>),
    Plans(Vec<PlanView>),
    Changes,
}
