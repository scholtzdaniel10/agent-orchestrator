use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use legate_providers::{AgentEvent, Bin, LEGATE_NOTE, ProcessExit, RunHandle};

pub struct Temp {
    path: PathBuf,
}

impl Temp {
    pub fn new(prefix: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("legate-{prefix}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path).expect("temp dir");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        for _ in 0..20 {
            if std::fs::remove_dir_all(&self.path).is_ok() || !self.path.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

pub struct Running {
    pub handle: RunHandle,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.handle.kill();
    }
}

pub fn fake(args: &[&str]) -> Bin {
    Bin::with_args(env!("CARGO_BIN_EXE_fake-cli"), args.iter().copied())
}

pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

pub fn fixture_text(name: &str) -> String {
    std::fs::read_to_string(fixture(name)).expect("fixture")
}

pub fn events_of(parse: impl Fn(&str) -> Vec<AgentEvent>, name: &str) -> Vec<AgentEvent> {
    let mut events = Vec::new();
    for line in fixture_text(name).split('\n') {
        events.extend(parse(line.trim()));
    }
    events
}

pub fn expect_events(json: &str) -> Vec<AgentEvent> {
    serde_json::from_str(json).expect("expected events")
}

pub async fn collect(handle: &mut RunHandle) -> (Vec<AgentEvent>, ProcessExit) {
    let mut events = Vec::new();
    while let Some(event) = handle.events.recv().await {
        events.push(event);
    }
    let exit = handle.exit().await;
    (events, exit)
}

pub async fn cli_args(handle: &mut RunHandle) -> Vec<String> {
    let (_events, exit) = collect(handle).await;
    assert_eq!(exit.code, Some(0), "stderr={}", exit.stderr);
    serde_json::from_str(&exit.stderr)
        .unwrap_or_else(|err| panic!("argv json ({err}): {}", exit.stderr))
}

pub fn without_note(args: &[String]) -> Vec<String> {
    let at = args
        .iter()
        .position(|arg| arg == "--append-system-prompt")
        .expect("note flag");
    assert_eq!(args.get(at + 1).map(String::as_str), Some(LEGATE_NOTE));
    assert_eq!(
        args.iter().rposition(|arg| arg == "--append-system-prompt"),
        Some(at)
    );
    let mut out = args[..at].to_vec();
    out.extend_from_slice(&args[at + 2..]);
    out
}

pub async fn timeout<T>(fut: impl std::future::Future<Output = T>, ms: u64) -> T {
    match tokio::time::timeout(Duration::from_millis(ms), fut).await {
        Ok(value) => value,
        Err(_) => panic!("timed out after {ms}ms"),
    }
}

pub fn mkdir(path: &Path) {
    std::fs::create_dir_all(path).expect("mkdir");
}

pub fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

pub fn path_text(path: &Path) -> String {
    path.to_str().unwrap_or("").to_string()
}

pub fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|err| err.into_inner())
}

pub fn pid_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let Ok(output) = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        else {
            return false;
        };
        String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\""))
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
}

pub fn read_pid(path: &Path) -> u32 {
    read(path).trim().parse().expect("pid")
}
