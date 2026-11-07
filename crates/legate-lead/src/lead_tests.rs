//! Ports of `src/core/lead/lead.test.ts`. `lead.gate.ts` is not here: it needs signed-in CLIs.

#![allow(clippy::too_many_lines)]

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::future::Future;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use legate_core::{
    AgentEvent, BridgeInfo, JobAccess, JobRecord, JobStatus, JobType, LeadMessage, LeadMessageKind,
    LeadProposal, LeadRole, LeadStatus, ProposalStatus, ProposedJob, ProviderId, RunOptions,
    UsageWindow, load_rules,
};
use legate_providers::ProcessExit;
use legate_router::{
    ActiveRun, BoxFuture, CwdSource, ExitStatus, FrozenClock, Orchestrator, OrchestratorOpts,
    RunJob, Runner, SubmitAccess, SubmitExtra, headroom,
};
use legate_store::{NewWindow, RunOutcome, RunRow, Store};
use tokio::sync::Notify;

use crate::LeadError;
use crate::lead::{Lead, LeadOpts, NO_PLAN, project_line};
use crate::ports::{JobRequest, JobSink, LeadAdapter, LeadSession};
use crate::prompts::LEAD_INSTRUCTIONS;

const DIR: &str = "lead-work";

fn bridge() -> BridgeInfo {
    BridgeInfo {
        url: "http://127.0.0.1:53124/mcp".into(),
        token: "bridge-token".into(),
        tools: vec![
            "list_workers".into(),
            "send_job".into(),
            "get_result".into(),
            "get_status".into(),
        ],
    }
}

fn limit_notice(provider: ProviderId) -> String {
    format!(
        "[The lead's plan ({}) hit its usage limit. Send your message again to continue on another plan.]",
        provider.as_str()
    )
}

fn drive(fut: impl Future<Output = ()> + 'static) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async move { tokio::task::LocalSet::new().run_until(fut).await });
}

struct Flag {
    open: AtomicBool,
    notify: Notify,
}

impl Flag {
    fn new() -> Arc<Self> {
        Arc::new(Self {
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
    gate: Option<Arc<Flag>>,
    exit_code: Option<i32>,
    stderr: String,
    throw_on_run: bool,
}

impl Default for Script {
    fn default() -> Self {
        Self {
            events: Vec::new(),
            gate: None,
            exit_code: Some(0),
            stderr: String::new(),
            throw_on_run: false,
        }
    }
}

fn done(session: &str, text: &str) -> Script {
    Script {
        events: vec![
            init(session, None),
            AgentEvent::Usage {
                utilization: 0.2,
                resets_at: None,
                windows: None,
            },
            result(true, text, Some(session), 1.0, 1),
        ],
        ..Script::default()
    }
}

fn init(session: &str, model: Option<&str>) -> AgentEvent {
    AgentEvent::Init {
        session_id: session.to_string(),
        model: model.map(str::to_string),
    }
}

fn result(ok: bool, text: &str, session: Option<&str>, cost: f64, tokens: i64) -> AgentEvent {
    AgentEvent::Result {
        ok,
        text: text.to_string(),
        session_id: session.map(str::to_string),
        duration_ms: None,
        cost_usd: Some(cost),
        tokens: Some(tokens),
    }
}

fn events(events: Vec<AgentEvent>) -> Script {
    Script {
        events,
        ..Script::default()
    }
}

fn gated(gate: &Arc<Flag>, events: Vec<AgentEvent>) -> Script {
    Script {
        events,
        gate: Some(Arc::clone(gate)),
        ..Script::default()
    }
}

fn boom() -> Script {
    Script {
        throw_on_run: true,
        ..Script::default()
    }
}

#[derive(Clone, Debug)]
struct Recorded {
    job_id: String,
    prompt: String,
    cwd: String,
    opts: RunOptions,
}

struct LaneInner {
    id: ProviderId,
    scripts: Mutex<VecDeque<Script>>,
    calls: Mutex<Vec<Recorded>>,
    kills: Arc<AtomicUsize>,
    installed: AtomicBool,
    signed_in: AtomicBool,
}

#[derive(Clone)]
struct Lane(Arc<LaneInner>);

impl Lane {
    fn new(id: ProviderId, scripts: Vec<Script>) -> Self {
        Self(Arc::new(LaneInner {
            id,
            scripts: Mutex::new(VecDeque::from(scripts)),
            calls: Mutex::new(Vec::new()),
            kills: Arc::new(AtomicUsize::new(0)),
            installed: AtomicBool::new(true),
            signed_in: AtomicBool::new(true),
        }))
    }

    fn set_signed_in(&self, signed_in: bool) {
        self.0.signed_in.store(signed_in, Ordering::SeqCst);
    }

    fn calls(&self) -> Vec<Recorded> {
        self.0.calls.lock().expect("calls").clone()
    }

    fn kills(&self) -> usize {
        self.0.kills.load(Ordering::SeqCst)
    }

    fn adapter(&self) -> Arc<dyn LeadAdapter> {
        Arc::new(self.clone())
    }

    fn runner(&self) -> Rc<dyn Runner> {
        Rc::new(self.clone())
    }
}

fn take_script(
    inner: &LaneInner,
    job_id: &str,
    prompt: &str,
    cwd: &str,
    opts: &RunOptions,
) -> Result<Script, String> {
    inner.calls.lock().expect("calls").push(Recorded {
        job_id: job_id.to_string(),
        prompt: prompt.to_string(),
        cwd: cwd.to_string(),
        opts: opts.clone(),
    });
    let Some(script) = inner.scripts.lock().expect("scripts").pop_front() else {
        return Err(format!("no script left for {}", inner.id.as_str()));
    };
    if script.throw_on_run {
        return Err("adapter exploded".to_string());
    }
    Ok(script)
}

fn text_is_limit(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("usage limit") || lower.contains("limit reached") {
        return true;
    }
    let mut rest = lower.as_str();
    while let Some(at) = rest.find("rate") {
        let after = &rest[at + 4..];
        if after.starts_with("limit")
            || after.get(1..).is_some_and(|tail| tail.starts_with("limit"))
        {
            return true;
        }
        rest = &rest[at + 1..];
    }
    false
}

struct LaneSession {
    events: Vec<AgentEvent>,
    index: usize,
    gate: Option<Arc<Flag>>,
    passed_gate: bool,
    killed: Arc<Flag>,
    kills: Arc<AtomicUsize>,
    exit_code: Option<i32>,
    stderr: String,
}

#[async_trait]
impl LeadSession for LaneSession {
    async fn recv(&mut self) -> Option<AgentEvent> {
        if !self.passed_gate {
            self.passed_gate = true;
            if let Some(gate) = &self.gate {
                wait_either(gate, &self.killed).await;
            }
        }
        if self.killed.tripped() || self.index >= self.events.len() {
            return None;
        }
        let event = self.events[self.index].clone();
        self.index += 1;
        Some(event)
    }

    async fn exit_status(&mut self) -> ProcessExit {
        ProcessExit {
            code: self.exit_code,
            stderr: self.stderr.clone(),
            unreadable: false,
        }
    }

    fn kill(&self) {
        self.kills.fetch_add(1, Ordering::SeqCst);
        self.killed.trip();
    }
}

#[async_trait]
impl LeadAdapter for Lane {
    fn id(&self) -> ProviderId {
        self.0.id
    }

    async fn is_installed(&self) -> bool {
        self.0.installed.load(Ordering::SeqCst)
    }

    async fn is_signed_in(&self) -> bool {
        self.0.signed_in.load(Ordering::SeqCst)
    }

    fn run(
        &self,
        job_id: &str,
        prompt: &str,
        cwd: &Path,
        opts: &RunOptions,
    ) -> Result<Box<dyn LeadSession>, String> {
        let script = take_script(&self.0, job_id, prompt, &cwd.to_string_lossy(), opts)?;
        Ok(Box::new(LaneSession {
            events: script.events,
            index: 0,
            gate: script.gate,
            passed_gate: false,
            killed: Flag::new(),
            kills: Arc::clone(&self.0.kills),
            exit_code: script.exit_code,
            stderr: script.stderr,
        }))
    }

    fn is_limit_error(&self, stderr: &str) -> bool {
        text_is_limit(stderr)
    }
}

enum Phase {
    Gate,
    Events,
}

struct WorkerRun {
    events: Vec<AgentEvent>,
    index: usize,
    phase: Phase,
    gate: Option<Arc<Flag>>,
    killed: Arc<Flag>,
    kills: Arc<AtomicUsize>,
    exit_code: Option<i32>,
    stderr: String,
}

impl ActiveRun for WorkerRun {
    fn next_event(&mut self) -> BoxFuture<'_, Option<AgentEvent>> {
        Box::pin(async move {
            tokio::task::yield_now().await;
            loop {
                if self.killed.tripped() {
                    return None;
                }
                match self.phase {
                    Phase::Gate => {
                        if let Some(gate) = self.gate.clone() {
                            let killed = Arc::clone(&self.killed);
                            wait_either(&gate, &killed).await;
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
                        return Some(event);
                    }
                }
            }
        })
    }

    fn exit(&mut self) -> BoxFuture<'_, Result<ExitStatus, String>> {
        let code = self.exit_code;
        let stderr = self.stderr.clone();
        Box::pin(async move {
            Ok(ExitStatus {
                code,
                stderr,
                unreadable: false,
            })
        })
    }

    fn kill(&self) {
        self.kills.fetch_add(1, Ordering::SeqCst);
        self.killed.trip();
    }

    fn killer(&self) -> Rc<dyn Fn()> {
        let killed = Arc::clone(&self.killed);
        let kills = Arc::clone(&self.kills);
        Rc::new(move || {
            kills.fetch_add(1, Ordering::SeqCst);
            killed.trip();
        })
    }
}

impl Runner for Lane {
    fn id(&self) -> ProviderId {
        self.0.id
    }

    fn is_installed(&self) -> BoxFuture<'_, bool> {
        let installed = self.0.installed.load(Ordering::SeqCst);
        Box::pin(async move { installed })
    }

    fn is_signed_in(&self) -> BoxFuture<'_, bool> {
        let signed_in = self.0.signed_in.load(Ordering::SeqCst);
        Box::pin(async move { signed_in })
    }

    fn version(&self) -> BoxFuture<'_, Option<String>> {
        Box::pin(async move { None })
    }

    fn is_limit_error(&self, stderr: &str) -> bool {
        text_is_limit(stderr)
    }

    fn start(
        &self,
        job: &RunJob,
        cwd: &str,
        opts: &RunOptions,
    ) -> Result<Box<dyn ActiveRun>, String> {
        let script = take_script(&self.0, &job.id, &job.prompt, cwd, opts)?;
        Ok(Box::new(WorkerRun {
            events: script.events,
            index: 0,
            phase: Phase::Gate,
            gate: script.gate,
            killed: Flag::new(),
            kills: Arc::clone(&self.0.kills),
            exit_code: script.exit_code,
            stderr: script.stderr,
        }))
    }
}

struct OrchJobs(Rc<Orchestrator>);

impl JobSink for OrchJobs {
    fn submit(&self, request: JobRequest) -> Result<JobRecord, LeadError> {
        Ok(self.0.submit(
            request.job_type,
            &request.prompt,
            request.provider,
            request.access.map(SubmitAccess::Level),
            SubmitExtra {
                group: request.group,
                lead_message: request.lead_message,
                routine: request.routine,
                goal: request.goal,
                base: request.base,
                title: request.title,
                project: request.project,
            },
        ))
    }

    fn list(&self) -> Vec<JobRecord> {
        self.0.list()
    }

    fn get(&self, id: &str) -> Option<JobRecord> {
        self.0.get(id)
    }

    fn cancel(&self, id: &str) -> Result<(), LeadError> {
        self.0
            .cancel(id)
            .map(|_| ())
            .map_err(|err| LeadError::message(err.to_string()))
    }

    fn on_update(&self, callback: Rc<dyn Fn(JobRecord)>) -> Box<dyn FnOnce()> {
        let sub = self.0.on_update(callback);
        Box::new(move || drop(sub))
    }
}

#[derive(Default)]
struct LeadEnv {
    now: Option<Rc<dyn Fn() -> i64>>,
    prefer: Option<Rc<dyn Fn() -> Option<ProviderId>>>,
    model_for: Option<Rc<dyn Fn(ProviderId) -> Option<String>>>,
    project: Option<Rc<dyn Fn() -> String>>,
}

fn at(now: i64) -> LeadEnv {
    LeadEnv {
        now: Some(Rc::new(move || now)),
        ..LeadEnv::default()
    }
}

fn prefer_id(id: ProviderId) -> LeadEnv {
    LeadEnv {
        prefer: Some(Rc::new(move || Some(id))),
        ..LeadEnv::default()
    }
}

fn project_of(project: impl Fn() -> String + 'static) -> LeadEnv {
    LeadEnv {
        project: Some(Rc::new(project)),
        ..LeadEnv::default()
    }
}

fn seed(store: &Store, provider: ProviderId, utilization: f64, started: i64) {
    store
        .save_windows(
            provider.as_str(),
            &[NewWindow {
                name: "reported".into(),
                utilization,
                resets_at: Some(started + 3_600_000),
            }],
            started,
        )
        .unwrap();
}

fn lead_runs(store: &Store) -> Vec<RunRow> {
    store
        .runs(None)
        .unwrap()
        .into_iter()
        .filter(|row| row.job_type == "lead")
        .collect()
}

fn memory() -> Rc<Store> {
    Rc::new(Store::open(":memory:").unwrap())
}

fn open_lead(lanes: &[Lane], store: &Rc<Store>, env: LeadEnv) -> Rc<Lead> {
    let project = env
        .project
        .unwrap_or_else(|| Rc::new(|| "/project".to_string()));
    Lead::new(LeadOpts {
        adapters: lanes.iter().map(Lane::adapter).collect(),
        store: Rc::clone(store),
        rules: load_rules(None).unwrap(),
        bridge: Arc::new(bridge()),
        dir: DIR.to_string(),
        project,
        prefer: env.prefer,
        now: env.now,
        model_for: env.model_for,
    })
}

async fn with_lead<F, Fut>(lanes: &[Lane], env: LeadEnv, body: F)
where
    F: FnOnce(Rc<Lead>, Rc<Store>) -> Fut,
    Fut: Future<Output = ()>,
{
    let store = memory();
    let lead = open_lead(lanes, &store, env);
    lead.init().await.unwrap();
    body(lead, store).await;
}

async fn with_orch<F, Fut>(cursor: &Lane, claude: &Lane, body: F)
where
    F: FnOnce(Rc<Lead>, Rc<Orchestrator>, Rc<Store>) -> Fut,
    Fut: Future<Output = ()>,
{
    let store = memory();
    let rules = load_rules(None).unwrap();
    let orch = Orchestrator::new(OrchestratorOpts {
        runners: vec![cursor.runner(), claude.runner()],
        store: Rc::clone(&store),
        rules: rules.clone(),
        cwd: CwdSource::Fixed("/project".to_string()),
        clock: Some(Rc::new(FrozenClock::new(1_700_000_000_000))),
        model_for: None,
        workspaces: None,
        on_changes: None,
        owner_id: Some("lead-test".to_string()),
    })
    .unwrap();
    orch.init().await;
    let lead = open_lead(
        &[cursor.clone(), claude.clone()],
        &store,
        LeadEnv::default(),
    );
    lead.init().await.unwrap();
    lead.attach_jobs(Rc::new(OrchJobs(Rc::clone(&orch))));
    body(lead, orch, store).await;
}

async fn parked(lead: &Lead) {
    for _ in 0..100 {
        if lead.current_turn().is_some() {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("lead turn did not start");
}

fn spawn_send(
    lead: &Rc<Lead>,
    text: &'static str,
) -> tokio::task::JoinHandle<Result<LeadMessage, LeadError>> {
    let lead = Rc::clone(lead);
    tokio::task::spawn_local(async move { lead.send(text).await })
}

async fn quiet(lead: &Lead) {
    tokio::time::timeout(Duration::from_secs(5), lead.when_quiet())
        .await
        .expect("when_quiet timed out");
}

async fn settle(orch: &Orchestrator) {
    tokio::time::timeout(Duration::from_secs(5), orch.idle())
        .await
        .expect("idle timed out");
}

fn contains_err(err: LeadError, needle: &str) {
    let message = err.to_string();
    assert!(message.contains(needle), "{message}");
}

fn restored(lead_message: &str) -> JobRecord {
    JobRecord {
        id: "job-old".into(),
        job_type: JobType::Planning,
        prompt: "stale".into(),
        provider: Some(ProviderId::Cursor),
        status: JobStatus::Failed,
        output: String::new(),
        failed_over: Vec::new(),
        steps: None,
        steps_dropped: None,
        model: None,
        reason: None,
        edit: None,
        access: None,
        change: None,
        group: None,
        lead_message: Some(lead_message.to_string()),
        routine: None,
        title: None,
        session_id: None,
        goal: None,
        base: None,
        error: Some("interrupted when the app closed".into()),
        owner_id: None,
        heartbeat_at: None,
        cancel_requested: None,
        extra: Default::default(),
    }
}

fn job(title: &str, prompt: &str) -> ProposedJob {
    ProposedJob {
        title: title.to_string(),
        job_type: JobType::Planning,
        access: JobAccess::Read,
        worker: None,
        prompt: prompt.to_string(),
    }
}

#[test]
fn instructions_tell_the_lead_when_a_worker_may_edit_and_not_to_wait_on_jobs() {
    assert!(LEAD_INSTRUCTIONS.contains(
        "Access is read (files only, the default), edit (change files in a separate worktree the person reviews and merges) or full (change files and run commands there); it cannot go above the ceiling the person set (see list_workers)."
    ));
    assert!(LEAD_INSTRUCTIONS.contains("Do not wait for results"));
    assert!(LEAD_INSTRUCTIONS.contains("propose_jobs"));
    assert!(LEAD_INSTRUCTIONS.contains("Nothing runs until the person presses Run"));
    assert!(!LEAD_INSTRUCTIONS.contains("send_job"));
    assert!(LEAD_INSTRUCTIONS.contains(
        "Every job runs in the open project named at the top of each message. If the person asks for work in a different project or folder, do not send a job: tell them in one line to open that project in the sidebar first, then ask again. If a job fails, report its error as it is and do not guess at causes you have not checked."
    ));
    assert!(!LEAD_INSTRUCTIONS.contains("read-only for now"));
}

#[test]
fn picks_the_provider_with_more_headroom() {
    let now = 1_700_000_000_000;
    drive(async move {
        let claude = Lane::new(ProviderId::Claude, vec![done("c", "ok")]);
        let cursor = Lane::new(ProviderId::Cursor, vec![done("u", "ok")]);
        let claude_lane = claude.clone();
        let cursor_lane = cursor.clone();
        with_lead(&[claude, cursor], at(now), |lead, store| async move {
            seed(&store, ProviderId::Claude, 0.2, now);
            seed(&store, ProviderId::Cursor, 0.7, now);
            let msg = lead.send("plan this").await.unwrap();
            assert_eq!(msg.provider, Some(ProviderId::Claude));
            assert_eq!(claude_lane.calls().len(), 1);
            assert_eq!(cursor_lane.calls().len(), 0);
        })
        .await;
    });
}

#[test]
fn a_headroom_tie_goes_to_cursor() {
    let now = 1_700_000_000_000;
    drive(async move {
        let claude = Lane::new(ProviderId::Claude, vec![done("c", "ok")]);
        let cursor = Lane::new(ProviderId::Cursor, vec![done("u", "ok")]);
        let claude_lane = claude.clone();
        let cursor_lane = cursor.clone();
        with_lead(&[claude, cursor], at(now), |lead, store| async move {
            seed(&store, ProviderId::Claude, 0.4, now);
            seed(&store, ProviderId::Cursor, 0.4, now);
            let msg = lead.send("plan this").await.unwrap();
            assert_eq!(msg.provider, Some(ProviderId::Cursor));
            assert_eq!(cursor_lane.calls().len(), 1);
            assert_eq!(claude_lane.calls().len(), 0);
        })
        .await;
    });
}

#[test]
fn prefer_wins_when_available_and_is_ignored_when_unavailable() {
    let now = 1_700_000_000_000;
    drive(async move {
        let preferred = Lane::new(ProviderId::Claude, vec![done("c", "ok")]);
        let other = Lane::new(ProviderId::Cursor, vec![done("u", "ok")]);
        let preferred_lane = preferred.clone();
        let other_lane = other.clone();
        let mut env = at(now);
        env.prefer = Some(Rc::new(|| Some(ProviderId::Claude)));
        with_lead(&[preferred, other], env, |lead, store| async move {
            seed(&store, ProviderId::Claude, 0.8, now);
            seed(&store, ProviderId::Cursor, 0.1, now);
            let msg = lead.send("use the preferred plan").await.unwrap();
            assert_eq!(msg.provider, Some(ProviderId::Claude));
            assert_eq!(preferred_lane.calls().len(), 1);
            assert_eq!(other_lane.calls().len(), 0);
        })
        .await;

        let unavailable = Lane::new(ProviderId::Claude, vec![done("no", "ok")]);
        unavailable.set_signed_in(false);
        let cursor = Lane::new(ProviderId::Cursor, vec![done("yes", "ok")]);
        let unavailable_lane = unavailable.clone();
        let cursor_lane = cursor.clone();
        with_lead(
            &[unavailable, cursor],
            prefer_id(ProviderId::Claude),
            |lead, _store| async move {
                let msg = lead.send("fallback").await.unwrap();
                assert_eq!(msg.provider, Some(ProviderId::Cursor));
                assert_eq!(unavailable_lane.calls().len(), 0);
                assert_eq!(cursor_lane.calls().len(), 1);
            },
        )
        .await;
    });
}

#[test]
fn a_resting_preferred_plan_is_skipped_for_one_with_headroom() {
    let now = 1_700_000_000_000;
    drive(async move {
        let claude = Lane::new(ProviderId::Claude, vec![done("c", "ok")]);
        let cursor = Lane::new(ProviderId::Cursor, vec![done("u", "ok")]);
        let claude_lane = claude.clone();
        let cursor_lane = cursor.clone();
        let mut env = at(now);
        env.prefer = Some(Rc::new(|| Some(ProviderId::Claude)));
        with_lead(&[claude, cursor], env, |lead, store| async move {
            store.set_resting("claude", now + 3_600_000).unwrap();
            let msg = lead.send("carry on").await.unwrap();
            assert_eq!(msg.provider, Some(ProviderId::Cursor));
            assert_eq!(claude_lane.calls().len(), 0);
            assert_eq!(cursor_lane.calls().len(), 1);
        })
        .await;
    });
}

#[test]
fn no_plan_with_headroom_reports_an_error_and_spawns_nothing() {
    let now = 1_700_000_000_000;
    drive(async move {
        let claude = Lane::new(ProviderId::Claude, vec![done("c", "ok")]);
        let cursor = Lane::new(ProviderId::Cursor, vec![done("u", "ok")]);
        let claude_lane = claude.clone();
        let cursor_lane = cursor.clone();
        with_lead(&[claude, cursor], at(now), |lead, store| async move {
            seed(&store, ProviderId::Claude, 1.0, now);
            seed(&store, ProviderId::Cursor, 1.0, now);
            let msg = lead.send("hello").await.unwrap();
            assert_eq!(msg.status, LeadStatus::Error);
            assert_eq!(msg.text, NO_PLAN);
            assert!(msg.provider.is_none());
            assert_eq!(claude_lane.calls().len(), 0);
            assert_eq!(cursor_lane.calls().len(), 0);
            assert!(lead_runs(&store).is_empty());
            assert!(lead.provider().is_none());
            assert_eq!(
                lead.messages()
                    .iter()
                    .map(|message| message.role)
                    .collect::<Vec<_>>(),
                vec![LeadRole::User, LeadRole::Lead]
            );
        })
        .await;
    });
}

#[test]
fn the_first_turn_sends_instructions_the_bridge_and_no_resume() {
    drive(async move {
        let cursor = Lane::new(ProviderId::Cursor, vec![done("sess-1", "started")]);
        let lane = cursor.clone();
        with_lead(&[cursor], LeadEnv::default(), |lead, _store| async move {
            let text = "plan the work";
            let msg = lead.send(text).await.unwrap();
            let calls = lane.calls();
            assert_eq!(calls.len(), 1);
            let call = &calls[0];
            assert_eq!(call.job_id, msg.id);
            assert!(call.prompt.starts_with(LEAD_INSTRUCTIONS));
            assert!(call.prompt.ends_with(text));
            assert_eq!(
                call.prompt,
                format!(
                    "{LEAD_INSTRUCTIONS}\n\n{}\n\nUser request:\n{text}",
                    project_line("/project")
                )
            );
            assert!(call.opts.resume.is_none());
            assert_eq!(call.opts.bridge, Some(bridge()));
            assert_eq!(call.cwd, DIR);
            assert_eq!(msg.status, LeadStatus::Done);
            assert_eq!(lead.provider(), Some(ProviderId::Cursor));
        })
        .await;
    });
}

#[test]
fn lead_turns_name_the_open_project_and_an_empty_path_adds_nothing() {
    assert_eq!(project_line(""), "");
    let project = "/work/fast-mcp";
    let line = format!("Open project: fast-mcp ({project})");
    assert_eq!(project_line(project), line);
    let line_for_task = line.clone();
    drive(async move {
        let cursor = Lane::new(
            ProviderId::Cursor,
            vec![done("sess-1", "first"), done("sess-1", "second")],
        );
        let lane = cursor.clone();
        with_lead(
            &[cursor],
            project_of(move || project.to_string()),
            |lead, _store| async move {
                let first = "refactor the parser";
                lead.send(first).await.unwrap();
                let opening = lane.calls()[0].prompt.clone();
                assert!(opening.contains(&line_for_task));
                assert!(opening.contains("User request:"));
                assert_eq!(
                    opening,
                    format!("{LEAD_INSTRUCTIONS}\n\n{line_for_task}\n\nUser request:\n{first}")
                );
                assert_eq!(lead.messages()[0].text, first);
                let second = "and the tests";
                lead.send(second).await.unwrap();
                let resumed = lane.calls()[1].prompt.clone();
                assert!(resumed.starts_with(&line_for_task));
                assert_eq!(resumed, format!("{line_for_task}\n\n{second}"));
                assert_eq!(lead.messages()[2].text, second);
            },
        )
        .await;

        let bare = Lane::new(ProviderId::Cursor, vec![done("s", "a"), done("s", "b")]);
        let lane = bare.clone();
        with_lead(
            &[bare],
            project_of(String::new),
            |lead, _store| async move {
                lead.send("hello").await.unwrap();
                assert_eq!(
                    lane.calls()[0].prompt,
                    format!("{LEAD_INSTRUCTIONS}\n\nUser request:\nhello")
                );
                lead.send("again").await.unwrap();
                assert_eq!(lane.calls()[1].prompt, "again");
            },
        )
        .await;
    });
}

#[test]
fn a_later_turn_resumes_the_same_provider_even_when_headroom_changes() {
    let now = 1_700_000_000_000;
    drive(async move {
        let claude = Lane::new(
            ProviderId::Claude,
            vec![
                events(vec![
                    init("sess-1", None),
                    AgentEvent::Usage {
                        utilization: 0.99,
                        resets_at: None,
                        windows: Some(vec![UsageWindow {
                            name: "reported".into(),
                            utilization: 0.99,
                            resets_at: None,
                        }]),
                    },
                    result(true, "first", Some("sess-1"), 1.0, 2),
                ]),
                events(vec![result(true, "second", Some("sess-1"), 1.0, 2)]),
            ],
        );
        let cursor = Lane::new(ProviderId::Cursor, vec![done("other", "ok")]);
        let claude_lane = claude.clone();
        let cursor_lane = cursor.clone();
        with_lead(&[claude, cursor], at(now), |lead, store| async move {
            seed(&store, ProviderId::Claude, 0.15, now);
            seed(&store, ProviderId::Cursor, 0.55, now);
            lead.send("plan the work").await.unwrap();
            let rules = load_rules(None).unwrap();
            assert!(
                headroom(ProviderId::Cursor, store.as_ref(), &rules, now).unwrap()
                    > headroom(ProviderId::Claude, store.as_ref(), &rules, now).unwrap()
            );
            let second = "what did they say";
            let msg = lead.send(second).await.unwrap();
            assert_eq!(claude_lane.calls().len(), 2);
            assert_eq!(cursor_lane.calls().len(), 0);
            assert_eq!(
                claude_lane.calls()[1].prompt,
                format!("{}\n\n{second}", project_line("/project"))
            );
            assert_eq!(
                claude_lane.calls()[1].opts.resume.as_deref(),
                Some("sess-1")
            );
            assert_eq!(claude_lane.calls()[1].opts.bridge, Some(bridge()));
            assert_eq!(claude_lane.calls()[1].cwd, DIR);
            assert_eq!(msg.provider, Some(ProviderId::Claude));
            assert_eq!(lead.provider(), Some(ProviderId::Claude));
            assert_eq!(
                lead_runs(&store)
                    .iter()
                    .map(|row| row.session_id.clone())
                    .collect::<Vec<_>>(),
                vec![Some("sess-1".to_string()), Some("sess-1".to_string())]
            );
        })
        .await;
    });
}

#[test]
fn streams_text_updates_and_messages_returns_copies_in_order() {
    drive(async move {
        let cursor = Lane::new(
            ProviderId::Cursor,
            vec![events(vec![
                AgentEvent::Text {
                    text: "alpha".into(),
                },
                AgentEvent::Text {
                    text: "beta".into(),
                },
                result(true, "ignored", Some("s"), 1.0, 1),
            ])],
        );
        with_lead(&[cursor], LeadEnv::default(), |lead, _store| async move {
            let seen = Rc::new(RefCell::new(Vec::<LeadMessage>::new()));
            let seen_cb = Rc::clone(&seen);
            let _unsub = lead.on_update(Rc::new(move |message, _chat| {
                seen_cb.borrow_mut().push(message)
            }));
            let msg = lead.send("please split this").await.unwrap();
            assert_eq!(msg.status, LeadStatus::Done);
            assert_eq!(msg.text, "alpha\nbeta");
            assert!(
                seen.borrow()
                    .iter()
                    .any(|message| message.role == LeadRole::Lead && message.text == "alpha")
            );
            assert!(
                seen.borrow()
                    .iter()
                    .any(|message| message.role == LeadRole::Lead && message.text == "alpha\nbeta")
            );
            seen.borrow_mut()
                .iter_mut()
                .find(|message| message.role == LeadRole::Lead && message.text == "alpha")
                .unwrap()
                .text = "changed".into();
            let mut messages = lead.messages();
            assert_eq!(
                messages
                    .iter()
                    .map(|message| message.role)
                    .collect::<Vec<_>>(),
                vec![LeadRole::User, LeadRole::Lead]
            );
            assert_eq!(messages[0].text, "please split this");
            assert_eq!(messages[0].status, LeadStatus::Done);
            assert_eq!(messages[1].text, "alpha\nbeta");
            assert_eq!(messages[1].status, LeadStatus::Done);
            assert_eq!(messages[1].provider, Some(ProviderId::Cursor));
            assert_ne!(messages[0].id, messages[1].id);
            messages[1].text = "mutated".into();
            assert_eq!(lead.messages()[1].text, "alpha\nbeta");
        })
        .await;
    });
}

#[test]
fn writes_one_lead_run_row_with_cost_tokens_utilization_and_session_id() {
    let now = 1_700_000_000_000;
    drive(async move {
        let cursor = Lane::new(
            ProviderId::Cursor,
            vec![events(vec![
                init("sess-1", None),
                AgentEvent::Usage {
                    utilization: 0.33,
                    resets_at: None,
                    windows: None,
                },
                result(true, "from-result", Some("sess-1"), 1.25, 40),
            ])],
        );
        with_lead(&[cursor], at(now), |lead, store| async move {
            let msg = lead.send("question").await.unwrap();
            assert_eq!(msg.text, "from-result");
            let runs = lead_runs(&store);
            assert_eq!(runs.len(), 1);
            let row = &runs[0];
            assert_eq!(row.job_id, msg.id);
            assert_eq!(row.provider, "cursor");
            assert_eq!(row.job_type, "lead");
            assert_eq!(row.started_at, now);
            assert_eq!(row.cost_usd, Some(1.25));
            assert_eq!(row.tokens, Some(40));
            assert_eq!(row.utilization, Some(0.33));
            assert_eq!(row.outcome, RunOutcome::Ok);
            assert_eq!(row.session_id.as_deref(), Some("sess-1"));
            assert!(row.duration_ms.unwrap_or(-1) >= 0);
        })
        .await;
    });
}

#[test]
fn a_limit_event_rests_the_provider_and_the_next_turn_switches_plan() {
    let now = 1_700_000_000_000;
    let resets_at = 1_800_000_000;
    drive(async move {
        let claude = Lane::new(
            ProviderId::Claude,
            vec![events(vec![
                init("old", None),
                AgentEvent::Text {
                    text: "partial".into(),
                },
                AgentEvent::Usage {
                    utilization: 0.4,
                    resets_at: None,
                    windows: None,
                },
                AgentEvent::Limit {
                    resets_at: Some(resets_at),
                    message: "spent".into(),
                },
            ])],
        );
        let cursor = Lane::new(ProviderId::Cursor, vec![done("fresh", "continued")]);
        let claude_lane = claude.clone();
        let cursor_lane = cursor.clone();
        with_lead(&[claude, cursor], at(now), |lead, store| async move {
            seed(&store, ProviderId::Claude, 0.1, now);
            seed(&store, ProviderId::Cursor, 0.6, now);
            let first = lead.send("do it").await.unwrap();
            assert_eq!(first.status, LeadStatus::Error);
            assert_eq!(
                first.text,
                format!("partial\n{}", limit_notice(ProviderId::Claude))
            );
            assert_eq!(first.provider, Some(ProviderId::Claude));
            assert_eq!(
                store.resting_until("claude").unwrap(),
                Some(resets_at * 1000)
            );
            assert_eq!(claude_lane.kills(), 1);
            assert!(lead.provider().is_none());
            let row = &lead_runs(&store)[0];
            assert_eq!(row.outcome, RunOutcome::Limit);
            assert_eq!(row.provider, "claude");
            assert_eq!(row.utilization, Some(0.4));
            assert_eq!(row.job_id, first.id);

            let second = lead.send("again").await.unwrap();
            assert_eq!(cursor_lane.calls().len(), 1);
            assert_eq!(claude_lane.calls().len(), 1);
            assert!(cursor_lane.calls()[0].opts.resume.is_none());
            assert_eq!(
                cursor_lane.calls()[0].prompt,
                format!(
                    "{LEAD_INSTRUCTIONS}\n\n{}\n\nUser request:\nagain",
                    project_line("/project")
                )
            );
            assert_eq!(cursor_lane.calls()[0].opts.bridge, Some(bridge()));
            assert_eq!(second.status, LeadStatus::Done);
            assert_eq!(second.provider, Some(ProviderId::Cursor));
            assert_eq!(second.text, "continued");
            assert_eq!(
                lead_runs(&store)
                    .iter()
                    .map(|row| row.outcome)
                    .collect::<Vec<_>>(),
                vec![RunOutcome::Limit, RunOutcome::Ok]
            );
        })
        .await;
    });
}

#[test]
fn a_limit_on_stderr_uses_the_default_rest_hours() {
    let now = 5_000_000;
    drive(async move {
        let claude = Lane::new(
            ProviderId::Claude,
            vec![Script {
                events: Vec::new(),
                exit_code: Some(1),
                stderr: "usage limit reached".into(),
                ..Script::default()
            }],
        );
        let lane = claude.clone();
        with_lead(&[claude], at(now), |lead, store| async move {
            let msg = lead.send("do it").await.unwrap();
            assert_eq!(msg.status, LeadStatus::Error);
            assert_eq!(msg.text, limit_notice(ProviderId::Claude));
            assert_eq!(lane.kills(), 1);
            assert!(lead.provider().is_none());
            let hours = load_rules(None).unwrap().default_rest_hours;
            assert_eq!(
                store.resting_until("claude").unwrap(),
                Some(now + (hours * 3_600_000.0) as i64)
            );
            assert_eq!(lead_runs(&store)[0].outcome, RunOutcome::Limit);
            assert_eq!(lead_runs(&store)[0].provider, "claude");
        })
        .await;
    });
}

#[test]
fn a_failed_result_and_a_stderr_tail_become_errors() {
    drive(async move {
        let stderr = format!(" {}END ", "b".repeat(600));
        let claude = Lane::new(
            ProviderId::Claude,
            vec![
                Script {
                    events: vec![result(false, "bad result", None, 2.0, 3)],
                    exit_code: Some(1),
                    stderr: "stderr message".into(),
                    ..Script::default()
                },
                Script {
                    events: Vec::new(),
                    exit_code: Some(1),
                    stderr: stderr.clone(),
                    ..Script::default()
                },
            ],
        );
        with_lead(&[claude], LeadEnv::default(), |lead, store| async move {
            let first = lead.send("one").await.unwrap();
            let second = lead.send("two").await.unwrap();
            assert_eq!(first.status, LeadStatus::Error);
            assert_eq!(first.text, "bad result");
            let trimmed = stderr.trim();
            let tail: String = trimmed
                .chars()
                .skip(trimmed.chars().count() - 500)
                .collect();
            assert_eq!(second.status, LeadStatus::Error);
            assert_eq!(second.text, tail);
            assert!(second.text.ends_with("END"));
            assert_eq!(
                lead_runs(&store)
                    .iter()
                    .map(|row| row.outcome)
                    .collect::<Vec<_>>(),
                vec![RunOutcome::Error, RunOutcome::Error]
            );
            assert_eq!(lead_runs(&store)[0].cost_usd, Some(2.0));
            assert_eq!(lead_runs(&store)[0].tokens, Some(3));
            assert_eq!(store.resting_until("claude").unwrap(), None);
        })
        .await;
    });
}

#[test]
fn a_thrown_adapter_becomes_an_error_and_the_next_send_works() {
    drive(async move {
        let claude = Lane::new(
            ProviderId::Claude,
            vec![
                boom(),
                events(vec![result(true, "recovered", Some("s"), 1.0, 1)]),
            ],
        );
        let lane = claude.clone();
        with_lead(&[claude], LeadEnv::default(), |lead, store| async move {
            let failed = lead.send("boom").await.unwrap();
            assert_eq!(failed.status, LeadStatus::Error);
            assert_eq!(failed.text, "adapter exploded");
            assert_eq!(
                lead_runs(&store)
                    .iter()
                    .map(|row| row.outcome)
                    .collect::<Vec<_>>(),
                vec![RunOutcome::Error]
            );
            let next = lead.send("next").await.unwrap();
            assert_eq!(next.status, LeadStatus::Done);
            assert_eq!(next.text, "recovered");
            assert!(lane.calls()[1].opts.resume.is_none());
            assert!(lane.calls()[1].prompt.starts_with(LEAD_INSTRUCTIONS));
            assert_eq!(
                lead_runs(&store)
                    .iter()
                    .map(|row| row.outcome)
                    .collect::<Vec<_>>(),
                vec![RunOutcome::Error, RunOutcome::Ok]
            );
        })
        .await;
    });
}

#[test]
fn current_turn_is_the_lead_message_id_while_streaming_and_null_otherwise() {
    drive(async move {
        let held = Flag::new();
        let cursor = Lane::new(
            ProviderId::Cursor,
            vec![gated(
                &held,
                vec![init("s1", None), result(true, "ok", Some("s1"), 1.0, 1)],
            )],
        );
        with_lead(&[cursor], LeadEnv::default(), |lead, _store| async move {
            assert!(lead.current_turn().is_none());
            let pending = spawn_send(&lead, "hello");
            parked(&lead).await;
            let during = lead
                .messages()
                .into_iter()
                .find(|message| message.role == LeadRole::Lead)
                .unwrap();
            assert_eq!(lead.current_turn().as_deref(), Some(during.id.as_str()));
            held.trip();
            let done = pending.await.unwrap().unwrap();
            assert_eq!(done.id, during.id);
            assert!(lead.current_turn().is_none());
        })
        .await;
    });
}

#[test]
fn rejects_blank_text_and_a_send_while_busy_reset_and_unsubscribe_behave() {
    drive(async move {
        let held = Flag::new();
        let cursor = Lane::new(
            ProviderId::Cursor,
            vec![
                gated(
                    &held,
                    vec![init("s1", None), result(true, "ok", Some("s1"), 1.0, 1)],
                ),
                events(vec![result(true, "after", Some("s2"), 1.0, 1)]),
            ],
        );
        let lane = cursor.clone();
        with_lead(&[cursor], LeadEnv::default(), |lead, _store| async move {
            contains_err(lead.send("").await.unwrap_err(), "empty message");
            contains_err(lead.send(" \n ").await.unwrap_err(), "empty message");
            assert!(lead.messages().is_empty());

            let seen = Rc::new(RefCell::new(Vec::<LeadMessage>::new()));
            let seen_cb = Rc::clone(&seen);
            let unsub = lead.on_update(Rc::new(move |message, _chat| {
                seen_cb.borrow_mut().push(message)
            }));
            let pending = spawn_send(&lead, "hello");
            parked(&lead).await;
            contains_err(lead.send("more").await.unwrap_err(), "lead is busy");
            contains_err(lead.reset(None).unwrap_err(), "lead is busy");
            assert_eq!(lane.calls().len(), 1);
            held.trip();
            let first = pending.await.unwrap().unwrap();
            assert_eq!(first.status, LeadStatus::Done);
            let count = seen.borrow().len();
            assert!(count > 0);
            unsub();
            lead.reset(None).unwrap();
            assert!(lead.messages().is_empty());
            assert!(lead.provider().is_none());
            let second = lead.send("next").await.unwrap();
            assert_eq!(second.status, LeadStatus::Done);
            assert_eq!(seen.borrow().len(), count);
            assert!(lane.calls()[1].opts.resume.is_none());
            assert!(lane.calls()[1].prompt.starts_with(LEAD_INSTRUCTIONS));
            assert_eq!(lead.messages().len(), 2);
        })
        .await;
    });
}

#[test]
fn model_for_reaches_the_run_with_bridge_and_resume_and_init_sets_the_message_model() {
    drive(async move {
        let chosen = Rc::new(RefCell::new("opus".to_string()));
        let cursor = Lane::new(
            ProviderId::Cursor,
            vec![
                events(vec![
                    init("sess-1", Some("claude-opus-4-8")),
                    result(true, "first", Some("sess-1"), 1.0, 1),
                ]),
                events(vec![
                    init("sess-1", Some("claude-sonnet-4-6")),
                    result(true, "second", Some("sess-1"), 1.0, 1),
                ]),
            ],
        );
        let lane = cursor.clone();
        let read = Rc::clone(&chosen);
        let env = LeadEnv {
            model_for: Some(Rc::new(move |_| Some(read.borrow().clone()))),
            ..LeadEnv::default()
        };
        with_lead(&[cursor], env, |lead, _store| async move {
            let seen = Rc::new(RefCell::new(Vec::<LeadMessage>::new()));
            let seen_cb = Rc::clone(&seen);
            let _unsub = lead.on_update(Rc::new(move |message, _chat| {
                seen_cb.borrow_mut().push(message)
            }));
            let first = lead.send("plan the work").await.unwrap();
            assert_eq!(lane.calls()[0].opts.bridge, Some(bridge()));
            assert_eq!(lane.calls()[0].opts.model.as_deref(), Some("opus"));
            assert!(lane.calls()[0].opts.resume.is_none());
            assert_eq!(first.model.as_deref(), Some("claude-opus-4-8"));
            assert_eq!(lead.messages()[1].model.as_deref(), Some("claude-opus-4-8"));
            assert!(
                seen.borrow()
                    .iter()
                    .any(|message| message.model.as_deref() == Some("claude-opus-4-8"))
            );
            let mut copied = lead.messages()[1].clone();
            copied.model = Some("mutated".into());
            assert_eq!(lead.messages()[1].model.as_deref(), Some("claude-opus-4-8"));

            *chosen.borrow_mut() = "sonnet".into();
            let second = lead.send("continue").await.unwrap();
            assert_eq!(lane.calls()[1].opts.bridge, Some(bridge()));
            assert_eq!(lane.calls()[1].opts.resume.as_deref(), Some("sess-1"));
            assert_eq!(lane.calls()[1].opts.model.as_deref(), Some("sonnet"));
            assert_eq!(second.model.as_deref(), Some("claude-sonnet-4-6"));
            assert_eq!(
                lead.messages()[3].model.as_deref(),
                Some("claude-sonnet-4-6")
            );
            assert_eq!(lead.messages()[1].model.as_deref(), Some("claude-opus-4-8"));
        })
        .await;
    });
}

#[test]
fn a_usage_event_with_windows_is_saved() {
    let now = 1_800_000_000_000;
    drive(async move {
        let cursor = Lane::new(
            ProviderId::Cursor,
            vec![events(vec![
                init("sess-1", None),
                AgentEvent::Usage {
                    utilization: 0.33,
                    resets_at: None,
                    windows: Some(vec![
                        UsageWindow {
                            name: "five_hour".into(),
                            utilization: 0.2,
                            resets_at: Some(1_800_000_050),
                        },
                        UsageWindow {
                            name: "seven_day".into(),
                            utilization: 0.33,
                            resets_at: Some(1_800_000_100),
                        },
                    ]),
                },
                result(true, "ok", Some("sess-1"), 1.0, 1),
            ])],
        );
        with_lead(&[cursor], at(now), |lead, store| async move {
            lead.send("hi").await.unwrap();
            let windows = store.windows("cursor").unwrap();
            assert_eq!(windows.len(), 2);
            assert_eq!(windows[0].name, "five_hour");
            assert_eq!(windows[0].utilization, 0.2);
            assert_eq!(windows[0].resets_at, Some(1_800_000_050_000));
            assert_eq!(windows[0].observed_at, now);
            assert_eq!(windows[1].name, "seven_day");
            assert_eq!(windows[1].utilization, 0.33);
            assert_eq!(windows[1].resets_at, Some(1_800_000_100_000));
            assert_eq!(windows[1].observed_at, now);
            assert_eq!(lead_runs(&store)[0].utilization, Some(0.33));
        })
        .await;
    });
}

#[test]
fn a_function_prefer_is_read_each_turn_and_a_change_drops_the_session() {
    drive(async move {
        let prefer = Rc::new(Cell::new(ProviderId::Cursor));
        let reads = Rc::new(Cell::new(0i32));
        let claude = Lane::new(ProviderId::Claude, vec![done("c-sess", "from claude")]);
        let cursor = Lane::new(ProviderId::Cursor, vec![done("u-sess", "from cursor")]);
        let claude_lane = claude.clone();
        let cursor_lane = cursor.clone();
        let prefer_read = Rc::clone(&prefer);
        let reads_read = Rc::clone(&reads);
        let env = LeadEnv {
            prefer: Some(Rc::new(move || {
                reads_read.set(reads_read.get() + 1);
                Some(prefer_read.get())
            })),
            ..LeadEnv::default()
        };
        with_lead(&[claude, cursor], env, |lead, _store| async move {
            let first = lead.send("one").await.unwrap();
            assert_eq!(first.provider, Some(ProviderId::Cursor));
            assert!(cursor_lane.calls()[0].opts.resume.is_none());
            assert!(cursor_lane.calls()[0].prompt.starts_with(LEAD_INSTRUCTIONS));
            let kept = lead.messages();
            let after_first = reads.get();
            prefer.set(ProviderId::Claude);
            let second = lead.send("two").await.unwrap();
            assert!(reads.get() > after_first);
            assert_eq!(second.provider, Some(ProviderId::Claude));
            assert_eq!(claude_lane.calls().len(), 1);
            assert!(claude_lane.calls()[0].opts.resume.is_none());
            assert_eq!(
                claude_lane.calls()[0].prompt,
                format!(
                    "{LEAD_INSTRUCTIONS}\n\n{}\n\nUser request:\ntwo",
                    project_line("/project")
                )
            );
            assert_eq!(cursor_lane.calls().len(), 1);
            assert_eq!(&lead.messages()[..2], &kept[..]);
            assert_eq!(lead.messages().len(), 4);
        })
        .await;
    });
}

#[test]
fn changing_preference_does_not_drop_the_session_when_unavailable_or_resting() {
    let now = 1_800_000_000_000;
    drive(async move {
        let prefer = Rc::new(Cell::new(ProviderId::Cursor));
        let unavailable = Lane::new(ProviderId::Claude, vec![done("no", "ok")]);
        unavailable.set_signed_in(false);
        let cursor = Lane::new(
            ProviderId::Cursor,
            vec![done("u1", "first"), done("u1", "second")],
        );
        let unavailable_lane = unavailable.clone();
        let cursor_lane = cursor.clone();
        let prefer_read = Rc::clone(&prefer);
        let prefer_in = Rc::clone(&prefer);
        let env = LeadEnv {
            prefer: Some(Rc::new(move || Some(prefer_read.get()))),
            ..LeadEnv::default()
        };
        with_lead(&[unavailable, cursor], env, |lead, _store| async move {
            lead.send("one").await.unwrap();
            prefer_in.set(ProviderId::Claude);
            lead.send("two").await.unwrap();
            assert_eq!(cursor_lane.calls().len(), 2);
            assert_eq!(cursor_lane.calls()[1].opts.resume.as_deref(), Some("u1"));
            assert_eq!(
                cursor_lane.calls()[1].prompt,
                format!("{}\n\ntwo", project_line("/project"))
            );
            assert_eq!(unavailable_lane.calls().len(), 0);
        })
        .await;

        prefer.set(ProviderId::Cursor);
        let claude = Lane::new(ProviderId::Claude, vec![done("c", "ok")]);
        let resting = Lane::new(
            ProviderId::Cursor,
            vec![done("u2", "first"), done("u2", "second")],
        );
        let claude_lane = claude.clone();
        let resting_lane = resting.clone();
        let prefer_read = Rc::clone(&prefer);
        let mut env = at(now);
        env.prefer = Some(Rc::new(move || Some(prefer_read.get())));
        with_lead(&[claude, resting], env, |lead, store| async move {
            lead.send("one").await.unwrap();
            store.set_resting("claude", now + 10_000).unwrap();
            prefer.set(ProviderId::Claude);
            lead.send("two").await.unwrap();
            assert_eq!(resting_lane.calls().len(), 2);
            assert_eq!(resting_lane.calls()[1].opts.resume.as_deref(), Some("u2"));
            assert_eq!(
                resting_lane.calls()[1].prompt,
                format!("{}\n\ntwo", project_line("/project"))
            );
            assert_eq!(claude_lane.calls().len(), 0);
        })
        .await;
    });
}

#[test]
fn messages_and_session_survive_a_new_lead_on_the_same_store_after_open_latest() {
    drive(async move {
        let store = memory();
        let first = Lane::new(ProviderId::Cursor, vec![done("sess-keep", "remembered")]);
        let lead = open_lead(&[first], &store, project_of(|| "/proj".to_string()));
        lead.init().await.unwrap();
        lead.send("keep this chat").await.unwrap();
        assert_eq!(lead.messages().len(), 2);
        assert_eq!(lead.provider(), Some(ProviderId::Cursor));

        let resumed = Lane::new(ProviderId::Cursor, vec![done("sess-keep", "again")]);
        let lane = resumed.clone();
        let again = open_lead(&[resumed], &store, project_of(|| "/proj".to_string()));
        again.init().await.unwrap();
        again.open_latest().unwrap();
        assert_eq!(
            again
                .messages()
                .iter()
                .map(|message| message.text.clone())
                .collect::<Vec<_>>(),
            vec!["keep this chat".to_string(), "remembered".to_string()]
        );
        assert_eq!(again.provider(), Some(ProviderId::Cursor));
        again.send("continue").await.unwrap();
        assert_eq!(lane.calls()[0].opts.resume.as_deref(), Some("sess-keep"));
        assert_eq!(
            lane.calls()[0].prompt,
            format!("{}\n\ncontinue", project_line("/proj"))
        );
    });
}

#[test]
fn reset_then_send_creates_a_second_chat_both_listed_newest_first() {
    drive(async move {
        let ticks = Rc::new(Cell::new(1000i64));
        let clock = Rc::clone(&ticks);
        let cursor = Lane::new(
            ProviderId::Cursor,
            vec![done("s1", "one"), done("s2", "two")],
        );
        let env = LeadEnv {
            now: Some(Rc::new(move || {
                let next = clock.get() + 1;
                clock.set(next);
                next
            })),
            ..LeadEnv::default()
        };
        with_lead(&[cursor], env, |lead, _store| async move {
            lead.send("first chat topic").await.unwrap();
            let first_id = lead.chats().unwrap()[0].id.clone();
            lead.reset(None).unwrap();
            assert!(lead.messages().is_empty());
            assert!(lead.provider().is_none());
            lead.send("second chat topic").await.unwrap();
            let listed = lead.chats().unwrap();
            assert_eq!(listed.len(), 2);
            assert_eq!(
                listed
                    .iter()
                    .map(|chat| chat.title.as_str())
                    .collect::<Vec<_>>(),
                vec!["second chat topic", "first chat topic"]
            );
            assert!(listed[0].active);
            assert!(!listed[1].active);
            assert_eq!(listed[1].id, first_id);
        })
        .await;
    });
}

#[test]
fn chats_of_another_project_are_not_listed_and_cannot_be_opened() {
    drive(async move {
        let project = Rc::new(RefCell::new("/a".to_string()));
        let read = Rc::clone(&project);
        let cursor = Lane::new(ProviderId::Cursor, vec![done("sa", "a"), done("sb", "b")]);
        let store = memory();
        let lead = open_lead(&[cursor], &store, project_of(move || read.borrow().clone()));
        lead.init().await.unwrap();
        lead.send("only in a").await.unwrap();
        let id = lead.chats().unwrap()[0].id.clone();
        *project.borrow_mut() = "/b".into();
        assert!(lead.chats().unwrap().is_empty());
        contains_err(lead.open(&id).unwrap_err(), "unknown chat");
        lead.send("only in b").await.unwrap();
        assert_eq!(
            lead.chats()
                .unwrap()
                .iter()
                .map(|chat| chat.title.clone())
                .collect::<Vec<_>>(),
            vec!["only in b".to_string()]
        );
    });
}

#[test]
fn a_turn_that_sends_two_jobs_ends_without_waiting() {
    drive(async move {
        let lead_held = Flag::new();
        let worker_held = Flag::new();
        let cursor = Lane::new(
            ProviderId::Cursor,
            vec![
                gated(
                    &lead_held,
                    vec![
                        init("lead-s", None),
                        result(true, "handed off", Some("lead-s"), 0.0, 0),
                    ],
                ),
                gated(
                    &worker_held,
                    vec![result(true, "worker-one", Some("w1"), 0.0, 0)],
                ),
                gated(
                    &worker_held,
                    vec![result(true, "worker-two", Some("w2"), 0.0, 0)],
                ),
                done("lead-s", "both finished"),
            ],
        );
        let claude = Lane::new(ProviderId::Claude, vec![]);
        claude.set_signed_in(false);
        let lane = cursor.clone();
        with_orch(&cursor, &claude, |lead, orch, _store| async move {
            let pending = spawn_send(&lead, "split this");
            parked(&lead).await;
            let turn = lead.current_turn().unwrap();
            orch.submit(
                JobType::Planning,
                "job one",
                None,
                None,
                SubmitExtra {
                    lead_message: Some(turn.clone()),
                    ..SubmitExtra::default()
                },
            );
            orch.submit(
                JobType::Review,
                "job two",
                None,
                None,
                SubmitExtra {
                    lead_message: Some(turn),
                    ..SubmitExtra::default()
                },
            );
            lead_held.trip();
            let msg = pending.await.unwrap().unwrap();
            assert_eq!(msg.status, LeadStatus::Done);
            assert!(
                orch.list()
                    .iter()
                    .any(|job| matches!(job.status, JobStatus::Running | JobStatus::Queued))
            );
            worker_held.trip();
            settle(&orch).await;
            quiet(&lead).await;
            let reports: Vec<_> = lead
                .messages()
                .into_iter()
                .filter(|message| message.kind == Some(LeadMessageKind::Report))
                .collect();
            assert_eq!(reports.len(), 1);
            assert_eq!(reports[0].status, LeadStatus::Done);
            assert_eq!(lane.calls().len(), 4);
        })
        .await;
    });
}

#[test]
fn when_both_jobs_finish_exactly_one_follow_up_runs() {
    drive(async move {
        let cursor = Lane::new(
            ProviderId::Cursor,
            vec![
                done("lead-s", "sent"),
                events(vec![result(true, "alpha output", Some("w1"), 0.0, 0)]),
                events(vec![result(true, "beta output", Some("w2"), 0.0, 0)]),
                done("lead-s", "summary"),
            ],
        );
        let claude = Lane::new(ProviderId::Claude, vec![]);
        claude.set_signed_in(false);
        let lane = cursor.clone();
        with_orch(&cursor, &claude, |lead, orch, _store| async move {
            let turn = lead.send("two jobs").await.unwrap();
            orch.submit(
                JobType::Planning,
                "a",
                None,
                None,
                SubmitExtra {
                    lead_message: Some(turn.id.clone()),
                    ..SubmitExtra::default()
                },
            );
            orch.submit(
                JobType::Planning,
                "b",
                None,
                None,
                SubmitExtra {
                    lead_message: Some(turn.id),
                    ..SubmitExtra::default()
                },
            );
            settle(&orch).await;
            quiet(&lead).await;
            let calls = lane.calls();
            assert_eq!(calls.len(), 4);
            assert!(calls[3].prompt.contains("alpha output"));
            assert!(calls[3].prompt.contains("beta output"));
            assert!(calls[3].prompt.starts_with("Worker jobs"));
            assert_eq!(calls[3].opts.resume.as_deref(), Some("lead-s"));
            assert_eq!(
                lead.messages()
                    .iter()
                    .filter(|message| message.kind == Some(LeadMessageKind::Report))
                    .count(),
                1
            );
        })
        .await;
    });
}

#[test]
fn a_failed_job_is_reported_as_failed_in_the_follow_up_prompt() {
    drive(async move {
        let cursor = Lane::new(
            ProviderId::Cursor,
            vec![
                done("s", "go"),
                Script {
                    events: vec![result(false, "bad", None, 0.0, 0)],
                    exit_code: Some(1),
                    ..Script::default()
                },
                done("s", "report"),
            ],
        );
        let claude = Lane::new(ProviderId::Claude, vec![]);
        claude.set_signed_in(false);
        let lane = cursor.clone();
        with_orch(&cursor, &claude, |lead, orch, _store| async move {
            let turn = lead.send("one job").await.unwrap();
            orch.submit(
                JobType::Debugging,
                "fail me",
                None,
                None,
                SubmitExtra {
                    lead_message: Some(turn.id),
                    ..SubmitExtra::default()
                },
            );
            settle(&orch).await;
            quiet(&lead).await;
            assert!(lane.calls()[2].prompt.contains("status failed"));
        })
        .await;
    });
}

#[test]
fn follow_up_outputs_are_cut_in_the_prompt() {
    drive(async move {
        let long = "z".repeat(5000);
        let cursor = Lane::new(
            ProviderId::Cursor,
            vec![
                done("s", "go"),
                events(vec![result(true, &long, Some("w"), 0.0, 0)]),
                done("s", "report"),
            ],
        );
        let claude = Lane::new(ProviderId::Claude, vec![]);
        claude.set_signed_in(false);
        let lane = cursor.clone();
        with_orch(&cursor, &claude, |lead, orch, _store| async move {
            let turn = lead.send("one job").await.unwrap();
            orch.submit(
                JobType::Planning,
                "long",
                None,
                None,
                SubmitExtra {
                    lead_message: Some(turn.id),
                    ..SubmitExtra::default()
                },
            );
            settle(&orch).await;
            quiet(&lead).await;
            let prompt = lane.calls()[2].prompt.clone();
            assert!(prompt.contains("cut to 4000 characters"));
            assert!(prompt.len() < long.len() + 500);
        })
        .await;
    });
}

#[test]
fn a_follow_up_waits_for_a_busy_lead() {
    drive(async move {
        let lead_held = Flag::new();
        let worker_held = Flag::new();
        let cursor = Lane::new(
            ProviderId::Cursor,
            vec![
                gated(
                    &lead_held,
                    vec![init("s1", None), result(true, "first", Some("s1"), 0.0, 0)],
                ),
                gated(
                    &worker_held,
                    vec![result(true, "job out", Some("w"), 0.0, 0)],
                ),
                done("s1", "report"),
            ],
        );
        let claude = Lane::new(ProviderId::Claude, vec![]);
        claude.set_signed_in(false);
        let lane = cursor.clone();
        with_orch(&cursor, &claude, |lead, orch, _store| async move {
            let pending = spawn_send(&lead, "hold");
            parked(&lead).await;
            let turn = lead.current_turn().unwrap();
            orch.submit(
                JobType::Planning,
                "quick",
                None,
                None,
                SubmitExtra {
                    lead_message: Some(turn),
                    ..SubmitExtra::default()
                },
            );
            worker_held.trip();
            settle(&orch).await;
            assert!(
                !lead
                    .messages()
                    .iter()
                    .any(|message| message.kind == Some(LeadMessageKind::Report))
            );
            lead_held.trip();
            pending.await.unwrap().unwrap();
            quiet(&lead).await;
            assert!(
                lead.messages()
                    .iter()
                    .any(|message| message.kind == Some(LeadMessageKind::Report))
            );
            assert_eq!(lane.calls().len(), 3);
        })
        .await;
    });
}

#[test]
fn a_follow_up_that_sends_no_jobs_causes_no_further_turns() {
    drive(async move {
        let cursor = Lane::new(
            ProviderId::Cursor,
            vec![
                done("s", "go"),
                events(vec![result(true, "done", Some("w"), 0.0, 0)]),
                done("s", "report only"),
            ],
        );
        let claude = Lane::new(ProviderId::Claude, vec![]);
        claude.set_signed_in(false);
        let lane = cursor.clone();
        with_orch(&cursor, &claude, |lead, orch, _store| async move {
            let turn = lead.send("one job").await.unwrap();
            orch.submit(
                JobType::Planning,
                "solo",
                None,
                None,
                SubmitExtra {
                    lead_message: Some(turn.id),
                    ..SubmitExtra::default()
                },
            );
            settle(&orch).await;
            quiet(&lead).await;
            assert_eq!(lane.calls().len(), 3);
        })
        .await;
    });
}

#[test]
fn a_report_is_saved_to_the_chat_that_owns_the_jobs_when_another_chat_is_open() {
    drive(async move {
        let worker_held = Flag::new();
        let cursor = Lane::new(
            ProviderId::Cursor,
            vec![
                done("s1", "chat a"),
                gated(
                    &worker_held,
                    vec![result(true, "worker", Some("w"), 0.0, 0)],
                ),
                done("s1", "report for a"),
                done("s2", "chat b reply"),
            ],
        );
        let claude = Lane::new(ProviderId::Claude, vec![]);
        claude.set_signed_in(false);
        with_orch(&cursor, &claude, |lead, orch, store| async move {
            let turn_a = lead.send("work in chat a").await.unwrap();
            let chat_a = lead
                .chats()
                .unwrap()
                .into_iter()
                .find(|chat| chat.active)
                .unwrap()
                .id;
            orch.submit(
                JobType::Planning,
                "job",
                None,
                None,
                SubmitExtra {
                    lead_message: Some(turn_a.id),
                    ..SubmitExtra::default()
                },
            );
            lead.reset(None).unwrap();
            lead.send("chat b").await.unwrap();
            assert_eq!(lead.chats().unwrap().len(), 2);
            let chat_b = lead
                .chats()
                .unwrap()
                .into_iter()
                .find(|chat| chat.active)
                .unwrap()
                .id;
            assert_ne!(chat_b, chat_a);
            worker_held.trip();
            settle(&orch).await;
            quiet(&lead).await;
            let messages_a = store.messages(&chat_a).unwrap();
            assert!(
                messages_a
                    .iter()
                    .any(|message| message.kind == Some(LeadMessageKind::Report))
            );
            assert!(
                !lead
                    .messages()
                    .iter()
                    .any(|message| message.kind == Some(LeadMessageKind::Report))
            );
        })
        .await;
    });
}

#[test]
fn restore_skips_follow_ups_for_interrupted_jobs() {
    drive(async move {
        let cursor = Lane::new(ProviderId::Cursor, vec![done("s", "first")]);
        let claude = Lane::new(ProviderId::Claude, vec![]);
        claude.set_signed_in(false);
        let lane = cursor.clone();
        with_orch(&cursor, &claude, |lead, orch, _store| async move {
            let turn = lead.send("before restart").await.unwrap();
            lead.note_restored_jobs(&[restored(&turn.id)]);
            orch.submit(
                JobType::Planning,
                "after restore",
                Some(ProviderId::Cursor),
                None,
                SubmitExtra {
                    lead_message: Some(turn.id),
                    ..SubmitExtra::default()
                },
            );
            settle(&orch).await;
            quiet(&lead).await;
            assert_eq!(lane.calls().len(), 2);
            assert!(
                !lead
                    .messages()
                    .iter()
                    .any(|message| message.kind == Some(LeadMessageKind::Report))
            );
        })
        .await;
    });
}

#[test]
fn open_while_busy_throws() {
    drive(async move {
        let held = Flag::new();
        let cursor = Lane::new(
            ProviderId::Cursor,
            vec![gated(
                &held,
                vec![init("s1", None), result(true, "ok", Some("s1"), 1.0, 1)],
            )],
        );
        with_lead(&[cursor], LeadEnv::default(), |lead, _store| async move {
            let pending = spawn_send(&lead, "hello");
            parked(&lead).await;
            let id = lead.chats().unwrap()[0].id.clone();
            contains_err(lead.open(&id).unwrap_err(), "lead is busy");
            contains_err(lead.open_latest().unwrap_err(), "lead is busy");
            held.trip();
            pending.await.unwrap().unwrap();
        })
        .await;
    });
}

#[test]
fn rename_validates_title_and_project_remove_current_chat_resets_to_empty() {
    drive(async move {
        let project = Rc::new(RefCell::new("/a".to_string()));
        let read = Rc::clone(&project);
        let cursor = Lane::new(ProviderId::Cursor, vec![done("s1", "a"), done("s2", "b")]);
        with_lead(
            &[cursor],
            project_of(move || read.borrow().clone()),
            |lead, store| async move {
                lead.send("first topic").await.unwrap();
                let id = lead.chats().unwrap()[0].id.clone();
                contains_err(lead.rename(&id, "").unwrap_err(), "invalid title");
                contains_err(lead.rename(&id, "   ").unwrap_err(), "invalid title");
                contains_err(
                    lead.rename(&id, &"x".repeat(81)).unwrap_err(),
                    "invalid title",
                );
                lead.rename(&id, "  Renamed chat  ").unwrap();
                assert_eq!(lead.chats().unwrap()[0].title, "Renamed chat");
                assert_eq!(store.chat(&id).unwrap().unwrap().title, "Renamed chat");

                *project.borrow_mut() = "/b".into();
                contains_err(lead.rename(&id, "nope").unwrap_err(), "unknown chat");
                contains_err(lead.remove(&id).unwrap_err(), "unknown chat");
                *project.borrow_mut() = "/a".into();
                lead.open(&id).unwrap();
                lead.remove(&id).unwrap();
                assert!(lead.messages().is_empty());
                assert!(lead.chats().unwrap().is_empty());
                assert!(store.chat(&id).unwrap().is_none());
            },
        )
        .await;
    });
}

#[test]
fn remove_while_its_turn_runs_throws_other_chats_stay() {
    drive(async move {
        let held = Flag::new();
        let cursor = Lane::new(
            ProviderId::Cursor,
            vec![
                gated(
                    &held,
                    vec![init("s1", None), result(true, "ok", Some("s1"), 1.0, 1)],
                ),
                done("s2", "kept"),
            ],
        );
        with_lead(&[cursor], LeadEnv::default(), |lead, store| async move {
            let pending = spawn_send(&lead, "busy chat");
            parked(&lead).await;
            let busy_id = lead.chats().unwrap()[0].id.clone();
            contains_err(lead.remove(&busy_id).unwrap_err(), "lead is busy");
            held.trip();
            pending.await.unwrap().unwrap();
            lead.reset(None).unwrap();
            lead.send("keep me").await.unwrap();
            let keep_id = lead.chats().unwrap()[0].id.clone();
            lead.open(&busy_id).unwrap();
            lead.remove(&busy_id).unwrap();
            assert!(store.chat(&busy_id).unwrap().is_none());
            assert_eq!(
                lead.chats()
                    .unwrap()
                    .iter()
                    .map(|chat| chat.id.clone())
                    .collect::<Vec<_>>(),
                vec![keep_id]
            );
        })
        .await;
    });
}

#[test]
fn a_direct_chat_talks_to_the_chosen_plan() {
    drive(async move {
        let claude = Lane::new(
            ProviderId::Claude,
            vec![done("sess-d", "hi"), done("sess-d", "again")],
        );
        let cursor = Lane::new(ProviderId::Cursor, vec![]);
        let claude_lane = claude.clone();
        let cursor_lane = cursor.clone();
        with_lead(
            &[claude, cursor],
            LeadEnv::default(),
            |lead, _store| async move {
                lead.reset(Some(ProviderId::Claude)).unwrap();
                assert_eq!(lead.mode(), Some(ProviderId::Claude));
                lead.send("what does this repo do").await.unwrap();
                lead.send("and the tests").await.unwrap();
                assert_eq!(cursor_lane.calls().len(), 0);
                let calls = claude_lane.calls();
                assert_eq!(calls[0].prompt, "what does this repo do");
                assert_eq!(calls[0].cwd, "/project");
                assert!(calls[0].opts.bridge.is_none());
                assert_eq!(calls[1].opts.resume.as_deref(), Some("sess-d"));
                let chat = lead.chats().unwrap().into_iter().next().unwrap();
                assert_eq!(chat.direct, Some(ProviderId::Claude));
                lead.reset(None).unwrap();
                assert!(lead.mode().is_none());
                lead.open(&chat.id).unwrap();
                assert_eq!(lead.mode(), Some(ProviderId::Claude));
            },
        )
        .await;
    });
}

#[test]
fn a_proposed_plan_waits_for_run() {
    drive(async move {
        let lead_held = Flag::new();
        let cursor = Lane::new(
            ProviderId::Cursor,
            vec![
                gated(
                    &lead_held,
                    vec![
                        init("lead-s", None),
                        result(true, "Plan ready", Some("lead-s"), 0.0, 0),
                    ],
                ),
                events(vec![
                    init("w1", None),
                    result(true, "one", Some("w1"), 0.0, 0),
                ]),
                events(vec![
                    init("w2", None),
                    result(true, "two", Some("w2"), 0.0, 0),
                ]),
                done("lead-s", "both done"),
            ],
        );
        let claude = Lane::new(ProviderId::Claude, vec![]);
        claude.set_signed_in(false);
        let lane = cursor.clone();
        with_orch(&cursor, &claude, |lead, orch, _store| async move {
            let pending = spawn_send(&lead, "split the router");
            parked(&lead).await;
            let turn = lead.current_turn().unwrap();
            lead.attach_proposal(
                &turn,
                LeadProposal {
                    summary: "Two jobs".into(),
                    status: ProposalStatus::Waiting,
                    jobs: vec![
                        job("Queue", "look at the queue"),
                        job("Failover", "look at failover"),
                    ],
                    job_ids: None,
                },
            )
            .unwrap();
            lead_held.trip();
            let message = pending.await.unwrap().unwrap();
            assert_eq!(
                message.proposal.as_ref().unwrap().status,
                ProposalStatus::Waiting
            );
            assert!(orch.list().is_empty());
            assert_eq!(
                lane.calls()[0].opts.read_dirs,
                Some(vec!["/project".into()])
            );

            contains_err(lead.run_proposal("nope", None).unwrap_err(), "not open");
            let started = lead
                .run_proposal(
                    &message.id,
                    Some(vec![
                        job("Queue only", "look at the queue"),
                        job("Dropped", " "),
                    ]),
                )
                .unwrap();
            assert_eq!(started.len(), 1);
            assert_eq!(started[0].title.as_deref(), Some("Queue only"));
            assert_eq!(
                started[0].lead_message.as_deref(),
                Some(message.id.as_str())
            );
            let saved = lead
                .messages()
                .into_iter()
                .find(|item| item.id == message.id)
                .unwrap();
            let proposal = saved.proposal.unwrap();
            assert_eq!(proposal.status, ProposalStatus::Running);
            assert_eq!(proposal.job_ids, Some(vec![started[0].id.clone()]));
            assert_eq!(
                proposal
                    .jobs
                    .iter()
                    .map(|job| job.title.as_str())
                    .collect::<Vec<_>>(),
                vec!["Queue only"]
            );
            contains_err(
                lead.run_proposal(&message.id, None).unwrap_err(),
                "already ran",
            );

            settle(&orch).await;
            quiet(&lead).await;
            assert_eq!(
                orch.get(&started[0].id).unwrap().session_id.as_deref(),
                Some("w1")
            );
            assert_eq!(
                lead.messages()
                    .iter()
                    .filter(|item| item.kind == Some(LeadMessageKind::Report))
                    .count(),
                1
            );
        })
        .await;
    });
}

#[test]
fn a_dismissed_plan_never_runs() {
    drive(async move {
        let cursor = Lane::new(ProviderId::Cursor, vec![done("lead-s", "plan")]);
        let claude = Lane::new(ProviderId::Claude, vec![]);
        claude.set_signed_in(false);
        with_orch(&cursor, &claude, |lead, orch, _store| async move {
            let message = lead.send("do a thing").await.unwrap();
            contains_err(lead.dismiss_proposal(&message.id).unwrap_err(), "not open");
            lead.attach_proposal(
                &message.id,
                LeadProposal {
                    summary: "s".into(),
                    status: ProposalStatus::Waiting,
                    jobs: vec![job("T", "p")],
                    job_ids: None,
                },
            )
            .unwrap();
            lead.dismiss_proposal(&message.id).unwrap();
            assert_eq!(
                lead.messages()
                    .into_iter()
                    .find(|item| item.id == message.id)
                    .unwrap()
                    .proposal
                    .unwrap()
                    .status,
                ProposalStatus::Dismissed
            );
            contains_err(
                lead.run_proposal(&message.id, None).unwrap_err(),
                "already ran or was dismissed",
            );
            assert!(orch.list().is_empty());
        })
        .await;
    });
}
