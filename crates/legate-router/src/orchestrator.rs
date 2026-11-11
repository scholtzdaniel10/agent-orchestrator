//! Event-driven job queue from `src/core/router/orchestrator.ts`.
//!
//! Dispatch waits on runner events and [`tokio::sync::Notify`], not a polling timer.
//! Heartbeats and the cross-process watch are deadlines on [`crate::Clock`].
//! Picks read [`crate::RouterState`]. Rest and reported windows are refreshed from the
//! store first, because another process (and the tests) write those rows directly.
//! Run weights stay in the state's deque.

use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::{Rc, Weak};
use std::time::Instant;

use legate_core::{
    AgentEvent, FileEdit, JobAccess, JobRecord, JobStatus, JobStep, JobStepStatus, JobType,
    PlanProblem, ProviderId, RunOptions, StepPhase, StepTool, UsageWindow, VersionStatus, plan_of,
};
use legate_store::{NewWindow, RunOutcome, RunRow, Store};
use tokio::sync::Notify;

use crate::clock::{SystemClock, system_now_ms};
use crate::ids::RunningCounts;
use crate::ports::{ActiveRun, Clock, ExitStatus, RunJob, Runner, Workspaces};
use crate::state::RouterState;
use crate::usage::{Pick, RouterError};
use crate::versions::{compare_to_tested, unknown_format_message};

pub const JOB_HEARTBEAT_MS: i64 = 5_000;
pub const JOB_WATCH_MS: i64 = 2_000;
pub const JOB_STALE_MS: i64 = 20_000;
const KEPT_JOBS: usize = 200;
const STEP_FLUSH_MS: i64 = 200;
const STEP_CAP: usize = 400;

#[derive(Debug, thiserror::Error)]
pub enum OrchestratorError {
    #[error("job is not running")]
    NotRunning,
    #[error(transparent)]
    Router(#[from] RouterError),
    #[error(transparent)]
    Store(#[from] legate_store::StoreError),
}

#[derive(Clone, Debug)]
pub struct WorkerWindow {
    pub name: String,
    pub used: f64,
    pub resets_at: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct WorkerInfo {
    pub id: ProviderId,
    pub available: bool,
    pub problem: Option<PlanProblem>,
    pub headroom: f64,
    pub resting_until: Option<i64>,
    pub busy: bool,
    pub running: usize,
    pub queued: usize,
    pub resets_at: Option<i64>,
    pub at_risk: bool,
    pub windows: Vec<WorkerWindow>,
    pub version: Option<String>,
    pub version_status: VersionStatus,
}

/// `true` / `false` map to edit / read. A level is kept as given.
#[derive(Clone, Copy, Debug)]
pub enum SubmitAccess {
    Flag(bool),
    Level(JobAccess),
}

#[derive(Clone, Debug, Default)]
pub struct SubmitExtra {
    pub group: Option<String>,
    pub lead_message: Option<String>,
    pub routine: Option<String>,
    pub goal: Option<String>,
    pub base: Option<String>,
    pub title: Option<String>,
    pub project: Option<String>,
}

pub enum CwdSource {
    Fixed(String),
    Dynamic(Rc<dyn Fn() -> String>),
}

pub struct OrchestratorOpts {
    pub runners: Vec<Rc<dyn Runner>>,
    pub store: Rc<Store>,
    pub rules: legate_core::RouterRules,
    pub cwd: CwdSource,
    pub clock: Option<Rc<dyn Clock>>,
    pub model_for: Option<Rc<dyn Fn(ProviderId) -> Option<String>>>,
    pub workspaces: Option<Rc<dyn Workspaces>>,
    pub on_changes: Option<Rc<dyn Fn()>>,
    pub owner_id: Option<String>,
}

struct InternalJob {
    id: String,
    job_type: JobType,
    original_prompt: String,
    prompt: String,
    provider: Option<ProviderId>,
    status: JobStatus,
    output: String,
    failed_over: Vec<ProviderId>,
    model: Option<String>,
    reason: Option<String>,
    chosen: bool,
    access: JobAccess,
    change: Option<String>,
    group: Option<String>,
    lead_message: Option<String>,
    routine: Option<String>,
    title: Option<String>,
    session_id: Option<String>,
    goal: Option<String>,
    base: Option<String>,
    error: Option<String>,
    project: String,
    created_at: i64,
    steps: Vec<JobStep>,
    steps_dropped: i64,
    owner_id: Option<String>,
    heartbeat_at: Option<i64>,
    cancel_requested: bool,
}

impl InternalJob {
    fn edits(&self) -> bool {
        self.access != JobAccess::Read
    }
}

struct HeldResult {
    ok: bool,
    text: String,
    session_id: Option<String>,
    cost_usd: Option<f64>,
    tokens: Option<i64>,
}

type Listener = Rc<dyn Fn(JobRecord)>;

struct Inner {
    order: Vec<String>,
    jobs: HashMap<String, InternalJob>,
    queues: HashMap<ProviderId, VecDeque<String>>,
    running: HashMap<ProviderId, HashSet<String>>,
    killers: HashMap<String, Rc<dyn Fn()>>,
    abandoning: HashSet<String>,
    cancelling: HashSet<String>,
    listeners: Vec<(u64, Listener)>,
    next_listener: u64,
    step_flush_at: HashMap<String, i64>,
    step_timers: HashMap<String, u64>,
    project: Option<String>,
    watching: bool,
    heartbeat_timer: Option<u64>,
    watch_timer: Option<u64>,
    candidates: Vec<ProviderId>,
    problems: HashMap<ProviderId, PlanProblem>,
    versions: HashMap<ProviderId, Option<String>>,
}

impl Inner {
    fn new() -> Self {
        Self {
            order: Vec::new(),
            jobs: HashMap::new(),
            queues: HashMap::new(),
            running: HashMap::new(),
            killers: HashMap::new(),
            abandoning: HashSet::new(),
            cancelling: HashSet::new(),
            listeners: Vec::new(),
            next_listener: 1,
            step_flush_at: HashMap::new(),
            step_timers: HashMap::new(),
            project: None,
            watching: false,
            heartbeat_timer: None,
            watch_timer: None,
            candidates: Vec::new(),
            problems: HashMap::new(),
            versions: HashMap::new(),
        }
    }
}

struct PreparedRun {
    run: Box<dyn ActiveRun>,
    runner: Rc<dyn Runner>,
}

pub struct Orchestrator {
    this: Weak<Orchestrator>,
    inner: std::cell::RefCell<Inner>,
    notify: std::sync::Arc<Notify>,
    clock: Rc<dyn Clock>,
    runners: Vec<Rc<dyn Runner>>,
    store: Rc<Store>,
    rules: legate_core::RouterRules,
    state: std::cell::RefCell<RouterState>,
    cwd: CwdSource,
    model_for: Option<Rc<dyn Fn(ProviderId) -> Option<String>>>,
    workspaces: Option<Rc<dyn Workspaces>>,
    on_changes: Option<Rc<dyn Fn()>>,
    owner_id: String,
}

pub struct Subscription {
    orch: Weak<Orchestrator>,
    id: u64,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(orch) = self.orch.upgrade() {
            orch.inner
                .borrow_mut()
                .listeners
                .retain(|(id, _)| *id != self.id);
        }
    }
}

pub struct WatchGuard {
    orch: Weak<Orchestrator>,
}

impl Drop for WatchGuard {
    fn drop(&mut self) {
        if let Some(orch) = self.orch.upgrade() {
            orch.inner.borrow_mut().watching = false;
            orch.sync_timers();
        }
    }
}

impl Orchestrator {
    pub fn new(opts: OrchestratorOpts) -> Result<Rc<Self>, OrchestratorError> {
        let now = opts
            .clock
            .as_ref()
            .map(|clock| clock.now_ms())
            .unwrap_or_else(system_now_ms);
        let state = RouterState::hydrate(&opts.store, &opts.rules, now)?;
        let clock = opts
            .clock
            .unwrap_or_else(|| Rc::new(SystemClock::new()) as Rc<dyn Clock>);
        let owner_id = opts
            .owner_id
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        Ok(Rc::new_cyclic(|this| Self {
            this: this.clone(),
            inner: std::cell::RefCell::new(Inner::new()),
            notify: std::sync::Arc::new(Notify::new()),
            clock,
            runners: opts.runners,
            store: opts.store,
            rules: opts.rules,
            state: std::cell::RefCell::new(state),
            cwd: opts.cwd,
            model_for: opts.model_for,
            workspaces: opts.workspaces,
            on_changes: opts.on_changes,
            owner_id,
        }))
    }

    pub fn owner_id(&self) -> &str {
        &self.owner_id
    }

    pub async fn init(&self) {
        self.recheck().await;
    }

    pub async fn recheck(&self) {
        let mut available = Vec::new();
        let mut problems = HashMap::new();
        let mut versions = HashMap::new();
        for runner in &self.runners {
            let installed = runner.is_installed().await;
            let version = runner.version().await;
            versions.insert(runner.id(), version);
            if !installed {
                problems.insert(runner.id(), PlanProblem::NotInstalled);
                continue;
            }
            if !runner.is_signed_in().await {
                problems.insert(runner.id(), PlanProblem::SignedOut);
                continue;
            }
            available.push(runner.id());
        }
        let mut inner = self.inner.borrow_mut();
        inner.candidates = available;
        inner.problems = problems;
        inner.versions = versions;
    }

    pub fn submit(
        &self,
        job_type: JobType,
        prompt: &str,
        provider: Option<ProviderId>,
        access: Option<SubmitAccess>,
        extra: SubmitExtra,
    ) -> JobRecord {
        let now = self.clock.now_ms();
        let granted = resolve_access(access);
        let id = uuid::Uuid::new_v4().to_string();
        let project = extra.project.unwrap_or_else(|| self.working_dir());
        let job = InternalJob {
            id: id.clone(),
            job_type,
            original_prompt: prompt.to_string(),
            prompt: prompt.to_string(),
            provider: None,
            status: JobStatus::Queued,
            output: String::new(),
            failed_over: Vec::new(),
            model: None,
            reason: None,
            chosen: false,
            access: granted,
            change: None,
            group: extra.group,
            lead_message: extra.lead_message,
            routine: extra.routine,
            title: extra.title,
            session_id: None,
            goal: extra.goal,
            base: extra.base,
            error: None,
            project: project.clone(),
            created_at: now,
            steps: Vec::new(),
            steps_dropped: 0,
            owner_id: Some(self.owner_id.clone()),
            heartbeat_at: Some(now),
            cancel_requested: false,
        };
        {
            let mut inner = self.inner.borrow_mut();
            inner.order.push(id.clone());
            inner.jobs.insert(id.clone(), job);
        }
        self.prune(&project);

        if let Some(provider) = provider {
            if let Some(job) = self.inner.borrow_mut().jobs.get_mut(&id) {
                job.provider = Some(provider);
            }
            if !self.inner.borrow().candidates.contains(&provider) {
                return self.fail_now(&id, &format!("{} is not signed in", provider.as_str()));
            }
            self.refresh_usage();
            let resting = self
                .state
                .borrow()
                .resting_deadline(plan_of(provider))
                .is_some_and(|until| until > self.clock.now_ms());
            if resting {
                return self.fail_now(&id, &format!("{} is resting", provider.as_str()));
            }
            {
                let mut inner = self.inner.borrow_mut();
                if let Some(job) = inner.jobs.get_mut(&id) {
                    job.chosen = true;
                    job.reason = Some("chosen".to_string());
                }
            }
            self.enqueue(provider, &id);
            self.emit_status(&id);
            self.pump(provider);
            return self.record_of(&id);
        }

        self.refresh_usage();
        let candidates = self.inner.borrow().candidates.clone();
        let running = self.running_counts();
        let now = self.clock.now_ms();
        let pick =
            self.state
                .borrow_mut()
                .pick_with_reason(job_type, &candidates, now, &[], &running);
        let Some(pick) = pick else {
            return self.fail_now(&id, "no provider available");
        };
        self.assign_pick(&id, pick);
        self.enqueue(pick.provider, &id);
        self.emit_status(&id);
        self.pump(pick.provider);
        self.record_of(&id)
    }

    pub fn restore(&self, project: &str) {
        self.inner.borrow_mut().project = Some(project.to_string());
        let active: Vec<InternalJob> = {
            let inner = self.inner.borrow();
            inner
                .order
                .iter()
                .filter_map(|id| inner.jobs.get(id))
                .filter(|job| self.is_ours(job) && is_open(job.status))
                .map(clone_job)
                .collect()
        };
        {
            let mut inner = self.inner.borrow_mut();
            inner.order.clear();
            inner.jobs.clear();
            if active.is_empty() {
                inner.queues.clear();
                inner.running.clear();
            }
            for job in active {
                inner.order.push(job.id.clone());
                inner.jobs.insert(job.id.clone(), job);
            }
        }
        let loaded = self
            .store
            .jobs(project, KEPT_JOBS as i64)
            .unwrap_or_else(|err| {
                tracing::error!("{err}");
                Vec::new()
            });
        let now = self.clock.now_ms();
        for record in loaded {
            if self.inner.borrow().jobs.contains_key(&record.id) {
                continue;
            }
            let mut job = from_record(record, project.to_string(), now);
            let interrupt = self.should_interrupt(&job);
            if interrupt {
                job.status = JobStatus::Failed;
                job.error = Some("interrupted when the app closed".to_string());
            }
            let id = job.id.clone();
            self.inner.borrow_mut().order.push(id.clone());
            self.inner.borrow_mut().jobs.insert(id.clone(), job);
            if interrupt {
                self.emit_status(&id);
            }
        }
        self.sync_timers();
    }

    pub fn watch_store(self: &Rc<Self>) -> WatchGuard {
        {
            let mut inner = self.inner.borrow_mut();
            inner.watching = true;
            if inner.project.is_none() {
                inner.project = Some(self.working_dir());
            }
        }
        self.sync_timers();
        WatchGuard {
            orch: Rc::downgrade(self),
        }
    }

    pub fn list(&self) -> Vec<JobRecord> {
        let project = self.current_project();
        let inner = self.inner.borrow();
        inner
            .order
            .iter()
            .filter_map(|id| inner.jobs.get(id))
            .filter(|job| job.project == project)
            .map(to_record)
            .collect()
    }

    /// Background project tools must still see completed jobs evicted from the active view.
    pub fn list_project(&self, project: &str) -> Vec<JobRecord> {
        let mut records = self
            .store
            .jobs(project, KEPT_JOBS as i64)
            .unwrap_or_default();
        let inner = self.inner.borrow();
        for job in inner
            .order
            .iter()
            .filter_map(|id| inner.jobs.get(id))
            .filter(|job| job.project == project)
        {
            let record = to_record(job);
            if let Some(saved) = records.iter_mut().find(|saved| saved.id == record.id) {
                *saved = record;
            } else {
                records.push(record);
            }
        }
        records
    }

    pub fn project_job(&self, project: &str, id: &str) -> Option<JobRecord> {
        self.list_project(project)
            .into_iter()
            .find(|job| job.id == id)
    }

    pub fn active_in(&self, project: &str) -> usize {
        let inner = self.inner.borrow();
        inner
            .jobs
            .values()
            .filter(|job| job.project == project && is_open(job.status))
            .count()
    }

    pub fn active_changes_in(&self, project: &str) -> Vec<String> {
        let inner = self.inner.borrow();
        let mut ids = Vec::new();
        for job in inner.jobs.values() {
            if job.project == project
                && is_open(job.status)
                && let Some(change) = &job.change
                && !ids.contains(change)
            {
                ids.push(change.clone());
            }
        }
        ids
    }

    pub fn project_of(&self, id: &str) -> Option<String> {
        self.inner
            .borrow()
            .jobs
            .get(id)
            .map(|job| job.project.clone())
    }

    pub fn get(&self, id: &str) -> Option<JobRecord> {
        self.inner.borrow().jobs.get(id).map(to_record)
    }

    pub fn workers(&self) -> Vec<WorkerInfo> {
        self.refresh_usage();
        let now = self.clock.now_ms();
        let mut infos = Vec::with_capacity(self.runners.len());
        for runner in &self.runners {
            let id = runner.id();
            let usage = self.state.borrow_mut().plan_usage(id, now);
            let headroom = self.state.borrow_mut().headroom(id, now);
            let resting = self
                .state
                .borrow()
                .resting_deadline(plan_of(id))
                .filter(|until| *until > now);
            let inner = self.inner.borrow();
            let running = inner.running.get(&id).map(|set| set.len()).unwrap_or(0);
            let queued = inner.queues.get(&id).map(|queue| queue.len()).unwrap_or(0);
            let version = inner.versions.get(&id).cloned().unwrap_or(None);
            infos.push(WorkerInfo {
                id,
                available: inner.candidates.contains(&id),
                problem: inner.problems.get(&id).copied(),
                headroom,
                resting_until: resting,
                busy: running > 0,
                running,
                queued,
                resets_at: usage.resets_at,
                at_risk: usage.at_risk,
                windows: usage
                    .windows
                    .into_iter()
                    .map(|window| WorkerWindow {
                        name: window.name,
                        used: window.used,
                        resets_at: window.resets_at,
                    })
                    .collect(),
                version_status: compare_to_tested(id, version.as_deref()),
                version,
            });
        }
        infos
    }

    pub fn cancel(&self, id: &str) -> Result<JobRecord, OrchestratorError> {
        let status = self.inner.borrow().jobs.get(id).map(|job| job.status);
        let Some(status) = status else {
            return Err(OrchestratorError::NotRunning);
        };
        if !is_open(status) {
            return Err(OrchestratorError::NotRunning);
        }
        let foreign = self
            .inner
            .borrow()
            .jobs
            .get(id)
            .is_some_and(|job| self.is_live_foreign(job));
        if foreign {
            if let Err(err) = self.store.request_cancel(id) {
                tracing::error!("{err}");
            }
            if let Some(job) = self.inner.borrow_mut().jobs.get_mut(id) {
                job.cancel_requested = true;
            }
            return Ok(self.record_of(id));
        }
        if status == JobStatus::Queued {
            let provider = self
                .inner
                .borrow()
                .jobs
                .get(id)
                .and_then(|job| job.provider);
            if let Some(provider) = provider
                && let Some(queue) = self.inner.borrow_mut().queues.get_mut(&provider)
            {
                queue.retain(|queued| queued != id);
            }
            if let Some(job) = self.inner.borrow_mut().jobs.get_mut(id) {
                job.status = JobStatus::Failed;
                job.error = Some("stopped".to_string());
            }
            self.emit_status(id);
            self.resolve_idle();
            return Ok(self.record_of(id));
        }
        self.inner.borrow_mut().cancelling.insert(id.to_string());
        self.kill_job(id);
        Ok(self.record_of(id))
    }

    pub fn on_update(&self, cb: Rc<dyn Fn(JobRecord)>) -> Subscription {
        let mut inner = self.inner.borrow_mut();
        let id = inner.next_listener;
        inner.next_listener += 1;
        inner.listeners.push((id, cb));
        Subscription {
            orch: self.this.clone(),
            id,
        }
    }

    pub async fn idle(&self) {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if !self.busy() {
                return;
            }
            notified.await;
        }
    }

    pub fn busy(&self) -> bool {
        let inner = self.inner.borrow();
        inner
            .jobs
            .values()
            .any(|job| self.is_ours(job) && is_open(job.status))
    }

    fn assign_pick(&self, id: &str, pick: Pick) {
        if let Some(job) = self.inner.borrow_mut().jobs.get_mut(id) {
            job.provider = Some(pick.provider);
            job.reason = Some(pick.reason.to_string());
        }
    }

    fn fail_now(&self, id: &str, error: &str) -> JobRecord {
        if let Some(job) = self.inner.borrow_mut().jobs.get_mut(id) {
            job.status = JobStatus::Failed;
            job.error = Some(error.to_string());
        }
        self.emit_status(id);
        self.resolve_idle();
        self.record_of(id)
    }

    fn record_of(&self, id: &str) -> JobRecord {
        self.inner
            .borrow()
            .jobs
            .get(id)
            .map(to_record)
            .unwrap_or_else(|| empty_record(id))
    }

    fn working_dir(&self) -> String {
        match &self.cwd {
            CwdSource::Fixed(path) => path.clone(),
            CwdSource::Dynamic(fun) => fun(),
        }
    }

    fn current_project(&self) -> String {
        self.inner
            .borrow()
            .project
            .clone()
            .unwrap_or_else(|| self.working_dir())
    }

    fn refresh_usage(&self) {
        if let Err(err) = self.state.borrow_mut().pull_control(&self.store) {
            tracing::error!("{err}");
        }
    }

    fn enqueue(&self, provider: ProviderId, id: &str) {
        self.inner
            .borrow_mut()
            .queues
            .entry(provider)
            .or_default()
            .push_back(id.to_string());
    }

    fn running_counts(&self) -> RunningCounts {
        let mut counts = RunningCounts::default();
        for (id, jobs) in &self.inner.borrow().running {
            counts.set(*id, u32::try_from(jobs.len()).unwrap_or(u32::MAX));
        }
        counts
    }

    fn plan_running(&self, provider: ProviderId) -> u32 {
        let plan = plan_of(provider);
        let inner = self.inner.borrow();
        let mut total = 0u32;
        for (id, jobs) in &inner.running {
            if plan_of(*id) == plan {
                total = total.saturating_add(u32::try_from(jobs.len()).unwrap_or(u32::MAX));
            }
        }
        total
    }

    fn pump_plan(&self, provider: ProviderId) {
        let plan = plan_of(provider);
        let ids: Vec<ProviderId> = self
            .runners
            .iter()
            .map(|runner| runner.id())
            .filter(|id| plan_of(*id) == plan)
            .collect();
        for id in ids {
            self.pump(id);
        }
    }

    fn pump(&self, provider: ProviderId) {
        let limit = u32::try_from(self.rules.max_parallel.max(0)).unwrap_or(u32::MAX);
        loop {
            if self.plan_running(provider) >= limit {
                return;
            }
            let Some(id) = self
                .inner
                .borrow_mut()
                .queues
                .get_mut(&provider)
                .and_then(|queue| queue.pop_front())
            else {
                return;
            };
            let queued = self
                .inner
                .borrow()
                .jobs
                .get(&id)
                .is_some_and(|job| job.status == JobStatus::Queued);
            if !queued {
                continue;
            }
            {
                let mut inner = self.inner.borrow_mut();
                inner
                    .running
                    .entry(provider)
                    .or_default()
                    .insert(id.clone());
                if let Some(job) = inner.jobs.get_mut(&id) {
                    job.status = JobStatus::Running;
                    job.provider = Some(provider);
                }
            }
            self.emit_status(&id);
            let edits = self
                .inner
                .borrow()
                .jobs
                .get(&id)
                .is_some_and(InternalJob::edits);
            if edits {
                self.spawn_edit(id, provider);
            } else {
                match self.launch(&id, provider) {
                    Ok(prepared) => {
                        self.spawn_prepared(id, provider, prepared, self.clock.now_ms())
                    }
                    Err(message) => {
                        let started_at = self.clock.now_ms();
                        self.fail_thrown(&id, provider, &message, started_at, 0, None, None);
                        self.clear_slot(provider, &id);
                        self.resolve_idle();
                    }
                }
            }
        }
    }

    fn spawn_edit(&self, job_id: String, provider: ProviderId) {
        let Some(orch) = self.this.upgrade() else {
            return;
        };
        let started_at = self.clock.now_ms();
        tokio::task::spawn_local(async move {
            let instant = Instant::now();
            let edits = orch
                .drive_edit(&job_id, provider, started_at, instant)
                .await;
            orch.after_drive(&job_id, provider, edits);
        });
    }

    fn spawn_prepared(
        &self,
        job_id: String,
        provider: ProviderId,
        prepared: PreparedRun,
        started_at: i64,
    ) {
        let Some(orch) = self.this.upgrade() else {
            return;
        };
        tokio::task::spawn_local(async move {
            let instant = Instant::now();
            let edits = orch
                .drive_prepared(&job_id, provider, started_at, instant, prepared)
                .await;
            orch.after_drive(&job_id, provider, edits);
        });
    }

    async fn drive_edit(
        &self,
        job_id: &str,
        provider: ProviderId,
        started_at: i64,
        instant: Instant,
    ) -> bool {
        let project = self
            .inner
            .borrow()
            .jobs
            .get(job_id)
            .map(|job| job.project.clone())
            .unwrap_or_default();
        let Some(cwd) = self.edit_folder(job_id, &project).await else {
            return false;
        };
        if self.finish_cancelled(
            job_id,
            provider,
            started_at,
            elapsed_ms(instant),
            None,
            None,
        ) {
            return false;
        }
        if self.finish_abandoned(job_id, provider) {
            return false;
        }
        match self.launch_in(job_id, provider, &cwd) {
            Ok(prepared) => {
                self.consume(job_id, provider, started_at, instant, prepared)
                    .await
            }
            Err(message) => {
                self.fail_thrown(
                    job_id,
                    provider,
                    &message,
                    started_at,
                    elapsed_ms(instant),
                    None,
                    None,
                );
                true
            }
        }
    }

    async fn drive_prepared(
        &self,
        job_id: &str,
        provider: ProviderId,
        started_at: i64,
        instant: Instant,
        prepared: PreparedRun,
    ) -> bool {
        let edits = self
            .inner
            .borrow()
            .jobs
            .get(job_id)
            .is_some_and(InternalJob::edits);
        self.consume(job_id, provider, started_at, instant, prepared)
            .await;
        edits
    }

    async fn consume(
        &self,
        job_id: &str,
        provider: ProviderId,
        started_at: i64,
        instant: Instant,
        mut prepared: PreparedRun,
    ) -> bool {
        let edits = self
            .inner
            .borrow()
            .jobs
            .get(job_id)
            .is_some_and(InternalJob::edits);
        let mut utilization = None;
        let mut result: Option<HeldResult> = None;
        let mut limit_at: Option<Option<i64>> = None;
        loop {
            if self.inner.borrow().abandoning.contains(job_id)
                || self.inner.borrow().cancelling.contains(job_id)
            {
                break;
            }
            let Some(event) = prepared.run.next_event().await else {
                break;
            };
            if self.inner.borrow().abandoning.contains(job_id)
                || self.inner.borrow().cancelling.contains(job_id)
            {
                break;
            }
            match event {
                AgentEvent::Limit { resets_at, .. } => {
                    limit_at = Some(resets_at);
                    break;
                }
                other => match self.apply_event(job_id, provider, other) {
                    Applied::Used(used) => utilization = Some(used),
                    Applied::Result(held) => result = Some(held),
                    Applied::Quiet => {}
                },
            }
        }
        let stop = limit_at.is_some()
            || self.inner.borrow().abandoning.contains(job_id)
            || self.inner.borrow().cancelling.contains(job_id);
        if stop {
            self.kill_job(job_id);
        }
        let exit = match prepared.run.exit().await {
            Ok(exit) => exit,
            Err(message) => {
                let stopping = limit_at.is_some()
                    || self.inner.borrow().abandoning.contains(job_id)
                    || self.inner.borrow().cancelling.contains(job_id);
                if !stopping {
                    self.fail_thrown(
                        job_id,
                        provider,
                        &message,
                        started_at,
                        elapsed_ms(instant),
                        utilization,
                        result.as_ref(),
                    );
                    return edits;
                }
                ExitStatus {
                    code: None,
                    stderr: message,
                    unreadable: false,
                }
            }
        };
        let duration = elapsed_ms(instant);
        if self.finish_cancelled(
            job_id,
            provider,
            started_at,
            duration,
            utilization,
            result.as_ref(),
        ) {
            return edits;
        }
        if self.finish_abandoned(job_id, provider) {
            return edits;
        }
        let code_bad = exit.code != Some(0);
        let limited =
            limit_at.is_some() || (code_bad && prepared.runner.is_limit_error(&exit.stderr));
        if limited {
            if limit_at.is_none() {
                self.kill_job(job_id);
            }
            let resets = limit_at.flatten().filter(|seconds| *seconds != 0);
            self.apply_limit(
                job_id,
                provider,
                resets,
                started_at,
                duration,
                utilization,
                result.as_ref(),
            );
            return edits;
        }
        if exit.unreadable {
            self.insert_run(
                job_id,
                provider,
                started_at,
                duration,
                utilization,
                result.as_ref(),
                RunOutcome::Error,
            );
            let version = self
                .inner
                .borrow()
                .versions
                .get(&provider)
                .cloned()
                .unwrap_or(None);
            if let Some(job) = self.inner.borrow_mut().jobs.get_mut(job_id) {
                job.status = JobStatus::Failed;
                job.error = Some(unknown_format_message(provider, version.as_deref()));
            }
            self.emit_status(job_id);
            return edits;
        }
        self.complete(
            job_id,
            provider,
            result.as_ref(),
            &exit.stderr,
            started_at,
            duration,
            utilization,
        );
        edits
    }

    fn after_drive(&self, job_id: &str, provider: ProviderId, edits: bool) {
        if edits && let Some(callback) = &self.on_changes {
            callback();
        }
        self.clear_slot(provider, job_id);
        self.pump_plan(provider);
        self.resolve_idle();
    }

    fn clear_slot(&self, provider: ProviderId, job_id: &str) {
        let mut inner = self.inner.borrow_mut();
        if let Some(set) = inner.running.get_mut(&provider) {
            set.remove(job_id);
            if set.is_empty() {
                inner.running.remove(&provider);
            }
        }
        // A failover may already be running on another plan. Leave its kill handle in place.
        let still = inner.running.values().any(|set| set.contains(job_id));
        if !still {
            inner.killers.remove(job_id);
            inner.abandoning.remove(job_id);
            inner.cancelling.remove(job_id);
        }
    }

    async fn edit_folder(&self, job_id: &str, project: &str) -> Option<String> {
        let base = self
            .inner
            .borrow()
            .jobs
            .get(job_id)
            .and_then(|job| job.base.clone());
        let Some(spaces) = &self.workspaces else {
            self.fail_edit(job_id, "no worktrees");
            return None;
        };
        match spaces.create(project, job_id, base.as_deref()).await {
            Ok(created) => {
                if let Some(job) = self.inner.borrow_mut().jobs.get_mut(job_id) {
                    job.change = Some(created.id);
                }
                self.emit(job_id);
                Some(created.path)
            }
            Err(message) => {
                self.fail_edit(job_id, &message);
                None
            }
        }
    }

    fn fail_edit(&self, job_id: &str, message: &str) {
        if let Some(job) = self.inner.borrow_mut().jobs.get_mut(job_id) {
            job.status = JobStatus::Failed;
            job.error = Some(format!("cannot edit here: {message}"));
        }
        self.emit_status(job_id);
    }

    fn launch(&self, job_id: &str, provider: ProviderId) -> Result<PreparedRun, String> {
        let cwd = self
            .inner
            .borrow()
            .jobs
            .get(job_id)
            .map(|job| job.project.clone())
            .unwrap_or_default();
        self.launch_in(job_id, provider, &cwd)
    }

    fn launch_in(
        &self,
        job_id: &str,
        provider: ProviderId,
        cwd: &str,
    ) -> Result<PreparedRun, String> {
        let runner = self
            .runner(provider)
            .ok_or_else(|| format!("no adapter for {}", provider.as_str()))?;
        let args = self.run_args(job_id);
        let opts = self.run_options(job_id, provider);
        let active = runner.start(&args, cwd, &opts)?;
        let killer = active.killer();
        self.inner
            .borrow_mut()
            .killers
            .insert(job_id.to_string(), killer);
        if self.inner.borrow().cancelling.contains(job_id) {
            self.kill_job(job_id);
        }
        Ok(PreparedRun {
            run: active,
            runner,
        })
    }

    fn run_args(&self, job_id: &str) -> RunJob {
        let inner = self.inner.borrow();
        let job = inner.jobs.get(job_id);
        RunJob {
            id: job_id.to_string(),
            prompt: job.map(|job| job.prompt.clone()).unwrap_or_default(),
        }
    }

    fn run_options(&self, job_id: &str, provider: ProviderId) -> RunOptions {
        let access = self.inner.borrow().jobs.get(job_id).map(|job| job.access);
        let edits = self
            .inner
            .borrow()
            .jobs
            .get(job_id)
            .is_some_and(InternalJob::edits);
        RunOptions {
            access,
            edit: edits.then_some(true),
            model: self
                .model_for
                .as_ref()
                .and_then(|model_for| model_for(provider)),
            ..RunOptions::default()
        }
    }

    fn runner(&self, id: ProviderId) -> Option<Rc<dyn Runner>> {
        self.runners
            .iter()
            .find(|runner| runner.id() == id)
            .cloned()
    }

    fn apply_event(&self, job_id: &str, provider: ProviderId, event: AgentEvent) -> Applied {
        match event {
            AgentEvent::Init { session_id, model } => {
                if let Some(job) = self.inner.borrow_mut().jobs.get_mut(job_id) {
                    job.session_id = Some(session_id);
                    if let Some(model) = model {
                        job.model = Some(model);
                    }
                }
                self.emit(job_id);
                Applied::Quiet
            }
            AgentEvent::Text { text } => {
                self.append_text(job_id, &text);
                Applied::Quiet
            }
            AgentEvent::Usage {
                utilization,
                windows,
                ..
            } => {
                if let Some(windows) = windows {
                    self.save_usage_windows(provider, &windows);
                }
                Applied::Used(utilization)
            }
            AgentEvent::Result {
                ok,
                text,
                session_id,
                cost_usd,
                tokens,
                ..
            } => {
                let empty_output = self
                    .inner
                    .borrow()
                    .jobs
                    .get(job_id)
                    .is_some_and(|job| job.output.is_empty());
                if !text.is_empty() && empty_output {
                    if let Some(job) = self.inner.borrow_mut().jobs.get_mut(job_id) {
                        job.output = text.clone();
                    }
                    self.emit(job_id);
                }
                Applied::Result(HeldResult {
                    ok,
                    text,
                    session_id,
                    cost_usd,
                    tokens,
                })
            }
            AgentEvent::Step {
                id,
                phase,
                tool,
                title,
                detail,
                ok,
                edit,
            } => {
                self.apply_step(
                    job_id,
                    &id,
                    phase,
                    tool,
                    &title,
                    detail.as_deref(),
                    ok,
                    edit,
                );
                Applied::Quiet
            }
            AgentEvent::Limit { .. } => Applied::Quiet,
        }
    }

    fn save_usage_windows(&self, provider: ProviderId, windows: &[UsageWindow]) {
        let plan = plan_of(provider);
        let observed = self.clock.now_ms();
        let rows: Vec<NewWindow> = windows
            .iter()
            .map(|window| NewWindow {
                name: window.name.clone(),
                utilization: window.utilization,
                resets_at: window.resets_at.map(|seconds| seconds.saturating_mul(1000)),
            })
            .collect();
        if let Err(err) = self.store.save_windows(plan.as_str(), &rows, observed) {
            tracing::error!("{err}");
        }
        // Store order is by name, which is what scoring reads. Pull that order back.
        self.refresh_usage();
    }

    fn append_text(&self, job_id: &str, text: &str) {
        if let Some(job) = self.inner.borrow_mut().jobs.get_mut(job_id) {
            if job.output.is_empty() {
                job.output = text.to_string();
            } else {
                job.output.push('\n');
                job.output.push_str(text);
            }
        }
        self.emit(job_id);
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_step(
        &self,
        job_id: &str,
        step_id: &str,
        phase: StepPhase,
        tool: StepTool,
        title: &str,
        detail: Option<&str>,
        ok: Option<bool>,
        edit: Option<FileEdit>,
    ) {
        let now = self.clock.now_ms();
        if phase == StepPhase::Start {
            let step = JobStep {
                id: step_id.to_string(),
                tool,
                title: title.to_string(),
                detail: detail.map(str::to_string),
                status: JobStepStatus::Running,
                started_at: now,
                ended_at: None,
                edit,
            };
            if let Some(job) = self.inner.borrow_mut().jobs.get_mut(job_id) {
                push_step(job, step);
            }
        } else {
            let mut inner = self.inner.borrow_mut();
            let Some(job) = inner.jobs.get_mut(job_id) else {
                return;
            };
            if let Some(existing) = job.steps.iter_mut().find(|step| step.id == step_id) {
                existing.title = title.to_string();
                existing.tool = tool;
                existing.status = if ok == Some(false) {
                    JobStepStatus::Failed
                } else {
                    JobStepStatus::Done
                };
                existing.ended_at = Some(now);
                if let Some(detail) = detail {
                    existing.detail = Some(detail.to_string());
                }
                if let Some(edit) = edit {
                    existing.edit = Some(edit);
                }
            } else if tool == StepTool::Think && extend_thinking(job, detail, title, now) {
            } else {
                let step = JobStep {
                    id: step_id.to_string(),
                    tool,
                    title: title.to_string(),
                    detail: detail.map(str::to_string),
                    status: if ok == Some(false) {
                        JobStepStatus::Failed
                    } else {
                        JobStepStatus::Done
                    },
                    started_at: now,
                    ended_at: Some(now),
                    edit,
                };
                push_step(job, step);
            }
        }
        self.push_step_update(job_id);
    }

    fn push_step_update(&self, job_id: &str) {
        let now = self.clock.now_ms();
        let last = self
            .inner
            .borrow()
            .step_flush_at
            .get(job_id)
            .copied()
            .unwrap_or(0);
        if now - last >= STEP_FLUSH_MS {
            self.inner
                .borrow_mut()
                .step_flush_at
                .insert(job_id.to_string(), now);
            self.persist(job_id);
            self.emit(job_id);
            return;
        }
        if self.inner.borrow().step_timers.contains_key(job_id) {
            return;
        }
        let wait = (STEP_FLUSH_MS - (now - last)).max(0);
        let weak = self.this.clone();
        let owned = job_id.to_string();
        let timer = self.clock.set_timeout(
            wait,
            Rc::new(move || {
                let Some(orch) = weak.upgrade() else {
                    return;
                };
                orch.inner.borrow_mut().step_timers.remove(&owned);
                let now = orch.clock.now_ms();
                orch.inner
                    .borrow_mut()
                    .step_flush_at
                    .insert(owned.clone(), now);
                orch.persist(&owned);
                orch.emit(&owned);
            }),
        );
        self.inner
            .borrow_mut()
            .step_timers
            .insert(job_id.to_string(), timer);
    }

    fn clear_step_timer(&self, job_id: &str) {
        let timer = self.inner.borrow_mut().step_timers.remove(job_id);
        if let Some(timer) = timer {
            self.clock.clear_timeout(timer);
        }
        self.inner.borrow_mut().step_flush_at.remove(job_id);
    }

    fn finish_cancelled(
        &self,
        job_id: &str,
        provider: ProviderId,
        started_at: i64,
        duration_ms: i64,
        utilization: Option<f64>,
        result: Option<&HeldResult>,
    ) -> bool {
        if !self.inner.borrow_mut().cancelling.remove(job_id) {
            return false;
        }
        let running = self
            .inner
            .borrow()
            .jobs
            .get(job_id)
            .is_some_and(|job| job.status == JobStatus::Running);
        if !running {
            return true;
        }
        self.insert_run(
            job_id,
            provider,
            started_at,
            duration_ms,
            utilization,
            result,
            RunOutcome::Error,
        );
        if let Some(job) = self.inner.borrow_mut().jobs.get_mut(job_id) {
            job.status = JobStatus::Failed;
            job.error = Some("stopped".to_string());
        }
        self.emit_status(job_id);
        true
    }

    fn finish_abandoned(&self, job_id: &str, provider: ProviderId) -> bool {
        if !self.inner.borrow_mut().abandoning.remove(job_id) {
            return false;
        }
        let running = self
            .inner
            .borrow()
            .jobs
            .get(job_id)
            .is_some_and(|job| job.status == JobStatus::Running);
        if !running {
            return true;
        }
        let chosen = self
            .inner
            .borrow()
            .jobs
            .get(job_id)
            .is_some_and(|job| job.chosen);
        if chosen {
            if let Some(job) = self.inner.borrow_mut().jobs.get_mut(job_id) {
                job.status = JobStatus::Failed;
                job.error = Some(format!("{} hit its usage limit", provider.as_str()));
            }
            self.emit_status(job_id);
            return true;
        }
        let output = self
            .inner
            .borrow()
            .jobs
            .get(job_id)
            .map(|job| job.output.clone())
            .unwrap_or_default();
        self.requeue(job_id, provider, true, &output);
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn complete(
        &self,
        job_id: &str,
        provider: ProviderId,
        result: Option<&HeldResult>,
        stderr: &str,
        started_at: i64,
        duration_ms: i64,
        utilization: Option<f64>,
    ) {
        let ok = result.is_some_and(|result| result.ok);
        self.insert_run(
            job_id,
            provider,
            started_at,
            duration_ms,
            utilization,
            result,
            if ok {
                RunOutcome::Ok
            } else {
                RunOutcome::Error
            },
        );
        if let Some(job) = self.inner.borrow_mut().jobs.get_mut(job_id) {
            if ok {
                job.status = JobStatus::Done;
                job.error = None;
            } else {
                job.status = JobStatus::Failed;
                job.error = Some(failure_message(
                    result.map(|result| result.text.as_str()),
                    stderr,
                ));
            }
        }
        self.emit_status(job_id);
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_limit(
        &self,
        job_id: &str,
        provider: ProviderId,
        resets_at_seconds: Option<i64>,
        started_at: i64,
        duration_ms: i64,
        utilization: Option<f64>,
        result: Option<&HeldResult>,
    ) {
        let now = self.clock.now_ms();
        let until = resets_at_seconds
            .map(|seconds| seconds.saturating_mul(1000))
            .unwrap_or_else(|| self.rest_until(now));
        let plan = plan_of(provider);
        if let Err(err) = self.store.set_resting(plan.as_str(), until) {
            tracing::error!("{err}");
        }
        self.state.borrow_mut().set_resting(plan, Some(until));
        self.insert_run(
            job_id,
            provider,
            started_at,
            duration_ms,
            utilization,
            result,
            RunOutcome::Limit,
        );
        self.abandon_others(provider, job_id);
        let chosen = self
            .inner
            .borrow()
            .jobs
            .get(job_id)
            .is_some_and(|job| job.chosen);
        if chosen {
            if let Some(job) = self.inner.borrow_mut().jobs.get_mut(job_id) {
                job.status = JobStatus::Failed;
                job.error = Some(format!("{} hit its usage limit", provider.as_str()));
            }
            self.emit_status(job_id);
            self.reroute_queued(provider);
            return;
        }
        let output = self
            .inner
            .borrow()
            .jobs
            .get(job_id)
            .map(|job| job.output.clone())
            .unwrap_or_default();
        self.requeue(job_id, provider, true, &output);
        self.reroute_queued(provider);
    }

    fn rest_until(&self, now: i64) -> i64 {
        let ms = self.rules.default_rest_hours * 3_600_000.0;
        let add = if ms.is_finite() { ms as i64 } else { 0 };
        now.saturating_add(add)
    }

    fn abandon_others(&self, provider: ProviderId, except_id: &str) {
        let plan = plan_of(provider);
        let victims: Vec<String> = {
            let inner = self.inner.borrow();
            let mut victims = Vec::new();
            for (id, jobs) in &inner.running {
                if plan_of(*id) != plan {
                    continue;
                }
                for job_id in jobs {
                    if job_id != except_id {
                        victims.push(job_id.clone());
                    }
                }
            }
            victims
        };
        for job_id in victims {
            self.inner.borrow_mut().abandoning.insert(job_id.clone());
            self.kill_job(&job_id);
        }
    }

    fn reroute_queued(&self, provider: ProviderId) {
        let plan = plan_of(provider);
        let adapters: Vec<ProviderId> = self
            .runners
            .iter()
            .map(|runner| runner.id())
            .filter(|id| plan_of(*id) == plan)
            .collect();
        for adapter in adapters {
            let waiting = self
                .inner
                .borrow_mut()
                .queues
                .insert(adapter, VecDeque::new())
                .unwrap_or_default();
            for id in waiting {
                let matches = self.inner.borrow().jobs.get(&id).is_some_and(|job| {
                    job.status == JobStatus::Queued && job.provider == Some(adapter)
                });
                if !matches {
                    continue;
                }
                let chosen = self
                    .inner
                    .borrow()
                    .jobs
                    .get(&id)
                    .is_some_and(|job| job.chosen);
                if chosen {
                    if let Some(job) = self.inner.borrow_mut().jobs.get_mut(&id) {
                        job.status = JobStatus::Failed;
                        job.error = Some(format!("{} hit its usage limit", adapter.as_str()));
                    }
                    self.emit_status(&id);
                    continue;
                }
                self.requeue(&id, adapter, false, "");
            }
        }
    }

    fn requeue(&self, job_id: &str, from: ProviderId, record_failure: bool, output: &str) {
        if record_failure && let Some(job) = self.inner.borrow_mut().jobs.get_mut(job_id) {
            job.failed_over.push(from);
        }
        self.refresh_usage();
        let (job_type, failed_over) = {
            let inner = self.inner.borrow();
            let Some(job) = inner.jobs.get(job_id) else {
                return;
            };
            (job.job_type, job.failed_over.clone())
        };
        let candidates = self.inner.borrow().candidates.clone();
        let running = self.running_counts();
        let now = self.clock.now_ms();
        let next = self.state.borrow_mut().pick_provider(
            job_type,
            &candidates,
            now,
            &failed_over,
            &running,
        );
        let Some(next) = next else {
            if let Some(job) = self.inner.borrow_mut().jobs.get_mut(job_id) {
                job.status = JobStatus::Failed;
                job.provider = None;
                job.error = Some("all providers are at their limit".to_string());
            }
            self.emit_status(job_id);
            return;
        };
        if record_failure {
            let original = self
                .inner
                .borrow()
                .jobs
                .get(job_id)
                .map(|job| job.original_prompt.clone())
                .unwrap_or_default();
            let edit = self
                .inner
                .borrow()
                .jobs
                .get(job_id)
                .is_some_and(InternalJob::edits);
            let prompt = handoff_prompt(&original, from, output, edit);
            if let Some(job) = self.inner.borrow_mut().jobs.get_mut(job_id) {
                job.prompt = prompt;
            }
        }
        if let Some(job) = self.inner.borrow_mut().jobs.get_mut(job_id) {
            job.output.clear();
            job.status = JobStatus::Queued;
            job.provider = Some(next);
            job.reason = Some("failover".to_string());
            job.model = None;
            job.session_id = None;
            job.error = None;
        }
        self.enqueue(next, job_id);
        self.emit_status(job_id);
        self.pump(next);
    }

    #[allow(clippy::too_many_arguments)]
    fn fail_thrown(
        &self,
        job_id: &str,
        provider: ProviderId,
        message: &str,
        started_at: i64,
        duration_ms: i64,
        utilization: Option<f64>,
        result: Option<&HeldResult>,
    ) {
        self.insert_run(
            job_id,
            provider,
            started_at,
            duration_ms,
            utilization,
            result,
            RunOutcome::Error,
        );
        if let Some(job) = self.inner.borrow_mut().jobs.get_mut(job_id) {
            job.status = JobStatus::Failed;
            job.error = Some(message.to_string());
        }
        self.emit_status(job_id);
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_run(
        &self,
        job_id: &str,
        provider: ProviderId,
        started_at: i64,
        duration_ms: i64,
        utilization: Option<f64>,
        result: Option<&HeldResult>,
        outcome: RunOutcome,
    ) {
        let job_type = self
            .inner
            .borrow()
            .jobs
            .get(job_id)
            .map(|job| job.job_type.as_str().to_string())
            .unwrap_or_else(|| "planning".to_string());
        let row = RunRow {
            id: None,
            job_id: job_id.to_string(),
            provider: provider.as_str().to_string(),
            job_type,
            started_at,
            duration_ms: Some(duration_ms),
            cost_usd: result.and_then(|result| result.cost_usd),
            tokens: result.and_then(|result| result.tokens),
            utilization,
            outcome,
            session_id: result.and_then(|result| result.session_id.clone()),
        };
        if let Err(err) = self.store.insert_run(&row) {
            tracing::error!("{err}");
            return;
        }
        self.state.borrow_mut().record_run(&row);
    }

    fn kill_job(&self, job_id: &str) {
        let killer = self.inner.borrow().killers.get(job_id).cloned();
        if let Some(killer) = killer {
            killer();
        }
    }

    fn emit_status(&self, job_id: &str) {
        self.clear_step_timer(job_id);
        let now = self.clock.now_ms();
        if let Some(job) = self.inner.borrow_mut().jobs.get_mut(job_id)
            && job.status != JobStatus::Running
        {
            let ok = job.status == JobStatus::Done;
            close_steps(job, ok, now);
        }
        self.persist(job_id);
        self.emit(job_id);
        self.sync_timers();
    }

    fn persist(&self, job_id: &str) {
        let foreign = self
            .inner
            .borrow()
            .jobs
            .get(job_id)
            .is_some_and(|job| self.is_live_foreign(job));
        if foreign {
            return;
        }
        let now = self.clock.now_ms();
        if let Some(job) = self.inner.borrow_mut().jobs.get_mut(job_id)
            && (job.owner_id.is_none() || job.owner_id.as_deref() == Some(self.owner_id.as_str()))
        {
            job.owner_id = Some(self.owner_id.clone());
            job.heartbeat_at = Some(now);
        }
        let Some((project, created_at, record)) = self
            .inner
            .borrow()
            .jobs
            .get(job_id)
            .map(|job| (job.project.clone(), job.created_at, to_record(job)))
        else {
            return;
        };
        if let Err(err) = self.store.save_job(&project, created_at, &record) {
            tracing::error!("{err}");
        }
    }

    fn emit(&self, job_id: &str) {
        let Some(snap) = self.inner.borrow().jobs.get(job_id).map(to_record) else {
            return;
        };
        let listeners: Vec<_> = self
            .inner
            .borrow()
            .listeners
            .iter()
            .map(|(_, cb)| Rc::clone(cb))
            .collect();
        for listener in listeners {
            listener(snap.clone());
        }
    }

    fn resolve_idle(&self) {
        if !self.busy() {
            self.notify.notify_waiters();
        }
    }

    fn is_ours(&self, job: &InternalJob) -> bool {
        job.owner_id.as_deref() == Some(self.owner_id.as_str())
    }

    fn is_live_foreign(&self, job: &InternalJob) -> bool {
        let Some(owner) = &job.owner_id else {
            return false;
        };
        if owner == &self.owner_id {
            return false;
        }
        let Some(beat) = job.heartbeat_at else {
            return false;
        };
        self.clock.now_ms() - beat < JOB_STALE_MS
    }

    fn should_interrupt(&self, job: &InternalJob) -> bool {
        if !is_open(job.status) {
            return false;
        }
        match &job.owner_id {
            None => true,
            Some(owner) if owner == &self.owner_id => true,
            Some(_) => match job.heartbeat_at {
                None => true,
                Some(beat) => self.clock.now_ms() - beat >= JOB_STALE_MS,
            },
        }
    }

    fn prune(&self, project: &str) {
        let mut finished: Vec<(i64, String)> = self
            .inner
            .borrow()
            .jobs
            .values()
            .filter(|job| job.project == project && is_finished(job.status))
            .map(|job| (job.created_at, job.id.clone()))
            .collect();
        if finished.len() <= KEPT_JOBS {
            return;
        }
        finished.sort_by_key(|(created, _)| *created);
        let extra = finished.len() - KEPT_JOBS;
        let drop_ids: HashSet<String> =
            finished.into_iter().take(extra).map(|(_, id)| id).collect();
        let mut inner = self.inner.borrow_mut();
        inner.order.retain(|id| !drop_ids.contains(id));
        for id in drop_ids {
            inner.jobs.remove(&id);
        }
    }

    fn sync_timers(&self) {
        let need_beat = self.has_owned_active();
        let beat = self.inner.borrow().heartbeat_timer;
        if need_beat && beat.is_none() {
            let weak = self.this.clone();
            let id = self.clock.set_interval(
                JOB_HEARTBEAT_MS,
                Rc::new(move || {
                    if let Some(orch) = weak.upgrade() {
                        orch.on_heartbeat();
                    }
                }),
            );
            self.inner.borrow_mut().heartbeat_timer = Some(id);
        } else if !need_beat && let Some(id) = beat {
            self.clock.clear_interval(id);
            self.inner.borrow_mut().heartbeat_timer = None;
        }
        let need_watch = self.inner.borrow().watching || self.has_foreign_unfinished();
        let watch = self.inner.borrow().watch_timer;
        if need_watch && watch.is_none() {
            let weak = self.this.clone();
            let id = self.clock.set_interval(
                JOB_WATCH_MS,
                Rc::new(move || {
                    if let Some(orch) = weak.upgrade() {
                        orch.poll_store();
                    }
                }),
            );
            self.inner.borrow_mut().watch_timer = Some(id);
        } else if !need_watch && let Some(id) = watch {
            self.clock.clear_interval(id);
            self.inner.borrow_mut().watch_timer = None;
        }
    }

    fn has_owned_active(&self) -> bool {
        self.busy()
    }

    fn has_foreign_unfinished(&self) -> bool {
        let inner = self.inner.borrow();
        inner.jobs.values().any(|job| {
            job.owner_id.is_some()
                && job.owner_id.as_deref() != Some(self.owner_id.as_str())
                && is_open(job.status)
        })
    }

    fn on_heartbeat(&self) {
        let ids: Vec<String> = {
            let inner = self.inner.borrow();
            inner
                .order
                .iter()
                .filter_map(|id| inner.jobs.get(id))
                .filter(|job| self.is_ours(job) && is_open(job.status))
                .map(|job| job.id.clone())
                .collect()
        };
        for id in ids {
            match self.store.is_cancel_requested(&id) {
                Ok(true) => {
                    let _ = self.cancel(&id);
                }
                Ok(false) => self.persist(&id),
                Err(err) => tracing::error!("{err}"),
            }
        }
        self.sync_timers();
    }

    fn poll_store(&self) {
        let Some(project) = self.inner.borrow().project.clone() else {
            return;
        };
        let records = match self.store.jobs(&project, KEPT_JOBS as i64) {
            Ok(records) => records,
            Err(err) => {
                tracing::error!("{err}");
                return;
            }
        };
        let now = self.clock.now_ms();
        for record in records {
            let known = self.inner.borrow().jobs.contains_key(&record.id);
            if !known {
                if !self.inner.borrow().watching {
                    continue;
                }
                let mut job = from_record(record, project.clone(), now);
                let interrupt = self.should_interrupt(&job);
                if interrupt {
                    job.status = JobStatus::Failed;
                    job.error = Some("interrupted when the app closed".to_string());
                }
                let id = job.id.clone();
                self.inner.borrow_mut().order.push(id.clone());
                self.inner.borrow_mut().jobs.insert(id.clone(), job);
                if interrupt {
                    self.emit_status(&id);
                } else {
                    self.emit(&id);
                }
                continue;
            }
            let ours = self
                .inner
                .borrow()
                .jobs
                .get(&record.id)
                .is_some_and(|job| self.is_ours(job));
            if ours {
                continue;
            }
            let changed = self
                .inner
                .borrow()
                .jobs
                .get(&record.id)
                .is_some_and(|job| !same_snapshot(job, &record));
            if let Some(job) = self.inner.borrow_mut().jobs.get_mut(&record.id) {
                adopt(job, &record);
            }
            let interrupt = self
                .inner
                .borrow()
                .jobs
                .get(&record.id)
                .is_some_and(|job| self.should_interrupt(job));
            if interrupt {
                if let Some(job) = self.inner.borrow_mut().jobs.get_mut(&record.id) {
                    job.status = JobStatus::Failed;
                    job.error = Some("interrupted when the app closed".to_string());
                }
                self.emit_status(&record.id);
                continue;
            }
            if changed {
                self.emit(&record.id);
            }
        }
        self.sync_timers();
    }
}

fn is_open(status: JobStatus) -> bool {
    status == JobStatus::Queued || status == JobStatus::Running
}

fn is_finished(status: JobStatus) -> bool {
    status == JobStatus::Done || status == JobStatus::Failed
}

fn resolve_access(value: Option<SubmitAccess>) -> JobAccess {
    match value {
        Some(SubmitAccess::Flag(true)) => JobAccess::Edit,
        Some(SubmitAccess::Flag(false)) | None => JobAccess::Read,
        Some(SubmitAccess::Level(level)) => level,
    }
}

fn elapsed_ms(start: Instant) -> i64 {
    i64::try_from(start.elapsed().as_millis()).unwrap_or(i64::MAX)
}

fn push_step(job: &mut InternalJob, step: JobStep) {
    job.steps.push(step);
    while job.steps.len() > STEP_CAP {
        job.steps.remove(0);
        job.steps_dropped += 1;
    }
}

fn close_steps(job: &mut InternalJob, ok: bool, now: i64) {
    for step in &mut job.steps {
        if step.status != JobStepStatus::Running {
            continue;
        }
        step.status = if ok {
            JobStepStatus::Done
        } else {
            JobStepStatus::Failed
        };
        step.ended_at = Some(now);
    }
}

fn extend_thinking(job: &mut InternalJob, detail: Option<&str>, title: &str, now: i64) -> bool {
    let Some(last) = job.steps.last() else {
        return false;
    };
    if last.tool != StepTool::Think || last.status != JobStepStatus::Done {
        return false;
    }
    let previous = last.detail.clone().unwrap_or_else(|| last.title.clone());
    let addition = detail.unwrap_or(title);
    let combined: String = previous.chars().chain(addition.chars()).collect();
    let text = tail_chars(&combined, 4000);
    let title = first_line(&text);
    let Some(last) = job.steps.last_mut() else {
        return false;
    };
    last.detail = Some(text);
    last.title = title;
    last.ended_at = Some(now);
    true
}

fn first_line(text: &str) -> String {
    for line in text.split('\n') {
        let line = line.trim_end_matches('\r');
        if !line.trim().is_empty() {
            return prefix_chars(line.trim(), 120);
        }
    }
    String::new()
}

fn prefix_chars(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

fn tail_chars(text: &str, n: usize) -> String {
    let count = text.chars().count();
    if count <= n {
        text.to_string()
    } else {
        text.chars().skip(count - n).collect()
    }
}

fn handoff_prompt(original: &str, from: ProviderId, output: &str, edit: bool) -> String {
    let mut prompt = original.to_string();
    if !output.is_empty() {
        let tail = tail_chars(output, 2000);
        prompt.push_str(&format!(
            "\n\n[Handoff: a previous attempt on {} stopped at its usage limit. Its output so far:]\n{tail}",
            from.as_str()
        ));
    }
    if edit {
        let note = "The files you need are already changed in this folder; continue from them.";
        if output.is_empty() {
            prompt.push_str("\n\n");
        } else {
            prompt.push('\n');
        }
        prompt.push_str(note);
    }
    prompt
}

fn failure_message(result_text: Option<&str>, stderr: &str) -> String {
    if let Some(text) = result_text.filter(|text| !text.is_empty()) {
        return text.to_string();
    }
    let trimmed = stderr.trim();
    if let Some(model) = unrecognized_model(trimmed) {
        return format!(
            "This CLI version does not know the model {model}. Update the CLI, or pick another model in Usage."
        );
    }
    if trimmed.chars().count() <= 500 {
        trimmed.to_string()
    } else {
        tail_chars(trimmed, 500)
    }
}

fn unrecognized_model(text: &str) -> Option<String> {
    let marker = "unrecognized_model]";
    let start = text.find(marker)? + marker.len();
    let rest = text[start..].trim_start();
    let prefix = "{\"model\":\"";
    let after = rest.strip_prefix(prefix)?;
    let end = after.find('"')?;
    if end == 0 || end > 80 {
        return None;
    }
    Some(after[..end].to_string())
}

fn to_record(job: &InternalJob) -> JobRecord {
    JobRecord {
        id: job.id.clone(),
        job_type: job.job_type,
        prompt: job.prompt.clone(),
        provider: job.provider,
        status: job.status,
        output: job.output.clone(),
        failed_over: job.failed_over.clone(),
        steps: Some(job.steps.clone()),
        steps_dropped: Some(job.steps_dropped),
        model: job.model.clone(),
        reason: job.reason.clone(),
        edit: (job.access != JobAccess::Read).then_some(true),
        access: Some(job.access),
        change: job.change.clone(),
        group: job.group.clone(),
        lead_message: job.lead_message.clone(),
        routine: job.routine.clone(),
        title: job.title.clone(),
        session_id: job.session_id.clone(),
        goal: job.goal.clone(),
        base: job.base.clone(),
        error: job.error.clone(),
        owner_id: job.owner_id.clone(),
        heartbeat_at: job.heartbeat_at,
        cancel_requested: job.cancel_requested.then_some(true),
        extra: std::collections::BTreeMap::new(),
    }
}

fn empty_record(id: &str) -> JobRecord {
    JobRecord {
        id: id.to_string(),
        job_type: JobType::Planning,
        prompt: String::new(),
        provider: None,
        status: JobStatus::Failed,
        output: String::new(),
        failed_over: Vec::new(),
        steps: Some(Vec::new()),
        steps_dropped: Some(0),
        model: None,
        reason: None,
        edit: None,
        access: Some(JobAccess::Read),
        change: None,
        group: None,
        lead_message: None,
        routine: None,
        title: None,
        session_id: None,
        goal: None,
        base: None,
        error: Some("missing job".to_string()),
        owner_id: None,
        heartbeat_at: None,
        cancel_requested: None,
        extra: std::collections::BTreeMap::new(),
    }
}

fn clone_job(job: &InternalJob) -> InternalJob {
    InternalJob {
        id: job.id.clone(),
        job_type: job.job_type,
        original_prompt: job.original_prompt.clone(),
        prompt: job.prompt.clone(),
        provider: job.provider,
        status: job.status,
        output: job.output.clone(),
        failed_over: job.failed_over.clone(),
        model: job.model.clone(),
        reason: job.reason.clone(),
        chosen: job.chosen,
        access: job.access,
        change: job.change.clone(),
        group: job.group.clone(),
        lead_message: job.lead_message.clone(),
        routine: job.routine.clone(),
        title: job.title.clone(),
        session_id: job.session_id.clone(),
        goal: job.goal.clone(),
        base: job.base.clone(),
        error: job.error.clone(),
        project: job.project.clone(),
        created_at: job.created_at,
        steps: job.steps.clone(),
        steps_dropped: job.steps_dropped,
        owner_id: job.owner_id.clone(),
        heartbeat_at: job.heartbeat_at,
        cancel_requested: job.cancel_requested,
    }
}

fn from_record(record: JobRecord, project: String, created_at: i64) -> InternalJob {
    let access = match record.access {
        Some(access) => access,
        None if record.edit == Some(true) => JobAccess::Edit,
        None => JobAccess::Read,
    };
    InternalJob {
        id: record.id,
        job_type: record.job_type,
        original_prompt: record.prompt.clone(),
        prompt: record.prompt,
        provider: record.provider,
        status: record.status,
        output: record.output,
        failed_over: record.failed_over,
        model: record.model,
        reason: record.reason.clone(),
        chosen: record.reason.as_deref() == Some("chosen"),
        access,
        change: record.change,
        group: record.group,
        lead_message: record.lead_message,
        routine: record.routine,
        title: record.title,
        session_id: record.session_id,
        goal: record.goal,
        base: record.base,
        error: record.error,
        project,
        created_at,
        steps: record.steps.unwrap_or_default(),
        steps_dropped: record.steps_dropped.unwrap_or(0),
        owner_id: record.owner_id,
        heartbeat_at: record.heartbeat_at,
        cancel_requested: record.cancel_requested == Some(true),
    }
}

fn same_snapshot(job: &InternalJob, record: &JobRecord) -> bool {
    job.status == record.status
        && job.output == record.output
        && job.error == record.error
        && job.provider == record.provider
        && job.model == record.model
        && job.change == record.change
        && job.steps == record.steps.clone().unwrap_or_default()
}

fn adopt(job: &mut InternalJob, record: &JobRecord) {
    job.status = record.status;
    job.output = record.output.clone();
    job.provider = record.provider;
    job.failed_over = record.failed_over.clone();
    job.steps = record.steps.clone().unwrap_or_default();
    job.steps_dropped = record.steps_dropped.unwrap_or(0);
    job.error = record.error.clone();
    job.model = record.model.clone();
    if let Some(session) = &record.session_id {
        job.session_id = Some(session.clone());
    }
    if let Some(reason) = &record.reason {
        job.reason = Some(reason.clone());
        job.chosen = reason == "chosen";
    }
    if let Some(change) = &record.change {
        job.change = Some(change.clone());
    }
    if let Some(owner) = &record.owner_id {
        job.owner_id = Some(owner.clone());
    }
    if let Some(beat) = record.heartbeat_at {
        job.heartbeat_at = Some(beat);
    }
    if record.cancel_requested == Some(true) {
        job.cancel_requested = true;
    }
    if let Some(access) = record.access {
        job.access = access;
    }
}

enum Applied {
    Quiet,
    Used(f64),
    Result(HeldResult),
}
