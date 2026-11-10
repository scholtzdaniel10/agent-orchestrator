use std::collections::HashMap;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use legate_core::{Platform, ProviderId, TerminalInfo, TerminalProvider, TerminalStatus};
use legate_store::{RunOutcome, RunRow, Store, TERMINAL_SCROLLBACK_CAP, TerminalRow};
use tokio::sync::{broadcast, oneshot};
use uuid::Uuid;

use crate::claims::TerminalClaims;
use crate::env::{process_env, terminal_env};
use crate::error::PtyError;
use crate::launch::{
    ChatFn, Launch, LaunchOpts, OpenOptions, ShellOptions, parse_session_id, setup_launch,
};
use crate::process::{KillTree, PtyProcess, SpawnOpts, SpawnPty, default_kill_tree, spawn_pty};
use crate::schedule::{Scheduler, thread_scheduler};
use crate::syncutil::lock;
use crate::textutil::{initial_chars, push_capped, tail_chars};
use crate::{
    FRESH_MARK, OUTPUT_FLUSH, READER_COALESCE_BYTES, RESTORE_MARK, SNAPSHOT_MAX_CHARS,
    WRITE_MAX_CHARS, host_platform,
};

const STORE_FLUSH: Duration = Duration::from_millis(2_000);
const CREATE_CHAT: Duration = Duration::from_millis(10_000);
const CLOSE_CHANGE: Duration = Duration::from_millis(5_000);
const PROBATION_MS: i64 = 20_000;
const DEFAULT_COLS: u16 = 100;
const DEFAULT_ROWS: u16 = 30;
const DEFAULT_MAX: usize = crate::DEFAULT_MAX_TERMINALS;

/// Future returned by [`PtyHost::close_change`]. The matching processes are signalled
/// before this is polled; awaiting it waits until they exit or five seconds pass.
pub type CloseWait = Pin<Box<dyn Future<Output = ()> + Send>>;

pub type LaunchFn = Arc<dyn Fn(ProviderId, &LaunchOpts) -> Launch + Send + Sync>;

/// `true` when a saved working directory is still on disk.
pub type ExistsFn = Arc<dyn Fn(&str) -> bool + Send + Sync>;

type DataCb = Arc<dyn Fn(&str, &str) + Send + Sync>;
type UpdateCb = Arc<dyn Fn(&TerminalInfo, bool) + Send + Sync>;

/// One coalesced chunk of terminal output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalChunk {
    pub id: String,
    pub data: String,
}

/// Project directory used for every session that does not pass its own.
pub fn fixed_cwd(dir: impl Into<String>) -> Arc<dyn Fn() -> String + Send + Sync> {
    let dir = dir.into();
    Arc::new(move || dir.clone())
}

/// Dependencies for [`PtyHost`]. `launch` and `cwd` are required; the rest have defaults.
pub struct PtyHostOptions {
    pub launch: LaunchFn,
    pub cwd: Arc<dyn Fn() -> String + Send + Sync>,
    pub model_for: Option<Arc<dyn Fn(ProviderId) -> Option<String> + Send + Sync>>,
    pub store: Option<Arc<Mutex<Store>>>,
    pub spawn: Option<SpawnPty>,
    pub kill_tree: Option<KillTree>,
    pub now: Option<Arc<dyn Fn() -> i64 + Send + Sync>>,
    pub scheduler: Option<Arc<dyn Scheduler>>,
    pub create_chat: Option<ChatFn>,
    pub max_terminals: Option<usize>,
    pub exists: Option<ExistsFn>,
    pub platform: Option<Platform>,
}

#[derive(Clone)]
pub struct PtyHost {
    inner: Arc<HostInner>,
}

struct HostInner {
    state: Mutex<State>,
    launch: LaunchFn,
    cwd: Arc<dyn Fn() -> String + Send + Sync>,
    model_for: Option<Arc<dyn Fn(ProviderId) -> Option<String> + Send + Sync>>,
    store: Option<Arc<Mutex<Store>>>,
    spawn: SpawnPty,
    kill_tree: KillTree,
    now: Arc<dyn Fn() -> i64 + Send + Sync>,
    scheduler: Arc<dyn Scheduler>,
    create_chat: Option<ChatFn>,
    max_terminals: usize,
    exists: ExistsFn,
    platform: Platform,
    output: broadcast::Sender<TerminalChunk>,
    claims: TerminalClaims,
    errors: AtomicU64,
}

struct State {
    sessions: Vec<Session>,
    asleep: Vec<Asleep>,
    by_id: HashMap<String, u64>,
    opened: HashMap<ProviderId, u32>,
    next_key: u64,
    next_listener: u64,
    data_listeners: Vec<DataListener>,
    update_listeners: Vec<UpdateListener>,
}

struct Session {
    key: u64,
    generation: u64,
    info: TerminalInfo,
    proc: Arc<Mutex<Box<dyn PtyProcess>>>,
    snapshot: String,
    snapshot_chars: usize,
    pending: String,
    data_timer: Option<u64>,
    store_timer: Option<u64>,
    cols: f64,
    rows: f64,
    project: String,
    cwd: String,
    exit_waiters: Vec<oneshot::Sender<()>>,
    on_probation: bool,
    relaunching: bool,
    removed: bool,
    exited: bool,
    closed_by_host: bool,
    logged: bool,
}

struct Asleep {
    info: TerminalInfo,
    project: String,
}

struct DataListener {
    id: u64,
    cb: DataCb,
}

struct UpdateListener {
    id: u64,
    cb: UpdateCb,
}

enum SubKind {
    Data,
    Update,
}

#[derive(Clone, Copy)]
enum TimerKind {
    Data,
    Store,
}

/// Drops the listener. Hold this for as long as the callback should run.
pub struct Subscription {
    kind: SubKind,
    id: u64,
    host: Arc<HostInner>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let mut state = lock(&self.host.state);
        match self.kind {
            SubKind::Data => state
                .data_listeners
                .retain(|listener| listener.id != self.id),
            SubKind::Update => state
                .update_listeners
                .retain(|listener| listener.id != self.id),
        }
    }
}

impl Subscription {
    pub fn detach(self) {
        drop(self);
    }
}

struct BootArgs {
    id: String,
    provider: TerminalProvider,
    title: String,
    model: Option<String>,
    launch_opts: LaunchOpts,
    launch_spec: Option<Launch>,
    cols: f64,
    rows: f64,
    snapshot: String,
    started_at: i64,
    on_probation: bool,
    cwd: Option<String>,
    change: Option<String>,
    project: Option<String>,
}

impl PtyHost {
    pub fn new(options: PtyHostOptions) -> Self {
        let (output, _) = broadcast::channel(256);
        Self {
            inner: Arc::new(HostInner {
                state: Mutex::new(State {
                    sessions: Vec::new(),
                    asleep: Vec::new(),
                    by_id: HashMap::new(),
                    opened: HashMap::new(),
                    next_key: 1,
                    next_listener: 1,
                    data_listeners: Vec::new(),
                    update_listeners: Vec::new(),
                }),
                launch: options.launch,
                cwd: options.cwd,
                model_for: options.model_for,
                store: options.store,
                spawn: options.spawn.unwrap_or_else(|| Arc::new(spawn_pty)),
                kill_tree: options
                    .kill_tree
                    .unwrap_or_else(|| Arc::new(default_kill_tree)),
                now: options.now.unwrap_or_else(|| Arc::new(system_now_ms)),
                scheduler: options.scheduler.unwrap_or_else(thread_scheduler),
                create_chat: options.create_chat,
                max_terminals: options.max_terminals.unwrap_or(DEFAULT_MAX),
                exists: options
                    .exists
                    .unwrap_or_else(|| Arc::new(|path| Path::new(path).exists())),
                platform: options.platform.unwrap_or_else(host_platform),
                output,
                claims: TerminalClaims::new(),
                errors: AtomicU64::new(0),
            }),
        }
    }

    /// Live output. A slow receiver skips ahead (`Lagged`); call [`PtyHost::snapshot`] to resync.
    pub fn subscribe(&self) -> broadcast::Receiver<TerminalChunk> {
        self.inner.output.subscribe()
    }

    pub async fn open(
        &self,
        provider: ProviderId,
        cols: f64,
        rows: f64,
        opts: OpenOptions,
    ) -> Result<TerminalInfo, PtyError> {
        if self.live_count() >= self.inner.max_terminals {
            return Err(PtyError::TooMany);
        }
        let (launch_opts, model, session_id) = self.prepare_launch(provider, &opts).await;
        let n = {
            let state = lock(&self.inner.state);
            state.opened.get(&provider).copied().unwrap_or(0) + 1
        };
        let name = provider.as_str();
        let title = match &opts.change {
            Some(change) => format!("{name} {n} · {change}"),
            None => format!("{name} {n}"),
        };
        let info = self.boot(BootArgs {
            id: Uuid::new_v4().to_string(),
            provider: terminal_provider(provider),
            title,
            model,
            launch_opts,
            launch_spec: None,
            cols,
            rows,
            snapshot: String::new(),
            started_at: (self.inner.now)(),
            on_probation: false,
            cwd: opts.cwd,
            change: opts.change,
            project: None,
        })?;
        lock(&self.inner.state).opened.insert(provider, n);
        self.persist_open(&info, session_id)?;
        Ok(info)
    }

    pub fn open_shell(&self, opts: ShellOptions) -> Result<TerminalInfo, PtyError> {
        if self.live_count() >= self.inner.max_terminals {
            return Err(PtyError::TooMany);
        }
        let spec = setup_launch(&opts.commands, self.inner.platform);
        let info = self.boot(BootArgs {
            id: Uuid::new_v4().to_string(),
            provider: TerminalProvider::Shell,
            title: opts.title,
            model: None,
            launch_opts: LaunchOpts::default(),
            launch_spec: Some(spec),
            cols: opts.cols,
            rows: opts.rows,
            snapshot: String::new(),
            started_at: (self.inner.now)(),
            on_probation: false,
            cwd: Some(opts.cwd),
            change: opts.change,
            project: None,
        })?;
        self.persist_open(&info, None)?;
        Ok(info)
    }

    /// Relaunch saved agent sessions for `project`. Shell rows are deleted.
    /// Rows whose saved directory is gone are deleted. Already-open ids are skipped.
    pub fn restore(&self, project: &str, cols: f64, rows: f64) -> Vec<TerminalInfo> {
        let Some(store) = &self.inner.store else {
            return Vec::new();
        };
        let saved = match lock(store).terminals(project) {
            Ok(saved) => saved,
            Err(err) => {
                self.log(&err.to_string());
                return Vec::new();
            }
        };
        let mut restored = Vec::new();
        for row in saved {
            if self.live_count() >= self.inner.max_terminals {
                break;
            }
            if self.is_open(&row.id) {
                continue;
            }
            if row.provider == "shell" {
                self.delete_row(&row.id);
                continue;
            }
            let Some(provider) = as_provider(&row.provider) else {
                continue;
            };
            let saved_cwd = nonempty(row.cwd.clone());
            if let Some(dir) = &saved_cwd
                && !(self.inner.exists)(dir)
            {
                self.delete_row(&row.id);
                continue;
            }
            let model = row.model.clone();
            let mut session_id = row.session_id.clone();
            let mut launch_opts = LaunchOpts {
                model: model.clone(),
                ..LaunchOpts::default()
            };
            let original_session = row.session_id.clone();
            if let Some(id) = session_id.clone() {
                launch_opts.resume = Some(id);
            } else if provider == ProviderId::Claude {
                let id = Uuid::new_v4().to_string();
                session_id = Some(id.clone());
                launch_opts.session_id = Some(id);
            }
            let snapshot = format!("{}{RESTORE_MARK}", row.scrollback);
            match self.boot(BootArgs {
                id: row.id.clone(),
                provider: terminal_provider(provider),
                title: row.title.clone(),
                model,
                launch_opts,
                launch_spec: None,
                cols,
                rows,
                snapshot,
                started_at: (self.inner.now)(),
                on_probation: original_session.is_some(),
                cwd: saved_cwd,
                change: nonempty(row.change_id.clone()),
                project: Some(project.to_string()),
            }) {
                Ok(info) => {
                    let mut state = lock(&self.inner.state);
                    let n = state.opened.get(&provider).copied().unwrap_or(0) + 1;
                    state.opened.insert(provider, n);
                    drop(state);
                    if session_id != original_session {
                        self.rewrite_session(&row, session_id, info.started_at);
                    }
                    restored.push(info);
                }
                Err(err) => self.log(&err.to_string()),
            }
        }
        restored
    }

    pub fn write(&self, id: &str, data: &str) {
        if !within_write_limit(data) {
            return;
        }
        let Some(proc) = self.running_proc(id) else {
            return;
        };
        if let Err(err) = lock(&proc).write(data) {
            self.log(&err.to_string());
        }
    }

    /// Resize a running PTY. The size is clamped; a rejected resize is ignored.
    pub fn resize(&self, id: &str, cols: f64, rows: f64) {
        let Some(proc) = self.running_proc(id) else {
            return;
        };
        let (cols, rows) = clamp_size(cols, rows);
        let _ = lock(&proc).resize(cols, rows);
    }

    /// `claimTerminal`: this window now owns the pty size.
    pub fn claim_terminal(&self, id: &str, window_id: u64) {
        self.inner.claims.claim(id, window_id);
    }

    /// Resize when `window_id` owns the terminal. The first caller claims an unowned one.
    pub fn resize_claimed(&self, id: &str, window_id: u64, cols: f64, rows: f64) -> bool {
        if !self.inner.claims.allow_resize(id, window_id) {
            return false;
        }
        self.resize(id, cols, rows);
        true
    }

    pub fn release_window(&self, window_id: u64) {
        self.inner.claims.release_window(window_id);
    }

    pub fn close(&self, id: &str) {
        let sleeping = {
            let mut state = lock(&self.inner.state);
            let pos = state.asleep.iter().position(|entry| entry.info.id == id);
            pos.map(|pos| state.asleep.remove(pos))
        };
        if let Some(entry) = sleeping {
            self.delete_row(id);
            self.emit_update(&entry.info, true);
            return;
        }
        let Some(key) = self.key_if_live(id) else {
            return;
        };
        self.teardown(key, false);
    }

    pub fn close_change(&self, change_id: &str) -> CloseWait {
        let waits = self.begin_close_change(change_id);
        Box::pin(async move {
            let joined = async move {
                for wait in waits {
                    let _ = wait.await;
                }
            };
            let _ = tokio::time::timeout(CLOSE_CHANGE, joined).await;
        })
    }

    pub fn close_all(&self) {
        lock(&self.inner.state).asleep.clear();
        let keys: Vec<u64> = {
            let state = lock(&self.inner.state);
            state
                .sessions
                .iter()
                .filter(|session| !session.removed)
                .map(|session| session.key)
                .collect()
        };
        for key in keys {
            self.flush_store(key);
            self.teardown(key, true);
        }
    }

    pub fn list(&self) -> Vec<TerminalInfo> {
        let state = lock(&self.inner.state);
        let mut out = Vec::new();
        for session in &state.sessions {
            if !session.removed {
                out.push(session.info.clone());
            }
        }
        for entry in &state.asleep {
            out.push(entry.info.clone());
        }
        out
    }

    /// List a project's live/sleeping terminals without stopping other projects.
    pub fn list_project(&self, project: &str) -> Vec<TerminalInfo> {
        let state = lock(&self.inner.state);
        state
            .sessions
            .iter()
            .filter(|session| !session.removed && session.project == project)
            .map(|session| session.info.clone())
            .chain(
                state
                    .asleep
                    .iter()
                    .filter(|entry| entry.project == project)
                    .map(|entry| entry.info.clone()),
            )
            .collect()
    }

    /// Stop a resumable agent and keep its row. Shells, missing rows, and rows without a
    /// session id stay running. Returns whether the session went to sleep.
    pub fn sleep(&self, id: &str) -> bool {
        let Some(store) = &self.inner.store else {
            return false;
        };
        let Some((project, provider)) = self.running_meta(id) else {
            return false;
        };
        if provider == TerminalProvider::Shell {
            return false;
        }
        let has_session = match lock(store).terminals(&project) {
            Ok(rows) => rows
                .iter()
                .any(|row| row.id == id && row.session_id.is_some()),
            Err(err) => {
                self.log(&err.to_string());
                return false;
            }
        };
        if !has_session {
            return false;
        }
        let Some(key) = self.running_key(id) else {
            return false;
        };
        self.flush_data(key);
        self.flush_store(key);
        let prepared = {
            let mut state = lock(&self.inner.state);
            let Some(session) = session_mut(&mut state, key) else {
                return false;
            };
            if session.removed || session.exited || session.info.status != TerminalStatus::Running {
                return false;
            }
            session.relaunching = false;
            session.closed_by_host = true;
            session.removed = true;
            let mut info = session.info.clone();
            info.status = TerminalStatus::Sleeping;
            let project = session.project.clone();
            let proc = Arc::clone(&session.proc);
            state.asleep.push(Asleep {
                info: info.clone(),
                project,
            });
            Some((proc, info))
        };
        let Some((proc, info)) = prepared else {
            return false;
        };
        self.kill_proc(&proc);
        self.emit_update(&info, false);
        true
    }

    /// Start a sleeping terminal again with `--resume` of its saved session.
    pub fn wake(&self, id: &str, cols: f64, rows: f64) -> Result<Option<TerminalInfo>, PtyError> {
        let Some(store) = &self.inner.store else {
            return Ok(None);
        };
        let entry = {
            let mut state = lock(&self.inner.state);
            let pos = state.asleep.iter().position(|item| item.info.id == id);
            match pos {
                Some(pos) => state.asleep.remove(pos),
                None => return Ok(None),
            }
        };
        let saved = lock(store).terminals(&entry.project)?;
        let row = saved.into_iter().find(|row| row.id == id);
        let Some(row) = row else {
            self.emit_update(&entry.info, true);
            return Ok(None);
        };
        let Some(provider) = as_provider(&row.provider) else {
            self.emit_update(&entry.info, true);
            return Ok(None);
        };
        let Some(session_id) = row.session_id.clone() else {
            self.emit_update(&entry.info, true);
            return Ok(None);
        };
        let saved_cwd = nonempty(row.cwd.clone());
        if let Some(dir) = &saved_cwd
            && !(self.inner.exists)(dir)
        {
            self.delete_row(id);
            self.emit_update(&entry.info, true);
            return Ok(None);
        }
        let (cols, rows) = clamp_size(cols, rows);
        let info = self.boot(BootArgs {
            id: id.to_string(),
            provider: terminal_provider(provider),
            title: row.title,
            model: row.model.clone(),
            launch_opts: LaunchOpts {
                model: row.model,
                resume: Some(session_id),
                ..LaunchOpts::default()
            },
            launch_spec: None,
            cols: f64::from(cols),
            rows: f64::from(rows),
            snapshot: format!("{}{RESTORE_MARK}", row.scrollback),
            started_at: (self.inner.now)(),
            on_probation: true,
            cwd: saved_cwd,
            change: nonempty(row.change_id),
            project: Some(entry.project),
        })?;
        Ok(Some(info))
    }

    /// Rolling output kept for a pane that attaches after the session started.
    /// Empty once the session is removed. Capped at [`SNAPSHOT_MAX_CHARS`].
    pub fn snapshot(&self, id: &str) -> String {
        let state = lock(&self.inner.state);
        let Some(key) = state.by_id.get(id).copied() else {
            return String::new();
        };
        let Some(session) = session_ref(&state, key) else {
            return String::new();
        };
        if session.removed {
            String::new()
        } else {
            session.snapshot.clone()
        }
    }

    pub fn on_data<F>(&self, cb: F) -> Subscription
    where
        F: Fn(&str, &str) + Send + Sync + 'static,
    {
        let cb: DataCb = Arc::new(cb);
        let mut state = lock(&self.inner.state);
        let id = state.next_listener;
        state.next_listener = state.next_listener.saturating_add(1);
        state.data_listeners.push(DataListener { id, cb });
        Subscription {
            kind: SubKind::Data,
            id,
            host: Arc::clone(&self.inner),
        }
    }

    pub fn on_update<F>(&self, cb: F) -> Subscription
    where
        F: Fn(&TerminalInfo, bool) + Send + Sync + 'static,
    {
        let cb: UpdateCb = Arc::new(cb);
        let mut state = lock(&self.inner.state);
        let id = state.next_listener;
        state.next_listener = state.next_listener.saturating_add(1);
        state.update_listeners.push(UpdateListener { id, cb });
        Subscription {
            kind: SubKind::Update,
            id,
            host: Arc::clone(&self.inner),
        }
    }

    #[cfg(test)]
    pub(crate) fn logged_errors(&self) -> u64 {
        self.inner.errors.load(Ordering::Relaxed)
    }
}

impl PtyHost {
    async fn prepare_launch(
        &self,
        provider: ProviderId,
        opts: &OpenOptions,
    ) -> (LaunchOpts, Option<String>, Option<String>) {
        let model = self
            .inner
            .model_for
            .as_ref()
            .and_then(|model_for| model_for(provider));
        let prompt = opts.prompt.clone().filter(|prompt| !prompt.is_empty());
        let take_over = opts.resume.clone().filter(|resume| !resume.is_empty());
        let session_id = if take_over.is_some() {
            take_over.clone()
        } else {
            self.fresh_session_id(provider).await
        };
        let mut launch_opts = LaunchOpts {
            model: model.clone(),
            ..LaunchOpts::default()
        };
        if let Some(id) = &take_over {
            launch_opts.resume = Some(id.clone());
        } else if let Some(id) = &session_id {
            if provider == ProviderId::Claude || prompt.is_some() {
                launch_opts.session_id = Some(id.clone());
            } else {
                launch_opts.resume = Some(id.clone());
            }
        }
        if let Some(prompt) = prompt {
            launch_opts.prompt = Some(prompt);
        }
        (launch_opts, model, session_id)
    }

    async fn fresh_session_id(&self, provider: ProviderId) -> Option<String> {
        if provider == ProviderId::Claude {
            return Some(Uuid::new_v4().to_string());
        }
        if provider == ProviderId::Codex || provider == ProviderId::Local {
            return None;
        }
        let create = self.inner.create_chat.clone()?;
        match tokio::time::timeout(CREATE_CHAT, create()).await {
            Ok(Ok(raw)) => parse_session_id(&raw),
            _ => None,
        }
    }

    fn boot(&self, args: BootArgs) -> Result<TerminalInfo, PtyError> {
        let (cols, rows) = clamp_size(args.cols, args.rows);
        let cwd = args.cwd.clone().unwrap_or_else(|| (self.inner.cwd)());
        let project = args.project.clone().unwrap_or_else(|| (self.inner.cwd)());
        let spec = match &args.launch_spec {
            Some(spec) => spec.clone(),
            None => {
                let provider = agent_id(args.provider).ok_or(PtyError::ShellHasNoCli)?;
                (self.inner.launch)(provider, &args.launch_opts)
            }
        };
        let env = terminal_env(&spec.env.clone().unwrap_or_else(process_env));
        let proc = match (self.inner.spawn)(
            &spec.command,
            &spec.args,
            &SpawnOpts {
                name: "xterm-256color".to_string(),
                cols,
                rows,
                cwd: cwd.clone(),
                env,
            },
        ) {
            Ok(proc) => proc,
            Err(err) => {
                return Err(PtyError::Start {
                    provider: provider_name(args.provider).to_string(),
                    message: err.to_string(),
                });
            }
        };
        let (snapshot, snapshot_chars) = initial_chars(&args.snapshot);
        let info = TerminalInfo {
            id: args.id.clone(),
            provider: args.provider,
            title: args.title,
            status: TerminalStatus::Running,
            exit_code: None,
            model: args.model,
            started_at: args.started_at,
            change: args.change,
        };
        let key = {
            let mut state = lock(&self.inner.state);
            let key = state.next_key;
            state.next_key = state.next_key.saturating_add(1);
            state.by_id.insert(info.id.clone(), key);
            state.sessions.push(Session {
                key,
                generation: 0,
                info: info.clone(),
                proc: Arc::new(Mutex::new(proc)),
                snapshot,
                snapshot_chars,
                pending: String::new(),
                data_timer: None,
                store_timer: None,
                cols: args.cols,
                rows: args.rows,
                project,
                cwd,
                exit_waiters: Vec::new(),
                on_probation: args.on_probation,
                relaunching: false,
                removed: false,
                exited: false,
                closed_by_host: false,
                logged: false,
            });
            key
        };
        self.attach(key);
        let (emit_snapshot, removed) = {
            let state = lock(&self.inner.state);
            match session_ref(&state, key) {
                Some(session) => (session.snapshot.clone(), session.removed),
                None => (String::new(), true),
            }
        };
        if !removed && !emit_snapshot.is_empty() {
            self.emit_data(&info.id, &emit_snapshot);
        }
        if !removed {
            self.emit_update(&info, false);
        }
        Ok(info)
    }

    fn attach(&self, key: u64) {
        let (proc, generation) = {
            let mut state = lock(&self.inner.state);
            let Some(session) = session_mut(&mut state, key) else {
                return;
            };
            session.generation = session.generation.saturating_add(1);
            (Arc::clone(&session.proc), session.generation)
        };
        let queued = {
            let host = self.clone();
            let mut pty = lock(&proc);
            pty.on_data(Box::new(move |data| {
                host.on_pty_data(key, generation, data)
            }));
            let host = self.clone();
            pty.on_exit(Box::new(move |code| {
                host.on_pty_exit(key, generation, code)
            }));
            pty.take_queued()
        };
        for data in queued.data {
            self.on_pty_data(key, generation, data);
        }
        if let Some(code) = queued.exit {
            self.on_pty_exit(key, generation, code);
        }
    }

    fn on_pty_data(&self, key: u64, generation: u64, data: String) {
        if data.is_empty() {
            return;
        }
        let (big, arm_data, arm_store) = {
            let mut state = lock(&self.inner.state);
            let Some(session) = session_mut(&mut state, key) else {
                return;
            };
            if session.generation != generation || session.exited {
                return;
            }
            push_capped(
                &mut session.snapshot,
                &mut session.snapshot_chars,
                &data,
                SNAPSHOT_MAX_CHARS,
            );
            session.pending.push_str(&data);
            let big = session.pending.len() >= READER_COALESCE_BYTES;
            let arm_data = session.data_timer.is_none();
            let arm_store = self.inner.store.is_some() && session.store_timer.is_none();
            (big, arm_data, arm_store)
        };
        if arm_store {
            self.arm_timer(key, TimerKind::Store, STORE_FLUSH);
        }
        if big {
            self.flush_data(key);
        } else if arm_data {
            self.arm_timer(key, TimerKind::Data, OUTPUT_FLUSH);
        }
    }

    fn on_pty_exit(&self, key: u64, generation: u64, code: Option<i64>) {
        let relaunch = {
            let state = lock(&self.inner.state);
            let Some(session) = session_ref(&state, key) else {
                return;
            };
            if session.generation != generation || session.exited {
                return;
            }
            should_relaunch(session, code, (self.inner.now)())
        };
        if relaunch {
            self.begin_relaunch(key, code);
        } else {
            self.finish_exit(key, code);
        }
    }

    fn begin_relaunch(&self, key: u64, code: Option<i64>) {
        self.flush_data(key);
        self.flush_store(key);
        let provider = {
            let mut state = lock(&self.inner.state);
            let Some(session) = session_mut(&mut state, key) else {
                return;
            };
            let waiters = std::mem::take(&mut session.exit_waiters);
            session.exited = true;
            session.on_probation = false;
            session.relaunching = true;
            let provider = session.info.provider;
            drop(state);
            signal(waiters);
            provider
        };
        match provider {
            TerminalProvider::Cursor => self.spawn_relaunch(key, ProviderId::Cursor, code),
            TerminalProvider::Grok => self.spawn_relaunch(key, ProviderId::Grok, code),
            _ => self.relaunch_claude(key, code),
        }
    }

    fn relaunch_claude(&self, key: u64, fallback: Option<i64>) {
        let Some(provider) = self.provider_of(key).and_then(agent_id) else {
            self.finish_exit(key, fallback);
            return;
        };
        let session_id = Uuid::new_v4().to_string();
        let opts = LaunchOpts {
            model: self.model_of(key),
            session_id: Some(session_id.clone()),
            ..LaunchOpts::default()
        };
        if !self.apply_fresh(key, provider, opts, fallback) {
            return;
        }
        self.update_session_row(key, Some(session_id));
    }

    fn spawn_relaunch(&self, key: u64, provider: ProviderId, fallback: Option<i64>) {
        let host = self.clone();
        let work = async move {
            let session_id = host.fresh_session_id(provider).await;
            if !host.still_relaunching(key) {
                return;
            }
            let mut opts = LaunchOpts {
                model: host.model_of(key),
                ..LaunchOpts::default()
            };
            if let Some(id) = session_id.clone() {
                opts.resume = Some(id);
            }
            if host.apply_fresh(key, provider, opts, fallback) {
                host.update_session_row(key, session_id);
            }
        };
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            drop(handle.spawn(work));
        } else {
            let _ = std::thread::Builder::new()
                .name("legate-pty-relaunch".to_string())
                .spawn(move || {
                    if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                        .enable_time()
                        .build()
                    {
                        runtime.block_on(work);
                    }
                });
        }
    }

    fn apply_fresh(
        &self,
        key: u64,
        provider: ProviderId,
        opts: LaunchOpts,
        fallback: Option<i64>,
    ) -> bool {
        if !self.still_relaunching(key) {
            return false;
        }
        if let Err(err) = self.respawn(key, provider, &opts) {
            self.log(&err.to_string());
            {
                let mut state = lock(&self.inner.state);
                if let Some(session) = session_mut(&mut state, key) {
                    session.relaunching = false;
                }
            }
            self.finish_exit(key, fallback);
            return false;
        }
        let (id, info) = {
            let mut state = lock(&self.inner.state);
            let Some(session) = session_mut(&mut state, key) else {
                return false;
            };
            session.relaunching = false;
            session.exited = false;
            session.info.status = TerminalStatus::Running;
            session.info.exit_code = None;
            push_capped(
                &mut session.snapshot,
                &mut session.snapshot_chars,
                FRESH_MARK,
                SNAPSHOT_MAX_CHARS,
            );
            (session.info.id.clone(), session.info.clone())
        };
        self.emit_data(&id, FRESH_MARK);
        self.emit_update(&info, false);
        true
    }

    fn respawn(&self, key: u64, provider: ProviderId, opts: &LaunchOpts) -> Result<(), PtyError> {
        let (cols, rows, cwd) = {
            let state = lock(&self.inner.state);
            let session = session_ref(&state, key)
                .ok_or_else(|| PtyError::Message("session is gone".to_string()))?;
            (session.cols, session.rows, session.cwd.clone())
        };
        let spec = (self.inner.launch)(provider, opts);
        let (cols, rows) = clamp_size(cols, rows);
        let env = terminal_env(&spec.env.clone().unwrap_or_else(process_env));
        let proc = (self.inner.spawn)(
            &spec.command,
            &spec.args,
            &SpawnOpts {
                name: "xterm-256color".to_string(),
                cols,
                rows,
                cwd,
                env,
            },
        )?;
        {
            let mut state = lock(&self.inner.state);
            let session = session_mut(&mut state, key)
                .ok_or_else(|| PtyError::Message("session is gone".to_string()))?;
            session.proc = Arc::new(Mutex::new(proc));
        }
        self.attach(key);
        Ok(())
    }

    fn finish_exit(&self, key: u64, code: Option<i64>) {
        let waiters = {
            let mut state = lock(&self.inner.state);
            let Some(session) = session_mut(&mut state, key) else {
                return;
            };
            let waiters = std::mem::take(&mut session.exit_waiters);
            session.exited = true;
            session.relaunching = false;
            waiters
        };
        signal(waiters);
        self.flush_data(key);
        self.flush_store(key);
        let outcome = {
            let mut state = lock(&self.inner.state);
            let Some(session) = session_mut(&mut state, key) else {
                return;
            };
            session.info.status = TerminalStatus::Exited;
            session.info.exit_code = code;
            let removed = session.removed;
            let shell = session.info.provider == TerminalProvider::Shell;
            let id = session.info.id.clone();
            let info = session.info.clone();
            (removed, shell, id, info)
        };
        let (removed, shell, id, info) = outcome;
        if shell {
            self.delete_row(&id);
        }
        if !removed {
            self.emit_update(&info, false);
        }
        self.write_run(key);
        let mut state = lock(&self.inner.state);
        forget(&mut state, key);
    }

    fn teardown(&self, key: u64, keep_row: bool) {
        let kill = {
            let mut state = lock(&self.inner.state);
            let Some(session) = session_mut(&mut state, key) else {
                return;
            };
            session.relaunching = false;
            if session.info.status == TerminalStatus::Running {
                session.closed_by_host = true;
                Some(Arc::clone(&session.proc))
            } else {
                None
            }
        };
        if let Some(proc) = kill {
            self.kill_proc(&proc);
        }
        let info = {
            let mut state = lock(&self.inner.state);
            let Some(session) = session_mut(&mut state, key) else {
                return;
            };
            if session.removed {
                return;
            }
            let id = session.info.id.clone();
            let info = session.info.clone();
            session.removed = true;
            (id, info)
        };
        let (id, info) = info;
        if !keep_row {
            self.delete_row(&id);
        }
        self.emit_update(&info, true);
        let mut state = lock(&self.inner.state);
        forget(&mut state, key);
    }

    fn begin_close_change(&self, change_id: &str) -> Vec<oneshot::Receiver<()>> {
        let asleep = {
            let mut state = lock(&self.inner.state);
            let mut gone = Vec::new();
            state.asleep.retain(|entry| {
                if entry.info.change.as_deref() == Some(change_id) {
                    gone.push(entry.info.clone());
                    false
                } else {
                    true
                }
            });
            gone
        };
        for info in &asleep {
            self.delete_row(&info.id);
            self.emit_update(info, true);
        }
        let keys: Vec<u64> = {
            let state = lock(&self.inner.state);
            state
                .sessions
                .iter()
                .filter(|session| {
                    !session.removed && session.info.change.as_deref() == Some(change_id)
                })
                .map(|session| session.key)
                .collect()
        };
        let mut waits = Vec::new();
        for key in keys {
            let (tx, rx) = oneshot::channel();
            let armed = {
                let mut state = lock(&self.inner.state);
                match session_mut(&mut state, key) {
                    Some(session) if !session.exited => {
                        session.exit_waiters.push(tx);
                        true
                    }
                    _ => false,
                }
            };
            if armed {
                waits.push(rx);
            }
            self.teardown(key, false);
        }
        waits
    }

    fn persist_open(
        &self,
        info: &TerminalInfo,
        session_id: Option<String>,
    ) -> Result<(), PtyError> {
        let Some(store) = &self.inner.store else {
            return Ok(());
        };
        let (project, cwd) = {
            let state = lock(&self.inner.state);
            let Some(key) = state.by_id.get(&info.id).copied() else {
                return Ok(());
            };
            let Some(session) = session_ref(&state, key) else {
                return Ok(());
            };
            (session.project.clone(), session.cwd.clone())
        };
        lock(store).save_terminal(&TerminalRow {
            id: info.id.clone(),
            project,
            provider: provider_name(info.provider).to_string(),
            model: info.model.clone(),
            session_id,
            title: info.title.clone(),
            created_at: info.started_at,
            updated_at: info.started_at,
            scrollback: String::new(),
            cwd: Some(cwd),
            change_id: info.change.clone(),
        })?;
        Ok(())
    }

    fn rewrite_session(&self, row: &TerminalRow, session_id: Option<String>, updated_at: i64) {
        let Some(store) = &self.inner.store else {
            return;
        };
        let mut row = row.clone();
        row.session_id = session_id;
        row.updated_at = updated_at;
        if let Err(err) = lock(store).save_terminal(&row) {
            self.log(&err.to_string());
        }
    }

    fn update_session_row(&self, key: u64, session_id: Option<String>) {
        let Some(store) = &self.inner.store else {
            return;
        };
        let (project, id) = {
            let state = lock(&self.inner.state);
            let Some(session) = session_ref(&state, key) else {
                return;
            };
            (session.project.clone(), session.info.id.clone())
        };
        let now = (self.inner.now)();
        let guard = lock(store);
        let rows = match guard.terminals(&project) {
            Ok(rows) => rows,
            Err(err) => {
                drop(guard);
                self.log(&err.to_string());
                return;
            }
        };
        let Some(mut row) = rows.into_iter().find(|row| row.id == id) else {
            return;
        };
        row.session_id = session_id;
        row.updated_at = now;
        if let Err(err) = guard.save_terminal(&row) {
            drop(guard);
            self.log(&err.to_string());
        }
    }

    fn write_run(&self, key: u64) {
        let Some(store) = &self.inner.store else {
            return;
        };
        let row = {
            let mut state = lock(&self.inner.state);
            let Some(session) = session_mut(&mut state, key) else {
                return;
            };
            if session.logged {
                return;
            }
            session.logged = true;
            if session.info.provider == TerminalProvider::Shell {
                return;
            }
            let outcome = if session.closed_by_host || session.info.exit_code == Some(0) {
                RunOutcome::Ok
            } else {
                RunOutcome::Error
            };
            RunRow {
                id: None,
                job_id: session.info.id.clone(),
                provider: provider_name(session.info.provider).to_string(),
                job_type: "terminal".to_string(),
                started_at: session.info.started_at,
                duration_ms: Some((self.inner.now)().saturating_sub(session.info.started_at)),
                cost_usd: None,
                tokens: None,
                utilization: None,
                outcome,
                session_id: None,
            }
        };
        if let Err(err) = lock(store).insert_run(&row) {
            self.log(&err.to_string());
        }
    }

    fn flush_data(&self, key: u64) {
        let (timer, id, data) = {
            let mut state = lock(&self.inner.state);
            let Some(session) = session_mut(&mut state, key) else {
                return;
            };
            let timer = session.data_timer.take();
            let data = std::mem::take(&mut session.pending);
            (timer, session.info.id.clone(), data)
        };
        if let Some(timer) = timer.filter(|id| *id != 0) {
            self.inner.scheduler.cancel(timer);
        }
        if !data.is_empty() {
            self.emit_data(&id, &data);
        }
    }

    fn flush_store(&self, key: u64) {
        let (timer, id, text) = {
            let mut state = lock(&self.inner.state);
            let Some(session) = session_mut(&mut state, key) else {
                return;
            };
            let timer = session.store_timer.take();
            let text = tail_chars(&session.snapshot, TERMINAL_SCROLLBACK_CAP);
            (timer, session.info.id.clone(), text)
        };
        if let Some(timer) = timer.filter(|id| *id != 0) {
            self.inner.scheduler.cancel(timer);
        }
        let Some(store) = &self.inner.store else {
            return;
        };
        if let Err(err) = lock(store).update_terminal_scrollback(&id, &text, (self.inner.now)()) {
            self.log(&err.to_string());
        }
    }

    fn arm_timer(&self, key: u64, kind: TimerKind, delay: Duration) {
        {
            let mut state = lock(&self.inner.state);
            let Some(session) = session_mut(&mut state, key) else {
                return;
            };
            let slot = timer_slot(session, kind);
            if slot.is_some() {
                return;
            }
            *slot = Some(0);
        }
        let host = self.clone();
        let id = self.inner.scheduler.schedule(
            delay,
            Box::new(move || match kind {
                TimerKind::Data => host.on_data_timer(key),
                TimerKind::Store => host.on_store_timer(key),
            }),
        );
        let cancel = {
            let mut state = lock(&self.inner.state);
            match session_mut(&mut state, key) {
                Some(session) => {
                    let slot = timer_slot(session, kind);
                    if *slot == Some(0) {
                        *slot = Some(id);
                        false
                    } else {
                        true
                    }
                }
                None => true,
            }
        };
        if cancel {
            self.inner.scheduler.cancel(id);
        }
    }

    fn on_data_timer(&self, key: u64) {
        let payload = {
            let mut state = lock(&self.inner.state);
            let Some(session) = session_mut(&mut state, key) else {
                return;
            };
            session.data_timer = None;
            if session.pending.is_empty() {
                return;
            }
            let data = std::mem::take(&mut session.pending);
            Some((session.info.id.clone(), data))
        };
        if let Some((id, data)) = payload {
            self.emit_data(&id, &data);
        }
    }

    fn on_store_timer(&self, key: u64) {
        self.flush_store(key);
    }

    fn kill_proc(&self, proc: &Arc<Mutex<Box<dyn PtyProcess>>>) {
        let mut pty = lock(proc);
        let kill = Arc::clone(&self.inner.kill_tree);
        let result = catch_unwind(AssertUnwindSafe(|| kill(&mut **pty)));
        if result.is_err() {
            self.log("kill tree panicked");
        }
    }

    fn emit_data(&self, id: &str, data: &str) {
        if data.is_empty() {
            return;
        }
        let _ = self.inner.output.send(TerminalChunk {
            id: id.to_string(),
            data: data.to_string(),
        });
        let listeners: Vec<_> = lock(&self.inner.state)
            .data_listeners
            .iter()
            .map(|listener| Arc::clone(&listener.cb))
            .collect();
        for listener in listeners {
            let result = catch_unwind(AssertUnwindSafe(|| listener(id, data)));
            if result.is_err() {
                self.log("terminal data listener panicked");
            }
        }
    }

    fn emit_update(&self, info: &TerminalInfo, removed: bool) {
        let listeners: Vec<_> = lock(&self.inner.state)
            .update_listeners
            .iter()
            .map(|listener| Arc::clone(&listener.cb))
            .collect();
        for listener in listeners {
            let result = catch_unwind(AssertUnwindSafe(|| listener(info, removed)));
            if result.is_err() {
                self.log("terminal update listener panicked");
            }
        }
    }

    fn log(&self, message: &str) {
        tracing::error!("{message}");
        self.inner.errors.fetch_add(1, Ordering::Relaxed);
    }

    fn live_count(&self) -> usize {
        lock(&self.inner.state)
            .sessions
            .iter()
            .filter(|session| !session.removed)
            .count()
    }

    fn is_open(&self, id: &str) -> bool {
        let state = lock(&self.inner.state);
        state.by_id.contains_key(id) || state.asleep.iter().any(|entry| entry.info.id == id)
    }

    fn running_proc(&self, id: &str) -> Option<Arc<Mutex<Box<dyn PtyProcess>>>> {
        let state = lock(&self.inner.state);
        let key = *state.by_id.get(id)?;
        let session = session_ref(&state, key)?;
        if session.removed || session.exited || session.info.status != TerminalStatus::Running {
            return None;
        }
        Some(Arc::clone(&session.proc))
    }

    fn running_key(&self, id: &str) -> Option<u64> {
        let state = lock(&self.inner.state);
        let key = *state.by_id.get(id)?;
        let session = session_ref(&state, key)?;
        if session.removed || session.exited || session.info.status != TerminalStatus::Running {
            None
        } else {
            Some(key)
        }
    }

    fn running_meta(&self, id: &str) -> Option<(String, TerminalProvider)> {
        let state = lock(&self.inner.state);
        let key = *state.by_id.get(id)?;
        let session = session_ref(&state, key)?;
        if session.removed || session.exited || session.info.status != TerminalStatus::Running {
            None
        } else {
            Some((session.project.clone(), session.info.provider))
        }
    }

    fn key_if_live(&self, id: &str) -> Option<u64> {
        let state = lock(&self.inner.state);
        let key = *state.by_id.get(id)?;
        let session = session_ref(&state, key)?;
        if session.removed { None } else { Some(key) }
    }

    fn still_relaunching(&self, key: u64) -> bool {
        let state = lock(&self.inner.state);
        session_ref(&state, key).is_some_and(|session| !session.removed && session.relaunching)
    }

    fn model_of(&self, key: u64) -> Option<String> {
        let state = lock(&self.inner.state);
        session_ref(&state, key).and_then(|session| session.info.model.clone())
    }

    fn provider_of(&self, key: u64) -> Option<TerminalProvider> {
        let state = lock(&self.inner.state);
        session_ref(&state, key).map(|session| session.info.provider)
    }

    fn delete_row(&self, id: &str) {
        let Some(store) = &self.inner.store else {
            return;
        };
        if let Err(err) = lock(store).delete_terminal(id) {
            self.log(&err.to_string());
        }
    }
}

fn timer_slot(session: &mut Session, kind: TimerKind) -> &mut Option<u64> {
    match kind {
        TimerKind::Data => &mut session.data_timer,
        TimerKind::Store => &mut session.store_timer,
    }
}

fn should_relaunch(session: &Session, code: Option<i64>, now: i64) -> bool {
    if session.removed || session.closed_by_host || !session.on_probation {
        return false;
    }
    if code == Some(0) {
        return false;
    }
    now.saturating_sub(session.info.started_at) < PROBATION_MS
}

fn forget(state: &mut State, key: u64) {
    let Some(pos) = state.sessions.iter().position(|session| session.key == key) else {
        return;
    };
    if !state.sessions[pos].removed || !state.sessions[pos].exited {
        return;
    }
    let id = state.sessions[pos].info.id.clone();
    if state.by_id.get(&id) == Some(&key) {
        state.by_id.remove(&id);
    }
    state.sessions.remove(pos);
}

fn signal(waiters: Vec<oneshot::Sender<()>>) {
    for waiter in waiters {
        let _ = waiter.send(());
    }
}

fn session_mut(state: &mut State, key: u64) -> Option<&mut Session> {
    state.sessions.iter_mut().find(|session| session.key == key)
}

fn session_ref(state: &State, key: u64) -> Option<&Session> {
    state.sessions.iter().find(|session| session.key == key)
}

fn system_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn clamp_size(cols: f64, rows: f64) -> (u16, u16) {
    if !cols.is_finite() || !rows.is_finite() {
        return (DEFAULT_COLS, DEFAULT_ROWS);
    }
    (clamp_int(cols, 2, 500), clamp_int(rows, 1, 300))
}

fn clamp_int(value: f64, min: u16, max: u16) -> u16 {
    let truncated = value.trunc();
    if truncated < f64::from(min) {
        min
    } else if truncated > f64::from(max) {
        max
    } else {
        truncated as u16
    }
}

fn within_write_limit(data: &str) -> bool {
    let bytes = data.len();
    if bytes == 0 || bytes > WRITE_MAX_CHARS.saturating_mul(4) {
        return false;
    }
    if bytes <= WRITE_MAX_CHARS {
        return true;
    }
    let chars = data.chars().count();
    (1..=WRITE_MAX_CHARS).contains(&chars)
}

fn nonempty(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.is_empty())
}

fn provider_name(provider: TerminalProvider) -> &'static str {
    match provider {
        TerminalProvider::Claude => "claude",
        TerminalProvider::Codex => "codex",
        TerminalProvider::Cursor => "cursor",
        TerminalProvider::Grok => "grok",
        TerminalProvider::Local => "local",
        TerminalProvider::Shell => "shell",
    }
}

fn terminal_provider(provider: ProviderId) -> TerminalProvider {
    match provider {
        ProviderId::Claude => TerminalProvider::Claude,
        ProviderId::Codex => TerminalProvider::Codex,
        ProviderId::Cursor => TerminalProvider::Cursor,
        ProviderId::Grok => TerminalProvider::Grok,
        ProviderId::Local => TerminalProvider::Local,
    }
}

fn agent_id(provider: TerminalProvider) -> Option<ProviderId> {
    match provider {
        TerminalProvider::Claude => Some(ProviderId::Claude),
        TerminalProvider::Codex => Some(ProviderId::Codex),
        TerminalProvider::Cursor => Some(ProviderId::Cursor),
        TerminalProvider::Grok => Some(ProviderId::Grok),
        TerminalProvider::Local => Some(ProviderId::Local),
        TerminalProvider::Shell => None,
    }
}

fn as_provider(value: &str) -> Option<ProviderId> {
    match value {
        "claude" => Some(ProviderId::Claude),
        "codex" => Some(ProviderId::Codex),
        "cursor" => Some(ProviderId::Cursor),
        "grok" => Some(ProviderId::Grok),
        _ => None,
    }
}
