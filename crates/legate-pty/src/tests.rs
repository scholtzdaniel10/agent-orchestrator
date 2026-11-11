//! Ports of `src/core/pty/host.test.ts`, plus the resize-claim rule and a real process-tree kill.
//!
//! A non-string `write` (`12 as unknown as string` in the TypeScript test) cannot be
//! expressed here; empty and over-long writes are still rejected.

use std::collections::BTreeMap;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::oneshot;

use crate::process::{QueuedPty, SpawnOpts};
use crate::schedule::ManualScheduler;
use crate::{
    ChatFn, ExistsFn, FRESH_MARK, KillTree, Launch, LaunchFn, LaunchOpts, OpenOptions, Platform,
    ProviderId, PtyError, PtyHost, PtyHostOptions, PtyProcess, RESTORE_MARK, RunOutcome, RunRow,
    SNAPSHOT_MAX_CHARS, Scheduler, ShellOptions, SpawnPty, Store, TERMINAL_SCROLLBACK_CAP,
    TerminalClaims, TerminalInfo, TerminalProvider, TerminalRow, TerminalStatus, chat_fn,
    spawn_pty,
};

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().copied().map(str::to_string).collect()
}

fn env_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .copied()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn is_uuid(value: &str) -> bool {
    let parts: Vec<&str> = value.split('-').collect();
    parts.len() == 5
        && [8, 4, 4, 4, 12]
            == [
                parts[0].len(),
                parts[1].len(),
                parts[2].len(),
                parts[3].len(),
                parts[4].len(),
            ]
        && value
            .bytes()
            .all(|byte| byte == b'-' || byte.is_ascii_hexdigit())
}

fn memory_store() -> Arc<Mutex<Store>> {
    Arc::new(Mutex::new(Store::open(":memory:").expect("memory store")))
}

fn rows(store: &Arc<Mutex<Store>>, project: &str) -> Vec<TerminalRow> {
    store
        .lock()
        .expect("store")
        .terminals(project)
        .expect("terminals")
}

fn runs(store: &Arc<Mutex<Store>>) -> Vec<RunRow> {
    store.lock().expect("store").runs(None).expect("runs")
}

struct Saved {
    id: String,
    project: String,
    provider: String,
    model: Option<String>,
    session_id: Option<String>,
    title: String,
    scrollback: String,
    cwd: Option<String>,
    change_id: Option<String>,
}

impl Default for Saved {
    fn default() -> Self {
        Self {
            id: "term-1".to_string(),
            project: "work".to_string(),
            provider: "claude".to_string(),
            model: Some("opus".to_string()),
            session_id: Some("old-session".to_string()),
            title: "claude 1".to_string(),
            scrollback: "old".to_string(),
            cwd: None,
            change_id: None,
        }
    }
}

fn put_terminal(store: &Arc<Mutex<Store>>, saved: Saved) -> String {
    let id = saved.id.clone();
    store
        .lock()
        .expect("store")
        .save_terminal(&TerminalRow {
            id: id.clone(),
            project: saved.project,
            provider: saved.provider,
            model: saved.model,
            session_id: saved.session_id,
            title: saved.title,
            created_at: 1,
            updated_at: 1,
            scrollback: saved.scrollback,
            cwd: saved.cwd,
            change_id: saved.change_id,
        })
        .expect("save terminal");
    id
}

struct FakeState {
    pid: u32,
    written: Vec<String>,
    resized: Vec<(u16, u16)>,
    resize_error: bool,
    on_data: Option<Arc<dyn Fn(String) + Send + Sync>>,
    on_exit: Option<Arc<dyn Fn(Option<i64>) + Send + Sync>>,
}

#[derive(Clone)]
struct FakePty {
    inner: Arc<Mutex<FakeState>>,
}

impl FakePty {
    fn new(pid: u32) -> Self {
        Self {
            inner: Arc::new(Mutex::new(FakeState {
                pid,
                written: Vec::new(),
                resized: Vec::new(),
                resize_error: false,
                on_data: None,
                on_exit: None,
            })),
        }
    }

    fn pid(&self) -> u32 {
        self.inner.lock().expect("pty").pid
    }

    fn written(&self) -> Vec<String> {
        self.inner.lock().expect("pty").written.clone()
    }

    fn resized(&self) -> Vec<(u16, u16)> {
        self.inner.lock().expect("pty").resized.clone()
    }

    fn set_resize_error(&self, yes: bool) {
        self.inner.lock().expect("pty").resize_error = yes;
    }

    fn push(&self, data: &str) {
        let cb = self.inner.lock().expect("pty").on_data.clone();
        if let Some(cb) = cb {
            cb(data.to_string());
        }
    }

    fn exit(&self, code: i64) {
        let cb = self.inner.lock().expect("pty").on_exit.clone();
        if let Some(cb) = cb {
            cb(Some(code));
        }
    }
}

impl PtyProcess for FakePty {
    fn pid(&self) -> u32 {
        FakePty::pid(self)
    }

    fn on_data(&mut self, cb: Box<dyn Fn(String) + Send + Sync>) {
        self.inner.lock().expect("pty").on_data = Some(Arc::from(cb));
    }

    fn on_exit(&mut self, cb: Box<dyn Fn(Option<i64>) + Send + Sync>) {
        self.inner.lock().expect("pty").on_exit = Some(Arc::from(cb));
    }

    fn take_queued(&mut self) -> QueuedPty {
        QueuedPty::default()
    }

    fn write(&mut self, data: &str) -> Result<(), PtyError> {
        self.inner
            .lock()
            .expect("pty")
            .written
            .push(data.to_string());
        Ok(())
    }

    fn resize(&mut self, cols: u16, rows: u16) -> Result<(), PtyError> {
        let mut state = self.inner.lock().expect("pty");
        if state.resize_error {
            return Err(PtyError::Message("resize failed".to_string()));
        }
        state.resized.push((cols, rows));
        Ok(())
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct SpawnCall {
    command: String,
    args: Vec<String>,
    opts: SpawnOpts,
}

#[derive(Default)]
struct Parts {
    cwd: Option<Arc<dyn Fn() -> String + Send + Sync>>,
    max_terminals: Option<usize>,
    model_for: Option<Arc<dyn Fn(ProviderId) -> Option<String> + Send + Sync>>,
    store: Option<Arc<Mutex<Store>>>,
    now: Option<Arc<AtomicI64>>,
    launch: Option<LaunchFn>,
    create_chat: Option<ChatFn>,
    kill_tree: Option<KillTree>,
    spawn: Option<SpawnPty>,
    exists: Option<ExistsFn>,
    platform: Option<Platform>,
}

struct Harness {
    host: PtyHost,
    ptys: Arc<Mutex<Vec<FakePty>>>,
    calls: Arc<Mutex<Vec<SpawnCall>>>,
    clock: ManualScheduler,
}

impl Harness {
    fn pty(&self, index: usize) -> FakePty {
        self.ptys.lock().expect("ptys")[index].clone()
    }

    fn calls(&self) -> Vec<SpawnCall> {
        self.calls.lock().expect("calls").clone()
    }

    fn advance(&self, ms: u64) {
        self.clock.advance(Duration::from_millis(ms));
    }
}

fn default_launch(provider: ProviderId, opts: &LaunchOpts) -> Launch {
    let mut args = Vec::new();
    if let Some(resume) = &opts.resume {
        args.push("--resume".to_string());
        args.push(resume.clone());
    }
    if let Some(session_id) = &opts.session_id {
        args.push("--session-id".to_string());
        args.push(session_id.clone());
    }
    if let Some(model) = &opts.model {
        args.push("--model".to_string());
        args.push(model.clone());
    }
    if let Some(prompt) = &opts.prompt {
        args.push(prompt.clone());
    }
    Launch {
        command: provider.as_str().to_string(),
        args,
        env: None,
    }
}

fn make(parts: Parts) -> Harness {
    let ptys = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let clock = ManualScheduler::new();
    let now = parts.now.unwrap_or_else(|| Arc::new(AtomicI64::new(0)));
    let cwd = parts.cwd.unwrap_or_else(|| Arc::new(|| "work".to_string()));
    let launch = parts.launch.unwrap_or_else(|| Arc::new(default_launch));
    let spawn = if let Some(spawn) = parts.spawn {
        spawn
    } else {
        let ptys = Arc::clone(&ptys);
        let calls = Arc::clone(&calls);
        Arc::new(move |command: &str, args: &[String], opts: &SpawnOpts| {
            calls.lock().expect("calls").push(SpawnCall {
                command: command.to_string(),
                args: args.to_vec(),
                opts: opts.clone(),
            });
            let mut list = ptys.lock().expect("ptys");
            let pty = FakePty::new(1000 + u32::try_from(list.len()).unwrap_or(0));
            list.push(pty.clone());
            Ok(Box::new(pty) as Box<dyn PtyProcess>)
        })
    };
    let host = PtyHost::new(PtyHostOptions {
        launch,
        cwd,
        model_for: parts.model_for,
        store: parts.store,
        spawn: Some(spawn),
        kill_tree: parts.kill_tree,
        now: Some(Arc::new(move || now.load(Ordering::Relaxed))),
        scheduler: Some(Arc::new(clock.clone()) as Arc<dyn Scheduler>),
        create_chat: parts.create_chat,
        max_terminals: parts.max_terminals,
        exists: Some(parts.exists.unwrap_or_else(|| Arc::new(|_| true))),
        platform: parts.platform,
    });
    Harness {
        host,
        ptys,
        calls,
        clock,
    }
}

async fn open_default(host: &PtyHost, provider: ProviderId, cols: f64, rows: f64) -> TerminalInfo {
    host.open(provider, cols, rows, OpenOptions::default())
        .await
        .expect("open")
}

async fn yield_until(mut pred: impl FnMut() -> bool) -> bool {
    for _ in 0..50 {
        if pred() {
            return true;
        }
        tokio::task::yield_now().await;
    }
    pred()
}

fn titles(host: &PtyHost) -> Vec<String> {
    host.list().into_iter().map(|info| info.title).collect()
}

fn ids(host: &PtyHost) -> Vec<String> {
    host.list().into_iter().map(|info| info.id).collect()
}

fn work_dir() -> String {
    std::env::current_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .display()
        .to_string()
}

fn tail(text: &str) -> String {
    let count = text.chars().count();
    if count <= 800 {
        text.to_string()
    } else {
        text.chars().skip(count - 800).collect()
    }
}

/// Windows ConPTY asks for the cursor position (`ESC[6n`) and waits before the
/// child prints. A pane would answer; these tests do too, once, until `ready`.
fn answer_cursor_query(host: &PtyHost, id: &str, answered: &mut bool) {
    if *answered {
        return;
    }
    let snap = host.snapshot(id);
    if snap.contains("ready") || snap.contains("child:") {
        return;
    }
    if snap.contains("\u{1b}[6n") {
        host.write(id, "\u{1b}[1;1R");
        *answered = true;
    }
}

fn wait_until(mut pred: impl FnMut() -> bool, timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        if pred() {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn process_alive(pid: u32) -> bool {
    pid != 0 && process_alive_os(pid)
}

#[cfg(windows)]
fn process_alive_os(pid: u32) -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Ok(output) = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    else {
        return false;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    text.contains(&format!("\"{pid}\""))
}

#[cfg(not(windows))]
fn process_alive_os(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn child_pid(snapshot: &str) -> Option<u32> {
    let rest = snapshot.rsplit("child:").next()?;
    let digits: String = rest
        .chars()
        .skip_while(|ch| ch.is_whitespace())
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

fn echo_launch() -> Launch {
    if cfg!(windows) {
        Launch {
            command: "cmd.exe".to_string(),
            args: strings(&[
                "/D",
                "/Q",
                "/V:ON",
                "/C",
                "echo ready& set /p LINE=& echo got:!LINE!& exit /b 0",
            ]),
            env: None,
        }
    } else {
        Launch {
            command: "sh".to_string(),
            args: strings(&[
                "-c",
                "printf 'ready\\n'; IFS= read -r line; printf 'got:%s\\n' \"$line\"; exit 0",
            ]),
            env: None,
        }
    }
}

fn tree_launch() -> Launch {
    if cfg!(windows) {
        Launch {
            command: "powershell.exe".to_string(),
            args: strings(&[
                "-NoLogo",
                "-NoProfile",
                "-Command",
                "$p = Start-Process -FilePath \"$env:SystemRoot\\System32\\ping.exe\" -ArgumentList '-n','40','127.0.0.1' -PassThru -WindowStyle Hidden; Write-Output ('child:' + $p.Id); Start-Sleep -Seconds 40",
            ]),
            env: None,
        }
    } else {
        Launch {
            command: "sh".to_string(),
            args: strings(&["-c", "sleep 40 & echo child:$!; sleep 40"]),
            env: None,
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn open_returns_running_terminals_passes_the_model_and_clamps_the_spawn() {
    let cwd = "work".to_string();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_launch = Arc::clone(&seen);
    let now = Arc::new(AtomicI64::new(5_000));
    let h = make(Parts {
        cwd: Some(Arc::new(move || cwd.clone())),
        now: Some(Arc::clone(&now)),
        model_for: Some(Arc::new(|provider| {
            if provider == ProviderId::Claude {
                Some("opus".to_string())
            } else {
                Some("fast".to_string())
            }
        })),
        launch: Some(Arc::new(move |provider, opts| {
            seen_launch
                .lock()
                .expect("seen")
                .push((provider, opts.model.clone()));
            Launch {
                command: if provider == ProviderId::Claude {
                    "claude.exe".to_string()
                } else {
                    "node.exe".to_string()
                },
                args: {
                    let mut args = Vec::new();
                    if provider == ProviderId::Cursor {
                        args.push("index.js".to_string());
                    }
                    if let Some(model) = &opts.model {
                        args.push("--model".to_string());
                        args.push(model.clone());
                    }
                    args
                },
                env: Some(env_map(&[
                    ("PATH", "bin"),
                    ("NO_COLOR", "1"),
                    ("FORCE_COLOR", "0"),
                    ("KEEP", "yes"),
                ])),
            }
        })),
        ..Parts::default()
    });
    let updates = Arc::new(Mutex::new(Vec::new()));
    let updates_cb = Arc::clone(&updates);
    let _updates = h.host.on_update(move |info, removed| {
        updates_cb
            .lock()
            .expect("updates")
            .push((info.title.clone(), removed));
    });

    let first = open_default(&h.host, ProviderId::Claude, 80.9, 24.2).await;
    let second = open_default(&h.host, ProviderId::Claude, 0.0, 9_000.0).await;
    let third = open_default(&h.host, ProviderId::Cursor, f64::NAN, 10.0).await;

    assert_eq!(first.provider, TerminalProvider::Claude);
    assert_eq!(first.title, "claude 1");
    assert_eq!(first.status, TerminalStatus::Running);
    assert_eq!(first.exit_code, None);
    assert_eq!(first.model.as_deref(), Some("opus"));
    assert_eq!(first.started_at, 5_000);
    assert!(is_uuid(&first.id));
    assert_eq!(second.provider, TerminalProvider::Claude);
    assert_eq!(second.title, "claude 2");
    assert_eq!(second.model.as_deref(), Some("opus"));
    assert_eq!(third.provider, TerminalProvider::Cursor);
    assert_eq!(third.title, "cursor 1");
    assert_eq!(third.model.as_deref(), Some("fast"));
    assert_eq!(
        *seen.lock().expect("seen"),
        vec![
            (ProviderId::Claude, Some("opus".to_string())),
            (ProviderId::Claude, Some("opus".to_string())),
            (ProviderId::Cursor, Some("fast".to_string())),
        ]
    );
    let expected_env = env_map(&[("PATH", "bin"), ("KEEP", "yes"), ("COLORTERM", "truecolor")]);
    let calls = h.calls();
    assert_eq!(calls[0].command, "claude.exe");
    assert_eq!(calls[0].args, strings(&["--model", "opus"]));
    assert_eq!(calls[0].opts.name, "xterm-256color");
    assert_eq!((calls[0].opts.cols, calls[0].opts.rows), (80, 24));
    assert_eq!(calls[0].opts.cwd, "work");
    assert_eq!(calls[0].opts.env, expected_env);
    assert_eq!(calls[1].command, "claude.exe");
    assert_eq!((calls[1].opts.cols, calls[1].opts.rows), (2, 300));
    assert_eq!(calls[2].command, "node.exe");
    assert_eq!(calls[2].args, strings(&["index.js", "--model", "fast"]));
    assert_eq!((calls[2].opts.cols, calls[2].opts.rows), (100, 30));
    assert_eq!(
        titles(&h.host),
        strings(&["claude 1", "claude 2", "cursor 1"])
    );
    let mut listed = h.host.list();
    listed[0].title = "mutated".to_string();
    assert_eq!(h.host.list()[0].title, "claude 1");
    assert_ne!(listed[0].title, h.host.list()[0].title);
    assert_eq!(
        *updates.lock().expect("updates"),
        vec![
            ("claude 1".to_string(), false),
            ("claude 2".to_string(), false),
            ("cursor 1".to_string(), false),
        ]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cwd_is_read_when_a_terminal_opens() {
    let dir = Arc::new(Mutex::new("one".to_string()));
    let dir_for_cwd = Arc::clone(&dir);
    let h = make(Parts {
        cwd: Some(Arc::new(move || dir_for_cwd.lock().expect("dir").clone())),
        ..Parts::default()
    });
    open_default(&h.host, ProviderId::Claude, 80.0, 24.0).await;
    *dir.lock().expect("dir") = "two".to_string();
    open_default(&h.host, ProviderId::Cursor, 80.0, 24.0).await;
    let cwds: Vec<_> = h.calls().into_iter().map(|call| call.opts.cwd).collect();
    assert_eq!(cwds, strings(&["one", "two"]));
}

#[tokio::test(flavor = "current_thread")]
async fn open_throws_at_the_terminal_limit() {
    let h = make(Parts::default());
    for i in 0..16 {
        let provider = if i % 2 == 0 {
            ProviderId::Claude
        } else {
            ProviderId::Cursor
        };
        open_default(&h.host, provider, 80.0, 24.0).await;
    }
    assert_eq!(h.host.list().len(), 16);
    let err = h
        .host
        .open(ProviderId::Claude, 80.0, 24.0, OpenOptions::default())
        .await
        .expect_err("limit");
    assert_eq!(err.to_string(), "too many terminals");
    assert_eq!(h.host.list().len(), 16);
    assert_eq!(h.calls().len(), 16);
}

#[tokio::test(flavor = "current_thread")]
async fn a_throwing_spawn_names_the_provider_and_registers_nothing() {
    let fail = Arc::new(AtomicBool::new(true));
    let owned = Arc::new(Mutex::new(Vec::new()));
    let fail_spawn = Arc::clone(&fail);
    let owned_spawn = Arc::clone(&owned);
    let h = make(Parts {
        spawn: Some(Arc::new(move |_command, _args, _opts| {
            if fail_spawn.load(Ordering::SeqCst) {
                return Err(PtyError::Message("CreateProcess failed".to_string()));
            }
            let pty = FakePty::new(7);
            owned_spawn.lock().expect("ptys").push(pty.clone());
            Ok(Box::new(pty) as Box<dyn PtyProcess>)
        })),
        ..Parts::default()
    });
    let claude = h
        .host
        .open(ProviderId::Claude, 40.0, 12.0, OpenOptions::default())
        .await
        .expect_err("claude");
    assert_eq!(
        claude.to_string(),
        "could not start claude: CreateProcess failed"
    );
    let cursor = h
        .host
        .open(ProviderId::Cursor, 40.0, 12.0, OpenOptions::default())
        .await
        .expect_err("cursor");
    assert_eq!(
        cursor.to_string(),
        "could not start cursor: CreateProcess failed"
    );
    assert!(h.host.list().is_empty());
    fail.store(false, Ordering::SeqCst);
    let info = open_default(&h.host, ProviderId::Claude, 40.0, 12.0).await;
    assert_eq!(info.title, "claude 1");
    assert_eq!(h.host.list().len(), 1);
    assert_eq!(owned.lock().expect("ptys").len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn pty_data_is_coalesced_into_one_flush_and_the_snapshot_keeps_the_tail() {
    let h = make(Parts::default());
    let info = open_default(&h.host, ProviderId::Claude, 80.0, 24.0).await;
    let chunks = Arc::new(Mutex::new(Vec::new()));
    let chunks_cb = Arc::clone(&chunks);
    let _data = h.host.on_data(move |id, data| {
        chunks_cb
            .lock()
            .expect("chunks")
            .push(format!("{id}:{data}"));
    });
    let mut live = h.host.subscribe();
    let pty = h.pty(0);
    pty.push("a");
    pty.push("b");
    assert!(chunks.lock().expect("chunks").is_empty());
    assert_eq!(h.host.snapshot(&info.id), "ab");
    h.advance(11);
    assert!(chunks.lock().expect("chunks").is_empty());
    assert!(live.try_recv().is_err());
    h.advance(1);
    assert_eq!(
        *chunks.lock().expect("chunks"),
        vec![format!("{}:ab", info.id)]
    );
    let chunk = live.try_recv().expect("broadcast");
    assert_eq!(chunk.id, info.id);
    assert_eq!(chunk.data, "ab");
    pty.push("c");
    h.advance(12);
    assert_eq!(
        *chunks.lock().expect("chunks"),
        vec![format!("{}:ab", info.id), format!("{}:c", info.id)]
    );

    let capped = open_default(&h.host, ProviderId::Cursor, 80.0, 24.0).await;
    h.pty(1).push(&"a".repeat(SNAPSHOT_MAX_CHARS));
    h.pty(1).push("xyz");
    let snap = h.host.snapshot(&capped.id);
    assert_eq!(snap, format!("{}xyz", "a".repeat(SNAPSHOT_MAX_CHARS - 3)));
    assert_eq!(snap.chars().count(), SNAPSHOT_MAX_CHARS);
    assert_eq!(h.host.snapshot("missing"), "");
}

#[tokio::test(flavor = "current_thread")]
async fn write_and_resize_reach_a_running_pty_and_are_ignored_otherwise() {
    let h = make(Parts::default());
    let info = open_default(&h.host, ProviderId::Claude, 80.0, 24.0).await;
    let pty = h.pty(0);
    h.host.write(&info.id, "hi");
    h.host.resize(&info.id, 40.8, 12.0);
    let max = "x".repeat(65_536);
    h.host.write(&info.id, &max);
    h.host.write(&info.id, &"y".repeat(65_537));
    h.host.write(&info.id, "");
    // TypeScript also passes a number; Rust's `write` only accepts `&str`.
    h.host.write("missing", "nope");
    h.host.resize("missing", 10.0, 10.0);
    h.host.resize(&info.id, 1.0, 9_999.0);
    h.host.resize(&info.id, f64::INFINITY, 4.0);
    pty.set_resize_error(true);
    h.host.resize(&info.id, 20.0, 10.0);
    assert_eq!(pty.written(), vec!["hi".to_string(), max]);
    assert_eq!(pty.resized(), vec![(40, 12), (2, 300), (100, 30)]);
    pty.exit(0);
    h.host.write(&info.id, "late");
    h.host.resize(&info.id, 20.0, 10.0);
    assert_eq!(pty.written().len(), 2);
    assert_eq!(pty.resized().len(), 3);
}

#[tokio::test(flavor = "current_thread")]
async fn exit_flushes_pending_output_and_writes_one_terminal_run() {
    let store = memory_store();
    let now = Arc::new(AtomicI64::new(10_000));
    let h = make(Parts {
        store: Some(Arc::clone(&store)),
        now: Some(Arc::clone(&now)),
        ..Parts::default()
    });
    let order = Arc::new(Mutex::new(Vec::new()));
    let order_data = Arc::clone(&order);
    let _data = h.host.on_data(move |_, _| {
        order_data.lock().expect("order").push("data".to_string());
    });
    let order_update = Arc::clone(&order);
    let _updates = h.host.on_update(move |info, _| {
        let status = match info.status {
            TerminalStatus::Running => "running",
            TerminalStatus::Exited => "exited",
            TerminalStatus::Sleeping => "sleeping",
        };
        order_update.lock().expect("order").push(status.to_string());
    });
    let ok = open_default(&h.host, ProviderId::Claude, 80.0, 24.0).await;
    assert_eq!(*order.lock().expect("order"), strings(&["running"]));
    h.pty(0).push("hi");
    assert_eq!(h.host.snapshot(&ok.id), "hi");
    assert_eq!(*order.lock().expect("order"), strings(&["running"]));
    now.store(12_500, Ordering::Relaxed);
    h.pty(0).exit(0);
    assert_eq!(
        *order.lock().expect("order"),
        strings(&["running", "data", "exited"])
    );
    let listed = h.host.list().remove(0);
    assert_eq!(listed.status, TerminalStatus::Exited);
    assert_eq!(listed.exit_code, Some(0));
    assert_eq!(listed.id, ok.id);

    now.store(13_000, Ordering::Relaxed);
    let bad = open_default(&h.host, ProviderId::Cursor, 80.0, 24.0).await;
    now.store(13_400, Ordering::Relaxed);
    h.pty(1).exit(3);
    let logged = runs(&store);
    assert_eq!(logged.len(), 2);
    assert_eq!(logged[0].job_id, ok.id);
    assert_eq!(logged[0].provider, "claude");
    assert_eq!(logged[0].job_type, "terminal");
    assert_eq!(logged[0].started_at, 10_000);
    assert_eq!(logged[0].duration_ms, Some(2_500));
    assert_eq!(logged[0].cost_usd, None);
    assert_eq!(logged[0].tokens, None);
    assert_eq!(logged[0].utilization, None);
    assert_eq!(logged[0].outcome, RunOutcome::Ok);
    assert_eq!(logged[0].session_id, None);
    assert_eq!(logged[1].job_id, bad.id);
    assert_eq!(logged[1].provider, "cursor");
    assert_eq!(logged[1].job_type, "terminal");
    assert_eq!(logged[1].started_at, 13_000);
    assert_eq!(logged[1].duration_ms, Some(400));
    assert_eq!(logged[1].outcome, RunOutcome::Error);
    assert_eq!(logged[1].session_id, None);
    h.pty(0).exit(0);
    h.pty(1).exit(1);
    assert_eq!(runs(&store).len(), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn close_kills_the_tree_removes_the_terminal_and_still_logs_one_ok_row() {
    let store = memory_store();
    let killed = Arc::new(Mutex::new(Vec::new()));
    let killed_cb = Arc::clone(&killed);
    let now = Arc::new(AtomicI64::new(100));
    let h = make(Parts {
        store: Some(Arc::clone(&store)),
        now: Some(now),
        kill_tree: Some(Arc::new(move |proc| {
            killed_cb.lock().expect("killed").push(proc.pid());
        })),
        ..Parts::default()
    });
    let removed = Arc::new(Mutex::new(Vec::new()));
    let info = open_default(&h.host, ProviderId::Claude, 80.0, 24.0).await;
    let removed_cb = Arc::clone(&removed);
    let _updates = h.host.on_update(move |next, gone| {
        if gone {
            removed_cb.lock().expect("removed").push(next.clone());
        }
    });
    h.host.close(&info.id);
    assert_eq!(*killed.lock().expect("killed"), vec![h.pty(0).pid()]);
    assert!(h.host.list().is_empty());
    let gone = removed.lock().expect("removed").clone();
    assert_eq!(gone.len(), 1);
    assert_eq!(gone[0].id, info.id);
    assert_eq!(gone[0].status, TerminalStatus::Running);
    h.pty(0).exit(1);
    let logged = runs(&store);
    assert_eq!(logged.len(), 1);
    assert_eq!(logged[0].job_id, info.id);
    assert_eq!(logged[0].provider, "claude");
    assert_eq!(logged[0].job_type, "terminal");
    assert_eq!(logged[0].outcome, RunOutcome::Ok);
    assert_eq!(logged[0].cost_usd, None);
    assert_eq!(logged[0].tokens, None);
    assert_eq!(logged[0].utilization, None);
    assert_eq!(logged[0].session_id, None);
    h.host.close(&info.id);
    h.host.close("missing");
    assert_eq!(killed.lock().expect("killed").len(), 1);
    assert_eq!(runs(&store).len(), 1);

    open_default(&h.host, ProviderId::Cursor, 80.0, 24.0).await;
    open_default(&h.host, ProviderId::Claude, 80.0, 24.0).await;
    h.host.close_all();
    assert!(h.host.list().is_empty());
    assert_eq!(killed.lock().expect("killed").len(), 3);
    h.pty(1).exit(9);
    h.pty(2).exit(0);
    let logged = runs(&store);
    assert_eq!(logged.len(), 3);
    assert!(
        logged
            .iter()
            .all(|row| row.outcome == RunOutcome::Ok && row.job_type == "terminal")
    );
}

#[test]
fn terminal_env_drops_no_colour_variables_and_sets_colorterm() {
    let filtered = crate::terminal_env(&env_map(&[
        ("PATH", "bin"),
        ("NO_COLOR", "1"),
        ("no_color", "1"),
        ("FORCE_COLOR", "0"),
        ("CLICOLOR", "0"),
        ("TERM", "dumb"),
    ]));
    assert_eq!(
        filtered,
        env_map(&[
            ("PATH", "bin"),
            ("TERM", "dumb"),
            ("COLORTERM", "truecolor")
        ])
    );
    assert_eq!(
        crate::terminal_env(&env_map(&[("FORCE_COLOR", "1"), ("CLICOLOR", "1")])),
        env_map(&[
            ("FORCE_COLOR", "1"),
            ("CLICOLOR", "1"),
            ("COLORTERM", "truecolor")
        ])
    );
    assert_eq!(
        crate::terminal_env(&env_map(&[("COLORTERM", "8bit")]))
            .get("COLORTERM")
            .map(String::as_str),
        Some("truecolor")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_throwing_listener_does_not_stop_the_others() {
    let h = make(Parts::default());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_cb = Arc::clone(&seen);
    let _boom_data = h.host.on_data(|_, _| panic!("data listener"));
    let _data = h.host.on_data(move |_, data| {
        seen_cb.lock().expect("seen").push(data.to_string());
    });
    let titles_seen = Arc::new(Mutex::new(Vec::new()));
    let titles_cb = Arc::clone(&titles_seen);
    let _boom_update = h.host.on_update(|_, _| panic!("update listener"));
    let _updates = h.host.on_update(move |info, _| {
        titles_cb.lock().expect("titles").push(info.title.clone());
    });
    let info = open_default(&h.host, ProviderId::Claude, 80.0, 24.0).await;
    h.pty(0).push("q");
    h.advance(12);
    assert_eq!(*seen.lock().expect("seen"), strings(&["q"]));
    assert_eq!(*titles_seen.lock().expect("titles"), vec![info.title]);
    assert!(h.host.logged_errors() >= 2);
}

#[tokio::test(flavor = "current_thread")]
async fn a_real_pty_echoes_a_line_and_exits() {
    let spec = echo_launch();
    let host = PtyHost::new(PtyHostOptions {
        launch: Arc::new(move |_provider, _opts| spec.clone()),
        cwd: Arc::new(work_dir),
        model_for: None,
        store: None,
        spawn: None,
        kill_tree: None,
        now: None,
        scheduler: None,
        create_chat: None,
        max_terminals: None,
        exists: None,
        platform: None,
    });
    let exits = Arc::new(Mutex::new(Vec::new()));
    let exits_cb = Arc::clone(&exits);
    let _updates = host.on_update(move |info, removed| {
        if !removed && info.status == TerminalStatus::Exited {
            exits_cb.lock().expect("exits").push(info.clone());
        }
    });
    let info = open_default(&host, ProviderId::Claude, 80.0, 24.0).await;
    let mut answered = false;
    let ready = wait_until(
        || {
            answer_cursor_query(&host, &info.id, &mut answered);
            host.snapshot(&info.id).contains("ready")
        },
        Duration::from_secs(15),
    );
    assert!(ready, "snapshot tail: {}", tail(&host.snapshot(&info.id)));
    host.resize(&info.id, 40.0, 12.0);
    host.write(&info.id, "ping\r\n");
    let done = wait_until(
        || {
            host.snapshot(&info.id).contains("got:ping")
                && exits
                    .lock()
                    .expect("exits")
                    .iter()
                    .any(|row| row.exit_code == Some(0))
        },
        Duration::from_secs(15),
    );
    assert!(done, "snapshot tail: {}", tail(&host.snapshot(&info.id)));
    assert!(exits.lock().expect("exits").iter().any(|row| {
        row.id == info.id && row.status == TerminalStatus::Exited && row.exit_code == Some(0)
    }));
    host.close_all();
}

#[tokio::test(flavor = "current_thread")]
async fn claude_open_passes_session_id() {
    let h = make(Parts::default());
    open_default(&h.host, ProviderId::Claude, 80.0, 24.0).await;
    let args = h.calls()[0].args.clone();
    assert_eq!(args[0], "--session-id");
    assert!(is_uuid(&args[1]));
}

#[tokio::test(flavor = "current_thread")]
async fn cursor_open_calls_create_chat_then_passes_resume() {
    let created = Arc::new(AtomicUsize::new(0));
    let created_cb = Arc::clone(&created);
    let h = make(Parts {
        create_chat: Some(chat_fn(move || {
            created_cb.fetch_add(1, Ordering::SeqCst);
            async { Ok("chat-abc-12345678".to_string()) }
        })),
        ..Parts::default()
    });
    open_default(&h.host, ProviderId::Cursor, 80.0, 24.0).await;
    assert_eq!(created.load(Ordering::SeqCst), 1);
    assert_eq!(
        h.calls()[0].args,
        strings(&["--resume", "chat-abc-12345678"])
    );
}

#[tokio::test(flavor = "current_thread")]
async fn create_chat_failure_still_opens_a_terminal() {
    let h = make(Parts {
        create_chat: Some(chat_fn(|| async {
            Err(PtyError::Message("offline".to_string()))
        })),
        ..Parts::default()
    });
    let info = open_default(&h.host, ProviderId::Cursor, 80.0, 24.0).await;
    assert_eq!(info.status, TerminalStatus::Running);
    assert_eq!(h.calls().len(), 1);
    assert!(h.calls()[0].args.is_empty());

    let empty = make(Parts {
        create_chat: Some(chat_fn(|| async { Ok("not an id".to_string()) })),
        ..Parts::default()
    });
    open_default(&empty.host, ProviderId::Cursor, 80.0, 24.0).await;
    assert!(empty.calls()[0].args.is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn a_row_is_written_on_open() {
    let store = memory_store();
    let now = Arc::new(AtomicI64::new(50));
    let h = make(Parts {
        store: Some(Arc::clone(&store)),
        cwd: Some(Arc::new(|| "/proj".to_string())),
        now: Some(now),
        model_for: Some(Arc::new(|_| Some("opus".to_string()))),
        create_chat: Some(chat_fn(|| async { Ok("cursor-chat-0001".to_string()) })),
        ..Parts::default()
    });
    let claude = open_default(&h.host, ProviderId::Claude, 80.0, 24.0).await;
    let cursor = open_default(&h.host, ProviderId::Cursor, 80.0, 24.0).await;
    let saved = rows(&store, "/proj");
    assert_eq!(saved.len(), 2);
    assert_eq!(saved[0].id, claude.id);
    assert_eq!(saved[0].project, "/proj");
    assert_eq!(saved[0].provider, "claude");
    assert_eq!(saved[0].model.as_deref(), Some("opus"));
    assert_eq!(saved[0].title, "claude 1");
    assert_eq!(saved[0].created_at, 50);
    assert_eq!(saved[0].scrollback, "");
    assert_eq!(saved[0].cwd.as_deref(), Some("/proj"));
    assert_eq!(saved[0].change_id, None);
    assert!(saved[0].session_id.as_deref().is_some_and(is_uuid));
    assert_eq!(saved[1].id, cursor.id);
    assert_eq!(saved[1].provider, "cursor");
    assert_eq!(saved[1].session_id.as_deref(), Some("cursor-chat-0001"));
    assert_eq!(saved[1].title, "cursor 1");
}

#[tokio::test(flavor = "current_thread")]
async fn scrollback_flush_is_throttled_and_happens_on_exit() {
    let store = memory_store();
    let now = Arc::new(AtomicI64::new(1_000));
    let h = make(Parts {
        store: Some(Arc::clone(&store)),
        cwd: Some(Arc::new(|| "work".to_string())),
        now: Some(Arc::clone(&now)),
        ..Parts::default()
    });
    let info = open_default(&h.host, ProviderId::Claude, 80.0, 24.0).await;
    h.pty(0).push("one");
    assert_eq!(rows(&store, "work")[0].scrollback, "");
    h.advance(1_999);
    assert_eq!(rows(&store, "work")[0].scrollback, "");
    now.store(3_000, Ordering::Relaxed);
    h.advance(1);
    assert_eq!(rows(&store, "work")[0].scrollback, "one");
    h.pty(0).push("two");
    h.advance(1_999);
    assert_eq!(rows(&store, "work")[0].scrollback, "one");
    now.store(5_000, Ordering::Relaxed);
    h.advance(1);
    assert_eq!(rows(&store, "work")[0].scrollback, "onetwo");
    h.pty(0).push("three");
    now.store(5_500, Ordering::Relaxed);
    h.pty(0).exit(0);
    assert_eq!(rows(&store, "work")[0].scrollback, "onetwothree");
    assert_eq!(rows(&store, "work")[0].id, info.id);
}

#[tokio::test(flavor = "current_thread")]
async fn close_deletes_the_row_and_close_all_keeps_it() {
    let store = memory_store();
    let h = make(Parts {
        store: Some(Arc::clone(&store)),
        cwd: Some(Arc::new(|| "work".to_string())),
        now: Some(Arc::new(AtomicI64::new(1))),
        ..Parts::default()
    });
    let closed = open_default(&h.host, ProviderId::Claude, 80.0, 24.0).await;
    let kept = open_default(&h.host, ProviderId::Cursor, 80.0, 24.0).await;
    assert_eq!(rows(&store, "work").len(), 2);
    h.host.close(&closed.id);
    assert_eq!(
        rows(&store, "work")
            .into_iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        vec![kept.id.clone()]
    );
    h.host.close_all();
    assert!(h.host.list().is_empty());
    assert_eq!(
        rows(&store, "work")
            .into_iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        vec![kept.id.clone()]
    );
    h.pty(0).exit(0);
    h.pty(1).exit(0);
    assert_eq!(
        rows(&store, "work")
            .into_iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        vec![kept.id]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn restore_relaunches_with_resume_seeds_the_snapshot_keeps_ids_and_is_idempotent() {
    let store = memory_store();
    let first = make(Parts {
        store: Some(Arc::clone(&store)),
        cwd: Some(Arc::new(|| "alpha".to_string())),
        now: Some(Arc::new(AtomicI64::new(10))),
        model_for: Some(Arc::new(|provider| {
            if provider == ProviderId::Claude {
                Some("opus".to_string())
            } else {
                Some("fast".to_string())
            }
        })),
        create_chat: Some(chat_fn(|| async { Ok("cursor-session-01".to_string()) })),
        ..Parts::default()
    });
    let claude = open_default(&first.host, ProviderId::Claude, 80.0, 24.0).await;
    let cursor = open_default(&first.host, ProviderId::Cursor, 80.0, 24.0).await;
    first.pty(0).push("hello claude");
    first.pty(1).push("hello cursor");
    first.advance(2_000);
    first.host.close_all();

    store
        .lock()
        .expect("store")
        .save_terminal(&TerminalRow {
            id: "other".to_string(),
            project: "beta".to_string(),
            provider: "claude".to_string(),
            model: None,
            session_id: Some("other-session".to_string()),
            title: "claude 9".to_string(),
            created_at: 1,
            updated_at: 1,
            scrollback: "nope".to_string(),
            cwd: None,
            change_id: None,
        })
        .expect("save");

    let second = make(Parts {
        store: Some(Arc::clone(&store)),
        cwd: Some(Arc::new(|| "alpha".to_string())),
        now: Some(Arc::new(AtomicI64::new(99))),
        model_for: Some(Arc::new(|_| Some("ignored".to_string()))),
        ..Parts::default()
    });
    let updates = Arc::new(Mutex::new(Vec::new()));
    let updates_cb = Arc::clone(&updates);
    let _updates = second.host.on_update(move |info, removed| {
        if !removed {
            updates_cb.lock().expect("updates").push(info.clone());
        }
    });
    let restored = second.host.restore("alpha", 80.0, 24.0);
    assert_eq!(
        restored
            .iter()
            .map(|info| info.id.clone())
            .collect::<Vec<_>>(),
        vec![claude.id.clone(), cursor.id.clone()]
    );
    assert_eq!(restored[0].id, claude.id);
    assert_eq!(restored[0].provider, TerminalProvider::Claude);
    assert_eq!(restored[0].title, "claude 1");
    assert_eq!(restored[0].model.as_deref(), Some("opus"));
    assert_eq!(restored[0].status, TerminalStatus::Running);
    assert_eq!(restored[0].started_at, 99);
    assert_eq!(restored[1].provider, TerminalProvider::Cursor);
    assert_eq!(restored[1].model.as_deref(), Some("fast"));
    assert_eq!(restored[1].title, "cursor 1");
    let calls = second.calls();
    assert_eq!(calls[0].args[0], "--resume");
    assert!(is_uuid(&calls[0].args[1]));
    assert_eq!(calls[0].args[2..], strings(&["--model", "opus"]));
    assert_eq!(
        calls[1].args,
        strings(&["--resume", "cursor-session-01", "--model", "fast"])
    );
    assert_eq!(
        second.host.snapshot(&claude.id),
        format!("hello claude{RESTORE_MARK}")
    );
    assert_eq!(
        second.host.snapshot(&cursor.id),
        format!("hello cursor{RESTORE_MARK}")
    );
    assert_eq!(
        ids(&second.host),
        vec![claude.id.clone(), cursor.id.clone()]
    );
    assert_eq!(
        updates
            .lock()
            .expect("updates")
            .iter()
            .map(|info| info.id.clone())
            .collect::<Vec<_>>(),
        vec![claude.id, cursor.id]
    );
    assert!(second.host.restore("alpha", 80.0, 24.0).is_empty());
    assert_eq!(second.calls().len(), 2);
    assert_eq!(second.host.list().len(), 2);

    let other = make(Parts {
        store: Some(store),
        cwd: Some(Arc::new(|| "beta".to_string())),
        now: Some(Arc::new(AtomicI64::new(100))),
        ..Parts::default()
    });
    let beta = other.host.restore("beta", 40.0, 12.0);
    assert_eq!(
        beta.iter().map(|info| info.id.as_str()).collect::<Vec<_>>(),
        vec!["other"]
    );
    assert_eq!(
        other.calls()[0].args,
        strings(&["--resume", "other-session"])
    );
    assert_eq!(second.host.list().len(), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn restored_claude_that_dies_on_probation_relaunches_fresh_in_the_same_tab() {
    let store = memory_store();
    let now = Arc::new(AtomicI64::new(1_000));
    let id = put_terminal(&store, Saved::default());
    let h = make(Parts {
        store: Some(Arc::clone(&store)),
        now: Some(Arc::clone(&now)),
        ..Parts::default()
    });
    let restored = h.host.restore("work", 80.0, 24.0);
    assert_eq!(restored[0].id, id);
    assert_eq!(restored[0].title, "claude 1");
    assert_eq!(restored[0].status, TerminalStatus::Running);
    assert_eq!(h.calls()[0].args[0], "--resume");
    let updates = Arc::new(Mutex::new(Vec::new()));
    let updates_cb = Arc::clone(&updates);
    let _updates = h.host.on_update(move |info, removed| {
        let status = match info.status {
            TerminalStatus::Running => "running",
            TerminalStatus::Exited => "exited",
            TerminalStatus::Sleeping => "sleeping",
        };
        updates_cb
            .lock()
            .expect("updates")
            .push((status.to_string(), removed));
    });
    now.store(3_000, Ordering::Relaxed);
    h.pty(0).exit(1);
    let calls = h.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].args[0], "--session-id");
    assert!(is_uuid(&calls[1].args[1]));
    assert_ne!(calls[1].args[1], "old-session");
    assert_eq!(calls[1].opts.cols, 80);
    assert_eq!(calls[1].opts.rows, 24);
    assert_eq!(calls[1].opts.cwd, "work");
    let listed = h.host.list().remove(0);
    assert_eq!(listed.id, id);
    assert_eq!(listed.title, "claude 1");
    assert_eq!(listed.model.as_deref(), Some("opus"));
    assert_eq!(listed.status, TerminalStatus::Running);
    assert_eq!(listed.exit_code, None);
    assert_eq!(
        h.host.snapshot(&id),
        format!("old{RESTORE_MARK}{FRESH_MARK}")
    );
    let row = rows(&store, "work").remove(0);
    assert!(row.session_id.as_deref().is_some_and(is_uuid));
    assert_ne!(row.session_id.as_deref(), Some("old-session"));
    assert_eq!(row.updated_at, 3_000);
    assert_eq!(
        *updates.lock().expect("updates"),
        vec![("running".to_string(), false)]
    );
    assert!(runs(&store).is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn restored_terminal_that_exits_0_on_probation_stays_exited() {
    let store = memory_store();
    let now = Arc::new(AtomicI64::new(1_000));
    let id = put_terminal(&store, Saved::default());
    let h = make(Parts {
        store: Some(Arc::clone(&store)),
        now: Some(Arc::clone(&now)),
        ..Parts::default()
    });
    h.host.restore("work", 80.0, 24.0);
    now.store(3_000, Ordering::Relaxed);
    h.pty(0).exit(0);
    assert_eq!(h.calls().len(), 1);
    let listed = h.host.list().remove(0);
    assert_eq!(listed.id, id);
    assert_eq!(listed.status, TerminalStatus::Exited);
    assert_eq!(listed.exit_code, Some(0));
    assert_eq!(
        rows(&store, "work")[0].session_id.as_deref(),
        Some("old-session")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn restored_terminal_that_fails_after_probation_stays_exited() {
    let store = memory_store();
    let now = Arc::new(AtomicI64::new(1_000));
    let id = put_terminal(&store, Saved::default());
    let h = make(Parts {
        store: Some(store),
        now: Some(Arc::clone(&now)),
        ..Parts::default()
    });
    h.host.restore("work", 80.0, 24.0);
    now.store(22_000, Ordering::Relaxed);
    h.pty(0).exit(1);
    assert_eq!(h.calls().len(), 1);
    let listed = h.host.list().remove(0);
    assert_eq!(listed.id, id);
    assert_eq!(listed.status, TerminalStatus::Exited);
    assert_eq!(listed.exit_code, Some(1));
}

#[tokio::test(flavor = "current_thread")]
async fn input_does_not_end_probation() {
    let store = memory_store();
    let now = Arc::new(AtomicI64::new(1_000));
    let id = put_terminal(&store, Saved::default());
    let h = make(Parts {
        store: Some(store),
        now: Some(Arc::clone(&now)),
        ..Parts::default()
    });
    h.host.restore("work", 80.0, 24.0);
    h.host.write(&id, "hi");
    now.store(3_000, Ordering::Relaxed);
    h.pty(0).exit(1);
    assert_eq!(h.calls().len(), 2);
    assert_eq!(h.pty(0).written(), strings(&["hi"]));
    let listed = h.host.list().remove(0);
    assert_eq!(listed.id, id);
    assert_eq!(listed.status, TerminalStatus::Running);
    assert_eq!(listed.exit_code, None);
}

#[tokio::test(flavor = "current_thread")]
async fn a_fresh_relaunch_is_not_on_probation() {
    let store = memory_store();
    let now = Arc::new(AtomicI64::new(1_000));
    let id = put_terminal(&store, Saved::default());
    let h = make(Parts {
        store: Some(Arc::clone(&store)),
        now: Some(Arc::clone(&now)),
        ..Parts::default()
    });
    h.host.restore("work", 80.0, 24.0);
    now.store(2_000, Ordering::Relaxed);
    h.pty(0).exit(1);
    assert_eq!(h.calls().len(), 2);
    now.store(2_100, Ordering::Relaxed);
    h.pty(1).exit(1);
    assert_eq!(h.calls().len(), 2);
    let listed = h.host.list().remove(0);
    assert_eq!(listed.id, id);
    assert_eq!(listed.status, TerminalStatus::Exited);
    assert_eq!(listed.exit_code, Some(1));
    assert_eq!(runs(&store).len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn restored_cursor_that_fails_on_probation_gets_a_new_chat_id_or_none() {
    let store = memory_store();
    let now = Arc::new(AtomicI64::new(1_000));
    let id = put_terminal(
        &store,
        Saved {
            provider: "cursor".to_string(),
            title: "cursor 1".to_string(),
            model: Some("fast".to_string()),
            session_id: Some("cursor-old-01".to_string()),
            ..Saved::default()
        },
    );
    let created = Arc::new(AtomicUsize::new(0));
    let created_cb = Arc::clone(&created);
    let h = make(Parts {
        store: Some(Arc::clone(&store)),
        now: Some(Arc::clone(&now)),
        create_chat: Some(chat_fn(move || {
            created_cb.fetch_add(1, Ordering::SeqCst);
            async { Ok("cursor-new-01".to_string()) }
        })),
        ..Parts::default()
    });
    h.host.restore("work", 80.0, 24.0);
    assert_eq!(created.load(Ordering::SeqCst), 0);
    now.store(2_000, Ordering::Relaxed);
    h.pty(0).exit(1);
    assert!(
        yield_until(|| h.calls().len() == 2).await,
        "calls: {}",
        h.calls().len()
    );
    assert_eq!(created.load(Ordering::SeqCst), 1);
    assert_eq!(
        h.calls()[1].args,
        strings(&["--resume", "cursor-new-01", "--model", "fast"])
    );
    assert_eq!(
        rows(&store, "work")[0].session_id.as_deref(),
        Some("cursor-new-01")
    );
    let listed = h.host.list().remove(0);
    assert_eq!(listed.id, id);
    assert_eq!(listed.title, "cursor 1");
    assert_eq!(listed.status, TerminalStatus::Running);
    assert!(runs(&store).is_empty());

    let again = memory_store();
    let now = Arc::new(AtomicI64::new(1_000));
    put_terminal(
        &again,
        Saved {
            provider: "cursor".to_string(),
            title: "cursor 1".to_string(),
            model: None,
            session_id: Some("cursor-old-02".to_string()),
            ..Saved::default()
        },
    );
    let h = make(Parts {
        store: Some(Arc::clone(&again)),
        now: Some(Arc::clone(&now)),
        create_chat: Some(chat_fn(|| async {
            Err(PtyError::Message("offline".to_string()))
        })),
        ..Parts::default()
    });
    h.host.restore("work", 80.0, 24.0);
    now.store(2_000, Ordering::Relaxed);
    h.pty(0).exit(1);
    assert!(
        yield_until(|| h.calls().len() == 2).await,
        "calls: {}",
        h.calls().len()
    );
    assert!(h.calls()[1].args.is_empty());
    assert_eq!(rows(&again, "work")[0].session_id, None);
    assert_eq!(h.host.list()[0].status, TerminalStatus::Running);
}

#[tokio::test(flavor = "current_thread")]
async fn close_during_a_pending_cursor_relaunch_starts_no_process_and_deletes_the_row() {
    let store = memory_store();
    let now = Arc::new(AtomicI64::new(1_000));
    let id = put_terminal(
        &store,
        Saved {
            provider: "cursor".to_string(),
            title: "cursor 1".to_string(),
            session_id: Some("cursor-old-03".to_string()),
            ..Saved::default()
        },
    );
    let gate = Arc::new(Mutex::new(
        None::<oneshot::Sender<Result<String, PtyError>>>,
    ));
    let gate_set = Arc::clone(&gate);
    let h = make(Parts {
        store: Some(Arc::clone(&store)),
        now: Some(Arc::clone(&now)),
        create_chat: Some(chat_fn(move || {
            let (tx, rx) = oneshot::channel();
            *gate_set.lock().expect("gate") = Some(tx);
            async move {
                rx.await
                    .unwrap_or(Err(PtyError::Message("dropped".to_string())))
            }
        })),
        ..Parts::default()
    });
    h.host.restore("work", 80.0, 24.0);
    now.store(2_000, Ordering::Relaxed);
    h.pty(0).exit(1);
    assert!(
        yield_until(|| gate.lock().expect("gate").is_some()).await,
        "create_chat did not start"
    );
    assert_eq!(h.calls().len(), 1);
    h.host.close(&id);
    assert!(rows(&store, "work").is_empty());
    assert!(h.host.list().is_empty());
    let tx = gate.lock().expect("gate").take().expect("sender");
    tx.send(Ok("cursor-late-01".to_string())).expect("send");
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert_eq!(h.calls().len(), 1);
    assert!(rows(&store, "work").is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn open_passes_a_prompt_last_restore_does_not() {
    let store = memory_store();
    let first = make(Parts {
        store: Some(Arc::clone(&store)),
        cwd: Some(Arc::new(|| "work".to_string())),
        now: Some(Arc::new(AtomicI64::new(10))),
        create_chat: Some(chat_fn(|| async { Ok("cursor-chat-01".to_string()) })),
        ..Parts::default()
    });
    let claude = first
        .host
        .open(
            ProviderId::Claude,
            80.0,
            24.0,
            OpenOptions {
                prompt: Some("fix the login".to_string()),
                ..OpenOptions::default()
            },
        )
        .await
        .expect("claude");
    let first_args = first.calls()[0].args.clone();
    assert_eq!(first_args.last().map(String::as_str), Some("fix the login"));
    assert_eq!(first_args[0], "--session-id");
    let cursor = first
        .host
        .open(
            ProviderId::Cursor,
            80.0,
            24.0,
            OpenOptions {
                prompt: Some("fix the login".to_string()),
                ..OpenOptions::default()
            },
        )
        .await
        .expect("cursor");
    assert_eq!(
        first.calls()[1].args,
        strings(&["--session-id", "cursor-chat-01", "fix the login"])
    );
    first.host.close_all();

    let second = make(Parts {
        store: Some(store),
        cwd: Some(Arc::new(|| "work".to_string())),
        now: Some(Arc::new(AtomicI64::new(20))),
        ..Parts::default()
    });
    let restored = second.host.restore("work", 80.0, 24.0);
    assert_eq!(
        restored
            .iter()
            .map(|info| info.id.clone())
            .collect::<Vec<_>>(),
        vec![claude.id, cursor.id]
    );
    assert!(
        !second.calls()[0]
            .args
            .iter()
            .any(|arg| arg == "fix the login")
    );
    assert!(
        !second.calls()[1]
            .args
            .iter()
            .any(|arg| arg == "fix the login")
    );
    assert_eq!(second.calls()[0].args[0], "--resume");
    assert_eq!(second.calls()[1].args[0], "--resume");
}

#[tokio::test(flavor = "current_thread")]
async fn cwd_and_change_are_passed_to_the_spawn_and_saved() {
    let store = memory_store();
    let h = make(Parts {
        store: Some(Arc::clone(&store)),
        cwd: Some(Arc::new(|| "/proj".to_string())),
        now: Some(Arc::new(AtomicI64::new(7))),
        ..Parts::default()
    });
    let info = h
        .host
        .open(
            ProviderId::Claude,
            80.0,
            24.0,
            OpenOptions {
                cwd: Some("/wt/3fa9c1d2".to_string()),
                change: Some("3fa9c1d2".to_string()),
                ..OpenOptions::default()
            },
        )
        .await
        .expect("open");
    assert_eq!(h.calls()[0].opts.cwd, "/wt/3fa9c1d2");
    assert_eq!(info.title, "claude 1 · 3fa9c1d2");
    assert_eq!(info.change.as_deref(), Some("3fa9c1d2"));
    assert_eq!(info.provider, TerminalProvider::Claude);
    let row = rows(&store, "/proj").remove(0);
    assert_eq!(row.id, info.id);
    assert_eq!(row.project, "/proj");
    assert_eq!(row.cwd.as_deref(), Some("/wt/3fa9c1d2"));
    assert_eq!(row.change_id.as_deref(), Some("3fa9c1d2"));
    assert_eq!(row.title, "claude 1 · 3fa9c1d2");
}

#[tokio::test(flavor = "current_thread")]
async fn restore_uses_the_saved_cwd() {
    let store = memory_store();
    put_terminal(
        &store,
        Saved {
            cwd: Some("/saved/path".to_string()),
            change_id: Some("aabbccdd".to_string()),
            title: "claude 1 · aabbccdd".to_string(),
            ..Saved::default()
        },
    );
    let h = make(Parts {
        store: Some(store),
        cwd: Some(Arc::new(|| "work".to_string())),
        ..Parts::default()
    });
    let restored = h.host.restore("work", 80.0, 24.0);
    assert_eq!(h.calls()[0].opts.cwd, "/saved/path");
    assert_eq!(restored[0].id, "term-1");
    assert_eq!(restored[0].title, "claude 1 · aabbccdd");
    assert_eq!(restored[0].change.as_deref(), Some("aabbccdd"));
}

#[tokio::test(flavor = "current_thread")]
async fn restore_drops_a_row_whose_saved_folder_is_gone() {
    let store = memory_store();
    put_terminal(
        &store,
        Saved {
            cwd: Some("/gone".to_string()),
            ..Saved::default()
        },
    );
    put_terminal(
        &store,
        Saved {
            id: "term-2".to_string(),
            cwd: None,
            title: "claude 2".to_string(),
            ..Saved::default()
        },
    );
    let h = make(Parts {
        store: Some(Arc::clone(&store)),
        cwd: Some(Arc::new(|| "work".to_string())),
        exists: Some(Arc::new(|path| path != "/gone")),
        ..Parts::default()
    });
    let restored = h.host.restore("work", 80.0, 24.0);
    assert_eq!(
        restored
            .iter()
            .map(|info| info.id.as_str())
            .collect::<Vec<_>>(),
        vec!["term-2"]
    );
    assert_eq!(h.calls().len(), 1);
    assert_eq!(h.calls()[0].opts.cwd, "work");
    assert_eq!(
        rows(&store, "work")
            .into_iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        strings(&["term-2"])
    );
}

#[tokio::test(flavor = "current_thread")]
async fn close_change_closes_only_matching_terminals_and_resolves() {
    let h = make(Parts::default());
    let kept_a = h
        .host
        .open(
            ProviderId::Claude,
            80.0,
            24.0,
            OpenOptions {
                change: Some("bbb22222".to_string()),
                cwd: Some("wb".to_string()),
                ..OpenOptions::default()
            },
        )
        .await
        .expect("kept a");
    let gone = h
        .host
        .open(
            ProviderId::Cursor,
            80.0,
            24.0,
            OpenOptions {
                change: Some("aaa11111".to_string()),
                cwd: Some("wa".to_string()),
                ..OpenOptions::default()
            },
        )
        .await
        .expect("gone");
    let kept_b = open_default(&h.host, ProviderId::Claude, 80.0, 24.0).await;
    let pending = h.host.close_change("aaa11111");
    assert_eq!(h.host.list().len(), 2);
    let left = ids(&h.host);
    assert!(left.contains(&kept_a.id));
    assert!(left.contains(&kept_b.id));
    assert!(!left.contains(&gone.id));
    h.pty(1).exit(1);
    pending.await;
    assert_eq!(h.host.list().len(), 2);
    let left = ids(&h.host);
    assert!(left.contains(&kept_a.id));
    assert!(left.contains(&kept_b.id));
    h.host.close_change("missing0").await;
}

#[test]
fn open_shell_on_windows_runs_powershell_with_lastexitcode_checks() {
    let h = make(Parts {
        cwd: Some(Arc::new(|| "/proj".to_string())),
        now: Some(Arc::new(AtomicI64::new(9))),
        platform: Some(Platform::Win32),
        ..Parts::default()
    });
    let info = h
        .host
        .open_shell(ShellOptions {
            cwd: "/wt/aa11bb22".to_string(),
            change: Some("aa11bb22".to_string()),
            title: "setup · feature".to_string(),
            commands: strings(&["pnpm install", "pnpm build"]),
            cols: 80.0,
            rows: 24.0,
        })
        .expect("shell");
    assert_eq!(info.provider, TerminalProvider::Shell);
    assert_eq!(info.title, "setup · feature");
    assert_eq!(info.status, TerminalStatus::Running);
    assert_eq!(info.exit_code, None);
    assert_eq!(info.model, None);
    assert_eq!(info.started_at, 9);
    assert_eq!(info.change.as_deref(), Some("aa11bb22"));
    let calls = h.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].command, "powershell.exe");
    assert_eq!(
        calls[0].args[..3],
        strings(&["-NoLogo", "-NoProfile", "-Command"])
    );
    assert_eq!(
        calls[0].args[3],
        [
            "pnpm install",
            "if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }",
            "pnpm build",
            "if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }",
        ]
        .join("\n")
    );
    assert_eq!(calls[0].opts.cwd, "/wt/aa11bb22");
    assert_eq!(h.host.list()[0].title, "setup · feature");
}

#[test]
fn open_shell_elsewhere_joins_commands_with_and_and() {
    let h = make(Parts {
        platform: Some(Platform::Linux),
        ..Parts::default()
    });
    h.host
        .open_shell(ShellOptions {
            cwd: "/wt/x".to_string(),
            change: None,
            title: "setup · x".to_string(),
            commands: strings(&["npm ci", "npm test"]),
            cols: 40.0,
            rows: 12.0,
        })
        .expect("shell");
    let call = &h.calls()[0];
    assert_eq!(call.command, "sh");
    assert_eq!(call.args, strings(&["-c", "npm ci && npm test"]));
    assert!(call.args[1].contains(" && "));
    assert_eq!(call.opts.cwd, "/wt/x");
}

#[test]
fn open_shell_stop_at_first_failure_is_in_the_script() {
    let win = make(Parts {
        platform: Some(Platform::Win32),
        ..Parts::default()
    });
    win.host
        .open_shell(ShellOptions {
            cwd: "w".to_string(),
            change: None,
            title: "setup · w".to_string(),
            commands: strings(&["one", "two", "three"]),
            cols: 80.0,
            rows: 24.0,
        })
        .expect("win");
    let script = win.calls()[0].args[3].clone();
    assert!(script.contains("$LASTEXITCODE"));
    assert!(script.find("one").unwrap() < script.find("two").unwrap());
    assert!(script.find("two").unwrap() < script.find("three").unwrap());
    let unix = make(Parts {
        platform: Some(Platform::Darwin),
        ..Parts::default()
    });
    unix.host
        .open_shell(ShellOptions {
            cwd: "w".to_string(),
            change: None,
            title: "setup · w".to_string(),
            commands: strings(&["one", "two", "three"]),
            cols: 80.0,
            rows: 24.0,
        })
        .expect("unix");
    assert_eq!(
        unix.calls()[0].args,
        strings(&["-c", "one && two && three"])
    );
}

#[test]
fn a_shell_terminal_stays_after_exit_and_writes_no_usage_row() {
    let store = memory_store();
    let h = make(Parts {
        store: Some(Arc::clone(&store)),
        cwd: Some(Arc::new(|| "/proj".to_string())),
        now: Some(Arc::new(AtomicI64::new(4))),
        platform: Some(Platform::Win32),
        ..Parts::default()
    });
    let info = h
        .host
        .open_shell(ShellOptions {
            cwd: "/wt/aa".to_string(),
            change: Some("aa".to_string()),
            title: "setup · aa".to_string(),
            commands: strings(&["echo hi"]),
            cols: 80.0,
            rows: 24.0,
        })
        .expect("shell");
    assert_eq!(rows(&store, "/proj").len(), 1);
    h.pty(0).exit(0);
    let listed = h.host.list().remove(0);
    assert_eq!(listed.id, info.id);
    assert_eq!(listed.provider, TerminalProvider::Shell);
    assert_eq!(listed.status, TerminalStatus::Exited);
    assert_eq!(listed.exit_code, Some(0));
    assert!(rows(&store, "/proj").is_empty());
    assert!(runs(&store).is_empty());
    let failed = h
        .host
        .open_shell(ShellOptions {
            cwd: "/wt/bb".to_string(),
            change: None,
            title: "setup · bb".to_string(),
            commands: strings(&["false"]),
            cols: 80.0,
            rows: 24.0,
        })
        .expect("failed");
    h.pty(1).exit(1);
    let listed = h.host.list().remove(1);
    assert_eq!(listed.id, failed.id);
    assert_eq!(listed.status, TerminalStatus::Exited);
    assert_eq!(listed.exit_code, Some(1));
    assert!(runs(&store).is_empty());
}

#[test]
fn shell_rows_are_not_restored() {
    let store = memory_store();
    store
        .lock()
        .expect("store")
        .save_terminal(&TerminalRow {
            id: "shell-1".to_string(),
            project: "work".to_string(),
            provider: "shell".to_string(),
            model: None,
            session_id: None,
            title: "setup · old".to_string(),
            created_at: 1,
            updated_at: 1,
            scrollback: "npm i".to_string(),
            cwd: Some("/wt/old".to_string()),
            change_id: Some("oldold01".to_string()),
        })
        .expect("save shell");
    put_terminal(
        &store,
        Saved {
            id: "term-2".to_string(),
            title: "claude 1".to_string(),
            ..Saved::default()
        },
    );
    let h = make(Parts {
        store: Some(Arc::clone(&store)),
        cwd: Some(Arc::new(|| "work".to_string())),
        ..Parts::default()
    });
    let restored = h.host.restore("work", 80.0, 24.0);
    assert_eq!(
        restored
            .iter()
            .map(|info| info.id.as_str())
            .collect::<Vec<_>>(),
        vec!["term-2"]
    );
    assert!(
        restored
            .iter()
            .all(|info| info.provider != TerminalProvider::Shell)
    );
    assert_eq!(h.calls().len(), 1);
    assert_eq!(
        rows(&store, "work")
            .into_iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        strings(&["term-2"])
    );
}

#[tokio::test(flavor = "current_thread")]
async fn stored_scrollback_is_capped_at_the_last_200_000_characters() {
    let store = memory_store();
    let h = make(Parts {
        store: Some(Arc::clone(&store)),
        cwd: Some(Arc::new(|| "work".to_string())),
        now: Some(Arc::new(AtomicI64::new(1))),
        ..Parts::default()
    });
    open_default(&h.host, ProviderId::Claude, 80.0, 24.0).await;
    h.pty(0)
        .push(&format!("ab{}", "x".repeat(TERMINAL_SCROLLBACK_CAP)));
    h.advance(2_000);
    let saved = rows(&store, "work")[0].scrollback.clone();
    assert_eq!(saved.chars().count(), TERMINAL_SCROLLBACK_CAP);
    assert_eq!(saved, "x".repeat(TERMINAL_SCROLLBACK_CAP));
}

#[tokio::test(flavor = "current_thread")]
async fn sleep_stops_a_finished_agent_and_wake_resumes_its_session() {
    let store = memory_store();
    let h = make(Parts {
        store: Some(Arc::clone(&store)),
        cwd: Some(Arc::new(|| "work".to_string())),
        now: Some(Arc::new(AtomicI64::new(1))),
        ..Parts::default()
    });
    let updates = Arc::new(Mutex::new(Vec::new()));
    let updates_cb = Arc::clone(&updates);
    let _updates = h.host.on_update(move |info, removed| {
        let status = match info.status {
            TerminalStatus::Running => "running",
            TerminalStatus::Exited => "exited",
            TerminalStatus::Sleeping => "sleeping",
        };
        updates_cb
            .lock()
            .expect("updates")
            .push((status.to_string(), removed));
    });
    let info = open_default(&h.host, ProviderId::Claude, 80.0, 24.0).await;
    let session = h.calls()[0].args[1].clone();
    h.pty(0).push("done.\r\n");
    assert!(h.host.sleep(&info.id));
    let listed = h.host.list().remove(0);
    assert_eq!(listed.id, info.id);
    assert_eq!(listed.status, TerminalStatus::Sleeping);
    assert_eq!(
        updates
            .lock()
            .expect("updates")
            .last()
            .map(|(status, removed)| (status.as_str(), *removed)),
        Some(("sleeping", false))
    );
    h.pty(0).exit(0);
    assert_eq!(
        h.host
            .list()
            .into_iter()
            .map(|item| item.status)
            .collect::<Vec<_>>(),
        vec![TerminalStatus::Sleeping]
    );
    assert_eq!(
        rows(&store, "work")
            .into_iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        vec![info.id.clone()]
    );
    assert!(!h.host.sleep(&info.id));

    let woken = h
        .host
        .wake(&info.id, 100.0, 30.0)
        .expect("wake")
        .expect("session");
    assert_eq!(woken.id, info.id);
    assert_eq!(woken.status, TerminalStatus::Running);
    assert_eq!(h.calls()[1].args, vec!["--resume".to_string(), session]);
    assert_eq!(
        h.host
            .list()
            .into_iter()
            .map(|item| item.status)
            .collect::<Vec<_>>(),
        vec![TerminalStatus::Running]
    );
    assert!(h.host.wake(&info.id, 100.0, 30.0).expect("wake").is_none());

    assert!(h.host.sleep(&info.id));
    h.host.close(&info.id);
    assert!(h.host.list().is_empty());
    assert!(rows(&store, "work").is_empty());
}

#[test]
fn a_shell_never_sleeps() {
    let store = memory_store();
    let h = make(Parts {
        store: Some(store),
        cwd: Some(Arc::new(|| "work".to_string())),
        ..Parts::default()
    });
    let shell = h
        .host
        .open_shell(ShellOptions {
            cwd: "work".to_string(),
            change: None,
            title: "setup".to_string(),
            commands: strings(&["echo hi"]),
            cols: 80.0,
            rows: 24.0,
        })
        .expect("shell");
    assert!(!h.host.sleep(&shell.id));
}

#[test]
fn resize_follows_the_window_that_last_claimed_the_terminal() {
    let claims = TerminalClaims::new();
    assert!(claims.allow_resize("t1", 1));
    assert!(!claims.allow_resize("t1", 2));
    claims.claim("t1", 2);
    assert!(claims.allow_resize("t1", 2));
    assert!(!claims.allow_resize("t1", 1));
    claims.release_window(2);
    assert!(claims.allow_resize("t1", 1));
    assert!(!claims.allow_resize("t1", 2));
    claims.release_window(1);
    assert!(claims.allow_resize("t1", 2));
}

#[tokio::test(flavor = "current_thread")]
async fn claimed_resize_reaches_only_the_owning_window() {
    let h = make(Parts::default());
    let info = open_default(&h.host, ProviderId::Claude, 80.0, 24.0).await;
    assert!(h.host.resize_claimed(&info.id, 1, 40.0, 12.0));
    assert!(!h.host.resize_claimed(&info.id, 2, 50.0, 20.0));
    h.host.claim_terminal(&info.id, 2);
    assert!(h.host.resize_claimed(&info.id, 2, 60.0, 22.0));
    assert!(!h.host.resize_claimed(&info.id, 1, 70.0, 23.0));
    h.host.release_window(2);
    assert!(h.host.resize_claimed(&info.id, 1, 80.0, 24.0));
    assert_eq!(h.pty(0).resized(), vec![(40, 12), (60, 22), (80, 24)]);
}

#[tokio::test(flavor = "current_thread")]
async fn close_kills_the_process_tree() {
    let spec = tree_launch();
    let pids = Arc::new(Mutex::new(Vec::new()));
    let pids_spawn = Arc::clone(&pids);
    let host = PtyHost::new(PtyHostOptions {
        launch: Arc::new(move |_provider, _opts| spec.clone()),
        cwd: Arc::new(work_dir),
        model_for: None,
        store: None,
        spawn: Some(Arc::new(move |command, args, opts| {
            let proc = spawn_pty(command, args, opts)?;
            pids_spawn.lock().expect("pids").push(proc.pid());
            Ok(proc)
        })),
        kill_tree: None,
        now: None,
        scheduler: None,
        create_chat: None,
        max_terminals: None,
        exists: None,
        platform: None,
    });
    let info = open_default(&host, ProviderId::Claude, 80.0, 24.0).await;
    let mut answered = false;
    let ready = wait_until(
        || {
            answer_cursor_query(&host, &info.id, &mut answered);
            child_pid(&host.snapshot(&info.id)).is_some()
        },
        Duration::from_secs(15),
    );
    let snap = host.snapshot(&info.id);
    assert!(ready, "snapshot tail: {}", tail(&snap));
    let child = child_pid(&snap).expect("child pid");
    let parent = pids.lock().expect("pids")[0];
    assert!(process_alive(parent), "parent {parent} was not alive");
    assert!(process_alive(child), "child {child} was not alive");
    host.close(&info.id);
    let dead = wait_until(
        || !process_alive(parent) && !process_alive(child),
        Duration::from_secs(8),
    );
    assert!(
        dead,
        "parent {parent} alive={} child {child} alive={}",
        process_alive(parent),
        process_alive(child)
    );
    host.close_all();
}
