//! Spawn a provider CLI without a shell. Stdout is read line by line; stderr is kept
//! up to the same tail the TypeScript runner keeps. `kill` ends the whole tree.

use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use legate_core::AgentEvent;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::{OnceCell, mpsc::unbounded_channel, watch};

use crate::adapter::{Bin, EventStream, ProcessExit, RunHandle};
use crate::util::lock;

const STDERR_CAP: usize = 64 * 1024;

pub(crate) type LineParser = Arc<dyn Fn(&str) -> Vec<AgentEvent> + Send + Sync>;

#[derive(Debug, Clone)]
pub(crate) struct CapturedRun {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

pub(crate) struct SpawnSpec {
    pub bin: Bin,
    pub args: Vec<String>,
    pub prompt: String,
    pub cwd: std::path::PathBuf,
    pub env: Option<BTreeMap<String, String>>,
    pub parse: LineParser,
    pub cleanup: Option<Box<dyn FnOnce() + Send>>,
}

/// `--version` is started once and reused, until `is_installed` replaces it.
pub(crate) struct VersionCache {
    slot: Mutex<Option<Arc<VersionSlot>>>,
}

struct VersionSlot {
    cell: OnceCell<CapturedRun>,
    bin: Bin,
}

impl VersionCache {
    pub(crate) fn new() -> Self {
        Self {
            slot: Mutex::new(None),
        }
    }

    pub(crate) async fn installed(&self, bin: &Bin) -> bool {
        self.restart(bin).await.code == Some(0)
    }

    pub(crate) async fn stdout(&self, bin: &Bin) -> String {
        let slot = {
            let mut guard = lock(&self.slot);
            if let Some(existing) = guard.clone() {
                existing
            } else {
                let slot = Arc::new(VersionSlot {
                    cell: OnceCell::new(),
                    bin: bin.clone(),
                });
                *guard = Some(Arc::clone(&slot));
                slot
            }
        };
        slot.stdout().await
    }

    async fn restart(&self, bin: &Bin) -> CapturedRun {
        let slot = Arc::new(VersionSlot {
            cell: OnceCell::new(),
            bin: bin.clone(),
        });
        *lock(&self.slot) = Some(Arc::clone(&slot));
        slot.get().await
    }
}

impl VersionSlot {
    async fn stdout(&self) -> String {
        self.get().await.stdout
    }

    async fn get(&self) -> CapturedRun {
        self.cell
            .get_or_init(|| {
                let bin = self.bin.clone();
                async move { run_captured(&bin, &["--version".to_string()]).await }
            })
            .await
            .clone()
    }
}

/// Run a CLI to completion. Spawn errors resolve; they do not panic.
pub(crate) async fn run_captured(bin: &Bin, args: &[String]) -> CapturedRun {
    let mut cmd = Command::new(&bin.command);
    cmd.args(&bin.args)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    hide_window(&mut cmd);

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            return CapturedRun {
                code: None,
                stdout: String::new(),
                stderr: err.to_string(),
            };
        }
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (stdout, stderr, status) =
        tokio::join!(read_to_string(stdout), read_to_string(stderr), child.wait());
    CapturedRun {
        code: status.ok().and_then(|status| status.code()),
        stdout,
        stderr,
    }
}

pub(crate) fn spawn_cli(spec: SpawnSpec) -> RunHandle {
    let mut cmd = Command::new(&spec.bin.command);
    cmd.args(&spec.bin.args)
        .args(&spec.args)
        .current_dir(&spec.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(env) = &spec.env {
        cmd.env_clear();
        cmd.envs(env);
    }
    hide_window(&mut cmd);
    detach_group(&mut cmd);

    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            if let Some(cleanup) = spec.cleanup {
                cleanup();
            }
            return RunHandle::failed(err.to_string());
        }
    };

    let pid = child.id().unwrap_or(0);
    let tree = Arc::new(TreeKill::attach(&child, pid));
    let runtime = match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle,
        Err(_) => {
            tree.kill();
            if let Some(cleanup) = spec.cleanup {
                cleanup();
            }
            return RunHandle::failed("a tokio runtime is required to run a provider".to_string());
        }
    };

    let (event_tx, event_rx) = unbounded_channel();
    let (exit_tx, exit_rx) = watch::channel(None);
    let held = Arc::clone(&tree);
    runtime.spawn(async move {
        drive(child, spec, event_tx, exit_tx, held).await;
    });

    RunHandle::new(
        EventStream { rx: event_rx },
        exit_rx,
        Arc::new(move || tree.kill()),
    )
}

async fn drive(
    mut child: Child,
    mut spec: SpawnSpec,
    event_tx: tokio::sync::mpsc::UnboundedSender<AgentEvent>,
    exit_tx: watch::Sender<Option<ProcessExit>>,
    tree: Arc<TreeKill>,
) {
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdin = child.stdin.take();
    let stderr_buf = Arc::new(Mutex::new(String::new()));
    let had_result = Arc::new(AtomicBool::new(false));
    let prompt = std::mem::take(&mut spec.prompt);
    let parse = spec.parse;

    tokio::join!(
        write_prompt(stdin, prompt),
        read_stdout(stdout, parse, event_tx.clone(), Arc::clone(&had_result)),
        read_stderr(stderr, Arc::clone(&stderr_buf)),
    );

    let status = child.wait().await;
    match &status {
        Ok(_) => tree.disarm(),
        Err(_) => {
            tree.kill();
            tree.disarm();
        }
    }
    if let Some(cleanup) = spec.cleanup.take() {
        cleanup();
    }
    let code = status.ok().and_then(|status| status.code());
    let stderr_text = lock(&stderr_buf).clone();
    let unreadable = code == Some(0) && !had_result.load(Ordering::Acquire);
    let _ = exit_tx.send(Some(ProcessExit {
        code,
        stderr: stderr_text,
        unreadable,
    }));
    drop(event_tx);
}

async fn write_prompt(stdin: Option<ChildStdin>, prompt: String) {
    let Some(mut stdin) = stdin else {
        return;
    };
    let _ = stdin.write_all(prompt.as_bytes()).await;
    let _ = stdin.shutdown().await;
}

async fn read_stdout(
    stdout: Option<ChildStdout>,
    parse: LineParser,
    tx: tokio::sync::mpsc::UnboundedSender<AgentEvent>,
    had_result: Arc<AtomicBool>,
) {
    let Some(mut stdout) = stdout else {
        return;
    };
    let mut pending = String::new();
    let mut chunk = [0u8; 8192];
    let mut carry = Vec::new();
    loop {
        match stdout.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                carry.extend_from_slice(&chunk[..n]);
                drain_utf8(&mut carry, &mut pending, &parse, &tx, &had_result);
            }
        }
    }
    if !carry.is_empty() {
        let tail = String::from_utf8_lossy(&carry);
        push_lines(&mut pending, &tail, &parse, &tx, &had_result);
    }
    if !pending.is_empty() {
        emit_line(&pending, &parse, &tx, &had_result);
    }
}

fn drain_utf8(
    carry: &mut Vec<u8>,
    pending: &mut String,
    parse: &LineParser,
    tx: &tokio::sync::mpsc::UnboundedSender<AgentEvent>,
    had_result: &AtomicBool,
) {
    let valid = match std::str::from_utf8(carry) {
        Ok(text) => {
            if !text.is_empty() {
                push_lines(pending, text, parse, tx, had_result);
            }
            carry.len()
        }
        Err(err) => {
            let end = err.valid_up_to();
            if end > 0
                && let Ok(text) = std::str::from_utf8(&carry[..end])
            {
                push_lines(pending, text, parse, tx, had_result);
            }
            end
        }
    };
    if valid > 0 {
        carry.drain(..valid);
    } else if carry.len() > 8 {
        let lossy = String::from_utf8_lossy(carry);
        push_lines(pending, &lossy, parse, tx, had_result);
        carry.clear();
    }
}

fn push_lines(
    pending: &mut String,
    chunk: &str,
    parse: &LineParser,
    tx: &tokio::sync::mpsc::UnboundedSender<AgentEvent>,
    had_result: &AtomicBool,
) {
    pending.push_str(chunk);
    while let Some(nl) = pending.find('\n') {
        emit_line(&pending[..nl], parse, tx, had_result);
        pending.drain(..nl + 1);
    }
}

fn emit_line(
    line: &str,
    parse: &LineParser,
    tx: &tokio::sync::mpsc::UnboundedSender<AgentEvent>,
    had_result: &AtomicBool,
) {
    let trimmed = crate::util::trim_line(line);
    if trimmed.is_empty() {
        return;
    }
    for event in parse(trimmed) {
        if matches!(event, AgentEvent::Result { .. }) {
            had_result.store(true, Ordering::Release);
        }
        if tx.send(event).is_err() {
            return;
        }
    }
}

async fn read_stderr(stderr: Option<ChildStderr>, buf: Arc<Mutex<String>>) {
    let Some(mut stderr) = stderr else {
        return;
    };
    let mut chunk = [0u8; 8192];
    let mut carry = Vec::new();
    loop {
        match stderr.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                carry.extend_from_slice(&chunk[..n]);
                push_valid(&mut carry, &buf, true);
            }
        }
    }
    if !carry.is_empty() {
        let tail = String::from_utf8_lossy(&carry);
        push_capped(&mut lock(&buf), &tail);
    }
}

fn push_valid(carry: &mut Vec<u8>, buf: &Mutex<String>, cap: bool) {
    let valid = match std::str::from_utf8(carry) {
        Ok(text) => {
            if !text.is_empty() {
                append_text(buf, text, cap);
            }
            carry.len()
        }
        Err(err) => {
            let end = err.valid_up_to();
            if end > 0
                && let Ok(text) = std::str::from_utf8(&carry[..end])
            {
                append_text(buf, text, cap);
            }
            end
        }
    };
    if valid > 0 {
        carry.drain(..valid);
    } else if carry.len() > 8 {
        let lossy = String::from_utf8_lossy(carry).into_owned();
        append_text(buf, &lossy, cap);
        carry.clear();
    }
}

fn append_text(buf: &Mutex<String>, text: &str, cap: bool) {
    let mut guard = lock(buf);
    if cap {
        push_capped(&mut guard, text);
    } else {
        guard.push_str(text);
    }
}

fn push_capped(buf: &mut String, chunk: &str) {
    buf.push_str(chunk);
    if buf.len() <= STDERR_CAP {
        return;
    }
    let mut start = buf.len() - STDERR_CAP;
    while start < buf.len() && !buf.is_char_boundary(start) {
        start += 1;
    }
    buf.drain(..start);
}

async fn read_to_string(pipe: Option<impl AsyncReadExt + Unpin>) -> String {
    let Some(mut pipe) = pipe else {
        return String::new();
    };
    let mut out = String::new();
    let mut chunk = [0u8; 8192];
    let mut carry = Vec::new();
    loop {
        match pipe.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                carry.extend_from_slice(&chunk[..n]);
                let valid = match std::str::from_utf8(&carry) {
                    Ok(text) => {
                        out.push_str(text);
                        carry.len()
                    }
                    Err(err) => {
                        let end = err.valid_up_to();
                        if end > 0
                            && let Ok(text) = std::str::from_utf8(&carry[..end])
                        {
                            out.push_str(text);
                        }
                        end
                    }
                };
                if valid > 0 {
                    carry.drain(..valid);
                } else if carry.len() > 8 {
                    out.push_str(&String::from_utf8_lossy(&carry));
                    carry.clear();
                }
            }
        }
    }
    if !carry.is_empty() {
        out.push_str(&String::from_utf8_lossy(&carry));
    }
    out
}

fn hide_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

fn detach_group(cmd: &mut Command) {
    #[cfg(unix)]
    cmd.process_group(0);
    #[cfg(not(unix))]
    {
        let _ = cmd;
    }
}

struct TreeKill {
    pid: u32,
    armed: AtomicBool,
    #[cfg(windows)]
    job: Option<WinJob>,
    #[cfg(windows)]
    assigned: bool,
}

impl TreeKill {
    fn attach(child: &Child, pid: u32) -> Self {
        #[cfg(windows)]
        {
            let job = WinJob::create();
            let assigned = match (&job, child_handle(child)) {
                (Some(job), Some(handle)) => job.assign(handle),
                _ => false,
            };
            Self {
                pid,
                armed: AtomicBool::new(true),
                job,
                assigned,
            }
        }
        #[cfg(not(windows))]
        {
            let _ = child;
            Self {
                pid,
                armed: AtomicBool::new(true),
            }
        }
    }

    fn disarm(&self) {
        self.armed.store(false, Ordering::Release);
    }

    fn kill(&self) {
        if self.pid == 0 {
            return;
        }
        #[cfg(windows)]
        {
            if self.assigned
                && let Some(job) = &self.job
                && job.terminate()
            {
                return;
            }
            taskkill(self.pid);
        }
        #[cfg(unix)]
        kill_group(self.pid);
    }
}

impl Drop for TreeKill {
    fn drop(&mut self) {
        if !self.armed.load(Ordering::Acquire) {
            return;
        }
        self.kill();
    }
}

#[cfg(windows)]
fn child_handle(child: &Child) -> Option<windows_sys::Win32::Foundation::HANDLE> {
    child
        .raw_handle()
        .map(|handle| handle as windows_sys::Win32::Foundation::HANDLE)
}

#[cfg(windows)]
struct WinJob {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl WinJob {
    fn create() -> Option<Self> {
        use windows_sys::Win32::System::JobObjects::{
            CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };

        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return None;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) };
            return None;
        }
        Some(Self { handle })
    }

    fn assign(&self, process: windows_sys::Win32::Foundation::HANDLE) -> bool {
        unsafe {
            windows_sys::Win32::System::JobObjects::AssignProcessToJobObject(self.handle, process)
                != 0
        }
    }

    fn terminate(&self) -> bool {
        unsafe { windows_sys::Win32::System::JobObjects::TerminateJobObject(self.handle, 1) != 0 }
    }
}

#[cfg(windows)]
impl Drop for WinJob {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.handle) };
    }
}

#[cfg(windows)]
fn taskkill(pid: u32) {
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = std::process::Command::new("taskkill")
        .args(["/pid", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

#[cfg(unix)]
fn kill_group(pid: u32) {
    let Ok(pid) = i32::try_from(pid) else {
        return;
    };
    if pid <= 0 {
        return;
    }
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
}

#[cfg(windows)]
unsafe impl Send for WinJob {}

#[cfg(windows)]
unsafe impl Sync for WinJob {}
