//! Adapters the orchestrator, the lead, and the bridge call.
//!
//! `RunHandle`'s kill handle is private to `legate-providers`, so a run is a task
//! that forwards events and calls `kill` when the orchestrator asks.

use std::path::Path;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;

use async_trait::async_trait;
use legate_bridge::{BackendError, BridgeBackend, NewJob, WorkerSnapshot};
use legate_core::{AgentEvent, JobAccess, JobRecord, ProviderId, RunOptions};
use legate_git::{WorktreeLoc, Worktrees};
use legate_lead::{
    ApplyResult, CreatedTree, GoalExtra, JobRequest, JobSink, LeadError, PlannedJob,
};
use legate_providers::{LimitError, ProcessExit, ProviderAdapter, RunHandle};
use legate_router::{
    ActiveRun, BoxFuture, ExitStatus, RunJob, Runner, SubmitAccess, SubmitExtra, WorkerInfo,
    Workspaces, WorktreeCreated,
};
use tokio::sync::{mpsc, oneshot};

use crate::session::{Command, World};

pub struct AdapterRunner {
    adapter: Arc<dyn ProviderAdapter>,
}

impl AdapterRunner {
    pub fn new(adapter: Arc<dyn ProviderAdapter>) -> Self {
        Self { adapter }
    }
}

impl Runner for AdapterRunner {
    fn id(&self) -> ProviderId {
        self.adapter.id()
    }

    fn is_installed(&self) -> BoxFuture<'_, bool> {
        let adapter = Arc::clone(&self.adapter);
        Box::pin(async move { adapter.is_installed().await })
    }

    fn is_signed_in(&self) -> BoxFuture<'_, bool> {
        let adapter = Arc::clone(&self.adapter);
        Box::pin(async move { adapter.is_signed_in().await })
    }

    fn version(&self) -> BoxFuture<'_, Option<String>> {
        let adapter = Arc::clone(&self.adapter);
        Box::pin(async move { adapter.version().await })
    }

    fn is_limit_error(&self, stderr: &str) -> bool {
        self.adapter.is_limit_error(LimitError::Text(stderr))
    }

    fn start(
        &self,
        job: &RunJob,
        cwd: &str,
        opts: &RunOptions,
    ) -> Result<Box<dyn ActiveRun>, String> {
        let handle = self.adapter.run(&job.id, &job.prompt, Path::new(cwd), opts);
        Ok(Box::new(forward(handle)))
    }
}

struct Forwarded {
    events: mpsc::UnboundedReceiver<AgentEvent>,
    exit: Option<oneshot::Receiver<Result<ExitStatus, String>>>,
    kill_tx: mpsc::UnboundedSender<()>,
}

fn forward(mut handle: RunHandle) -> Forwarded {
    let (event_tx, events) = mpsc::unbounded_channel();
    let (exit_tx, exit) = oneshot::channel();
    let (kill_tx, mut kill_rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut killed = false;
        loop {
            tokio::select! {
                biased;
                _ = kill_rx.recv() => {
                    killed = true;
                    break;
                }
                event = handle.events.recv() => {
                    match event {
                        Some(event) => {
                            if event_tx.send(event).is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }
            }
        }
        if killed {
            handle.kill();
        }
        let status = handle.exit().await;
        let _ = exit_tx.send(Ok(map_exit(status)));
    });
    Forwarded {
        events,
        exit: Some(exit),
        kill_tx,
    }
}

fn map_exit(status: ProcessExit) -> ExitStatus {
    ExitStatus {
        code: status.code,
        stderr: status.stderr,
        unreadable: status.unreadable,
    }
}

impl ActiveRun for Forwarded {
    fn next_event(&mut self) -> BoxFuture<'_, Option<AgentEvent>> {
        Box::pin(async move { self.events.recv().await })
    }

    fn exit(&mut self) -> BoxFuture<'_, Result<ExitStatus, String>> {
        let exit = self.exit.take();
        Box::pin(async move {
            match exit {
                Some(exit) => exit
                    .await
                    .unwrap_or_else(|_| Err("provider run ended without a status".to_string())),
                None => Err("provider run ended without a status".to_string()),
            }
        })
    }

    fn kill(&self) {
        let _ = self.kill_tx.send(());
    }

    fn killer(&self) -> Rc<dyn Fn()> {
        let tx = self.kill_tx.clone();
        Rc::new(move || {
            let _ = tx.send(());
        })
    }
}

pub struct GitSpaces {
    root: std::path::PathBuf,
}

impl GitSpaces {
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn trees(&self) -> Worktrees {
        Worktrees::new(&self.root)
    }
}

impl Workspaces for GitSpaces {
    fn create(
        &self,
        project: &str,
        job_id: &str,
        base: Option<&str>,
    ) -> BoxFuture<'_, Result<WorktreeCreated, String>> {
        let trees = self.trees();
        let project = project.to_string();
        let job_id = job_id.to_string();
        let base = base.map(str::to_string);
        Box::pin(async move {
            let loc = trees
                .create(Path::new(&project), &job_id, base.as_deref())
                .await
                .map_err(|err| err.to_string())?;
            Ok(created(loc))
        })
    }

    fn remove(&self, _id: &str) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }

    fn changes(&self, _id: &str) -> BoxFuture<'_, Result<Vec<legate_core::ChangedFile>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

fn created(loc: WorktreeLoc) -> WorktreeCreated {
    WorktreeCreated {
        id: loc.id,
        path: loc.path.to_string_lossy().into_owned(),
        branch: loc.branch,
    }
}

pub struct OrchJobs {
    orch: Rc<legate_router::Orchestrator>,
    project: Option<String>,
}

impl OrchJobs {
    pub fn new(orch: Rc<legate_router::Orchestrator>) -> Self {
        Self {
            orch,
            project: None,
        }
    }
    pub fn for_project(orch: Rc<legate_router::Orchestrator>, project: String) -> Self {
        Self {
            orch,
            project: Some(project),
        }
    }
}

impl JobSink for OrchJobs {
    fn submit(&self, request: JobRequest) -> Result<JobRecord, LeadError> {
        let access = request.access.map(SubmitAccess::Level);
        Ok(self.orch.submit(
            request.job_type,
            &request.prompt,
            request.provider,
            access,
            SubmitExtra {
                group: request.group,
                lead_message: request.lead_message,
                routine: request.routine,
                goal: request.goal,
                base: request.base,
                title: request.title,
                project: self.project.clone().or(request.project),
            },
        ))
    }

    fn list(&self) -> Vec<JobRecord> {
        match &self.project {
            Some(project) => self.orch.list_project(project),
            None => self.orch.list(),
        }
    }

    fn get(&self, id: &str) -> Option<JobRecord> {
        match &self.project {
            Some(project) => self.orch.project_job(project, id),
            None => self.orch.get(id),
        }
    }

    fn cancel(&self, id: &str) -> Result<(), LeadError> {
        if self
            .project
            .as_ref()
            .is_some_and(|project| self.orch.project_of(id).as_ref() != Some(project))
        {
            return Err(LeadError::message("job belongs to another project"));
        }
        self.orch
            .cancel(id)
            .map(|_| ())
            .map_err(|err| LeadError::message(err.to_string()))
    }

    fn on_update(&self, callback: Rc<dyn Fn(JobRecord)>) -> Box<dyn FnOnce()> {
        let orch = self.orch.clone();
        let project = self.project.clone();
        let sub = self.orch.on_update(Rc::new(move |job| {
            if project
                .as_ref()
                .is_none_or(|owner| orch.project_of(&job.id).as_ref() == Some(owner))
            {
                callback(job);
            }
        }));
        Box::new(move || drop(sub))
    }
}

pub fn goal_submit(
    orch: &Rc<legate_router::Orchestrator>,
) -> Rc<dyn Fn(PlannedJob, GoalExtra) -> Result<JobRecord, LeadError>> {
    let orch = Rc::clone(orch);
    Rc::new(move |job, extra| {
        Ok(orch.submit(
            job.job_type,
            &job.prompt,
            None,
            Some(SubmitAccess::Level(job.access)),
            SubmitExtra {
                goal: Some(extra.goal),
                base: Some(extra.base),
                project: Some(extra.project),
                ..SubmitExtra::default()
            },
        ))
    })
}

pub struct GitGoals {
    root: std::path::PathBuf,
}

impl GitGoals {
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn trees(&self) -> Worktrees {
        Worktrees::new(&self.root)
    }
}

#[async_trait(?Send)]
impl legate_lead::GoalWorktrees for GitGoals {
    async fn create(&self, project: &str, id: &str) -> Result<CreatedTree, String> {
        let loc = self
            .trees()
            .create(Path::new(project), id, None)
            .await
            .map_err(|err| err.to_string())?;
        Ok(CreatedTree {
            id: loc.id,
            path: loc.path.to_string_lossy().into_owned(),
            branch: loc.branch,
        })
    }

    fn folder(&self, id: &str) -> String {
        self.trees()
            .folder(id)
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn branch_of(&self, id: &str) -> String {
        self.trees()
            .branch_of(id)
            .unwrap_or_else(|_| format!("orch/{id}"))
    }

    async fn apply_into(
        &self,
        from_id: &str,
        target: &str,
        base: &str,
    ) -> Result<ApplyResult, String> {
        let outcome = self
            .trees()
            .apply_into(from_id, Path::new(target), base)
            .await
            .map_err(|err| err.to_string())?;
        Ok(ApplyResult {
            ok: outcome.ok,
            empty: outcome.empty,
            message: outcome.message,
        })
    }

    async fn commit_all(&self, dir: &str, message: &str) -> Result<bool, String> {
        self.trees()
            .commit_all(Path::new(dir), message)
            .await
            .map_err(|err| err.to_string())
    }

    async fn discard(&self, project: &str, id: &str) -> Result<(), String> {
        self.trees()
            .discard(Path::new(project), id)
            .await
            .map_err(|err| err.to_string())
    }
}

/// Hop from the bridge's thread onto the orchestrator's `LocalSet`.
pub struct AppBackend {
    tx: mpsc::UnboundedSender<Command>,
    project: String,
}

impl AppBackend {
    pub fn new(tx: mpsc::UnboundedSender<Command>, project: String) -> Self {
        Self { tx, project }
    }

    fn ask<T, F>(&self, f: F) -> Pin<Box<dyn Future<Output = Result<T, BackendError>> + Send>>
    where
        T: Send + 'static,
        F: FnOnce(&World) -> Pin<Box<dyn Future<Output = Result<T, BackendError>> + 'static>>
            + Send
            + 'static,
    {
        let tx = self.tx.clone();
        Box::pin(async move {
            let (reply_tx, reply_rx) = oneshot::channel();
            tx.send(Command::Run(Box::new(move |world| {
                let fut = f(world);
                Box::pin(async move {
                    let _ = reply_tx.send(fut.await);
                })
            })))
            .map_err(|_| BackendError::new("legate is shutting down"))?;
            reply_rx
                .await
                .map_err(|_| BackendError::new("legate is shutting down"))?
        })
    }
}

use std::future::Future;

#[async_trait]
impl BridgeBackend for AppBackend {
    async fn max_access(&self) -> JobAccess {
        self.ask(move |world| {
            let access = world.settings.borrow().lead_access();
            Box::pin(async move { Ok(access) })
        })
        .await
        .unwrap_or(JobAccess::Read)
    }

    async fn lead_turn(&self) -> Option<String> {
        let project = self.project.clone();
        self.ask(move |world| {
            let turn = world
                .lead_sessions
                .borrow()
                .get(&project)
                .and_then(|lead| lead.current_turn());
            Box::pin(async move { Ok(turn) })
        })
        .await
        .ok()
        .flatten()
    }

    async fn workers(&self) -> Result<Vec<WorkerSnapshot>, BackendError> {
        self.ask(move |world| {
            let workers = world
                .orch
                .workers()
                .into_iter()
                .map(snapshot)
                .collect::<Vec<_>>();
            Box::pin(async move { Ok(workers) })
        })
        .await
    }

    async fn send_job(&self, job: NewJob) -> Result<JobRecord, BackendError> {
        let project = self.project.clone();
        self.ask(move |world| {
            let record = world.orch.submit(
                job.job_type,
                &job.prompt,
                None,
                Some(SubmitAccess::Level(job.access)),
                SubmitExtra {
                    lead_message: job.lead_message,
                    project: Some(project),
                    ..SubmitExtra::default()
                },
            );
            Box::pin(async move { Ok(record) })
        })
        .await
    }

    async fn list_jobs(&self) -> Result<Vec<JobRecord>, BackendError> {
        let project = self.project.clone();
        self.ask(move |world| {
            let jobs = world.orch.list_project(&project);
            Box::pin(async move { Ok(jobs) })
        })
        .await
    }

    async fn get_job(&self, id: &str) -> Result<Option<JobRecord>, BackendError> {
        let id = id.to_string();
        let project = self.project.clone();
        self.ask(move |world| {
            let job = world.orch.project_job(&project, &id);
            Box::pin(async move { Ok(job) })
        })
        .await
    }

    async fn wait_for_job(
        &self,
        id: &str,
        wait: std::time::Duration,
    ) -> Result<Option<JobRecord>, BackendError> {
        let id = id.to_string();
        let project = self.project.clone();
        self.ask(move |world| {
            let orch = Rc::clone(&world.orch);
            Box::pin(async move {
                let Some(job) = orch.project_job(&project, &id) else {
                    return Ok(None);
                };
                if matches!(
                    job.status,
                    legate_core::JobStatus::Done | legate_core::JobStatus::Failed
                ) {
                    return Ok(Some(job));
                }
                Ok(wait_job(&orch, &id, wait).await)
            })
        })
        .await
    }

    async fn propose_jobs(
        &self,
        turn: String,
        proposal: legate_core::LeadProposal,
    ) -> Result<(), BackendError> {
        let project = self.project.clone();
        self.ask(move |world| {
            let lead = world.lead_sessions.borrow().get(&project).cloned();
            let result = lead
                .ok_or_else(|| BackendError::new("project session is unavailable"))
                .and_then(|lead| {
                    lead.attach_proposal(&turn, proposal)
                        .map_err(|err| BackendError::new(err.to_string()))
                });
            Box::pin(async move { result })
        })
        .await
    }
}

fn snapshot(worker: WorkerInfo) -> WorkerSnapshot {
    WorkerSnapshot {
        id: worker.id,
        available: worker.available,
        headroom: worker.headroom,
        resting_until: worker.resting_until,
        busy: worker.busy,
        queued: u64::try_from(worker.queued).unwrap_or(u64::MAX),
    }
}

async fn wait_job(
    orch: &Rc<legate_router::Orchestrator>,
    id: &str,
    wait: std::time::Duration,
) -> Option<JobRecord> {
    let notify = std::sync::Arc::new(tokio::sync::Notify::new());
    let flag = std::rc::Rc::new(std::cell::Cell::new(false));
    let id_owned = id.to_string();
    let sub = {
        let notify = std::sync::Arc::clone(&notify);
        let flag = std::rc::Rc::clone(&flag);
        orch.on_update(std::rc::Rc::new(move |job| {
            if job.id == id_owned && is_terminal(job.status) {
                flag.set(true);
                notify.notify_one();
            }
        }))
    };
    if let Some(job) = orch.get(id)
        && is_terminal(job.status)
    {
        drop(sub);
        return Some(job);
    }
    let sleep = tokio::time::sleep(wait);
    tokio::pin!(sleep);
    loop {
        if flag.get() {
            break;
        }
        tokio::select! {
            _ = &mut sleep => break,
            _ = notify.notified() => {}
        }
    }
    drop(sub);
    orch.get(id)
}

fn is_terminal(status: legate_core::JobStatus) -> bool {
    matches!(
        status,
        legate_core::JobStatus::Done | legate_core::JobStatus::Failed
    )
}
