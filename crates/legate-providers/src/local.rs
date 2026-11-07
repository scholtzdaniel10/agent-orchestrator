//! Local models served by llama.cpp's `llama-server` or Ollama.
//!
//! Jobs run through [`liteagent`] in the job folder. Local has no CLI sessions:
//! [`RunOptions::resume`] is ignored and every job starts a fresh conversation.
//!
//! The system prompt is liteagent's [`DEFAULT_SYSTEM`]. The tool list depends
//! only on [`JobAccess`] (names, descriptions, and schemas), not on the job id
//! or the folder path, so a llama-server slot can reuse its prefix cache. One
//! [`Backend`] is created per server URL, model, and context length and cloned
//! into each job so that slot stays warm.
//!
//! Shell commands are one process in the work folder: `powershell -NoProfile
//! -Command` on Windows and `sh -c` elsewhere. They stop after 60 seconds and
//! their combined output is cut at 8,000 characters. `write_file` is jailed
//! lexically to the work folder, the same way liteagent's file tools are.

use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use legate_core::{
    AgentEvent, DEFAULT_LOCAL_MODEL, DEFAULT_LOCAL_NUM_CTX, DEFAULT_LOCAL_URL, FileEdit, JobAccess,
    LOCAL_NUM_CTX_MAX, LOCAL_NUM_CTX_MIN, ModelOption, ProviderId, RunOptions, StepPhase, StepTool,
};
use liteagent::Error as ModelError;
use liteagent::{
    Backend, DEFAULT_SYSTEM, EscalationReason, RunStats, ServerKind, StepEvent, Tool,
    default_tools, object_schema,
};
use serde_json::Value;
use tokio::runtime::Handle;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::adapter::{EventStream, LimitError, ProcessExit, ProviderAdapter, RunHandle};
use crate::edit_size::{cap_detail, edit_size, first_line, relative_to_cwd, write_size};
use crate::util::{job_access, lock, non_empty, path_string};
use crate::values::token_total;

/// Shell commands are killed after this long.
const SHELL_TIMEOUT: Duration = Duration::from_secs(60);
/// Combined shell stdout and stderr, in characters.
const SHELL_OUTPUT_CAP: usize = 8_000;

/// Where local jobs send their prompts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalConfig {
    /// Base URL of llama-server or Ollama, without a trailing slash.
    pub url: String,
    /// Model name. Ollama requires it; llama-server ignores it and keeps the loaded model.
    pub model: String,
    /// When set, liteagent's small model tries each step first.
    pub small_model: Option<String>,
    /// Ollama `options.num_ctx`. llama-server uses the server's own `-c`.
    pub num_ctx: u32,
}

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            url: DEFAULT_LOCAL_URL.to_string(),
            model: DEFAULT_LOCAL_MODEL.to_string(),
            small_model: None,
            num_ctx: ctx_u32(DEFAULT_LOCAL_NUM_CTX),
        }
    }
}

impl LocalConfig {
    /// Copies the local fields from settings. Context length is clamped.
    pub fn from_prefs(prefs: &legate_core::Prefs) -> Self {
        Self {
            url: prefs.local_url.clone(),
            model: prefs.local_model.clone(),
            small_model: prefs.local_small_model.clone(),
            num_ctx: ctx_u32(prefs.local_num_ctx),
        }
    }
}

/// Runs [`ProviderId::Local`] on a model server already running on this machine.
pub struct LocalAdapter {
    inner: Arc<Inner>,
}

struct Inner {
    config: Mutex<LocalConfig>,
    /// Server kind probed once per URL so a second model does not repeat detection.
    kinds: Mutex<HashMap<String, ServerKind>>,
    /// Untouched templates. Jobs clone one and never write back into the map.
    backends: Mutex<HashMap<String, Backend>>,
}

struct Stop {
    flag: AtomicBool,
    /// PID of the shell command currently running, or 0.
    shell_pid: AtomicU32,
}

impl Stop {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            flag: AtomicBool::new(false),
            shell_pid: AtomicU32::new(0),
        })
    }

    fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        let pid = self.shell_pid.load(Ordering::SeqCst);
        if pid != 0 {
            kill_pid(pid);
        }
    }

    fn cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

struct TaskGuard {
    handle: Option<JoinHandle<()>>,
    stop: Arc<Stop>,
}

impl Drop for TaskGuard {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

struct JobLaunch {
    inner: Arc<Inner>,
    job_id: String,
    prompt: String,
    cwd: PathBuf,
    access: JobAccess,
    model: String,
    small_model: Option<String>,
    url: String,
    num_ctx: u32,
    stop: Arc<Stop>,
    events: UnboundedSender<AgentEvent>,
    exit_tx: watch::Sender<Option<ProcessExit>>,
}

struct StepMap {
    cwd: PathBuf,
    next_id: u64,
    open: Vec<OpenStep>,
    edits: Arc<EditLedger>,
}

/// Line counts captured before `write_file` replaces the file.
///
/// liteagent runs the tool before the caller observes `ToolCall`, so reading
/// the path at event time would diff the file against itself.
struct EditLedger {
    notes: Mutex<VecDeque<NotedEdit>>,
}

struct NotedEdit {
    path: String,
    added: i64,
    removed: i64,
}

struct OpenStep {
    id: String,
    name: String,
    tool: StepTool,
    title: String,
    edit: Option<FileEdit>,
}

enum Mapped {
    Events(Vec<AgentEvent>),
    Done {
        answer: Option<String>,
        stats: RunStats,
    },
}

impl LocalAdapter {
    pub fn new(config: LocalConfig) -> Self {
        Self {
            inner: Arc::new(Inner {
                config: Mutex::new(config),
                kinds: Mutex::new(HashMap::new()),
                backends: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn from_prefs(prefs: &legate_core::Prefs) -> Self {
        Self::new(LocalConfig::from_prefs(prefs))
    }

    pub fn config(&self) -> LocalConfig {
        lock(&self.inner.config).clone()
    }

    /// Replaces the URL, models, and context length used by later jobs.
    ///
    /// Backends already built for a different `(url, model, num_ctx)` stay in
    /// the cache until this process exits. A job that asks for the same triple
    /// again reuses the original backend.
    pub fn set_config(&self, config: LocalConfig) {
        *lock(&self.inner.config) = config;
    }

    /// Model names from Ollama `GET /api/tags`. Empty when the server has no tag list.
    pub async fn list_models(&self) -> Vec<ModelOption> {
        let config = self.config();
        let Some(body) = get_json(&config.url, "/api/tags").await else {
            return Vec::new();
        };
        tag_names(&body)
            .into_iter()
            .map(|name| ModelOption {
                label: name.clone(),
                id: name,
            })
            .collect()
    }
}

#[async_trait]
impl ProviderAdapter for LocalAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::Local
    }

    async fn is_installed(&self) -> bool {
        let config = self.config();
        match self
            .inner
            .cached(&config.url, &config.model, config.num_ctx)
            .await
        {
            Ok(backend) => backend.health().await,
            Err(_) => false,
        }
    }

    async fn version(&self) -> Option<String> {
        let config = self.config();
        let backend = self
            .inner
            .cached(&config.url, &config.model, config.num_ctx)
            .await
            .ok()?;
        match backend.kind() {
            ServerKind::Ollama => {
                let body = get_json(backend.url(), "/api/version").await?;
                body.get("version")
                    .and_then(Value::as_str)
                    .and_then(nonempty)
            }
            ServerKind::LlamaServer => {
                let body = get_json(backend.url(), "/props").await?;
                llama_build(&body)
            }
            ServerKind::OpenAi => None,
        }
    }

    async fn is_signed_in(&self) -> bool {
        let config = self.config();
        let Ok(backend) = self
            .inner
            .cached(&config.url, &config.model, config.num_ctx)
            .await
        else {
            return false;
        };
        match backend.kind() {
            ServerKind::Ollama => {
                let Some(body) = get_json(backend.url(), "/api/tags").await else {
                    return false;
                };
                model_present(&config.model, &tag_names(&body))
            }
            ServerKind::LlamaServer => backend.health().await,
            ServerKind::OpenAi => openai_has_model(backend.url(), &config.model).await,
        }
    }

    /// Starts a fresh liteagent conversation in `cwd`. `opts.resume` is ignored.
    fn run(&self, job_id: &str, prompt: &str, cwd: &Path, opts: &RunOptions) -> RunHandle {
        let Ok(runtime) = Handle::try_current() else {
            return RunHandle::failed(
                "a tokio runtime is required to run a local model".to_string(),
            );
        };
        let config = self.config();
        let (event_tx, event_rx) = unbounded_channel();
        let (exit_tx, exit_rx) = watch::channel(None);
        let stop = Stop::new();
        let launch = JobLaunch {
            inner: Arc::clone(&self.inner),
            job_id: job_id.to_string(),
            prompt: prompt.to_string(),
            cwd: cwd.to_path_buf(),
            access: job_access(opts),
            model: chosen_model(&config, opts),
            small_model: small_model_name(&config),
            url: normalize_base(&config.url),
            num_ctx: clamp_ctx(config.num_ctx),
            stop: Arc::clone(&stop),
            events: event_tx,
            exit_tx: exit_tx.clone(),
        };
        let task = runtime.spawn(async move {
            drive(launch).await;
        });
        let guard = Arc::new(Mutex::new(TaskGuard {
            handle: Some(task),
            stop,
        }));
        let kill_guard = Arc::clone(&guard);
        let kill = Arc::new(move || {
            let handle = {
                let mut slot = lock(&kill_guard);
                slot.stop.cancel();
                slot.handle.take()
            };
            if let Some(handle) = handle {
                handle.abort();
            }
            publish_exit(&exit_tx, cancelled_exit());
        });
        drop(guard);
        RunHandle::new(EventStream { rx: event_rx }, exit_rx, kill)
    }

    fn parse_event(&self, _line: &str) -> Option<AgentEvent> {
        None
    }

    fn is_limit_error(&self, error: LimitError<'_>) -> bool {
        match error {
            LimitError::Text(text) => limit_text(text),
            LimitError::Event(AgentEvent::Limit { message, .. }) => limit_text(message),
            LimitError::Event(_) => false,
        }
    }
}

impl Inner {
    async fn cached(&self, url: &str, model: &str, num_ctx: u32) -> Result<Backend, ModelError> {
        let url = normalize_base(url);
        let model = model.trim();
        let num_ctx = clamp_ctx(num_ctx);
        let key = format!("{url}\n{model}\n{num_ctx}");
        if let Some(hit) = lock(&self.backends).get(&key).cloned() {
            return Ok(hit);
        }
        let kind = self.kind_for(&url).await?;
        let mut backend = match kind {
            ServerKind::LlamaServer => Backend::llama(&url),
            ServerKind::Ollama => Backend::ollama(&url),
            ServerKind::OpenAi => Backend::openai(&url),
        };
        backend = backend.with_num_ctx(num_ctx);
        if !model.is_empty() {
            backend = backend.with_model(model);
        }
        let mut guard = lock(&self.backends);
        if let Some(hit) = guard.get(&key) {
            return Ok(hit.clone());
        }
        guard.insert(key, backend.clone());
        Ok(backend)
    }

    async fn kind_for(&self, url: &str) -> Result<ServerKind, ModelError> {
        let url = normalize_base(url);
        if let Some(kind) = lock(&self.kinds).get(&url).copied() {
            return Ok(kind);
        }
        let kind = Backend::connect(&url).await?.kind();
        lock(&self.kinds).insert(url, kind);
        Ok(kind)
    }
}

async fn drive(launch: JobLaunch) {
    if launch.stop.cancelled() {
        return;
    }
    let _ = launch.events.send(AgentEvent::Init {
        session_id: launch.job_id.clone(),
        model: Some(launch.model.clone()),
    });
    let status = match drive_job(&launch).await {
        Ok(status) => status,
        Err(err) => {
            if launch.stop.cancelled() {
                return;
            }
            let (limit, message) = failure_message(&err);
            if limit {
                let _ = launch.events.send(AgentEvent::Limit {
                    resets_at: None,
                    message: message.clone(),
                });
            } else {
                let _ =
                    launch
                        .events
                        .send(plain_result(&launch.job_id, false, &message, None, None));
            }
            ProcessExit {
                code: Some(1),
                stderr: message,
                unreadable: false,
            }
        }
    };
    if launch.stop.cancelled() {
        return;
    }
    let JobLaunch {
        events, exit_tx, ..
    } = launch;
    drop(events);
    publish_exit(&exit_tx, status);
}

async fn drive_job(launch: &JobLaunch) -> Result<ProcessExit, ModelError> {
    let big = launch
        .inner
        .cached(&launch.url, &launch.model, launch.num_ctx)
        .await?
        .with_name("big");
    let small = match &launch.small_model {
        Some(name) => Some(
            launch
                .inner
                .cached(&launch.url, name, launch.num_ctx)
                .await?
                .with_name("small"),
        ),
        None => None,
    };
    if launch.stop.cancelled() {
        return Ok(cancelled_exit());
    }
    let edits = EditLedger::new();
    let tools = tools_for(
        &launch.cwd,
        launch.access,
        Arc::clone(&launch.stop),
        Arc::clone(&edits),
    );
    let mut agent = liteagent::Agent::new(big, small, tools, DEFAULT_SYSTEM);
    let mut stream = agent.run_stream(launch.prompt.clone());
    let mut steps = StepMap::new(launch.cwd.clone(), edits);
    while let Some(item) = stream.next_event().await {
        if launch.stop.cancelled() {
            return Ok(cancelled_exit());
        }
        match item {
            Ok(event) => match steps.apply(event) {
                Mapped::Events(events) => {
                    for event in events {
                        if launch.events.send(event).is_err() {
                            return Ok(cancelled_exit());
                        }
                    }
                }
                Mapped::Done { answer, stats } => return Ok(finish(launch, answer, &stats)),
            },
            Err(err) => return Err(err),
        }
    }
    Ok(message_exit(
        launch,
        "the local model stopped before a final answer",
    ))
}

fn finish(launch: &JobLaunch, answer: Option<String>, stats: &RunStats) -> ProcessExit {
    let ok = answer.is_some();
    let text = match answer {
        Some(text) => text,
        None => "the local model stopped before a final answer".to_string(),
    };
    if ok && !text.is_empty() {
        let _ = launch.events.send(AgentEvent::Text { text: text.clone() });
    }
    let _ = launch.events.send(plain_result(
        &launch.job_id,
        ok,
        &text,
        Some(duration_ms(stats)),
        stats_tokens(stats),
    ));
    ProcessExit {
        code: Some(if ok { 0 } else { 1 }),
        stderr: if ok { String::new() } else { text },
        unreadable: false,
    }
}

fn message_exit(launch: &JobLaunch, text: &str) -> ProcessExit {
    let _ = launch
        .events
        .send(plain_result(&launch.job_id, false, text, None, None));
    ProcessExit {
        code: Some(1),
        stderr: text.to_string(),
        unreadable: false,
    }
}

fn plain_result(
    job_id: &str,
    ok: bool,
    text: &str,
    duration_ms: Option<f64>,
    tokens: Option<i64>,
) -> AgentEvent {
    AgentEvent::Result {
        ok,
        text: text.to_string(),
        session_id: Some(job_id.to_string()),
        duration_ms,
        cost_usd: None,
        tokens,
    }
}

fn duration_ms(stats: &RunStats) -> f64 {
    let ms = stats.wall_s * 1000.0;
    if ms.is_finite() && ms >= 0.0 { ms } else { 0.0 }
}

fn stats_tokens(stats: &RunStats) -> Option<i64> {
    token_total(stats.prompt_tokens as f64, stats.gen_tokens as f64)
}

impl EditLedger {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            notes: Mutex::new(VecDeque::new()),
        })
    }

    fn push(&self, path: &str, added: i64, removed: i64) {
        lock(&self.notes).push_back(NotedEdit {
            path: path.to_string(),
            added,
            removed,
        });
    }

    fn take(&self, path: &str) -> Option<NotedEdit> {
        let mut notes = lock(&self.notes);
        let index = notes.iter().position(|note| note.path == path)?;
        notes.remove(index)
    }
}

impl StepMap {
    fn new(cwd: PathBuf, edits: Arc<EditLedger>) -> Self {
        Self {
            cwd,
            next_id: 0,
            open: Vec::new(),
            edits,
        }
    }

    fn apply(&mut self, event: StepEvent) -> Mapped {
        match event {
            StepEvent::ToolCall { name, args, .. } => {
                let tool = map_tool(&name);
                let title = tool_title(&name, tool, &args, &self.cwd);
                let edit = if tool == StepTool::Edit {
                    edit_for(&self.cwd, &args, &self.edits)
                } else {
                    None
                };
                let detail = if tool == StepTool::Shell {
                    arg_str(&args, &["command", "cmd"]).and_then(detail_of)
                } else {
                    None
                };
                let id = self.alloc_id();
                let start = step_event(
                    id.clone(),
                    StepPhase::Start,
                    tool,
                    title.clone(),
                    detail,
                    None,
                    edit.clone(),
                );
                self.open.push(OpenStep {
                    id,
                    name,
                    tool,
                    title,
                    edit,
                });
                Mapped::Events(vec![start])
            }
            StepEvent::ToolResult { name, result } => {
                let open = self.take_open(&name);
                let (id, tool, title, edit) = match open {
                    Some(open) => (open.id, open.tool, open.title, open.edit),
                    None => (self.alloc_id(), map_tool(&name), name, None),
                };
                Mapped::Events(vec![step_event(
                    id,
                    StepPhase::End,
                    tool,
                    title,
                    detail_of(&result),
                    Some(tool_succeeded(&result)),
                    edit,
                )])
            }
            StepEvent::Escalation { reason } => Mapped::Events(vec![self.escalation(reason)]),
            StepEvent::Repeat { name, prior_result } => {
                let id = self.alloc_id();
                Mapped::Events(vec![step_event(
                    id,
                    StepPhase::End,
                    StepTool::Other,
                    format!("Repeated {name}"),
                    detail_of(&prior_result),
                    Some(true),
                    None,
                )])
            }
            StepEvent::Final { answer, stats, .. } => Mapped::Done { answer, stats },
        }
    }

    fn escalation(&mut self, reason: EscalationReason) -> AgentEvent {
        let (title, detail) = match reason {
            EscalationReason::Invalid => (
                "Asked the larger model".to_string(),
                Some("the small model returned invalid JSON".to_string()),
            ),
            EscalationReason::Asked { reason } => {
                ("Asked the larger model".to_string(), detail_of(&reason))
            }
            EscalationReason::UnknownTool { name } => (
                format!("Asked the larger model about {name}"),
                Some(format!("unknown tool {name}")),
            ),
            EscalationReason::Repeated { name } => (
                format!("Asked the larger model about {name}"),
                Some(format!("repeated {name}")),
            ),
        };
        step_event(
            self.alloc_id(),
            StepPhase::End,
            StepTool::Think,
            title,
            detail,
            Some(true),
            None,
        )
    }

    fn alloc_id(&mut self) -> String {
        self.next_id = self.next_id.saturating_add(1);
        self.next_id.to_string()
    }

    fn take_open(&mut self, name: &str) -> Option<OpenStep> {
        if let Some(index) = self.open.iter().rposition(|step| step.name == name) {
            return Some(self.open.remove(index));
        }
        self.open.pop()
    }
}

fn step_event(
    id: String,
    phase: StepPhase,
    tool: StepTool,
    title: String,
    detail: Option<String>,
    ok: Option<bool>,
    edit: Option<FileEdit>,
) -> AgentEvent {
    AgentEvent::Step {
        id,
        phase,
        tool,
        title,
        detail,
        ok,
        edit,
    }
}

fn map_tool(name: &str) -> StepTool {
    match name.to_ascii_lowercase().as_str() {
        "read_file" | "list_files" | "read" => StepTool::Read,
        "write" | "write_file" | "write_note" => StepTool::Edit,
        "shell" | "bash" | "powershell" => StepTool::Shell,
        _ => StepTool::Other,
    }
}

fn tool_title(name: &str, tool: StepTool, args: &Value, cwd: &Path) -> String {
    let cwd_text = path_string(cwd);
    match tool {
        StepTool::Read => {
            let raw = arg_str(args, &["path", "file_path", "folder", "dir"]).unwrap_or("");
            if raw.is_empty() {
                "Read".to_string()
            } else {
                format!("Read {}", relative_to_cwd(raw, Some(&cwd_text)))
            }
        }
        StepTool::Edit => {
            let raw = arg_str(args, &["path", "file_path"]).unwrap_or("");
            if raw.is_empty() {
                "Edited".to_string()
            } else {
                format!("Edited {}", relative_to_cwd(raw, Some(&cwd_text)))
            }
        }
        StepTool::Shell => {
            let command = arg_str(args, &["command", "cmd"]).unwrap_or("");
            if command.is_empty() {
                "shell".to_string()
            } else {
                format!("Ran {}", first_line(command))
            }
        }
        _ => name.to_string(),
    }
}

fn edit_for(cwd: &Path, args: &Value, edits: &EditLedger) -> Option<FileEdit> {
    let raw = arg_str(args, &["path", "file_path"])?;
    if raw.is_empty() {
        return None;
    }
    let resolved = resolve_under(cwd, raw).ok()?;
    let text = arg_str(args, &["text", "content"]).unwrap_or("");
    let (added, removed) = if let Some(note) = edits.take(raw) {
        (note.added, note.removed)
    } else if resolved.is_file() {
        match std::fs::read_to_string(&resolved) {
            Ok(old) => {
                let size = edit_size(&old, text);
                (size.added, size.removed)
            }
            Err(_) => {
                let size = write_size(text);
                (size.added, size.removed)
            }
        }
    } else {
        let size = write_size(text);
        (size.added, size.removed)
    };
    Some(FileEdit {
        path: relative_to_cwd(raw, Some(&path_string(cwd))),
        added,
        removed,
    })
}

fn detail_of(text: &str) -> Option<String> {
    if text.is_empty() {
        None
    } else {
        Some(cap_detail(text).to_string())
    }
}

fn tool_succeeded(result: &str) -> bool {
    !result
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("error:")
}

fn tools_for(root: &Path, access: JobAccess, stop: Arc<Stop>, edits: Arc<EditLedger>) -> Vec<Tool> {
    let mut tools: Vec<Tool> = default_tools(root)
        .into_iter()
        .filter(|tool| tool.name != "write_note")
        .collect();
    if matches!(access, JobAccess::Edit | JobAccess::Full) {
        tools.push(write_file_tool(root.to_path_buf(), edits));
    }
    if access == JobAccess::Full {
        tools.push(shell_tool(root.to_path_buf(), stop));
    }
    tools
}

fn write_file_tool(root: PathBuf, edits: Arc<EditLedger>) -> Tool {
    let string = serde_json::json!({"type": "string"});
    Tool::new(
        "write_file",
        "write a text file in the work folder",
        object_schema(&[("path", string.clone()), ("text", string)]),
        move |args| write_file(&root, &args, &edits),
    )
}

fn write_file(root: &Path, args: &Value, edits: &EditLedger) -> Result<String, String> {
    let path = arg_str(args, &["path", "file_path"]).unwrap_or("");
    if path.is_empty() {
        return Err("path must be a string".to_string());
    }
    let text = arg_str(args, &["text", "content"]).unwrap_or("");
    let full = resolve_under(root, path)?;
    let size = if full.is_file() {
        match std::fs::read_to_string(&full) {
            Ok(old) => edit_size(&old, text),
            Err(_) => write_size(text),
        }
    } else {
        write_size(text)
    };
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    std::fs::write(&full, text.as_bytes()).map_err(|err| err.to_string())?;
    edits.push(path, size.added, size.removed);
    Ok("ok".to_string())
}

fn shell_tool(root: PathBuf, stop: Arc<Stop>) -> Tool {
    let string = serde_json::json!({"type": "string"});
    Tool::new(
        "shell",
        "run one shell command in the work folder",
        object_schema(&[("command", string)]),
        move |args| {
            let command = arg_str(&args, &["command", "cmd"]).unwrap_or("");
            if command.trim().is_empty() {
                return Err("command must be a non-empty string".to_string());
            }
            Ok(run_shell(&root, command, &stop))
        },
    )
}

fn run_shell(root: &Path, command: &str, stop: &Stop) -> String {
    let root = root.to_path_buf();
    let command = command.to_string();
    block_for_shell(move || execute_shell(&root, &command, stop))
}

fn block_for_shell(work: impl FnOnce() -> String) -> String {
    match Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(work)
        }
        _ => work(),
    }
}

fn execute_shell(root: &Path, command: &str, stop: &Stop) -> String {
    if stop.cancelled() {
        return "error: cancelled".to_string();
    }
    let mut cmd = shell_command(command);
    cmd.current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_window(&mut cmd);
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => return format!("error: {err}"),
    };
    let pid = child.id();
    if pid != 0 {
        stop.shell_pid.store(pid, Ordering::SeqCst);
    }
    if stop.cancelled() {
        kill_pid(pid);
        let _ = child.wait();
        stop.shell_pid.store(0, Ordering::SeqCst);
        return "error: cancelled".to_string();
    }
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let out = match stdout {
            Some(pipe) => drain_capped(pipe, SHELL_OUTPUT_CAP.saturating_mul(4)),
            None => Vec::new(),
        };
        let err = match stderr {
            Some(pipe) => drain_capped(pipe, SHELL_OUTPUT_CAP.saturating_mul(4)),
            None => Vec::new(),
        };
        let status = child.wait();
        let _ = tx.send((status, out, err));
    });
    let started = Instant::now();
    let collected = loop {
        match rx.recv_timeout(Duration::from_millis(40)) {
            Ok(output) => break output,
            Err(RecvTimeoutError::Timeout) => {
                if stop.cancelled() {
                    kill_pid(pid);
                    let _ = rx.recv_timeout(Duration::from_secs(2));
                    stop.shell_pid.store(0, Ordering::SeqCst);
                    return "error: cancelled".to_string();
                }
                if started.elapsed() >= SHELL_TIMEOUT {
                    kill_pid(pid);
                    let _ = rx.recv_timeout(Duration::from_secs(2));
                    stop.shell_pid.store(0, Ordering::SeqCst);
                    return "error: timed out".to_string();
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                stop.shell_pid.store(0, Ordering::SeqCst);
                return "error: shell ended without output".to_string();
            }
        }
    };
    stop.shell_pid.store(0, Ordering::SeqCst);
    match collected {
        (Ok(status), stdout, stderr) => format_output(status, &stdout, &stderr),
        (Err(err), _, _) => format!("error: {err}"),
    }
}

fn drain_capped(mut pipe: impl Read, cap: usize) -> Vec<u8> {
    let mut buf = [0u8; 8192];
    let mut out = Vec::new();
    loop {
        match pipe.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if out.len() < cap {
                    let room = cap - out.len();
                    let take = n.min(room);
                    out.extend_from_slice(&buf[..take]);
                }
            }
        }
    }
    out
}

fn format_output(status: ExitStatus, stdout: &[u8], stderr: &[u8]) -> String {
    let mut text = String::from_utf8_lossy(stdout).into_owned();
    if !stderr.is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(stderr));
    }
    let text = cap_chars(&text, SHELL_OUTPUT_CAP);
    if status.success() {
        if text.is_empty() {
            "ok".to_string()
        } else {
            text
        }
    } else {
        let code = status
            .code()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".to_string());
        if text.is_empty() {
            format!("error: exit {code}")
        } else {
            format!("error: exit {code}\n{text}")
        }
    }
}

fn cap_chars(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max).collect();
    format!("{kept}\n...[trimmed]")
}

fn shell_command(command: &str) -> Command {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-Command", command]);
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", command]);
        cmd
    }
}

fn hide_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

fn kill_pid(pid: u32) {
    if pid == 0 {
        return;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = Command::new("taskkill")
            .args(["/pid", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }
    #[cfg(unix)]
    {
        let Ok(pid) = i32::try_from(pid) else {
            return;
        };
        if pid > 0 {
            // SAFETY: `pid` is a live process id this adapter spawned.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }
}

/// Lexical jail: `..` may not leave `root`. Component-wise, so a sibling
/// folder with `root` as a string prefix is outside.
fn resolve_under(root: &Path, user_path: &str) -> Result<PathBuf, String> {
    if user_path.is_empty() {
        return Err("path is empty".to_string());
    }
    let root_norm = lexical_normal(root);
    let candidate = Path::new(user_path);
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    let full = lexical_normal(&joined);
    if full == root_norm {
        return Err("path is the work folder".to_string());
    }
    if !full.starts_with(&root_norm) {
        return Err("path outside the work folder".to_string());
    }
    Ok(full)
}

fn lexical_normal(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            Component::RootDir => out.push(comp.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(part) => out.push(part),
        }
    }
    out
}

fn arg_str<'a>(args: &'a Value, keys: &[&str]) -> Option<&'a str> {
    let obj = args.as_object()?;
    for key in keys {
        if let Some(Value::String(value)) = obj.get(*key) {
            return Some(value);
        }
    }
    None
}

fn chosen_model(config: &LocalConfig, opts: &RunOptions) -> String {
    if let Some(model) = non_empty(&opts.model) {
        let trimmed = model.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    config.model.trim().to_string()
}

fn small_model_name(config: &LocalConfig) -> Option<String> {
    config.small_model.as_ref().and_then(|name| {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn ctx_u32(value: i64) -> u32 {
    let clamped = value.clamp(LOCAL_NUM_CTX_MIN, LOCAL_NUM_CTX_MAX);
    u32::try_from(clamped).unwrap_or(65_536)
}

fn clamp_ctx(value: u32) -> u32 {
    let min = u32::try_from(LOCAL_NUM_CTX_MIN).unwrap_or(4_096);
    let max = u32::try_from(LOCAL_NUM_CTX_MAX).unwrap_or(262_144);
    value.clamp(min, max)
}

fn normalize_base(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        DEFAULT_LOCAL_URL.to_string()
    } else {
        trimmed.to_string()
    }
}

fn failure_message(err: &ModelError) -> (bool, String) {
    let shown = err.to_string();
    let overflow = matches!(err, ModelError::Status { body, .. } if limit_text(body));
    let transport = matches!(
        err,
        ModelError::Http { .. } | ModelError::Detect { .. } | ModelError::Client(_)
    );
    if limit_text(&shown) || overflow {
        (true, shown)
    } else if transport {
        (true, format!("unreachable server: {shown}"))
    } else {
        (false, shown)
    }
}

fn limit_text(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    const NEEDLES: &[&str] = &[
        "unreachable",
        "connection refused",
        "could not detect a model server",
        "failed to connect",
        "no connection could be made",
        "tcp connect error",
        "os error 10061",
        "os error 111",
        "context length",
        "context size",
        "context window",
        "context overflow",
        "exceeds the available context",
        "exceeds context",
        "prompt is too long",
        "input is too long",
        "maximum context",
        "n_ctx",
    ];
    NEEDLES.iter().any(|needle| lower.contains(needle))
}

fn publish_exit(tx: &watch::Sender<Option<ProcessExit>>, exit: ProcessExit) {
    let mut pending = Some(exit);
    let _ = tx.send_if_modified(|slot| {
        if slot.is_some() {
            return false;
        }
        *slot = pending.take();
        true
    });
}

fn cancelled_exit() -> ProcessExit {
    ProcessExit {
        code: None,
        stderr: String::new(),
        unreadable: false,
    }
}

fn http_client() -> Result<&'static reqwest::Client, String> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .map_err(|err| err.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
}

async fn get_json(url: &str, path: &str) -> Option<Value> {
    let client = http_client().ok()?;
    let full = format!("{}{path}", normalize_base(url));
    let response = client.get(&full).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let text = response.text().await.ok()?;
    if text.trim().is_empty() {
        return None;
    }
    serde_json::from_str(&text).ok()
}

fn nonempty(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn llama_build(props: &Value) -> Option<String> {
    let info = props.get("build_info").or_else(|| props.get("build"))?;
    match info {
        Value::String(text) => nonempty(text),
        Value::Object(map) => {
            let commit = map
                .get("commit")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty());
            let number = map.get("number").and_then(number_text);
            match (number, commit) {
                (Some(number), Some(commit)) => Some(format!("b{number}-{commit}")),
                (None, Some(commit)) => Some(commit.to_string()),
                (Some(number), None) => Some(format!("b{number}")),
                (None, None) => map
                    .get("version")
                    .and_then(Value::as_str)
                    .and_then(nonempty),
            }
        }
        _ => None,
    }
}

fn number_text(value: &Value) -> Option<String> {
    if let Some(number) = value.as_u64() {
        return Some(number.to_string());
    }
    if let Some(number) = value.as_i64() {
        return Some(number.to_string());
    }
    None
}

fn tag_names(body: &Value) -> Vec<String> {
    let Some(models) = body.get("models").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for model in models {
        let Some(name) = model
            .get("name")
            .and_then(Value::as_str)
            .or_else(|| model.get("model").and_then(Value::as_str))
        else {
            continue;
        };
        if let Some(name) = nonempty(name) {
            names.push(name);
        }
    }
    names
}

fn model_present(configured: &str, names: &[String]) -> bool {
    let configured = configured.trim();
    !configured.is_empty() && names.iter().any(|name| same_model(configured, name))
}

fn same_model(configured: &str, listed: &str) -> bool {
    if configured.eq_ignore_ascii_case(listed) {
        return true;
    }
    let (bare, _) = split_tag(listed);
    if !bare.is_empty() && configured.eq_ignore_ascii_case(bare) {
        return true;
    }
    let (want, want_tag) = split_tag(configured);
    let (got, got_tag) = split_tag(listed);
    !want.is_empty() && want.eq_ignore_ascii_case(got) && want_tag.eq_ignore_ascii_case(got_tag)
}

fn split_tag(name: &str) -> (&str, &str) {
    match name.split_once(':') {
        Some((bare, tag)) => (bare, tag),
        None => (name, ""),
    }
}

async fn openai_has_model(url: &str, model: &str) -> bool {
    let Some(body) = get_json(url, "/v1/models").await else {
        return false;
    };
    body.get("data")
        .and_then(Value::as_array)
        .is_some_and(|rows| {
            rows.iter().any(|row| {
                row.get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| same_model(model, id))
            })
        })
}
