//! In-memory [`Drive`](crate::drive::Drive) for the command tests.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use legate_core::{
    ChangeSet, JobAccess, JobRecord, JobStatus, JobStep, JobStepStatus, JobType, ProjectInfo,
    ProviderId, StepTool,
};
use legate_git::{ListedName, MergeOutcome, OpenedPr};
use legate_lead::{Goal, GoalProposal, GoalStatus, PlannedJob, Routine, RoutineWorker};
use legate_providers::InteractiveCommand;

use crate::drive::{
    CreatedTree, Drive, DriveError, GoalRequest, JobRequest, JobRow, Listener, LiveEvent,
    SigHandler, Unsub,
};
use crate::format::PlanView;

pub fn words(args: &[&str]) -> Vec<String> {
    args.iter().map(|item| (*item).to_string()).collect()
}

pub fn job(id: &str, status: JobStatus) -> JobRecord {
    JobRecord {
        id: id.to_string(),
        job_type: JobType::Planning,
        prompt: "do the thing".to_string(),
        provider: Some(ProviderId::Claude),
        status,
        output: String::new(),
        failed_over: Vec::new(),
        steps: None,
        steps_dropped: None,
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
        error: None,
        owner_id: None,
        heartbeat_at: None,
        cancel_requested: None,
        extra: BTreeMap::new(),
    }
}

pub fn step(id: &str, tool: StepTool, title: &str, status: JobStepStatus) -> JobStep {
    JobStep {
        id: id.to_string(),
        tool,
        title: title.to_string(),
        detail: None,
        status,
        started_at: 1,
        ended_at: None,
        edit: None,
    }
}

pub fn row(created_at: i64, job: JobRecord) -> JobRow {
    JobRow {
        created_at: Some(created_at),
        job,
    }
}

pub fn plan(id: ProviderId) -> PlanView {
    PlanView {
        id,
        available: true,
        problem: None,
        headroom: 0.8,
        resting_until: None,
        busy: false,
        running: 0,
        version: Some("2.1.292".to_string()),
        version_status: legate_core::VersionStatus::Tested,
        at_risk: false,
    }
}

pub fn change(id: &str) -> ChangeSet {
    ChangeSet {
        id: id.to_string(),
        branch: format!("orch/{id}"),
        path: format!("/wt/{id}"),
        files: Vec::new(),
        insertions: 0,
        deletions: 0,
        name: None,
        setup: None,
    }
}

pub fn goal(id: &str) -> Goal {
    Goal {
        id: id.to_string(),
        project: "/proj".to_string(),
        text: "Ship login".to_string(),
        check: Some("pnpm test".to_string()),
        max_rounds: 5,
        quick: false,
        worktree: "wt1".to_string(),
        status: GoalStatus::Planning,
        rounds: Vec::new(),
        proposal: None,
        question: None,
        notes: Vec::new(),
        outcome: None,
        source: None,
        created_at: 1,
        updated_at: 1,
    }
}

pub fn routine(id: &str, name: &str) -> Routine {
    Routine {
        id: id.to_string(),
        name: name.to_string(),
        prompt: "run the tests".to_string(),
        job_type: JobType::Review,
        worker: RoutineWorker::Auto,
        access: JobAccess::Read,
        after: None,
    }
}

pub fn planned(job_type: JobType, access: JobAccess, prompt: &str) -> PlannedJob {
    PlannedJob {
        job_type,
        access,
        prompt: prompt.to_string(),
    }
}

pub fn proposal(summary: &str, jobs: Vec<PlannedJob>) -> GoalProposal {
    GoalProposal {
        summary: summary.to_string(),
        jobs,
    }
}

type SubmitHook = Arc<dyn Fn(&Fake, JobRequest) -> Result<JobRecord, DriveError> + Send + Sync>;
type CancelHook = Arc<dyn Fn(&Fake, &str) -> Result<JobRecord, DriveError> + Send + Sync>;
type TextHook = Arc<dyn Fn(&str) -> Result<String, DriveError> + Send + Sync>;
type MergeHook = Arc<dyn Fn(&str) -> Result<MergeOutcome, DriveError> + Send + Sync>;
type DiscardHook = Arc<dyn Fn(&str) -> Result<(), DriveError> + Send + Sync>;
type TreeHook = Arc<dyn Fn(&str) -> Result<CreatedTree, DriveError> + Send + Sync>;
type RoutineHook = Arc<dyn Fn(&str) -> Result<JobRecord, DriveError> + Send + Sync>;
type CreateGoalHook = Arc<dyn Fn(&Fake, GoalRequest) -> Result<Goal, DriveError> + Send + Sync>;
type GoalHook = Arc<dyn Fn(&Fake, &str) -> Result<Goal, DriveError> + Send + Sync>;
type AnswerHook = Arc<dyn Fn(&Fake, &str, &str) -> Result<Goal, DriveError> + Send + Sync>;
type AskHook = Box<dyn FnMut(&str) -> String + Send>;

struct Inner {
    out: Mutex<String>,
    err: Mutex<String>,
    version: String,
    project: Mutex<String>,
    now: Mutex<Box<dyn FnMut() -> i64 + Send>>,
    color: AtomicBool,
    tty: AtomicBool,
    stdin_tty: AtomicBool,
    columns: Mutex<u16>,
    rows: Mutex<u16>,
    read: Mutex<String>,
    ask: Mutex<AskHook>,
    asked: Mutex<String>,
    sigint: Mutex<Option<SigHandler>>,
    exited: Mutex<Vec<i32>>,
    inited: AtomicBool,
    plans: Mutex<Vec<PlanView>>,
    submit: Mutex<Option<SubmitHook>>,
    cancel: Mutex<Option<CancelHook>>,
    job_listeners: Mutex<Vec<Listener<JobRecord>>>,
    jobs: Mutex<Vec<JobRow>>,
    limit: Mutex<i64>,
    names: Mutex<Vec<ListedName>>,
    deleted: Mutex<Vec<String>>,
    info: Mutex<ProjectInfo>,
    changes: Mutex<Vec<ChangeSet>>,
    ids: Mutex<Option<Vec<String>>>,
    diff: Mutex<Option<TextHook>>,
    merge: Mutex<Option<MergeHook>>,
    discard: Mutex<Option<DiscardHook>>,
    create: Mutex<Option<TreeHook>>,
    routines: Mutex<Vec<Routine>>,
    run_routine: Mutex<Option<RoutineHook>>,
    pr: Mutex<OpenedPr>,
    goals: Mutex<Vec<Goal>>,
    create_goal: Mutex<Option<CreateGoalHook>>,
    approve: Mutex<Option<GoalHook>>,
    answer: Mutex<Option<AnswerHook>>,
    stop: Mutex<Option<GoalHook>>,
    goal_listeners: Mutex<Vec<Listener<Goal>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

#[derive(Clone)]
pub struct Fake(Arc<Inner>);

impl Fake {
    pub fn new() -> Self {
        Self(Arc::new(Inner {
            out: Mutex::new(String::new()),
            err: Mutex::new(String::new()),
            version: "1.0.0".to_string(),
            project: Mutex::new("/proj".to_string()),
            now: Mutex::new(Box::new(|| 10_000)),
            color: AtomicBool::new(false),
            tty: AtomicBool::new(false),
            stdin_tty: AtomicBool::new(true),
            columns: Mutex::new(80),
            rows: Mutex::new(24),
            read: Mutex::new(String::new()),
            ask: Mutex::new(Box::new(|_| String::new())),
            asked: Mutex::new(String::new()),
            sigint: Mutex::new(None),
            exited: Mutex::new(Vec::new()),
            inited: AtomicBool::new(false),
            plans: Mutex::new(Vec::new()),
            submit: Mutex::new(None),
            cancel: Mutex::new(None),
            job_listeners: Mutex::new(Vec::new()),
            jobs: Mutex::new(Vec::new()),
            limit: Mutex::new(0),
            names: Mutex::new(Vec::new()),
            deleted: Mutex::new(Vec::new()),
            info: Mutex::new(ProjectInfo {
                path: "/proj".to_string(),
                is_repo: true,
                branch: Some("main".to_string()),
            }),
            changes: Mutex::new(Vec::new()),
            ids: Mutex::new(None),
            diff: Mutex::new(None),
            merge: Mutex::new(None),
            discard: Mutex::new(None),
            create: Mutex::new(None),
            routines: Mutex::new(Vec::new()),
            run_routine: Mutex::new(None),
            pr: Mutex::new(OpenedPr {
                url: "https://example.test/pull/1".to_string(),
                branch: "orch/pr".to_string(),
            }),
            goals: Mutex::new(Vec::new()),
            create_goal: Mutex::new(None),
            approve: Mutex::new(None),
            answer: Mutex::new(None),
            stop: Mutex::new(None),
            goal_listeners: Mutex::new(Vec::new()),
        }))
    }

    pub fn out(&self) -> String {
        lock(&self.0.out).clone()
    }

    pub fn err(&self) -> String {
        lock(&self.0.err).clone()
    }

    pub fn set_project(&self, project: impl Into<String>) {
        *lock(&self.0.project) = project.into();
    }

    pub fn set_now<F>(&self, now: F)
    where
        F: FnMut() -> i64 + Send + 'static,
    {
        *lock(&self.0.now) = Box::new(now);
    }

    pub fn set_color(&self, color: bool) {
        self.0.color.store(color, Ordering::SeqCst);
    }

    pub fn set_stdin_tty(&self, tty: bool) {
        self.0.stdin_tty.store(tty, Ordering::SeqCst);
    }

    pub fn set_size(&self, columns: u16, rows: u16) {
        *lock(&self.0.columns) = columns;
        *lock(&self.0.rows) = rows;
    }

    pub fn set_read(&self, text: impl Into<String>) {
        *lock(&self.0.read) = text.into();
    }

    pub fn set_ask<F>(&self, ask: F)
    where
        F: FnMut(&str) -> String + Send + 'static,
    {
        *lock(&self.0.ask) = Box::new(ask);
    }

    pub fn asked(&self) -> String {
        lock(&self.0.asked).clone()
    }

    pub fn exited(&self) -> Vec<i32> {
        lock(&self.0.exited).clone()
    }

    pub fn inited(&self) -> bool {
        self.0.inited.load(Ordering::SeqCst)
    }

    pub fn set_plans(&self, plans: Vec<PlanView>) {
        *lock(&self.0.plans) = plans;
    }

    pub fn set_submit<F>(&self, submit: F)
    where
        F: Fn(&Fake, JobRequest) -> Result<JobRecord, DriveError> + Send + Sync + 'static,
    {
        *lock(&self.0.submit) = Some(Arc::new(submit));
    }

    pub fn set_cancel<F>(&self, cancel: F)
    where
        F: Fn(&Fake, &str) -> Result<JobRecord, DriveError> + Send + Sync + 'static,
    {
        *lock(&self.0.cancel) = Some(Arc::new(cancel));
    }

    pub fn emit_job(&self, job: JobRecord) {
        let listeners = lock(&self.0.job_listeners).clone();
        for listener in listeners {
            listener(job.clone());
        }
    }

    pub fn set_jobs(&self, jobs: Vec<JobRow>) {
        *lock(&self.0.jobs) = jobs;
    }

    pub fn limit(&self) -> i64 {
        *lock(&self.0.limit)
    }

    pub fn names(&self) -> Vec<ListedName> {
        lock(&self.0.names).clone()
    }

    pub fn deleted(&self) -> Vec<String> {
        lock(&self.0.deleted).clone()
    }

    pub fn set_info(&self, info: ProjectInfo) {
        *lock(&self.0.info) = info;
    }

    pub fn set_changes(&self, changes: Vec<ChangeSet>) {
        *lock(&self.0.changes) = changes;
    }

    pub fn set_ids(&self, ids: Vec<String>) {
        *lock(&self.0.ids) = Some(ids);
    }

    pub fn set_names(&self, names: Vec<ListedName>) {
        *lock(&self.0.names) = names;
    }

    pub fn set_diff<F>(&self, diff: F)
    where
        F: Fn(&str) -> Result<String, DriveError> + Send + Sync + 'static,
    {
        *lock(&self.0.diff) = Some(Arc::new(diff));
    }

    pub fn set_merge<F>(&self, merge: F)
    where
        F: Fn(&str) -> Result<MergeOutcome, DriveError> + Send + Sync + 'static,
    {
        *lock(&self.0.merge) = Some(Arc::new(merge));
    }

    pub fn set_discard<F>(&self, discard: F)
    where
        F: Fn(&str) -> Result<(), DriveError> + Send + Sync + 'static,
    {
        *lock(&self.0.discard) = Some(Arc::new(discard));
    }

    pub fn set_create<F>(&self, create: F)
    where
        F: Fn(&str) -> Result<CreatedTree, DriveError> + Send + Sync + 'static,
    {
        *lock(&self.0.create) = Some(Arc::new(create));
    }

    pub fn set_routines(&self, routines: Vec<Routine>) {
        *lock(&self.0.routines) = routines;
    }

    pub fn set_run_routine<F>(&self, run_routine: F)
    where
        F: Fn(&str) -> Result<JobRecord, DriveError> + Send + Sync + 'static,
    {
        *lock(&self.0.run_routine) = Some(Arc::new(run_routine));
    }

    pub fn set_goals(&self, goals: Vec<Goal>) {
        *lock(&self.0.goals) = goals;
    }

    pub fn set_create_goal<F>(&self, create: F)
    where
        F: Fn(&Fake, GoalRequest) -> Result<Goal, DriveError> + Send + Sync + 'static,
    {
        *lock(&self.0.create_goal) = Some(Arc::new(create));
    }

    pub fn set_approve<F>(&self, approve: F)
    where
        F: Fn(&Fake, &str) -> Result<Goal, DriveError> + Send + Sync + 'static,
    {
        *lock(&self.0.approve) = Some(Arc::new(approve));
    }

    pub fn set_answer<F>(&self, answer: F)
    where
        F: Fn(&Fake, &str, &str) -> Result<Goal, DriveError> + Send + Sync + 'static,
    {
        *lock(&self.0.answer) = Some(Arc::new(answer));
    }

    pub fn set_stop<F>(&self, stop: F)
    where
        F: Fn(&Fake, &str) -> Result<Goal, DriveError> + Send + Sync + 'static,
    {
        *lock(&self.0.stop) = Some(Arc::new(stop));
    }

    pub fn emit_goal(&self, goal: Goal) {
        let listeners = lock(&self.0.goal_listeners).clone();
        for listener in listeners {
            listener(goal.clone());
        }
    }

    pub fn fire_sigint(&self) {
        let handler = lock(&self.0.sigint).clone();
        if let Some(handler) = handler {
            handler();
        }
    }

    fn missing(kind: &str) -> DriveError {
        DriveError::new(format!("unexpected {kind}"))
    }
}

impl Drive for Fake {
    fn version(&self) -> &str {
        &self.0.version
    }

    fn project(&self) -> String {
        lock(&self.0.project).clone()
    }

    fn now(&self) -> i64 {
        lock(&self.0.now)()
    }

    fn color(&self) -> bool {
        self.0.color.load(Ordering::SeqCst)
    }

    fn tty(&self) -> bool {
        self.0.tty.load(Ordering::SeqCst)
    }

    fn term_size(&self) -> (u16, u16) {
        (*lock(&self.0.columns), *lock(&self.0.rows))
    }

    fn stdout(&self, text: &str) {
        lock(&self.0.out).push_str(text);
    }

    fn stderr(&self, text: &str) {
        lock(&self.0.err).push_str(text);
    }

    fn stdin_is_tty(&self) -> bool {
        self.0.stdin_tty.load(Ordering::SeqCst)
    }

    fn read_stdin(&self) -> impl Future<Output = String> + Send {
        let text = lock(&self.0.read).clone();
        async move { text }
    }

    fn ask(&self, prompt: &str) -> impl Future<Output = String> + Send {
        *lock(&self.0.asked) = prompt.to_string();
        let answer = lock(&self.0.ask)(prompt);
        async move { answer }
    }

    fn on_sigint(&self, handler: SigHandler) {
        *lock(&self.0.sigint) = Some(handler);
    }

    fn clear_sigint(&self) {
        *lock(&self.0.sigint) = None;
    }

    fn exit_now(&self, code: i32) {
        lock(&self.0.exited).push(code);
    }

    fn init(&self) -> impl Future<Output = Result<(), DriveError>> + Send {
        self.0.inited.store(true, Ordering::SeqCst);
        async { Ok(()) }
    }

    fn plans(&self) -> impl Future<Output = Result<Vec<PlanView>, DriveError>> + Send {
        let plans = lock(&self.0.plans).clone();
        async move { Ok(plans) }
    }

    fn submit(
        &self,
        request: JobRequest,
    ) -> impl Future<Output = Result<JobRecord, DriveError>> + Send {
        let hook = lock(&self.0.submit).clone();
        let fake = self.clone();
        async move {
            match hook {
                Some(hook) => hook(&fake, request),
                None => Err(Fake::missing("submit")),
            }
        }
    }

    fn cancel(&self, id: &str) -> impl Future<Output = Result<JobRecord, DriveError>> + Send {
        let hook = lock(&self.0.cancel).clone();
        let fake = self.clone();
        let id = id.to_string();
        async move {
            match hook {
                Some(hook) => hook(&fake, &id),
                None => Err(Fake::missing("cancel")),
            }
        }
    }

    fn on_job(&self, cb: Listener<JobRecord>) -> Unsub {
        lock(&self.0.job_listeners).push(cb);
        Unsub::new(|| {})
    }

    fn jobs_newest(&self, limit: i64) -> Vec<JobRow> {
        *lock(&self.0.limit) = limit;
        lock(&self.0.jobs).clone()
    }

    fn worktree_names(&self) -> Vec<ListedName> {
        lock(&self.0.names).clone()
    }

    fn set_worktree_name(&self, row: ListedName) {
        lock(&self.0.names).push(row);
    }

    fn delete_worktree_name(&self, id: &str) {
        lock(&self.0.deleted).push(id.to_string());
        lock(&self.0.names).retain(|row| row.id != id);
    }

    fn project_info(&self) -> impl Future<Output = Result<ProjectInfo, DriveError>> + Send {
        let info = lock(&self.0.info).clone();
        async move { Ok(info) }
    }

    fn changes(&self) -> impl Future<Output = Result<Vec<ChangeSet>, DriveError>> + Send {
        let changes = lock(&self.0.changes).clone();
        async move { Ok(changes) }
    }

    fn change_ids(&self) -> impl Future<Output = Result<Vec<String>, DriveError>> + Send {
        let ids = lock(&self.0.ids).clone().unwrap_or_else(|| {
            lock(&self.0.changes)
                .iter()
                .map(|change| change.id.clone())
                .collect()
        });
        async move { Ok(ids) }
    }

    fn folder(&self, id: &str) -> String {
        format!("/wt/{id}")
    }

    fn diff(&self, id: &str) -> impl Future<Output = Result<String, DriveError>> + Send {
        let hook = lock(&self.0.diff).clone();
        let id = id.to_string();
        async move {
            match hook {
                Some(hook) => hook(&id),
                None => Ok(String::new()),
            }
        }
    }

    fn merge(&self, id: &str) -> impl Future<Output = Result<MergeOutcome, DriveError>> + Send {
        let hook = lock(&self.0.merge).clone();
        let id = id.to_string();
        async move {
            match hook {
                Some(hook) => hook(&id),
                None => Ok(MergeOutcome {
                    ok: true,
                    message: "applied to main as staged changes".to_string(),
                }),
            }
        }
    }

    fn discard(&self, id: &str) -> impl Future<Output = Result<(), DriveError>> + Send {
        let hook = lock(&self.0.discard).clone();
        let id = id.to_string();
        async move {
            match hook {
                Some(hook) => hook(&id),
                None => Ok(()),
            }
        }
    }

    fn create_tree(
        &self,
        job_id: &str,
    ) -> impl Future<Output = Result<CreatedTree, DriveError>> + Send {
        let hook = lock(&self.0.create).clone();
        let job_id = job_id.to_string();
        async move {
            match hook {
                Some(hook) => hook(&job_id),
                None => Ok(CreatedTree {
                    id: "abcd1234".to_string(),
                    path: "/wt/abcd1234".to_string(),
                    branch: "orch/abcd1234".to_string(),
                }),
            }
        }
    }

    fn routines(&self) -> impl Future<Output = Result<Vec<Routine>, DriveError>> + Send {
        let routines = lock(&self.0.routines).clone();
        async move { Ok(routines) }
    }

    fn run_routine(&self, id: &str) -> Result<JobRecord, DriveError> {
        match lock(&self.0.run_routine).clone() {
            Some(hook) => hook(id),
            None => Err(Self::missing("routine")),
        }
    }

    fn open_pr(
        &self,
        _title: &str,
        _body: &str,
    ) -> impl Future<Output = Result<OpenedPr, DriveError>> + Send {
        let pr = lock(&self.0.pr).clone();
        async move { Ok(pr) }
    }

    fn goals(&self) -> impl Future<Output = Result<Vec<Goal>, DriveError>> + Send {
        let goals = lock(&self.0.goals).clone();
        async move { Ok(goals) }
    }

    fn create_goal(
        &self,
        input: GoalRequest,
    ) -> impl Future<Output = Result<Goal, DriveError>> + Send {
        let hook = lock(&self.0.create_goal).clone();
        let fake = self.clone();
        async move {
            match hook {
                Some(hook) => hook(&fake, input),
                None => Err(Fake::missing("goal")),
            }
        }
    }

    fn approve_goal(&self, id: &str) -> Result<Goal, DriveError> {
        match lock(&self.0.approve).clone() {
            Some(hook) => hook(self, id),
            None => Err(Self::missing("approve")),
        }
    }

    fn answer_goal(&self, id: &str, text: &str) -> Result<Goal, DriveError> {
        match lock(&self.0.answer).clone() {
            Some(hook) => hook(self, id, text),
            None => Err(Self::missing("answer")),
        }
    }

    fn stop_goal(&self, id: &str) -> Result<Goal, DriveError> {
        match lock(&self.0.stop).clone() {
            Some(hook) => hook(self, id),
            None => Err(Self::missing("stop")),
        }
    }

    fn on_goal(&self, cb: Listener<Goal>) -> Unsub {
        lock(&self.0.goal_listeners).push(cb);
        Unsub::new(|| {})
    }

    fn interactive(&self, _provider: ProviderId) -> InteractiveCommand {
        InteractiveCommand {
            command: "claude".to_string(),
            args: Vec::new(),
            env: None,
        }
    }

    async fn spawn_inherited(
        &self,
        _spec: &InteractiveCommand,
        _cwd: &str,
    ) -> Result<(), DriveError> {
        Ok(())
    }

    fn events(&self) -> Option<tokio::sync::broadcast::Receiver<LiveEvent>> {
        None
    }
}
