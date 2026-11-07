//! Ports of `src/core/router/orchestrator.test.ts`.
//! Fakes stand in for the provider and worktree crates.
#![allow(clippy::field_reassign_with_default)]

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use legate_core::{
    AgentEvent, FileEdit, JobAccess, JobRecord, JobStatus, JobStepStatus, JobType, PlanProblem,
    ProviderId, RunOptions, StepPhase, StepTool, UsageWindow, VersionStatus, load_rules,
};
use legate_router::{
    ActiveRun, BoxFuture, Clock, CwdSource, ExitStatus, FrozenClock, JOB_HEARTBEAT_MS,
    JOB_STALE_MS, JOB_WATCH_MS, ManualClock, Orchestrator, OrchestratorError, OrchestratorOpts,
    RunJob, Runner, SubmitAccess, SubmitExtra, Workspaces, WorktreeCreated, stderr_is_limit,
    tested_version, unknown_format_message,
};
use legate_store::{RunOutcome, Store};
use tokio::sync::Notify;

const CWD: &str = "C:\\work";

struct Flag {
    open: AtomicBool,
    notify: Notify,
}

impl Flag {
    fn new() -> Rc<Self> {
        Rc::new(Self {
            open: AtomicBool::new(false),
            notify: Notify::new(),
        })
    }

    fn trip(&self) {
        self.open.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }

    fn tripped(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }

    async fn wait(&self) {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.tripped() {
                return;
            }
            notified.await;
        }
    }
}

async fn wait_either(gate: &Flag, killed: &Flag) {
    loop {
        if killed.tripped() || gate.tripped() {
            return;
        }
        let left = gate.notify.notified();
        let right = killed.notify.notified();
        tokio::pin!(left);
        tokio::pin!(right);
        left.as_mut().enable();
        right.as_mut().enable();
        if killed.tripped() || gate.tripped() {
            return;
        }
        tokio::select! {
            _ = left => {}
            _ = right => {}
        }
    }
}

struct Script {
    events: Vec<AgentEvent>,
    gate: Option<Rc<Flag>>,
    pause: Option<Rc<Flag>>,
    exit_code: Option<i32>,
    stderr: String,
    unreadable: bool,
    throw_on_run: bool,
    on_start: Option<Rc<dyn Fn()>>,
}

impl Default for Script {
    fn default() -> Self {
        Self {
            events: Vec::new(),
            gate: None,
            pause: None,
            exit_code: Some(0),
            stderr: String::new(),
            unreadable: false,
            throw_on_run: false,
            on_start: None,
        }
    }
}

fn ok(text: &str) -> Script {
    Script {
        events: vec![AgentEvent::Result {
            ok: true,
            text: text.to_string(),
            session_id: Some("sess".to_string()),
            duration_ms: None,
            cost_usd: Some(1.0),
            tokens: Some(1),
        }],
        ..Script::default()
    }
}

fn text(value: &str) -> AgentEvent {
    AgentEvent::Text {
        text: value.to_string(),
    }
}

fn limit(resets_at: Option<i64>) -> AgentEvent {
    AgentEvent::Limit {
        resets_at,
        message: "spent".to_string(),
    }
}

fn step_end(id: &str, title: &str) -> AgentEvent {
    AgentEvent::Step {
        id: id.to_string(),
        phase: StepPhase::End,
        tool: StepTool::Read,
        title: title.to_string(),
        detail: None,
        ok: Some(true),
        edit: None,
    }
}

#[derive(Clone, Copy)]
enum Phase {
    Gate,
    Events,
    Pause,
}

struct FakeRun {
    events: Vec<AgentEvent>,
    index: usize,
    phase: Phase,
    gate: Option<Rc<Flag>>,
    pause: Option<Rc<Flag>>,
    killed: Rc<Flag>,
    exit_code: Option<i32>,
    stderr: String,
    unreadable: bool,
}

impl FakeRun {
    fn from_script(script: Script) -> Self {
        Self {
            events: script.events,
            index: 0,
            phase: Phase::Gate,
            gate: script.gate,
            pause: script.pause,
            killed: Flag::new(),
            exit_code: script.exit_code,
            stderr: script.stderr,
            unreadable: script.unreadable,
        }
    }
}

impl ActiveRun for FakeRun {
    fn next_event(&mut self) -> BoxFuture<'_, Option<AgentEvent>> {
        Box::pin(async move {
            // An async generator's first `next` is a yield point, so a test can
            // observe `running` before the events are applied.
            tokio::task::yield_now().await;
            loop {
                if self.killed.tripped() {
                    return None;
                }
                match self.phase {
                    Phase::Gate => {
                        if let Some(gate) = self.gate.clone() {
                            let killed = Rc::clone(&self.killed);
                            wait_either(&gate, &killed).await;
                            if self.killed.tripped() {
                                return None;
                            }
                        }
                        self.phase = Phase::Events;
                    }
                    Phase::Pause => {
                        if let Some(pause) = self.pause.clone() {
                            let killed = Rc::clone(&self.killed);
                            wait_either(&pause, &killed).await;
                            if self.killed.tripped() {
                                return None;
                            }
                        }
                        self.phase = Phase::Events;
                    }
                    Phase::Events => {
                        if self.index >= self.events.len() {
                            return None;
                        }
                        let event = self.events[self.index].clone();
                        self.index += 1;
                        if self.index == 1 && self.pause.is_some() {
                            self.phase = Phase::Pause;
                        }
                        return Some(event);
                    }
                }
            }
        })
    }

    fn exit(&mut self) -> BoxFuture<'_, Result<ExitStatus, String>> {
        let status = ExitStatus {
            code: self.exit_code,
            stderr: self.stderr.clone(),
            unreadable: self.unreadable,
        };
        Box::pin(async move { Ok(status) })
    }

    fn kill(&self) {
        self.killed.trip();
    }

    fn killer(&self) -> Rc<dyn Fn()> {
        let killed = Rc::clone(&self.killed);
        Rc::new(move || killed.trip())
    }
}

struct FakeAdapter {
    id: ProviderId,
    received: RefCell<Vec<RunJob>>,
    cwds: RefCell<Vec<String>>,
    options: RefCell<Vec<RunOptions>>,
    installed: Cell<bool>,
    signed_in: Cell<bool>,
    version: Cell<Option<String>>,
    scripts: RefCell<VecDeque<Script>>,
}

impl FakeAdapter {
    fn new(id: ProviderId, scripts: Vec<Script>) -> Rc<Self> {
        Rc::new(Self {
            id,
            received: RefCell::new(Vec::new()),
            cwds: RefCell::new(Vec::new()),
            options: RefCell::new(Vec::new()),
            installed: Cell::new(true),
            signed_in: Cell::new(true),
            version: Cell::new(tested_version(id).map(str::to_string)),
            scripts: RefCell::new(VecDeque::from(scripts)),
        })
    }

    fn prompts(&self) -> Vec<String> {
        self.received
            .borrow()
            .iter()
            .map(|job| job.prompt.clone())
            .collect()
    }
}

impl Runner for FakeAdapter {
    fn id(&self) -> ProviderId {
        self.id
    }

    fn is_installed(&self) -> BoxFuture<'_, bool> {
        let value = self.installed.get();
        Box::pin(async move { value })
    }

    fn is_signed_in(&self) -> BoxFuture<'_, bool> {
        let value = self.signed_in.get();
        Box::pin(async move { value })
    }

    fn version(&self) -> BoxFuture<'_, Option<String>> {
        let value = self.version.take();
        self.version.set(value.clone());
        Box::pin(async move { value })
    }

    fn is_limit_error(&self, stderr: &str) -> bool {
        stderr_is_limit(stderr)
    }

    fn start(
        &self,
        job: &RunJob,
        cwd: &str,
        opts: &RunOptions,
    ) -> Result<Box<dyn ActiveRun>, String> {
        self.received.borrow_mut().push(job.clone());
        self.cwds.borrow_mut().push(cwd.to_string());
        self.options.borrow_mut().push(opts.clone());
        let Some(script) = self.scripts.borrow_mut().pop_front() else {
            return Err(format!("no script left for {}", self.id.as_str()));
        };
        if let Some(on_start) = &script.on_start {
            on_start();
        }
        if script.throw_on_run {
            return Err("adapter exploded".to_string());
        }
        Ok(Box::new(FakeRun::from_script(script)))
    }
}

type MakeTree = Rc<dyn Fn(&str, &str, Option<&str>) -> Result<WorktreeCreated, String>>;

struct Trees {
    make: MakeTree,
}

impl Workspaces for Trees {
    fn create(
        &self,
        project: &str,
        job_id: &str,
        base: Option<&str>,
    ) -> BoxFuture<'_, Result<WorktreeCreated, String>> {
        let result = (self.make)(project, job_id, base);
        Box::pin(async move { result })
    }

    fn remove(&self, _id: &str) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }

    fn changes(&self, _id: &str) -> BoxFuture<'_, Result<Vec<legate_core::ChangedFile>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

fn trees(
    make: impl Fn(&str, &str, Option<&str>) -> Result<WorktreeCreated, String> + 'static,
) -> Rc<dyn Workspaces> {
    Rc::new(Trees {
        make: Rc::new(make),
    })
}

fn slice_tree(job_id: &str) -> WorktreeCreated {
    let id = job_id.chars().take(8).collect::<String>();
    WorktreeCreated {
        path: format!("wt/{id}"),
        branch: format!("orch/{id}"),
        id,
    }
}

#[derive(Default)]
struct Opts {
    clock: Option<Rc<dyn Clock>>,
    rules: Option<legate_core::RouterRules>,
    cwd: Option<CwdSource>,
    model_for: Option<Rc<dyn Fn(ProviderId) -> Option<String>>>,
    workspaces: Option<Rc<dyn Workspaces>>,
    on_changes: Option<Rc<dyn Fn()>>,
    owner_id: Option<String>,
}

fn drive(fut: impl Future<Output = ()>) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async move { tokio::task::LocalSet::new().run_until(fut).await });
}

fn runners(adapters: &[Rc<FakeAdapter>]) -> Vec<Rc<dyn Runner>> {
    adapters
        .iter()
        .map(|adapter| adapter.clone() as Rc<dyn Runner>)
        .collect()
}

async fn with_orch<F, Fut>(adapters: Vec<Rc<FakeAdapter>>, opts: Opts, body: F)
where
    F: FnOnce(Rc<Orchestrator>, Rc<Store>) -> Fut,
    Fut: Future<Output = ()>,
{
    let store = Rc::new(Store::open(":memory:").unwrap());
    let clock = opts
        .clock
        .unwrap_or_else(|| Rc::new(FrozenClock::new(1_700_000_000_000)) as Rc<dyn Clock>);
    let orch = Orchestrator::new(OrchestratorOpts {
        runners: runners(&adapters),
        store: Rc::clone(&store),
        rules: opts.rules.unwrap_or_else(|| load_rules(None).unwrap()),
        cwd: opts
            .cwd
            .unwrap_or_else(|| CwdSource::Fixed(CWD.to_string())),
        clock: Some(clock),
        model_for: opts.model_for,
        workspaces: opts.workspaces,
        on_changes: opts.on_changes,
        owner_id: opts.owner_id,
    })
    .unwrap();
    orch.init().await;
    body(orch, store).await;
}

fn rules_with(max_parallel: i64) -> legate_core::RouterRules {
    let mut rules = load_rules(None).unwrap();
    rules.max_parallel = max_parallel;
    rules
}

fn go(orch: &Orchestrator, kind: JobType, prompt: &str) -> JobRecord {
    orch.submit(kind, prompt, None, None, SubmitExtra::default())
}

fn go_on(orch: &Orchestrator, kind: JobType, prompt: &str, provider: ProviderId) -> JobRecord {
    orch.submit(kind, prompt, Some(provider), None, SubmitExtra::default())
}

fn clock_at(now: i64) -> Rc<dyn Clock> {
    Rc::new(FrozenClock::new(now))
}

async fn wait_for(mut pred: impl FnMut() -> bool) {
    let start = std::time::Instant::now();
    while !pred() {
        if start.elapsed() > Duration::from_secs(3) {
            panic!("timed out waiting for condition");
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn bare(id: &str, status: JobStatus, prompt: &str) -> JobRecord {
    JobRecord {
        id: id.to_string(),
        job_type: JobType::Planning,
        prompt: prompt.to_string(),
        provider: Some(ProviderId::Claude),
        status,
        output: "ok".to_string(),
        failed_over: Vec::new(),
        steps: None,
        steps_dropped: None,
        model: None,
        reason: None,
        edit: None,
        access: None,
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

#[test]
fn routes_a_job_to_the_provider_listed_first() {
    drive(async {
        let claude = FakeAdapter::new(ProviderId::Claude, vec![ok("plan")]);
        let cursor = FakeAdapter::new(ProviderId::Cursor, vec![ok("files")]);
        with_orch(
            vec![claude.clone(), cursor.clone()],
            Opts::default(),
            |orch, _| async move {
                go(&orch, JobType::Planning, "plan this");
                go(&orch, JobType::Boilerplate, "scaffold this");
                orch.idle().await;
                assert_eq!(claude.prompts(), ["plan this"]);
                assert_eq!(cursor.prompts(), ["scaffold this"]);
                assert_eq!(claude.cwds.borrow().as_slice(), [CWD]);
                assert!(orch.list().iter().all(|job| job.status == JobStatus::Done));
            },
        )
        .await;
    });
}

#[test]
fn runs_up_to_max_parallel_and_lets_providers_run_together() {
    drive(async {
        let claude_hold = Flag::new();
        let cursor_hold = Flag::new();
        let mut a = ok("a");
        a.gate = Some(claude_hold.clone());
        let mut b = ok("b");
        b.gate = Some(claude_hold.clone());
        let mut c = ok("c");
        c.gate = Some(cursor_hold.clone());
        let claude = FakeAdapter::new(ProviderId::Claude, vec![a, b]);
        let cursor = FakeAdapter::new(ProviderId::Cursor, vec![c]);
        with_orch(
            vec![claude.clone(), cursor.clone()],
            Opts::default(),
            |orch, _| async move {
                go(&orch, JobType::Planning, "c1");
                go(&orch, JobType::Refactor, "r1");
                go(&orch, JobType::Planning, "c2");
                assert_eq!(claude.prompts(), ["c1", "c2"]);
                assert_eq!(cursor.prompts(), ["r1"]);
                assert_eq!(orch.list()[2].status, JobStatus::Running);
                assert_eq!(orch.list()[2].provider, Some(ProviderId::Claude));
                assert_eq!(orch.workers()[0].running, 2);
                assert!(orch.workers()[0].busy);
                claude_hold.trip();
                cursor_hold.trip();
                orch.idle().await;
                assert!(orch.list().iter().all(|job| job.status == JobStatus::Done));
            },
        )
        .await;
    });
}

#[test]
fn a_successful_run_writes_one_ok_row_and_keeps_streamed_text() {
    drive(async {
        let now = 1_700_000_000_000;
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![Script {
                events: vec![
                    text("hello"),
                    text("world"),
                    AgentEvent::Usage {
                        utilization: 0.2,
                        resets_at: None,
                        windows: None,
                    },
                    AgentEvent::Result {
                        ok: true,
                        text: "ignored".into(),
                        session_id: Some("s1".into()),
                        duration_ms: None,
                        cost_usd: Some(1.5),
                        tokens: Some(80),
                    },
                ],
                ..Script::default()
            }],
        );
        let mut opts = Opts::default();
        opts.clock = Some(clock_at(now));
        with_orch(vec![claude], opts, |orch, store| async move {
            go(&orch, JobType::Debugging, "find it");
            orch.idle().await;
            assert_eq!(orch.list()[0].status, JobStatus::Done);
            assert_eq!(orch.list()[0].output, "hello\nworld");
            let runs = store.runs(None).unwrap();
            assert_eq!(runs.len(), 1);
            assert_eq!(runs[0].outcome, RunOutcome::Ok);
            assert_eq!(runs[0].cost_usd, Some(1.5));
            assert_eq!(runs[0].tokens, Some(80));
            assert_eq!(runs[0].utilization, Some(0.2));
            assert_eq!(runs[0].session_id.as_deref(), Some("s1"));
            assert_eq!(runs[0].provider, "claude");
            assert_eq!(runs[0].job_type, "debugging");
            assert_eq!(runs[0].started_at, now);
            assert!(runs[0].duration_ms.unwrap() >= 0);
        })
        .await;
    });
}

#[test]
fn uses_result_text_when_the_stream_produced_none() {
    drive(async {
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![Script {
                events: vec![AgentEvent::Result {
                    ok: true,
                    text: "from-result".into(),
                    session_id: None,
                    duration_ms: None,
                    cost_usd: Some(1.0),
                    tokens: Some(2),
                }],
                ..Script::default()
            }],
        );
        with_orch(vec![claude], Opts::default(), |orch, _| async move {
            go(&orch, JobType::Planning, "go");
            orch.idle().await;
            assert_eq!(orch.list()[0].output, "from-result");
        })
        .await;
    });
}

#[test]
fn a_limit_event_rests_claude_and_hands_the_job_to_cursor() {
    drive(async {
        let now = 1_700_000_000_000;
        let resets_at = 1_800_000_000;
        let produced = format!("HEAD{}", "y".repeat(2000));
        let held = Flag::new();
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![Script {
                events: vec![
                    text(&produced),
                    AgentEvent::Usage {
                        utilization: 0.4,
                        resets_at: None,
                        windows: None,
                    },
                    limit(Some(resets_at)),
                ],
                ..Script::default()
            }],
        );
        let cursor = FakeAdapter::new(
            ProviderId::Cursor,
            vec![Script {
                gate: Some(held.clone()),
                events: vec![AgentEvent::Result {
                    ok: true,
                    text: "done".into(),
                    session_id: Some("c".into()),
                    duration_ms: None,
                    cost_usd: Some(2.0),
                    tokens: Some(3),
                }],
                ..Script::default()
            }],
        );
        let mut opts = Opts::default();
        opts.clock = Some(clock_at(now));
        with_orch(vec![claude.clone(), cursor.clone()], opts, |orch, store| async move {
            go(&orch, JobType::Planning, "do the thing");
            wait_for(|| cursor.received.borrow().len() == 1).await;
            assert!(orch.busy());
            let handoff = "\n\n[Handoff: a previous attempt on claude stopped at its usage limit. Its output so far:]\n";
            assert_eq!(claude.prompts()[0], "do the thing");
            let expected_tail: String = produced.chars().rev().take(2000).collect::<String>().chars().rev().collect();
            assert_eq!(cursor.prompts()[0], format!("do the thing{handoff}{expected_tail}"));
            assert!(!cursor.prompts()[0].contains("HEAD"));
            held.trip();
            orch.idle().await;
            let job = &orch.list()[0];
            assert_eq!(job.status, JobStatus::Done);
            assert_eq!(job.provider, Some(ProviderId::Cursor));
            assert_eq!(job.failed_over, vec![ProviderId::Claude]);
            assert_eq!(job.output, "done");
            assert_eq!(store.resting_until("claude").unwrap(), Some(resets_at * 1000));
            let claude_runs = store.runs(Some("claude")).unwrap();
            assert_eq!(claude_runs[0].outcome, RunOutcome::Limit);
            assert_eq!(claude_runs[0].utilization, Some(0.4));
            assert_eq!(claude_runs[0].started_at, now);
            let cursor_runs = store.runs(Some("cursor")).unwrap();
            assert_eq!(cursor_runs[0].outcome, RunOutcome::Ok);
            assert_eq!(cursor_runs[0].cost_usd, Some(2.0));
            assert_eq!(cursor_runs[0].tokens, Some(3));
            assert_eq!(cursor_runs[0].session_id.as_deref(), Some("c"));
        })
        .await;
    });
}

#[test]
fn queued_jobs_of_a_resting_provider_are_re_picked() {
    drive(async {
        let held = Flag::new();
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![Script {
                gate: Some(held.clone()),
                events: vec![text("partial"), limit(Some(1_900_000_000))],
                ..Script::default()
            }],
        );
        let cursor = FakeAdapter::new(ProviderId::Cursor, vec![ok("one"), ok("two")]);
        cursor.signed_in.set(false);
        let mut opts = Opts::default();
        opts.rules = Some(rules_with(1));
        with_orch(vec![claude.clone(), cursor.clone()], opts, |orch, _| async move {
            go(&orch, JobType::Planning, "alpha");
            assert_eq!(claude.received.borrow().len(), 1);
            let second = go(&orch, JobType::Planning, "beta");
            assert_eq!(second.status, JobStatus::Queued);
            assert_eq!(second.provider, Some(ProviderId::Claude));
            assert!(cursor.received.borrow().is_empty());
            cursor.signed_in.set(true);
            orch.init().await;
            held.trip();
            orch.idle().await;
            assert_eq!(claude.received.borrow().len(), 1);
            assert!(cursor.prompts()[0].contains(
                "[Handoff: a previous attempt on claude stopped at its usage limit. Its output so far:]\npartial"
            ));
            assert_eq!(cursor.prompts()[1], "beta");
            assert!(orch.list().iter().all(|job| job.status == JobStatus::Done));
            assert_eq!(orch.list()[0].failed_over, vec![ProviderId::Claude]);
            assert!(orch.list()[1].failed_over.is_empty());
        })
        .await;
    });
}

#[test]
fn both_providers_limited_fails_the_job() {
    drive(async {
        let now = 1_700_000_000_000;
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![Script {
                events: vec![text("aaa"), limit(Some(1_900_000_000))],
                ..Script::default()
            }],
        );
        let cursor = FakeAdapter::new(
            ProviderId::Cursor,
            vec![Script {
                events: vec![text("bbb"), limit(None)],
                ..Script::default()
            }],
        );
        let mut opts = Opts::default();
        opts.clock = Some(clock_at(now));
        with_orch(
            vec![claude, cursor.clone()],
            opts,
            |orch, store| async move {
                go(&orch, JobType::Planning, "original");
                orch.idle().await;
                let job = &orch.list()[0];
                assert_eq!(job.status, JobStatus::Failed);
                assert_eq!(
                    job.error.as_deref(),
                    Some("all providers are at their limit")
                );
                assert_eq!(
                    job.failed_over,
                    vec![ProviderId::Claude, ProviderId::Cursor]
                );
                assert_eq!(cursor.prompts()[0].matches("[Handoff:").count(), 1);
                assert!(cursor.prompts()[0].contains("aaa"));
                let outcomes: Vec<_> = store
                    .runs(None)
                    .unwrap()
                    .into_iter()
                    .map(|run| run.outcome)
                    .collect();
                assert_eq!(outcomes, vec![RunOutcome::Limit, RunOutcome::Limit]);
                assert_eq!(
                    store.resting_until("cursor").unwrap(),
                    Some(now + 5 * 3_600_000)
                );
            },
        )
        .await;
    });
}

#[test]
fn stderr_limit_uses_the_default_rest_and_fails_over() {
    drive(async {
        let now = 5_000_000;
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![Script {
                events: vec![text("partial")],
                exit_code: Some(1),
                stderr: "usage limit reached".into(),
                ..Script::default()
            }],
        );
        let cursor = FakeAdapter::new(ProviderId::Cursor, vec![ok("recovered")]);
        let mut opts = Opts::default();
        opts.clock = Some(clock_at(now));
        with_orch(vec![claude, cursor.clone()], opts, |orch, store| async move {
            go(&orch, JobType::Planning, "original");
            orch.idle().await;
            assert_eq!(orch.list()[0].status, JobStatus::Done);
            assert_eq!(orch.list()[0].failed_over, vec![ProviderId::Claude]);
            assert!(cursor.prompts()[0].contains(
                "[Handoff: a previous attempt on claude stopped at its usage limit. Its output so far:]\npartial"
            ));
            assert_eq!(store.runs(Some("claude")).unwrap()[0].outcome, RunOutcome::Limit);
            assert_eq!(store.resting_until("claude").unwrap(), Some(now + 5 * 3_600_000));
        })
        .await;
    });
}

#[test]
fn a_thrown_adapter_fails_that_job_and_the_queue_continues() {
    drive(async {
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![
                Script {
                    throw_on_run: true,
                    ..Script::default()
                },
                ok("second"),
            ],
        );
        with_orch(
            vec![claude.clone()],
            Opts::default(),
            |orch, store| async move {
                go(&orch, JobType::Planning, "first");
                go(&orch, JobType::Planning, "second");
                orch.idle().await;
                assert_eq!(orch.list()[0].status, JobStatus::Failed);
                assert_eq!(orch.list()[0].error.as_deref(), Some("adapter exploded"));
                assert_eq!(orch.list()[1].status, JobStatus::Done);
                assert_eq!(orch.list()[1].output, "second");
                assert_eq!(claude.prompts(), ["first", "second"]);
                assert_eq!(store.resting_until("claude").unwrap(), None);
                let outcomes: Vec<_> = store
                    .runs(None)
                    .unwrap()
                    .into_iter()
                    .map(|run| run.outcome)
                    .collect();
                assert_eq!(outcomes, vec![RunOutcome::Error, RunOutcome::Ok]);
            },
        )
        .await;
    });
}

#[test]
fn on_update_receives_copies_and_unsubscribe_stops_them() {
    drive(async {
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![
                Script {
                    events: vec![
                        text("hello"),
                        AgentEvent::Result {
                            ok: true,
                            text: "ignored".into(),
                            session_id: None,
                            duration_ms: None,
                            cost_usd: Some(1.0),
                            tokens: Some(1),
                        },
                    ],
                    ..Script::default()
                },
                ok("next"),
            ],
        );
        with_orch(vec![claude], Opts::default(), |orch, _| async move {
            let snaps = Rc::new(RefCell::new(Vec::new()));
            let sub = {
                let snaps = snaps.clone();
                orch.on_update(Rc::new(move |job| snaps.borrow_mut().push(job)))
            };
            go(&orch, JobType::Planning, "prompt");
            orch.idle().await;
            assert_eq!(snaps.borrow()[0].status, JobStatus::Queued);
            assert!(snaps.borrow().iter().any(|snap| snap.output == "hello"));
            assert!(
                snaps
                    .borrow()
                    .iter()
                    .any(|snap| snap.status == JobStatus::Done && snap.output == "hello")
            );
            let n = snaps.borrow().len();
            drop(sub);
            go(&orch, JobType::Planning, "after");
            orch.idle().await;
            assert_eq!(snaps.borrow().len(), n);
            assert_eq!(orch.list()[1].status, JobStatus::Done);
        })
        .await;
    });
}

#[test]
fn an_unknown_model_fails_with_a_message_that_says_so() {
    drive(async {
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![Script {
                exit_code: Some(1),
                stderr: "[claude-code:unrecognized_model] {\"model\":\"claude-opus-5-5\",\"query_source\":\"sdk\"}".into(),
                ..Script::default()
            }],
        );
        with_orch(vec![claude], Opts::default(), |orch, _| async move {
            go(&orch, JobType::Planning, "one");
            orch.idle().await;
            assert_eq!(
                orch.list()[0].error.as_deref(),
                Some("This CLI version does not know the model claude-opus-5-5. Update the CLI, or pick another model in Usage.")
            );
        })
        .await;
    });
}

#[test]
fn failed_runs_prefer_result_text_else_a_trimmed_stderr_tail() {
    drive(async {
        let stderr = format!(" {}END ", "a".repeat(600));
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![
                Script {
                    events: vec![AgentEvent::Result {
                        ok: false,
                        text: "bad result".into(),
                        session_id: None,
                        duration_ms: None,
                        cost_usd: Some(1.0),
                        tokens: Some(1),
                    }],
                    exit_code: Some(1),
                    stderr: "stderr message".into(),
                    ..Script::default()
                },
                Script {
                    events: vec![AgentEvent::Result {
                        ok: false,
                        text: String::new(),
                        session_id: None,
                        duration_ms: None,
                        cost_usd: Some(1.0),
                        tokens: Some(1),
                    }],
                    exit_code: Some(1),
                    stderr: stderr.clone(),
                    ..Script::default()
                },
            ],
        );
        with_orch(vec![claude], Opts::default(), |orch, store| async move {
            go(&orch, JobType::Planning, "one");
            go(&orch, JobType::Planning, "two");
            orch.idle().await;
            assert_eq!(orch.list()[0].error.as_deref(), Some("bad result"));
            let trimmed = stderr.trim();
            let tail: String = trimmed
                .chars()
                .rev()
                .take(500)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            assert_eq!(orch.list()[1].error.as_deref(), Some(tail.as_str()));
            assert!(orch.list()[1].error.as_deref().unwrap().ends_with("END"));
            let outcomes: Vec<_> = store
                .runs(None)
                .unwrap()
                .into_iter()
                .map(|run| run.outcome)
                .collect();
            assert_eq!(outcomes, [RunOutcome::Error, RunOutcome::Error]);
            assert_eq!(store.resting_until("claude").unwrap(), None);
        })
        .await;
    });
}

#[test]
fn no_signed_in_provider_fails_the_job_and_init_can_be_called_again() {
    drive(async {
        let claude = FakeAdapter::new(ProviderId::Claude, vec![ok("a")]);
        let cursor = FakeAdapter::new(ProviderId::Cursor, vec![ok("b")]);
        claude.installed.set(false);
        cursor.signed_in.set(false);
        with_orch(
            vec![claude.clone(), cursor.clone()],
            Opts::default(),
            |orch, _| async move {
                let missing = go(&orch, JobType::Review, "look");
                assert_eq!(missing.status, JobStatus::Failed);
                assert_eq!(missing.error.as_deref(), Some("no provider available"));
                assert_eq!(missing.provider, None);
                orch.idle().await;
                claude.installed.set(true);
                claude.signed_in.set(true);
                cursor.signed_in.set(true);
                orch.init().await;
                go(&orch, JobType::Planning, "first");
                orch.idle().await;
                assert_eq!(claude.prompts(), ["first"]);
                claude.signed_in.set(false);
                orch.init().await;
                go(&orch, JobType::Planning, "second");
                orch.idle().await;
                assert_eq!(cursor.prompts(), ["second"]);
            },
        )
        .await;
    });
}

#[test]
fn workers_report_availability_rest_headroom_and_queue_depth() {
    drive(async {
        let now = 1_700_000_000_000;
        let held = Flag::new();
        let mut a = ok("a");
        a.gate = Some(held.clone());
        let mut b = ok("b");
        b.gate = Some(held.clone());
        let claude = FakeAdapter::new(ProviderId::Claude, vec![a, b]);
        let cursor = FakeAdapter::new(ProviderId::Cursor, vec![]);
        cursor.signed_in.set(false);
        let mut opts = Opts::default();
        opts.clock = Some(clock_at(now));
        with_orch(vec![claude, cursor], opts, |orch, store| async move {
            store.set_resting("claude", now - 1_000).unwrap();
            store.set_resting("cursor", now + 60_000).unwrap();
            go(&orch, JobType::Planning, "one");
            go(&orch, JobType::Planning, "two");
            let workers = orch.workers();
            assert_eq!(workers[0].id, ProviderId::Claude);
            assert!(workers[0].available);
            assert_eq!(workers[0].resting_until, None);
            assert!(workers[0].busy);
            assert_eq!(workers[0].running, 2);
            assert_eq!(workers[0].queued, 0);
            assert!(!workers[0].at_risk);
            assert!(workers[0].windows.is_empty());
            assert!(!workers[1].available);
            assert_eq!(workers[1].resting_until, Some(now + 60_000));
            assert!(!workers[1].busy);
            assert_eq!(workers[1].headroom, 0.0);
            held.trip();
            orch.idle().await;
            assert!(!orch.workers()[0].busy);
            assert_eq!(orch.workers()[0].running, 0);
        })
        .await;
    });
}

#[test]
fn model_for_is_read_when_each_run_starts() {
    drive(async {
        let chosen = Rc::new(RefCell::new("opus".to_string()));
        let claude = FakeAdapter::new(ProviderId::Claude, vec![ok("a"), ok("b")]);
        let mut opts = Opts::default();
        let chosen_for = chosen.clone();
        opts.model_for = Some(Rc::new(move |_| Some(chosen_for.borrow().clone())));
        with_orch(vec![claude.clone()], opts, |orch, _| async move {
            go(&orch, JobType::Planning, "one");
            orch.idle().await;
            assert_eq!(claude.options.borrow()[0].model.as_deref(), Some("opus"));
            *chosen.borrow_mut() = "sonnet".to_string();
            go(&orch, JobType::Planning, "two");
            orch.idle().await;
            assert_eq!(claude.options.borrow()[1].model.as_deref(), Some("sonnet"));
        })
        .await;
    });
}

#[test]
fn without_model_for_the_adapter_sees_no_model() {
    drive(async {
        let claude = FakeAdapter::new(ProviderId::Claude, vec![ok("a")]);
        with_orch(
            vec![claude.clone()],
            Opts::default(),
            |orch, _| async move {
                go(&orch, JobType::Planning, "one");
                orch.idle().await;
                assert_eq!(claude.options.borrow()[0].model, None);
            },
        )
        .await;
    });
}

#[test]
fn an_init_model_is_stored_copied_and_emitted() {
    drive(async {
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![Script {
                events: vec![
                    AgentEvent::Init {
                        session_id: "s1".into(),
                        model: Some("claude-opus-4-8".into()),
                    },
                    AgentEvent::Result {
                        ok: true,
                        text: "done".into(),
                        session_id: None,
                        duration_ms: None,
                        cost_usd: Some(1.0),
                        tokens: Some(1),
                    },
                ],
                ..Script::default()
            }],
        );
        with_orch(vec![claude], Opts::default(), |orch, _| async move {
            let snaps = Rc::new(RefCell::new(Vec::new()));
            let snaps_cb = snaps.clone();
            let _sub = orch.on_update(Rc::new(move |job| snaps_cb.borrow_mut().push(job)));
            let submitted = go(&orch, JobType::Planning, "prompt");
            orch.idle().await;
            assert_eq!(
                orch.get(&submitted.id).unwrap().model.as_deref(),
                Some("claude-opus-4-8")
            );
            assert!(
                snaps
                    .borrow()
                    .iter()
                    .any(|snap| snap.model.as_deref() == Some("claude-opus-4-8"))
            );
            let mut listed = orch.list()[0].clone();
            listed.model = Some("mutated".into());
            assert_eq!(
                orch.get(&submitted.id).unwrap().model.as_deref(),
                Some("claude-opus-4-8")
            );
        })
        .await;
    });
}

#[test]
fn failover_clears_the_model_until_the_next_provider_reports_one() {
    drive(async {
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![Script {
                events: vec![
                    AgentEvent::Init {
                        session_id: "s1".into(),
                        model: Some("opus".into()),
                    },
                    text("partial"),
                    limit(None),
                ],
                ..Script::default()
            }],
        );
        let cursor = FakeAdapter::new(
            ProviderId::Cursor,
            vec![Script {
                events: vec![
                    AgentEvent::Init {
                        session_id: "s2".into(),
                        model: Some("composer".into()),
                    },
                    AgentEvent::Result {
                        ok: true,
                        text: "recovered".into(),
                        session_id: None,
                        duration_ms: None,
                        cost_usd: Some(1.0),
                        tokens: Some(1),
                    },
                ],
                ..Script::default()
            }],
        );
        with_orch(
            vec![claude, cursor],
            Opts::default(),
            |orch, _| async move {
                let snaps = Rc::new(RefCell::new(Vec::new()));
                let snaps_cb = snaps.clone();
                let _sub = orch.on_update(Rc::new(move |job| snaps_cb.borrow_mut().push(job)));
                go(&orch, JobType::Planning, "original");
                orch.idle().await;
                let snaps = snaps.borrow();
                let opus_at = snaps
                    .iter()
                    .position(|snap| snap.model.as_deref() == Some("opus"))
                    .unwrap();
                let handed = snaps
                    .iter()
                    .position(|snap| {
                        snap.provider == Some(ProviderId::Cursor)
                            && snap.status == JobStatus::Queued
                    })
                    .unwrap();
                assert!(handed > opus_at);
                assert_eq!(snaps[handed].model, None);
                assert_eq!(orch.list()[0].model.as_deref(), Some("composer"));
            },
        )
        .await;
    });
}

#[test]
fn get_returns_a_copy() {
    drive(async {
        let held = Flag::new();
        let mut script = ok("secret");
        script.gate = Some(held.clone());
        let claude = FakeAdapter::new(ProviderId::Claude, vec![script]);
        with_orch(vec![claude], Opts::default(), |orch, _| async move {
            let submitted = go(&orch, JobType::Review, "read me");
            let mut first = orch.get(&submitted.id).unwrap();
            assert_eq!(first, submitted);
            first.output = "mutated".into();
            first.prompt = "changed".into();
            first.failed_over.push(ProviderId::Cursor);
            assert_eq!(orch.get(&submitted.id).unwrap(), submitted);
            assert!(orch.get("missing").is_none());
            held.trip();
            orch.idle().await;
            assert_eq!(orch.get(&submitted.id).unwrap().output, "secret");
        })
        .await;
    });
}

#[test]
fn a_usage_event_with_windows_changes_the_next_pick() {
    drive(async {
        let now = 1_800_000_000_000;
        let pause = Flag::new();
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![Script {
                pause: Some(pause.clone()),
                events: vec![
                    AgentEvent::Usage {
                        utilization: 0.9,
                        resets_at: None,
                        windows: Some(vec![UsageWindow {
                            name: "reported".into(),
                            utilization: 0.9,
                            resets_at: Some(1_800_003_600),
                        }]),
                    },
                    AgentEvent::Result {
                        ok: true,
                        text: "done".into(),
                        session_id: Some("s".into()),
                        duration_ms: None,
                        cost_usd: Some(1.0),
                        tokens: Some(1),
                    },
                ],
                ..Script::default()
            }],
        );
        let cursor = FakeAdapter::new(ProviderId::Cursor, vec![ok("next")]);
        let mut opts = Opts::default();
        opts.clock = Some(clock_at(now));
        with_orch(
            vec![claude.clone(), cursor.clone()],
            opts,
            |orch, store| async move {
                let first = go(&orch, JobType::Planning, "first");
                wait_for(|| store.windows("claude").unwrap().len() == 1).await;
                assert_eq!(orch.get(&first.id).unwrap().status, JobStatus::Running);
                let windows = store.windows("claude").unwrap();
                assert_eq!(windows[0].name, "reported");
                assert_eq!(windows[0].utilization, 0.9);
                assert_eq!(windows[0].resets_at, Some(1_800_003_600_000));
                assert_eq!(windows[0].observed_at, now);
                let second = go(&orch, JobType::Planning, "second");
                assert_eq!(second.provider, Some(ProviderId::Cursor));
                assert_eq!(second.reason.as_deref(), Some("more headroom"));
                pause.trip();
                orch.idle().await;
                assert_eq!(cursor.prompts(), ["second"]);
                assert_eq!(claude.received.borrow().len(), 1);
                assert_eq!(
                    store.runs(Some("claude")).unwrap()[0].outcome,
                    RunOutcome::Ok
                );
                assert_eq!(
                    store.runs(Some("claude")).unwrap()[0].utilization,
                    Some(0.9)
                );
            },
        )
        .await;
    });
}

#[test]
fn reason_is_set_and_copied() {
    drive(async {
        let claude = FakeAdapter::new(ProviderId::Claude, vec![ok("a")]);
        let cursor = FakeAdapter::new(ProviderId::Cursor, vec![ok("b")]);
        with_orch(
            vec![claude, cursor],
            Opts::default(),
            |orch, _| async move {
                let snaps = Rc::new(RefCell::new(Vec::new()));
                let snaps_cb = snaps.clone();
                let _sub = orch.on_update(Rc::new(move |job| snaps_cb.borrow_mut().push(job)));
                let submitted = go(&orch, JobType::Planning, "plan");
                assert_eq!(submitted.reason.as_deref(), Some("first choice"));
                assert_eq!(orch.list()[0].reason.as_deref(), Some("first choice"));
                assert_eq!(
                    orch.get(&submitted.id).unwrap().reason.as_deref(),
                    Some("first choice")
                );
                let mut queued = snaps
                    .borrow()
                    .iter()
                    .find(|snap| snap.status == JobStatus::Queued)
                    .unwrap()
                    .clone();
                queued.reason = Some("mutated".into());
                assert_eq!(
                    orch.get(&submitted.id).unwrap().reason.as_deref(),
                    Some("first choice")
                );
                orch.idle().await;
                assert_eq!(orch.list()[0].reason.as_deref(), Some("first choice"));
            },
        )
        .await;
    });
}

#[test]
fn submit_with_a_provider_runs_on_that_plan() {
    drive(async {
        let claude = FakeAdapter::new(ProviderId::Claude, vec![ok("plan")]);
        let cursor = FakeAdapter::new(ProviderId::Cursor, vec![ok("files")]);
        with_orch(
            vec![claude.clone(), cursor.clone()],
            Opts::default(),
            |orch, _| async move {
                let job = go_on(&orch, JobType::Planning, "on cursor", ProviderId::Cursor);
                assert_eq!(job.provider, Some(ProviderId::Cursor));
                assert_eq!(job.reason.as_deref(), Some("chosen"));
                orch.idle().await;
                assert_eq!(cursor.prompts(), ["on cursor"]);
                assert!(claude.received.borrow().is_empty());
                assert_eq!(orch.list()[0].reason.as_deref(), Some("chosen"));
            },
        )
        .await;
    });
}

#[test]
fn jobs_in_a_group_keep_it() {
    drive(async {
        let claude = FakeAdapter::new(ProviderId::Claude, vec![ok("a"), ok("c")]);
        let cursor = FakeAdapter::new(ProviderId::Cursor, vec![ok("b")]);
        with_orch(
            vec![claude, cursor],
            Opts::default(),
            |orch, _| async move {
                let mut extra = SubmitExtra::default();
                extra.group = Some("cmp-1".into());
                let a = orch.submit(
                    JobType::Planning,
                    "same",
                    Some(ProviderId::Claude),
                    None,
                    extra.clone(),
                );
                let b = orch.submit(
                    JobType::Planning,
                    "same",
                    Some(ProviderId::Cursor),
                    None,
                    extra,
                );
                let plain = go_on(&orch, JobType::Planning, "alone", ProviderId::Claude);
                assert_eq!(a.group.as_deref(), Some("cmp-1"));
                assert_eq!(b.group.as_deref(), Some("cmp-1"));
                assert_eq!(plain.group, None);
                assert_eq!(
                    orch.list()
                        .iter()
                        .find(|job| job.id == plain.id)
                        .unwrap()
                        .group,
                    None
                );
                orch.idle().await;
            },
        )
        .await;
    });
}

#[test]
fn jobs_keep_a_lead_message() {
    drive(async {
        let claude = FakeAdapter::new(ProviderId::Claude, vec![ok("a"), ok("b")]);
        with_orch(vec![claude], Opts::default(), |orch, _| async move {
            let mut extra = SubmitExtra::default();
            extra.lead_message = Some("m1".into());
            let linked = orch.submit(JobType::Planning, "from lead", None, None, extra);
            let plain = go(&orch, JobType::Planning, "alone");
            assert_eq!(linked.lead_message.as_deref(), Some("m1"));
            assert_eq!(plain.lead_message, None);
            assert_eq!(
                orch.list()
                    .iter()
                    .find(|job| job.id == linked.id)
                    .unwrap()
                    .lead_message
                    .as_deref(),
                Some("m1")
            );
            orch.idle().await;
        })
        .await;
    });
}

#[test]
fn a_chosen_plan_that_is_not_signed_in_or_resting_fails() {
    drive(async {
        let now = 1_800_000_000_000;
        let claude = FakeAdapter::new(ProviderId::Claude, vec![ok("no")]);
        let cursor = FakeAdapter::new(ProviderId::Cursor, vec![ok("no")]);
        cursor.signed_in.set(false);
        let mut opts = Opts::default();
        opts.clock = Some(clock_at(now));
        with_orch(
            vec![claude.clone(), cursor.clone()],
            opts,
            |orch, store| async move {
                let unsigned = go_on(
                    &orch,
                    JobType::Planning,
                    "cursor please",
                    ProviderId::Cursor,
                );
                assert_eq!(unsigned.status, JobStatus::Failed);
                assert_eq!(unsigned.error.as_deref(), Some("cursor is not signed in"));
                assert!(cursor.received.borrow().is_empty());
                store.set_resting("claude", now + 1000).unwrap();
                let resting = go_on(
                    &orch,
                    JobType::Boilerplate,
                    "claude please",
                    ProviderId::Claude,
                );
                assert_eq!(resting.error.as_deref(), Some("claude is resting"));
                assert!(claude.received.borrow().is_empty());
                orch.idle().await;
            },
        )
        .await;
    });
}

#[test]
fn a_chosen_job_that_hits_its_limit_fails_and_a_sibling_fails_over() {
    drive(async {
        let now = 1_800_000_000_000;
        let held = Flag::new();
        let stuck = Flag::new();
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![
                Script {
                    gate: Some(held.clone()),
                    events: vec![text("partial"), limit(Some(1_900_000_000))],
                    ..Script::default()
                },
                Script {
                    gate: Some(held.clone()),
                    pause: Some(stuck),
                    events: vec![text("sibling-partial")],
                    ..Script::default()
                },
            ],
        );
        let cursor = FakeAdapter::new(ProviderId::Cursor, vec![ok("moved")]);
        let mut opts = Opts::default();
        opts.clock = Some(clock_at(now));
        with_orch(
            vec![claude.clone(), cursor.clone()],
            opts,
            |orch, store| async move {
                let chosen = go_on(&orch, JobType::Planning, "pinned", ProviderId::Claude);
                assert_eq!(chosen.reason.as_deref(), Some("chosen"));
                let sibling = go(&orch, JobType::Planning, "sibling");
                assert_eq!(sibling.status, JobStatus::Running);
                assert_eq!(sibling.reason.as_deref(), Some("first choice"));
                held.trip();
                orch.idle().await;
                let chosen = orch.get(&chosen.id).unwrap();
                assert_eq!(chosen.status, JobStatus::Failed);
                assert_eq!(chosen.error.as_deref(), Some("claude hit its usage limit"));
                assert_eq!(chosen.provider, Some(ProviderId::Claude));
                let sibling = orch.get(&sibling.id).unwrap();
                assert_eq!(sibling.status, JobStatus::Done);
                assert_eq!(sibling.provider, Some(ProviderId::Cursor));
                assert_eq!(sibling.reason.as_deref(), Some("failover"));
                assert_eq!(sibling.output, "moved");
                assert_eq!(sibling.failed_over, vec![ProviderId::Claude]);
                assert_eq!(cursor.received.borrow().len(), 1);
                assert!(cursor.prompts()[0].contains("sibling"));
                assert_eq!(claude.received.borrow().len(), 2);
                assert_eq!(
                    store.resting_until("claude").unwrap(),
                    Some(1_900_000_000_000)
                );
                assert_eq!(
                    store.runs(Some("claude")).unwrap()[0].outcome,
                    RunOutcome::Limit
                );
            },
        )
        .await;
    });
}

#[test]
fn a_chosen_job_on_a_plan_that_hits_its_limit_fails_instead_of_moving() {
    drive(async {
        let now = 1_800_000_000_000;
        let held = Flag::new();
        let stuck = Flag::new();
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![
                Script {
                    gate: Some(held.clone()),
                    events: vec![text("partial"), limit(Some(1_900_000_000))],
                    ..Script::default()
                },
                Script {
                    gate: Some(held.clone()),
                    pause: Some(stuck),
                    events: vec![text("chosen-partial")],
                    ..Script::default()
                },
            ],
        );
        let cursor = FakeAdapter::new(ProviderId::Cursor, vec![ok("moved")]);
        let mut opts = Opts::default();
        opts.clock = Some(clock_at(now));
        with_orch(vec![claude, cursor.clone()], opts, |orch, _| async move {
            let runner = go(&orch, JobType::Planning, "runner");
            let chosen = go_on(&orch, JobType::Planning, "pinned", ProviderId::Claude);
            assert_eq!(chosen.status, JobStatus::Running);
            assert_eq!(chosen.reason.as_deref(), Some("chosen"));
            held.trip();
            orch.idle().await;
            let chosen = orch.get(&chosen.id).unwrap();
            assert_eq!(chosen.status, JobStatus::Failed);
            assert_eq!(chosen.error.as_deref(), Some("claude hit its usage limit"));
            let runner = orch.get(&runner.id).unwrap();
            assert_eq!(runner.status, JobStatus::Done);
            assert_eq!(runner.provider, Some(ProviderId::Cursor));
            assert_eq!(runner.reason.as_deref(), Some("failover"));
            assert!(cursor.prompts()[0].contains("runner"));
            assert!(!cursor.prompts()[0].contains("pinned"));
        })
        .await;
    });
}

#[test]
fn workers_report_resets_at_risk_and_windows() {
    drive(async {
        let now = 1_800_000_000_000;
        let week = 168 * 3_600_000;
        let claude = FakeAdapter::new(ProviderId::Claude, vec![]);
        let mut opts = Opts::default();
        opts.clock = Some(clock_at(now));
        with_orch(vec![claude], opts, |orch, store| async move {
            store
                .save_windows(
                    "claude",
                    &[
                        legate_store::NewWindow {
                            name: "five_hour".into(),
                            utilization: 0.2,
                            resets_at: Some(now + 3_600_000),
                        },
                        legate_store::NewWindow {
                            name: "seven_day".into(),
                            utilization: 0.5,
                            resets_at: Some(now + week / 10),
                        },
                    ],
                    now,
                )
                .unwrap();
            let worker = &orch.workers()[0];
            assert_eq!(worker.resets_at, Some(now + week / 10));
            assert!(worker.at_risk);
            assert_eq!(worker.windows[0].name, "five_hour");
            assert_eq!(worker.windows[0].used, 0.2);
            assert_eq!(worker.windows[1].name, "seven_day");
            assert_eq!(worker.windows[1].used, 0.5);
        })
        .await;
    });
}

#[test]
fn an_editing_job_runs_in_its_worktree() {
    drive(async {
        let claude = FakeAdapter::new(ProviderId::Claude, vec![ok("edited")]);
        let projects = Rc::new(RefCell::new(Vec::new()));
        let seen = projects.clone();
        let mut opts = Opts::default();
        opts.workspaces = Some(trees(move |project, job_id, _| {
            seen.borrow_mut().push(project.to_string());
            Ok(slice_tree(job_id))
        }));
        with_orch(vec![claude.clone()], opts, |orch, _| async move {
            let submitted = orch.submit(
                JobType::Boilerplate,
                "change it",
                None,
                Some(SubmitAccess::Flag(true)),
                SubmitExtra::default(),
            );
            assert_eq!(submitted.edit, Some(true));
            orch.idle().await;
            let id = submitted.id.chars().take(8).collect::<String>();
            let job = orch.get(&submitted.id).unwrap();
            assert_eq!(job.status, JobStatus::Done);
            assert_eq!(job.change.as_deref(), Some(id.as_str()));
            assert_eq!(claude.cwds.borrow().as_slice(), [format!("wt/{id}")]);
            assert_eq!(claude.options.borrow()[0].edit, Some(true));
            assert_eq!(projects.borrow().as_slice(), [CWD]);
        })
        .await;
    });
}

#[test]
fn a_failed_worktree_create_fails_the_job_without_spawning() {
    drive(async {
        let claude = FakeAdapter::new(ProviderId::Claude, vec![ok("nope")]);
        let mut opts = Opts::default();
        opts.workspaces = Some(trees(|_, _, _| Err("not a git repository".into())));
        with_orch(vec![claude.clone()], opts, |orch, store| async move {
            orch.submit(
                JobType::Boilerplate,
                "change it",
                None,
                Some(SubmitAccess::Flag(true)),
                SubmitExtra::default(),
            );
            orch.idle().await;
            assert_eq!(orch.list()[0].status, JobStatus::Failed);
            assert_eq!(
                orch.list()[0].error.as_deref(),
                Some("cannot edit here: not a git repository")
            );
            assert!(claude.received.borrow().is_empty());
            assert!(store.runs(None).unwrap().is_empty());
        })
        .await;
    });
}

#[test]
fn editing_without_worktrees_fails_without_spawning() {
    drive(async {
        let claude = FakeAdapter::new(ProviderId::Claude, vec![ok("nope")]);
        with_orch(
            vec![claude.clone()],
            Opts::default(),
            |orch, store| async move {
                orch.submit(
                    JobType::Boilerplate,
                    "change it",
                    None,
                    Some(SubmitAccess::Flag(true)),
                    SubmitExtra::default(),
                );
                orch.idle().await;
                assert_eq!(
                    orch.list()[0].error.as_deref(),
                    Some("cannot edit here: no worktrees")
                );
                assert!(claude.received.borrow().is_empty());
                assert!(store.runs(None).unwrap().is_empty());
            },
        )
        .await;
    });
}

#[test]
fn a_non_editing_job_does_not_touch_worktrees() {
    drive(async {
        let claude = FakeAdapter::new(ProviderId::Claude, vec![ok("read")]);
        let creates = Rc::new(Cell::new(0));
        let changes = Rc::new(Cell::new(0));
        let creates_cb = creates.clone();
        let mut opts = Opts::default();
        opts.workspaces = Some(trees(move |_, _, _| {
            creates_cb.set(creates_cb.get() + 1);
            Err("should not create".into())
        }));
        let changes_cb = changes.clone();
        opts.on_changes = Some(Rc::new(move || changes_cb.set(changes_cb.get() + 1)));
        with_orch(vec![claude.clone()], opts, |orch, _| async move {
            go(&orch, JobType::Planning, "look");
            orch.idle().await;
            assert_eq!(creates.get(), 0);
            assert_eq!(changes.get(), 0);
            assert_eq!(claude.cwds.borrow().as_slice(), [CWD]);
            assert_eq!(claude.options.borrow()[0].edit, None);
            assert_eq!(orch.list()[0].edit, None);
            assert_eq!(orch.list()[0].change, None);
        })
        .await;
    });
}

#[test]
fn failover_of_an_editing_job_keeps_the_folder_and_adds_the_handoff_line() {
    drive(async {
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![Script {
                events: vec![text("partial"), limit(None)],
                ..Script::default()
            }],
        );
        let cursor = FakeAdapter::new(ProviderId::Cursor, vec![ok("done")]);
        let mut opts = Opts::default();
        opts.workspaces = Some(trees(|_, job_id, _| Ok(slice_tree(job_id))));
        with_orch(
            vec![claude.clone(), cursor.clone()],
            opts,
            |orch, _| async move {
                orch.submit(
                    JobType::Planning,
                    "change it",
                    None,
                    Some(SubmitAccess::Flag(true)),
                    SubmitExtra::default(),
                );
                orch.idle().await;
                assert_eq!(
                    claude.cwds.borrow().as_slice(),
                    cursor.cwds.borrow().as_slice()
                );
                assert!(claude.cwds.borrow()[0].starts_with("wt/"));
                assert!(cursor.prompts()[0].contains(
                    "The files you need are already changed in this folder; continue from them."
                ));
                assert_eq!(cursor.options.borrow()[0].edit, Some(true));
                assert_eq!(orch.list()[0].status, JobStatus::Done);
                assert_eq!(orch.list()[0].edit, Some(true));
            },
        )
        .await;
    });
}

#[test]
fn a_full_access_job_runs_in_its_worktree() {
    drive(async {
        let claude = FakeAdapter::new(ProviderId::Claude, vec![ok("ran")]);
        let mut opts = Opts::default();
        opts.workspaces = Some(trees(|_, job_id, _| Ok(slice_tree(job_id))));
        with_orch(vec![claude.clone()], opts, |orch, _| async move {
            let submitted = orch.submit(
                JobType::Boilerplate,
                "do it",
                None,
                Some(SubmitAccess::Level(JobAccess::Full)),
                SubmitExtra::default(),
            );
            assert_eq!(submitted.access, Some(JobAccess::Full));
            assert_eq!(submitted.edit, Some(true));
            orch.idle().await;
            let id = submitted.id.chars().take(8).collect::<String>();
            let job = orch.get(&submitted.id).unwrap();
            assert_eq!(job.access, Some(JobAccess::Full));
            assert_eq!(job.change.as_deref(), Some(id.as_str()));
            assert_eq!(claude.options.borrow()[0].access, Some(JobAccess::Full));
            assert_eq!(claude.options.borrow()[0].edit, Some(true));
        })
        .await;
    });
}

#[test]
fn an_old_saved_job_with_edit_true_comes_back_as_edit() {
    drive(async {
        with_orch(
            vec![FakeAdapter::new(ProviderId::Claude, vec![])],
            Opts::default(),
            |orch, store| async move {
                let mut record = bare("old-edit", JobStatus::Done, "legacy");
                record.job_type = JobType::Boilerplate;
                record.edit = Some(true);
                record.change = Some("old-edit".into());
                record.output = "ok".into();
                store.save_job("/proj", 100, &record).unwrap();
                orch.restore("/proj");
                let job = &orch.list()[0];
                assert_eq!(job.id, "old-edit");
                assert_eq!(job.access, Some(JobAccess::Edit));
                assert_eq!(job.edit, Some(true));
                assert_eq!(job.change.as_deref(), Some("old-edit"));
            },
        )
        .await;
    });
}

#[test]
fn on_changes_fires_after_an_editing_run_of_any_outcome() {
    drive(async {
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![
                ok("edited"),
                Script {
                    events: vec![AgentEvent::Result {
                        ok: false,
                        text: "nope".into(),
                        session_id: None,
                        duration_ms: None,
                        cost_usd: None,
                        tokens: None,
                    }],
                    ..Script::default()
                },
            ],
        );
        let calls = Rc::new(Cell::new(0));
        let mut opts = Opts::default();
        opts.workspaces = Some(trees(|_, job_id, _| Ok(slice_tree(job_id))));
        let calls_cb = calls.clone();
        opts.on_changes = Some(Rc::new(move || calls_cb.set(calls_cb.get() + 1)));
        with_orch(vec![claude], opts, |orch, _| async move {
            orch.submit(
                JobType::Boilerplate,
                "change it",
                Some(ProviderId::Claude),
                Some(SubmitAccess::Flag(true)),
                SubmitExtra::default(),
            );
            orch.idle().await;
            assert_eq!(calls.get(), 1);
            orch.submit(
                JobType::Boilerplate,
                "fail it",
                Some(ProviderId::Claude),
                Some(SubmitAccess::Flag(true)),
                SubmitExtra::default(),
            );
            orch.idle().await;
            assert_eq!(calls.get(), 2);
            assert_eq!(orch.list()[1].status, JobStatus::Failed);
        })
        .await;
    });
}

#[test]
fn cwd_as_a_function_is_read_when_each_job_starts() {
    drive(async {
        let folder = Rc::new(RefCell::new("alpha".to_string()));
        let claude = FakeAdapter::new(ProviderId::Claude, vec![ok("a"), ok("b")]);
        let mut opts = Opts::default();
        let folder_cb = folder.clone();
        opts.cwd = Some(CwdSource::Dynamic(Rc::new(move || {
            folder_cb.borrow().clone()
        })));
        with_orch(vec![claude.clone()], opts, |orch, _| async move {
            go(&orch, JobType::Planning, "one");
            orch.idle().await;
            *folder.borrow_mut() = "beta".into();
            go(&orch, JobType::Planning, "two");
            orch.idle().await;
            assert_eq!(claude.cwds.borrow().as_slice(), ["alpha", "beta"]);
        })
        .await;
    });
}

#[test]
fn busy_is_true_while_any_job_is_queued_or_running() {
    drive(async {
        let held = Flag::new();
        let mut first = ok("a");
        first.gate = Some(held.clone());
        let claude = FakeAdapter::new(ProviderId::Claude, vec![first, ok("b")]);
        with_orch(vec![claude], Opts::default(), |orch, _| async move {
            assert!(!orch.busy());
            go(&orch, JobType::Planning, "one");
            go(&orch, JobType::Planning, "two");
            assert!(orch.busy());
            held.trip();
            orch.idle().await;
            assert!(!orch.busy());
        })
        .await;
    });
}

#[test]
fn a_finished_job_is_listed_after_restore() {
    drive(async {
        let store = Rc::new(Store::open(":memory:").unwrap());
        let clock = clock_at(1_700_000_000_000);
        let orch = Orchestrator::new(OrchestratorOpts {
            runners: runners(&[FakeAdapter::new(
                ProviderId::Claude,
                vec![ok("saved output")],
            )]),
            store: Rc::clone(&store),
            rules: load_rules(None).unwrap(),
            cwd: CwdSource::Fixed("/proj".into()),
            clock: Some(clock.clone()),
            model_for: None,
            workspaces: None,
            on_changes: None,
            owner_id: None,
        })
        .unwrap();
        orch.init().await;
        go(&orch, JobType::Planning, "remember me");
        orch.idle().await;
        let saved = store.jobs("/proj", 10).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].status, JobStatus::Done);
        assert_eq!(saved[0].output, "saved output");
        let again = Orchestrator::new(OrchestratorOpts {
            runners: runners(&[FakeAdapter::new(ProviderId::Claude, vec![])]),
            store: Rc::clone(&store),
            rules: load_rules(None).unwrap(),
            cwd: CwdSource::Fixed("/proj".into()),
            clock: Some(clock),
            model_for: None,
            workspaces: None,
            on_changes: None,
            owner_id: None,
        })
        .unwrap();
        again.init().await;
        again.restore("/proj");
        assert_eq!(again.list().len(), 1);
        assert_eq!(again.list()[0].output, "saved output");
        assert_eq!(again.list()[0].prompt, "remember me");
    });
}

#[test]
fn a_job_stored_as_running_comes_back_failed() {
    drive(async {
        with_orch(
            vec![FakeAdapter::new(ProviderId::Claude, vec![])],
            Opts::default(),
            |orch, store| async move {
                let mut record = bare("cut-off", JobStatus::Running, "halfway");
                record.output = "partial".into();
                store.save_job("/proj", 100, &record).unwrap();
                orch.restore("/proj");
                let job = &orch.list()[0];
                assert_eq!(job.status, JobStatus::Failed);
                assert_eq!(
                    job.error.as_deref(),
                    Some("interrupted when the app closed")
                );
                assert_eq!(job.output, "partial");
                let stored = &store.jobs("/proj", 10).unwrap()[0];
                assert_eq!(stored.status, JobStatus::Failed);
                assert_eq!(
                    stored.error.as_deref(),
                    Some("interrupted when the app closed")
                );
            },
        )
        .await;
    });
}

#[test]
fn restore_for_another_project_does_not_show_its_jobs() {
    drive(async {
        with_orch(
            vec![FakeAdapter::new(ProviderId::Claude, vec![])],
            Opts::default(),
            |orch, store| async move {
                store
                    .save_job("/a", 100, &bare("only-a", JobStatus::Done, "a"))
                    .unwrap();
                orch.restore("/b");
                assert!(orch.list().is_empty());
                orch.restore("/a");
                assert_eq!(orch.list()[0].id, "only-a");
            },
        )
        .await;
    });
}

#[test]
fn switching_project_while_jobs_run_keeps_them_in_their_project() {
    drive(async {
        let held = Flag::new();
        let mut first = ok("a");
        first.gate = Some(held.clone());
        let claude = FakeAdapter::new(ProviderId::Claude, vec![first, ok("b"), ok("c")]);
        let current = Rc::new(RefCell::new("project-a".to_string()));
        let mut opts = Opts::default();
        opts.rules = Some(rules_with(1));
        let current_cb = current.clone();
        opts.cwd = Some(CwdSource::Dynamic(Rc::new(move || {
            current_cb.borrow().clone()
        })));
        with_orch(vec![claude.clone()], opts, |orch, _| async move {
            go(&orch, JobType::Planning, "one");
            go(&orch, JobType::Planning, "two");
            assert_eq!(orch.list()[0].status, JobStatus::Running);
            assert_eq!(orch.list()[1].status, JobStatus::Queued);
            *current.borrow_mut() = "project-b".into();
            orch.restore("project-b");
            assert!(orch.busy());
            assert!(orch.list().is_empty());
            go(&orch, JobType::Planning, "three");
            assert_eq!(orch.list()[0].prompt, "three");
            held.trip();
            orch.idle().await;
            assert_eq!(claude.prompts(), ["one", "two", "three"]);
            assert_eq!(
                claude.cwds.borrow().as_slice(),
                ["project-a", "project-a", "project-b"]
            );
            orch.restore("project-a");
            let mut labels: Vec<_> = orch
                .list()
                .iter()
                .map(|job| format!("{} {:?}", job.prompt, job.status))
                .collect();
            labels.sort();
            assert_eq!(labels, ["one Done", "two Done"]);
        })
        .await;
    });
}

#[test]
fn max_parallel_2_queues_the_third_until_a_slot_frees() {
    drive(async {
        let hold = Flag::new();
        let third = Flag::new();
        let mut a = ok("a");
        a.gate = Some(hold.clone());
        let mut b = ok("b");
        b.gate = Some(hold.clone());
        let mut c = ok("c");
        let third_flag = third.clone();
        c.on_start = Some(Rc::new(move || third_flag.trip()));
        let claude = FakeAdapter::new(ProviderId::Claude, vec![a, b, c]);
        let mut opts = Opts::default();
        opts.rules = Some(rules_with(2));
        with_orch(vec![claude.clone()], opts, |orch, _| async move {
            go(&orch, JobType::Planning, "first");
            go(&orch, JobType::Planning, "second");
            go(&orch, JobType::Planning, "third");
            assert_eq!(claude.prompts(), ["first", "second"]);
            assert_eq!(orch.list()[2].status, JobStatus::Queued);
            assert_eq!(orch.workers()[0].running, 2);
            assert_eq!(orch.workers()[0].queued, 1);
            hold.trip();
            third.wait().await;
            assert_eq!(claude.prompts(), ["first", "second", "third"]);
            orch.idle().await;
            assert!(orch.list().iter().all(|job| job.status == JobStatus::Done));
        })
        .await;
    });
}

#[test]
fn jobs_start_in_submission_order_when_a_slot_frees() {
    drive(async {
        let hold = Flag::new();
        let started = Rc::new(RefCell::new(Vec::new()));
        let mut scripts = Vec::new();
        for name in ["a", "b", "c", "d"] {
            let mut script = ok(name);
            if name == "a" || name == "b" {
                script.gate = Some(hold.clone());
            }
            let started_cb = started.clone();
            let label = name.to_string();
            script.on_start = Some(Rc::new(move || started_cb.borrow_mut().push(label.clone())));
            scripts.push(script);
        }
        let claude = FakeAdapter::new(ProviderId::Claude, scripts);
        let mut opts = Opts::default();
        opts.rules = Some(rules_with(2));
        with_orch(vec![claude], opts, |orch, _| async move {
            for name in ["a", "b", "c", "d"] {
                go(&orch, JobType::Planning, name);
            }
            assert_eq!(started.borrow().as_slice(), ["a", "b"]);
            hold.trip();
            orch.idle().await;
            assert_eq!(started.borrow().as_slice(), ["a", "b", "c", "d"]);
        })
        .await;
    });
}

#[test]
fn a_limit_with_two_running_jobs_fails_over_both_once() {
    drive(async {
        let now = 1_800_000_000_000;
        let held = Flag::new();
        let stuck = Flag::new();
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![
                Script {
                    gate: Some(held.clone()),
                    events: vec![text("one"), limit(Some(1_900_000_000))],
                    ..Script::default()
                },
                Script {
                    gate: Some(held.clone()),
                    pause: Some(stuck),
                    events: vec![text("two")],
                    ..Script::default()
                },
            ],
        );
        let cursor = FakeAdapter::new(ProviderId::Cursor, vec![ok("moved-a"), ok("moved-b")]);
        let mut opts = Opts::default();
        opts.clock = Some(clock_at(now));
        opts.rules = Some(rules_with(2));
        with_orch(vec![claude, cursor.clone()], opts, |orch, _| async move {
            let first = go(&orch, JobType::Planning, "alpha");
            let second = go(&orch, JobType::Planning, "beta");
            assert_eq!(orch.workers()[0].running, 2);
            held.trip();
            orch.idle().await;
            let first = orch.get(&first.id).unwrap();
            let second = orch.get(&second.id).unwrap();
            assert_eq!(first.status, JobStatus::Done);
            assert_eq!(first.provider, Some(ProviderId::Cursor));
            assert_eq!(first.failed_over, vec![ProviderId::Claude]);
            assert_eq!(second.status, JobStatus::Done);
            assert_eq!(second.provider, Some(ProviderId::Cursor));
            assert_eq!(second.failed_over, vec![ProviderId::Claude]);
            assert_eq!(cursor.received.borrow().len(), 2);
        })
        .await;
    });
}

#[test]
fn max_parallel_1_queues_a_second_job() {
    drive(async {
        let held = Flag::new();
        let second = Flag::new();
        let mut first = ok("a");
        first.gate = Some(held.clone());
        let mut next = ok("b");
        let flag = second.clone();
        next.on_start = Some(Rc::new(move || flag.trip()));
        let claude = FakeAdapter::new(ProviderId::Claude, vec![first, next]);
        let mut opts = Opts::default();
        opts.rules = Some(rules_with(1));
        with_orch(vec![claude.clone()], opts, |orch, _| async move {
            go(&orch, JobType::Planning, "one");
            go(&orch, JobType::Planning, "two");
            assert_eq!(claude.prompts(), ["one"]);
            assert_eq!(orch.list()[1].status, JobStatus::Queued);
            assert_eq!(orch.workers()[0].running, 1);
            assert_eq!(orch.workers()[0].queued, 1);
            held.trip();
            second.wait().await;
            assert_eq!(claude.prompts(), ["one", "two"]);
            orch.idle().await;
        })
        .await;
    });
}

#[test]
fn auto_routing_skips_a_plan_at_its_parallel_limit() {
    drive(async {
        let held = Flag::new();
        let mut first = ok("a");
        first.gate = Some(held.clone());
        let claude = FakeAdapter::new(ProviderId::Claude, vec![first]);
        let cursor = FakeAdapter::new(ProviderId::Cursor, vec![ok("b"), ok("c")]);
        let mut opts = Opts::default();
        opts.rules = Some(rules_with(1));
        with_orch(vec![claude, cursor.clone()], opts, |orch, _| async move {
            go(&orch, JobType::Planning, "first");
            assert_eq!(orch.list()[0].provider, Some(ProviderId::Claude));
            let second = go(&orch, JobType::Planning, "second");
            assert_eq!(second.provider, Some(ProviderId::Cursor));
            assert_eq!(second.reason.as_deref(), Some("more headroom"));
            held.trip();
            orch.idle().await;
            assert_eq!(cursor.prompts(), ["second"]);
        })
        .await;
    });
}

#[test]
fn workers_report_not_installed_and_signed_out() {
    drive(async {
        let claude = FakeAdapter::new(ProviderId::Claude, vec![]);
        let cursor = FakeAdapter::new(ProviderId::Cursor, vec![]);
        claude.installed.set(false);
        cursor.signed_in.set(false);
        with_orch(
            vec![claude, cursor],
            Opts::default(),
            |orch, _| async move {
                assert_eq!(orch.workers()[0].problem, Some(PlanProblem::NotInstalled));
                assert!(!orch.workers()[0].available);
                assert_eq!(orch.workers()[1].problem, Some(PlanProblem::SignedOut));
                assert!(!orch.workers()[1].available);
            },
        )
        .await;
    });
}

#[test]
fn recheck_picks_up_plans_without_disturbing_a_running_job() {
    drive(async {
        let held = Flag::new();
        let mut script = ok("a");
        script.gate = Some(held.clone());
        let claude = FakeAdapter::new(ProviderId::Claude, vec![script]);
        let cursor = FakeAdapter::new(ProviderId::Cursor, vec![ok("b")]);
        cursor.signed_in.set(false);
        with_orch(
            vec![claude.clone(), cursor.clone()],
            Opts::default(),
            |orch, _| async move {
                let running = go(&orch, JobType::Planning, "keep going");
                assert_eq!(running.status, JobStatus::Running);
                assert_eq!(orch.workers()[1].problem, Some(PlanProblem::SignedOut));
                claude.signed_in.set(false);
                cursor.signed_in.set(true);
                orch.recheck().await;
                assert_eq!(orch.get(&running.id).unwrap().status, JobStatus::Running);
                assert_eq!(claude.prompts(), ["keep going"]);
                assert_eq!(orch.workers()[0].problem, Some(PlanProblem::SignedOut));
                assert!(orch.workers()[1].available);
                let rejected = go_on(&orch, JobType::Planning, "needs claude", ProviderId::Claude);
                assert_eq!(rejected.error.as_deref(), Some("claude is not signed in"));
                let on_cursor = go_on(&orch, JobType::Planning, "now cursor", ProviderId::Cursor);
                assert_eq!(on_cursor.provider, Some(ProviderId::Cursor));
                held.trip();
                orch.idle().await;
                assert_eq!(orch.get(&running.id).unwrap().status, JobStatus::Done);
                assert_eq!(
                    orch.get(&on_cursor.id).unwrap().provider,
                    Some(ProviderId::Cursor)
                );
            },
        )
        .await;
    });
}

#[test]
fn submit_to_a_plan_with_a_problem_says_not_signed_in() {
    drive(async {
        let claude = FakeAdapter::new(ProviderId::Claude, vec![ok("a")]);
        let cursor = FakeAdapter::new(ProviderId::Cursor, vec![]);
        cursor.installed.set(false);
        with_orch(
            vec![claude, cursor.clone()],
            Opts::default(),
            |orch, _| async move {
                let missing = go_on(
                    &orch,
                    JobType::Planning,
                    "cursor please",
                    ProviderId::Cursor,
                );
                assert_eq!(missing.status, JobStatus::Failed);
                assert_eq!(missing.error.as_deref(), Some("cursor is not signed in"));
                assert_eq!(missing.provider, Some(ProviderId::Cursor));
                assert!(cursor.received.borrow().is_empty());
                orch.idle().await;
            },
        )
        .await;
    });
}

#[test]
fn cancel_removes_a_queued_job() {
    drive(async {
        let held = Flag::new();
        let mut first = ok("a");
        first.gate = Some(held.clone());
        let claude = FakeAdapter::new(ProviderId::Claude, vec![first]);
        let mut opts = Opts::default();
        opts.rules = Some(rules_with(1));
        with_orch(vec![claude.clone()], opts, |orch, _| async move {
            go(&orch, JobType::Planning, "first");
            let queued = go(&orch, JobType::Planning, "second");
            assert_eq!(queued.status, JobStatus::Queued);
            let stopped = orch.cancel(&queued.id).unwrap();
            assert_eq!(stopped.status, JobStatus::Failed);
            assert_eq!(stopped.error.as_deref(), Some("stopped"));
            assert_eq!(orch.workers()[0].queued, 0);
            held.trip();
            orch.idle().await;
            assert_eq!(claude.prompts(), ["first"]);
        })
        .await;
    });
}

#[test]
fn cancel_kills_a_running_job_without_failover_or_rest() {
    drive(async {
        let held = Flag::new();
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![Script {
                gate: Some(held),
                events: vec![text("partial"), limit(None)],
                ..Script::default()
            }],
        );
        let cursor = FakeAdapter::new(ProviderId::Cursor, vec![ok("failover")]);
        with_orch(
            vec![claude, cursor.clone()],
            Opts::default(),
            |orch, store| async move {
                let job = go(&orch, JobType::Planning, "stop me");
                assert_eq!(job.status, JobStatus::Running);
                orch.cancel(&job.id).unwrap();
                let id = job.id.clone();
                wait_for(|| orch.get(&id).unwrap().status == JobStatus::Failed).await;
                let job = orch.get(&id).unwrap();
                assert_eq!(job.error.as_deref(), Some("stopped"));
                assert!(job.failed_over.is_empty());
                assert_eq!(store.resting_until("claude").unwrap(), None);
                assert_eq!(store.runs(None).unwrap()[0].outcome, RunOutcome::Error);
                assert!(cursor.received.borrow().is_empty());
                orch.idle().await;
            },
        )
        .await;
    });
}

#[test]
fn cancel_of_one_running_job_starts_the_next() {
    drive(async {
        let hold = Flag::new();
        let third = Flag::new();
        let mut a = ok("a");
        a.gate = Some(hold.clone());
        let mut b = ok("b");
        b.gate = Some(hold.clone());
        let mut c = ok("c");
        let flag = third.clone();
        c.on_start = Some(Rc::new(move || flag.trip()));
        let claude = FakeAdapter::new(ProviderId::Claude, vec![a, b, c]);
        let mut opts = Opts::default();
        opts.rules = Some(rules_with(2));
        with_orch(vec![claude.clone()], opts, |orch, _| async move {
            let first = go(&orch, JobType::Planning, "one");
            let second = go(&orch, JobType::Planning, "two");
            let queued = go(&orch, JobType::Planning, "three");
            assert_eq!(second.status, JobStatus::Running);
            assert_eq!(queued.status, JobStatus::Queued);
            orch.cancel(&first.id).unwrap();
            third.wait().await;
            assert_eq!(orch.get(&queued.id).unwrap().status, JobStatus::Running);
            let id = first.id.clone();
            wait_for(|| orch.get(&id).unwrap().status == JobStatus::Failed).await;
            assert_eq!(
                orch.get(&first.id).unwrap().error.as_deref(),
                Some("stopped")
            );
            hold.trip();
            orch.idle().await;
            assert_eq!(claude.prompts(), ["one", "two", "three"]);
        })
        .await;
    });
}

#[test]
fn a_limit_event_wins_over_unreadable_output() {
    drive(async {
        let now = 1_700_000_000_000;
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![Script {
                events: vec![limit(Some(1_900_000_000))],
                unreadable: true,
                ..Script::default()
            }],
        );
        let cursor = FakeAdapter::new(ProviderId::Cursor, vec![ok("moved")]);
        let mut opts = Opts::default();
        opts.clock = Some(clock_at(now));
        with_orch(vec![claude, cursor], opts, |orch, store| async move {
            go(&orch, JobType::Planning, "go");
            orch.idle().await;
            assert_eq!(orch.list()[0].status, JobStatus::Done);
            assert_eq!(orch.list()[0].provider, Some(ProviderId::Cursor));
            assert_eq!(
                store.resting_until("claude").unwrap(),
                Some(1_900_000_000_000)
            );
            assert_eq!(
                store.runs(Some("claude")).unwrap()[0].outcome,
                RunOutcome::Limit
            );
        })
        .await;
    });
}

#[test]
fn unreadable_output_fails_without_resting() {
    drive(async {
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![Script {
                events: vec![text("??")],
                unreadable: true,
                ..Script::default()
            }],
        );
        let cursor = FakeAdapter::new(ProviderId::Cursor, vec![ok("should-not-run")]);
        with_orch(
            vec![claude.clone(), cursor.clone()],
            Opts::default(),
            |orch, store| async move {
                go(&orch, JobType::Planning, "go");
                orch.idle().await;
                let version = claude.version.take();
                claude.version.set(version.clone());
                assert_eq!(
                    orch.list()[0].error.as_deref(),
                    Some(unknown_format_message(ProviderId::Claude, version.as_deref()).as_str())
                );
                assert!(orch.list()[0].failed_over.is_empty());
                assert_eq!(store.resting_until("claude").unwrap(), None);
                assert_eq!(store.runs(None).unwrap()[0].outcome, RunOutcome::Error);
                assert!(cursor.received.borrow().is_empty());
            },
        )
        .await;
    });
}

#[test]
fn workers_report_cli_version_and_tested_status() {
    drive(async {
        let claude = FakeAdapter::new(ProviderId::Claude, vec![]);
        let cursor = FakeAdapter::new(ProviderId::Cursor, vec![]);
        claude.version.set(Some("2.2.0".into()));
        cursor.version.set(None);
        with_orch(
            vec![claude, cursor],
            Opts::default(),
            |orch, _| async move {
                assert_eq!(orch.workers()[0].version.as_deref(), Some("2.2.0"));
                assert_eq!(orch.workers()[0].version_status, VersionStatus::Newer);
                assert_eq!(orch.workers()[1].version, None);
                assert_eq!(orch.workers()[1].version_status, VersionStatus::Unknown);
            },
        )
        .await;
    });
}

#[test]
fn cancel_of_a_finished_or_unknown_job_throws() {
    drive(async {
        let claude = FakeAdapter::new(ProviderId::Claude, vec![ok("a")]);
        with_orch(vec![claude], Opts::default(), |orch, _| async move {
            let job = go(&orch, JobType::Planning, "done soon");
            orch.idle().await;
            assert_eq!(
                orch.cancel(&job.id).unwrap_err().to_string(),
                "job is not running"
            );
            assert!(matches!(
                orch.cancel("missing-id"),
                Err(OrchestratorError::NotRunning)
            ));
        })
        .await;
    });
}

#[test]
fn step_start_and_end_pair() {
    drive(async {
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![Script {
                events: vec![
                    AgentEvent::Step {
                        id: "r1".into(),
                        phase: StepPhase::Start,
                        tool: StepTool::Read,
                        title: "Read src/app.ts".into(),
                        detail: None,
                        ok: None,
                        edit: None,
                    },
                    AgentEvent::Step {
                        id: "r1".into(),
                        phase: StepPhase::End,
                        tool: StepTool::Read,
                        title: "Read src/app.ts".into(),
                        detail: Some("contents".into()),
                        ok: Some(true),
                        edit: None,
                    },
                    step_end("r2", "Read other.ts"),
                    AgentEvent::Result {
                        ok: true,
                        text: "done".into(),
                        session_id: None,
                        duration_ms: None,
                        cost_usd: None,
                        tokens: None,
                    },
                ],
                ..Script::default()
            }],
        );
        with_orch(vec![claude], Opts::default(), |orch, _| async move {
            go(&orch, JobType::Planning, "go");
            orch.idle().await;
            let steps = orch.list()[0].steps.clone().unwrap();
            assert_eq!(steps.len(), 2);
            assert_eq!(steps[0].id, "r1");
            assert_eq!(steps[0].status, JobStepStatus::Done);
            assert_eq!(steps[0].detail.as_deref(), Some("contents"));
            assert_eq!(steps[1].id, "r2");
            assert_eq!(steps[1].status, JobStepStatus::Done);
            assert_eq!(orch.list()[0].steps_dropped, Some(0));
        })
        .await;
    });
}

#[test]
fn a_failed_tool_end_marks_the_step_failed_and_a_hanging_step_follows_the_job() {
    drive(async {
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![Script {
                events: vec![
                    AgentEvent::Step {
                        id: "ok".into(),
                        phase: StepPhase::Start,
                        tool: StepTool::Read,
                        title: "Read a".into(),
                        detail: None,
                        ok: None,
                        edit: None,
                    },
                    step_end("ok", "Read a"),
                    AgentEvent::Step {
                        id: "bad".into(),
                        phase: StepPhase::Start,
                        tool: StepTool::Read,
                        title: "Read b".into(),
                        detail: None,
                        ok: None,
                        edit: None,
                    },
                    AgentEvent::Step {
                        id: "bad".into(),
                        phase: StepPhase::End,
                        tool: StepTool::Read,
                        title: "Read b".into(),
                        detail: Some("nope".into()),
                        ok: Some(false),
                        edit: None,
                    },
                    AgentEvent::Step {
                        id: "hang".into(),
                        phase: StepPhase::Start,
                        tool: StepTool::Read,
                        title: "Read c".into(),
                        detail: None,
                        ok: None,
                        edit: None,
                    },
                    AgentEvent::Result {
                        ok: false,
                        text: "boom".into(),
                        session_id: None,
                        duration_ms: None,
                        cost_usd: None,
                        tokens: None,
                    },
                ],
                ..Script::default()
            }],
        );
        with_orch(vec![claude], Opts::default(), |orch, _| async move {
            go(&orch, JobType::Planning, "go");
            orch.idle().await;
            let job = &orch.list()[0];
            assert_eq!(job.status, JobStatus::Failed);
            let labels: Vec<_> = job
                .steps
                .as_ref()
                .unwrap()
                .iter()
                .map(|step| format!("{}:{:?}", step.id, step.status))
                .collect();
            assert_eq!(labels, ["ok:Done", "bad:Failed", "hang:Failed"]);
        })
        .await;
    });
}

#[test]
fn thinking_fragments_join_one_step() {
    drive(async {
        let think = |id: &str, detail: &str| AgentEvent::Step {
            id: id.to_string(),
            phase: StepPhase::End,
            tool: StepTool::Think,
            title: detail.trim().to_string(),
            detail: Some(detail.to_string()),
            ok: None,
            edit: None,
        };
        let cursor = FakeAdapter::new(
            ProviderId::Cursor,
            vec![Script {
                events: vec![
                    think("t1", "I need to append"),
                    think("t2", " the line,"),
                    think("t3", "\nthen run git status."),
                    step_end("r1", "Read notes.txt"),
                    think("t4", "The file exists."),
                    AgentEvent::Result {
                        ok: true,
                        text: "ok".into(),
                        session_id: None,
                        duration_ms: None,
                        cost_usd: None,
                        tokens: None,
                    },
                ],
                ..Script::default()
            }],
        );
        with_orch(vec![cursor], Opts::default(), |orch, _| async move {
            go(&orch, JobType::Planning, "go");
            orch.idle().await;
            let steps = orch.list()[0].steps.clone().unwrap();
            assert_eq!(
                steps.iter().map(|step| step.tool).collect::<Vec<_>>(),
                [StepTool::Think, StepTool::Read, StepTool::Think]
            );
            assert_eq!(steps[0].id, "t1");
            assert_eq!(steps[0].title, "I need to append the line,");
            assert_eq!(
                steps[0].detail.as_deref(),
                Some("I need to append the line,\nthen run git status.")
            );
            assert_eq!(steps[2].id, "t4");
            assert_eq!(steps[2].title, "The file exists.");
        })
        .await;
    });
}

#[test]
fn a_successful_job_marks_a_hanging_step_done() {
    drive(async {
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![Script {
                events: vec![
                    AgentEvent::Step {
                        id: "hang".into(),
                        phase: StepPhase::Start,
                        tool: StepTool::Read,
                        title: "Read c".into(),
                        detail: None,
                        ok: None,
                        edit: None,
                    },
                    AgentEvent::Result {
                        ok: true,
                        text: "ok".into(),
                        session_id: None,
                        duration_ms: None,
                        cost_usd: None,
                        tokens: None,
                    },
                ],
                ..Script::default()
            }],
        );
        with_orch(vec![claude], Opts::default(), |orch, _| async move {
            go(&orch, JobType::Planning, "go");
            orch.idle().await;
            assert_eq!(
                orch.list()[0].steps.as_ref().unwrap()[0].status,
                JobStepStatus::Done
            );
        })
        .await;
    });
}

#[test]
fn keeps_the_last_400_steps() {
    drive(async {
        let mut events = Vec::new();
        for i in 0..405 {
            events.push(step_end(&format!("s{i}"), &format!("Read {i}")));
        }
        events.push(AgentEvent::Result {
            ok: true,
            text: "ok".into(),
            session_id: None,
            duration_ms: None,
            cost_usd: None,
            tokens: None,
        });
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![Script {
                events,
                ..Script::default()
            }],
        );
        with_orch(vec![claude], Opts::default(), |orch, _| async move {
            go(&orch, JobType::Planning, "go");
            orch.idle().await;
            let job = &orch.list()[0];
            let steps = job.steps.as_ref().unwrap();
            assert_eq!(steps.len(), 400);
            assert_eq!(job.steps_dropped, Some(5));
            assert_eq!(steps[0].id, "s5");
            assert_eq!(steps[399].id, "s404");
        })
        .await;
    });
}

#[test]
fn old_saved_jobs_without_steps_load_with_an_empty_list() {
    drive(async {
        with_orch(
            vec![FakeAdapter::new(ProviderId::Claude, vec![])],
            Opts::default(),
            |orch, store| async move {
                store
                    .save_job("/proj", 100, &bare("old", JobStatus::Done, "go"))
                    .unwrap();
                orch.restore("/proj");
                assert_eq!(orch.list()[0].steps.as_ref().unwrap().len(), 0);
                assert_eq!(orch.list()[0].steps_dropped, Some(0));
            },
        )
        .await;
    });
}

#[test]
fn saved_steps_come_back_after_restore() {
    drive(async {
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![Script {
                events: vec![
                    AgentEvent::Step {
                        id: "e1".into(),
                        phase: StepPhase::End,
                        tool: StepTool::Edit,
                        title: "Edited src/app.ts".into(),
                        detail: None,
                        ok: Some(true),
                        edit: Some(FileEdit {
                            path: "src/app.ts".into(),
                            added: 4,
                            removed: 1,
                        }),
                    },
                    AgentEvent::Result {
                        ok: true,
                        text: "ok".into(),
                        session_id: None,
                        duration_ms: None,
                        cost_usd: None,
                        tokens: None,
                    },
                ],
                ..Script::default()
            }],
        );
        with_orch(vec![claude], Opts::default(), |orch, store| async move {
            go(&orch, JobType::Planning, "go");
            orch.idle().await;
            let saved = &store.jobs(CWD, 10).unwrap()[0];
            let edit = saved.steps.as_ref().unwrap()[0].edit.clone().unwrap();
            assert_eq!(edit.path, "src/app.ts");
            assert_eq!(edit.added, 4);
            let again = Orchestrator::new(OrchestratorOpts {
                runners: runners(&[FakeAdapter::new(ProviderId::Claude, vec![])]),
                store: Rc::clone(&store),
                rules: load_rules(None).unwrap(),
                cwd: CwdSource::Fixed(CWD.into()),
                clock: Some(clock_at(1_700_000_000_000)),
                model_for: None,
                workspaces: None,
                on_changes: None,
                owner_id: None,
            })
            .unwrap();
            again.init().await;
            again.restore(CWD);
            let listed = again.list();
            let step = &listed[0].steps.as_ref().unwrap()[0];
            assert_eq!(step.id, "e1");
            assert_eq!(step.edit.as_ref().unwrap().removed, 1);
        })
        .await;
    });
}

#[test]
fn step_updates_are_batched() {
    drive(async {
        let mut events = Vec::new();
        for i in 0..20 {
            events.push(step_end(&format!("s{i}"), &format!("Read {i}")));
        }
        events.push(AgentEvent::Result {
            ok: true,
            text: "ok".into(),
            session_id: None,
            duration_ms: None,
            cost_usd: None,
            tokens: None,
        });
        let claude = FakeAdapter::new(
            ProviderId::Claude,
            vec![Script {
                events,
                ..Script::default()
            }],
        );
        let mut opts = Opts::default();
        opts.clock = Some(clock_at(1_700_000_000_000));
        with_orch(vec![claude], opts, |orch, _| async move {
            let snaps = Rc::new(RefCell::new(Vec::new()));
            let snaps_cb = snaps.clone();
            let _sub = orch.on_update(Rc::new(move |job| snaps_cb.borrow_mut().push(job)));
            go(&orch, JobType::Planning, "go");
            orch.idle().await;
            let with_steps = snaps
                .borrow()
                .iter()
                .filter(|job| job.steps.as_ref().is_some_and(|steps| !steps.is_empty()))
                .count();
            assert!(with_steps < 20, "{with_steps}");
            assert_eq!(orch.list()[0].steps.as_ref().unwrap().len(), 20);
        })
        .await;
    });
}

struct Pair {
    store: Rc<Store>,
    clock_a: Rc<ManualClock>,
    clock_b: Rc<ManualClock>,
    orch_a: Rc<Orchestrator>,
    orch_b: Rc<Orchestrator>,
    hold: Rc<Flag>,
}

async fn two_orchs() -> Pair {
    let store = Rc::new(Store::open(":memory:").unwrap());
    let time = Rc::new(Cell::new(1_000_000));
    let clock_a = Rc::new(ManualClock::from_cell(time.clone()));
    let clock_b = Rc::new(ManualClock::from_cell(time));
    let hold = Flag::new();
    let claude_a = FakeAdapter::new(
        ProviderId::Claude,
        vec![Script {
            gate: Some(hold.clone()),
            events: vec![
                step_end("s1", "Read a.ts"),
                AgentEvent::Result {
                    ok: true,
                    text: "from-a".into(),
                    session_id: None,
                    duration_ms: None,
                    cost_usd: Some(1.0),
                    tokens: Some(1),
                },
            ],
            ..Script::default()
        }],
    );
    let claude_b = FakeAdapter::new(ProviderId::Claude, vec![]);
    let rules = load_rules(None).unwrap();
    let orch_a = Orchestrator::new(OrchestratorOpts {
        runners: runners(std::slice::from_ref(&claude_a)),
        store: Rc::clone(&store),
        rules: rules.clone(),
        cwd: CwdSource::Fixed("/proj".into()),
        clock: Some(clock_a.clone()),
        model_for: None,
        workspaces: None,
        on_changes: None,
        owner_id: Some("owner-a".into()),
    })
    .unwrap();
    let orch_b = Orchestrator::new(OrchestratorOpts {
        runners: runners(&[claude_b]),
        store: Rc::clone(&store),
        rules,
        cwd: CwdSource::Fixed("/proj".into()),
        clock: Some(clock_b.clone()),
        model_for: None,
        workspaces: None,
        on_changes: None,
        owner_id: Some("owner-b".into()),
    })
    .unwrap();
    orch_a.init().await;
    orch_b.init().await;
    Pair {
        store,
        clock_a,
        clock_b,
        orch_a,
        orch_b,
        hold,
    }
}

#[test]
fn restore_follows_a_job_another_process_is_running() {
    drive(async {
        let pair = two_orchs().await;
        let snaps = Rc::new(RefCell::new(Vec::new()));
        let snaps_cb = snaps.clone();
        let _sub = pair
            .orch_b
            .on_update(Rc::new(move |job| snaps_cb.borrow_mut().push(job)));
        let submitted = go(&pair.orch_a, JobType::Planning, "keep going");
        assert_eq!(submitted.status, JobStatus::Running);
        assert_eq!(submitted.owner_id.as_deref(), Some("owner-a"));
        pair.orch_b.restore("/proj");
        let foreign = pair.orch_b.get(&submitted.id).unwrap();
        assert_eq!(foreign.status, JobStatus::Running);
        assert_eq!(foreign.error, None);
        assert!(!pair.orch_b.busy());
        pair.hold.trip();
        pair.orch_a.idle().await;
        pair.clock_b.tick(JOB_WATCH_MS);
        let followed = pair.orch_b.get(&submitted.id).unwrap();
        assert_eq!(followed.status, JobStatus::Done);
        assert_eq!(followed.output, "from-a");
        assert_eq!(followed.steps.as_ref().unwrap()[0].id, "s1");
        assert!(
            snaps
                .borrow()
                .iter()
                .any(|snap| snap.status == JobStatus::Done && snap.id == submitted.id)
        );
    });
}

#[test]
fn a_dead_process_is_interrupted_after_its_heartbeat_goes_stale() {
    drive(async {
        let pair = two_orchs().await;
        let submitted = go(&pair.orch_a, JobType::Planning, "ghost");
        pair.orch_b.restore("/proj");
        assert_eq!(
            pair.orch_b.get(&submitted.id).unwrap().status,
            JobStatus::Running
        );
        pair.clock_a.clear_all();
        pair.clock_b.tick(JOB_STALE_MS);
        let job = pair.orch_b.get(&submitted.id).unwrap();
        assert_eq!(job.status, JobStatus::Failed);
        assert_eq!(
            job.error.as_deref(),
            Some("interrupted when the app closed")
        );
        pair.hold.trip();
        pair.orch_a.idle().await;
    });
}

#[test]
fn cancel_of_a_foreign_job_asks_the_owner_to_stop_it() {
    drive(async {
        let pair = two_orchs().await;
        let submitted = go(&pair.orch_a, JobType::Planning, "stop me");
        pair.orch_b.restore("/proj");
        let asked = pair.orch_b.cancel(&submitted.id).unwrap();
        assert_eq!(asked.status, JobStatus::Running);
        assert_eq!(asked.cancel_requested, Some(true));
        assert!(pair.store.is_cancel_requested(&submitted.id).unwrap());
        assert_eq!(
            pair.orch_a.get(&submitted.id).unwrap().status,
            JobStatus::Running
        );
        pair.clock_a.tick(JOB_HEARTBEAT_MS);
        let id = submitted.id.clone();
        let orch_a = pair.orch_a.clone();
        wait_for(move || orch_a.get(&id).unwrap().status == JobStatus::Failed).await;
        assert_eq!(
            pair.orch_a.get(&submitted.id).unwrap().error.as_deref(),
            Some("stopped")
        );
        pair.hold.trip();
        pair.orch_a.idle().await;
    });
}

#[test]
fn a_cursor_limit_rests_grok_and_a_grok_limit_rests_cursor() {
    drive(async {
        let now = 1_800_000_000_000;
        let resets_at = 1_900_000_000;
        let claude = FakeAdapter::new(ProviderId::Claude, vec![ok("c")]);
        let cursor = FakeAdapter::new(
            ProviderId::Cursor,
            vec![Script {
                events: vec![limit(Some(resets_at))],
                ..Script::default()
            }],
        );
        let grok = FakeAdapter::new(
            ProviderId::Grok,
            vec![Script {
                events: vec![limit(Some(resets_at + 10))],
                ..Script::default()
            }],
        );
        let mut opts = Opts::default();
        opts.clock = Some(clock_at(now));
        with_orch(
            vec![claude, cursor, grok.clone()],
            opts,
            |orch, store| async move {
                let before = orch.workers();
                let grok_room = before
                    .iter()
                    .find(|worker| worker.id == ProviderId::Grok)
                    .unwrap()
                    .headroom;
                let cursor_room = before
                    .iter()
                    .find(|worker| worker.id == ProviderId::Cursor)
                    .unwrap()
                    .headroom;
                assert_eq!(grok_room, cursor_room);
                go_on(&orch, JobType::Planning, "spend cursor", ProviderId::Cursor);
                orch.idle().await;
                assert_eq!(
                    store.resting_until("cursor").unwrap(),
                    Some(resets_at * 1000)
                );
                assert_eq!(store.resting_until("grok").unwrap(), None);
                let rested = orch.workers();
                assert_eq!(
                    rested
                        .iter()
                        .find(|worker| worker.id == ProviderId::Grok)
                        .unwrap()
                        .resting_until,
                    Some(resets_at * 1000)
                );
                let blocked = go_on(&orch, JobType::Boilerplate, "grok too", ProviderId::Grok);
                assert_eq!(blocked.error.as_deref(), Some("grok is resting"));
                assert!(grok.received.borrow().is_empty());
                store.clear_resting("cursor").unwrap();
                go_on(&orch, JobType::Boilerplate, "spend grok", ProviderId::Grok);
                orch.idle().await;
                assert_eq!(
                    store.resting_until("cursor").unwrap(),
                    Some((resets_at + 10) * 1000)
                );
                let spent = orch
                    .list()
                    .iter()
                    .find(|job| job.prompt == "spend grok")
                    .unwrap()
                    .clone();
                assert_eq!(spent.provider, Some(ProviderId::Grok));
                assert_eq!(spent.status, JobStatus::Failed);
                let cursor_too = go_on(&orch, JobType::Planning, "cursor too", ProviderId::Cursor);
                assert_eq!(cursor_too.error.as_deref(), Some("cursor is resting"));
            },
        )
        .await;
    });
}

#[test]
fn max_parallel_1_a_running_cursor_job_blocks_grok() {
    drive(async {
        let held = Flag::new();
        let grok_started = Flag::new();
        let claude = FakeAdapter::new(ProviderId::Claude, vec![ok("unused")]);
        let mut cursor_script = ok("from cursor");
        cursor_script.gate = Some(held.clone());
        let cursor = FakeAdapter::new(ProviderId::Cursor, vec![cursor_script]);
        let mut grok_script = ok("from grok");
        let flag = grok_started.clone();
        grok_script.on_start = Some(Rc::new(move || flag.trip()));
        let grok = FakeAdapter::new(ProviderId::Grok, vec![grok_script]);
        let mut opts = Opts::default();
        opts.rules = Some(rules_with(1));
        with_orch(
            vec![claude, cursor.clone(), grok.clone()],
            opts,
            |orch, _| async move {
                go_on(
                    &orch,
                    JobType::Refactor,
                    "cursor please",
                    ProviderId::Cursor,
                );
                let blocked = go_on(&orch, JobType::Refactor, "grok please", ProviderId::Grok);
                assert_eq!(blocked.status, JobStatus::Queued);
                assert_eq!(blocked.provider, Some(ProviderId::Grok));
                assert_eq!(cursor.prompts(), ["cursor please"]);
                assert!(grok.received.borrow().is_empty());
                held.trip();
                grok_started.wait().await;
                assert_eq!(grok.prompts(), ["grok please"]);
                orch.idle().await;
                assert!(orch.list().iter().all(|job| job.status == JobStatus::Done));
            },
        )
        .await;
    });
}

#[test]
fn watch_store_picks_up_a_job_started_in_another_process() {
    drive(async {
        let pair = two_orchs().await;
        pair.orch_b.restore("/proj");
        assert!(pair.orch_b.list().is_empty());
        let stop = pair.orch_b.watch_store();
        let submitted = go(&pair.orch_a, JobType::Planning, "new from a");
        assert!(pair.orch_b.list().is_empty());
        pair.clock_b.tick(JOB_WATCH_MS);
        let seen = &pair.orch_b.list()[0];
        assert_eq!(seen.id, submitted.id);
        assert_eq!(seen.status, JobStatus::Running);
        assert_eq!(seen.owner_id.as_deref(), Some("owner-a"));
        drop(stop);
        pair.hold.trip();
        pair.orch_a.idle().await;
    });
}

#[test]
fn a_long_session_keeps_at_most_200_finished_jobs() {
    drive(async {
        let claude = FakeAdapter::new(ProviderId::Claude, vec![]);
        let cursor = FakeAdapter::new(ProviderId::Cursor, vec![]);
        cursor.signed_in.set(false);
        let tick = Rc::new(Cell::new(1_800_000_000_000_i64));
        let mut opts = Opts::default();
        opts.clock = Some(Rc::new(FrozenClock::from_cell(tick.clone())));
        with_orch(vec![claude, cursor], opts, |orch, _| async move {
            let mut ids = Vec::new();
            for i in 0..205 {
                tick.set(tick.get() + 1);
                ids.push(
                    go_on(
                        &orch,
                        JobType::Planning,
                        &format!("job {i}"),
                        ProviderId::Cursor,
                    )
                    .id,
                );
            }
            let kept = orch.list();
            assert!(kept.len() <= 201);
            assert!(!kept.iter().any(|job| job.id == ids[0]));
            assert!(kept.iter().any(|job| job.id == ids[204]));
            assert!(orch.get(&ids[0]).is_none());
        })
        .await;
    });
}
