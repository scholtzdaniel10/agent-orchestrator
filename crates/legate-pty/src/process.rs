use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::error::PtyError;
use crate::syncutil::lock;
use crate::textutil::decode_utf8;
use crate::{READER_COALESCE, READER_COALESCE_BYTES};

/// Arguments the host passes to every spawn. `name` is the terminal type (`xterm-256color`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnOpts {
    pub name: String,
    pub cols: u16,
    pub rows: u16,
    pub cwd: String,
    pub env: BTreeMap<String, String>,
}

/// Bytes that arrived before the host installed its callbacks.
#[derive(Debug, Default)]
pub struct QueuedPty {
    pub data: Vec<String>,
    pub exit: Option<Option<i64>>,
}

/// A PTY process. The real implementation is [`spawn_pty`]; tests supply a fake.
pub trait PtyProcess: Send {
    fn pid(&self) -> u32;
    fn on_data(&mut self, cb: Box<dyn Fn(String) + Send + Sync>);
    fn on_exit(&mut self, cb: Box<dyn Fn(Option<i64>) + Send + Sync>);
    fn take_queued(&mut self) -> QueuedPty;
    fn write(&mut self, data: &str) -> Result<(), PtyError>;
    fn resize(&mut self, cols: u16, rows: u16) -> Result<(), PtyError>;
    fn kill(&mut self) -> Result<(), PtyError>;
}

pub type SpawnPty =
    Arc<dyn Fn(&str, &[String], &SpawnOpts) -> Result<Box<dyn PtyProcess>, PtyError> + Send + Sync>;

pub type KillTree = Arc<dyn Fn(&mut dyn PtyProcess) + Send + Sync>;

pub(crate) fn default_kill_tree(proc: &mut dyn PtyProcess) {
    let _ = proc.kill();
}

/// Open a PTY with ConPTY on Windows and a POSIX PTY elsewhere, and start the child.
pub fn spawn_pty(
    command: &str,
    args: &[String],
    opts: &SpawnOpts,
) -> Result<Box<dyn PtyProcess>, PtyError> {
    let mut env = opts.env.clone();
    if !env.contains_key("TERM") {
        env.insert("TERM".to_string(), opts.name.clone());
    }
    let mut cmd = CommandBuilder::new(command);
    for arg in args {
        cmd.arg(arg);
    }
    cmd.cwd(&opts.cwd);
    cmd.env_clear();
    for (key, value) in &env {
        cmd.env(key, value);
    }

    let system = native_pty_system();
    let pair = system
        .openpty(PtySize {
            rows: opts.rows,
            cols: opts.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|err| PtyError::Message(err.to_string()))?;

    #[cfg(unix)]
    let pgid = pair.master.process_group_leader();
    #[cfg(not(unix))]
    let pgid: Option<i32> = None;

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|err| PtyError::Message(err.to_string()))?;
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|err| PtyError::Message(err.to_string()))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|err| PtyError::Message(err.to_string()))?;
    let pid = child.process_id().unwrap_or(0);
    let killer = child.clone_killer();
    let bridge = Arc::new(IoBridge::new());

    Ok(Box::new(RealPty {
        master: pair.master,
        _slave: pair.slave,
        writer,
        killer,
        reader: Some(reader),
        child: Some(child),
        pid,
        pgid,
        bridge,
        started: false,
    }))
}

struct RealPty {
    master: Box<dyn MasterPty + Send>,
    _slave: Box<dyn portable_pty::SlavePty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn portable_pty::ChildKiller + Send + Sync>,
    reader: Option<Box<dyn Read + Send>>,
    child: Option<Box<dyn portable_pty::Child + Send + Sync>>,
    pid: u32,
    pgid: Option<i32>,
    bridge: Arc<IoBridge>,
    started: bool,
}

impl PtyProcess for RealPty {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn on_data(&mut self, cb: Box<dyn Fn(String) + Send + Sync>) {
        self.bridge.set_data(Arc::from(cb));
        self.maybe_start();
    }

    fn on_exit(&mut self, cb: Box<dyn Fn(Option<i64>) + Send + Sync>) {
        self.bridge.set_exit(Arc::from(cb));
        self.maybe_start();
    }

    fn take_queued(&mut self) -> QueuedPty {
        self.bridge.take_queued()
    }

    fn write(&mut self, data: &str) -> Result<(), PtyError> {
        self.writer
            .write_all(data.as_bytes())
            .map_err(|err| PtyError::Message(err.to_string()))?;
        let _ = self.writer.flush();
        Ok(())
    }

    fn resize(&mut self, cols: u16, rows: u16) -> Result<(), PtyError> {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|err| PtyError::Message(err.to_string()))
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        kill_process_tree(self.pid, self.pgid);
        let _ = self.killer.kill();
        Ok(())
    }
}

impl RealPty {
    fn maybe_start(&mut self) {
        if self.started || !self.bridge.ready() {
            return;
        }
        let Some(reader) = self.reader.take() else {
            return;
        };
        let Some(child) = self.child.take() else {
            return;
        };
        self.started = true;
        let done = Arc::new(Done::default());
        let (tx, rx) = mpsc::channel();
        let bridge = Arc::clone(&self.bridge);
        let flushed = Arc::clone(&done);
        let _ = std::thread::Builder::new()
            .name("legate-pty-read".to_string())
            .spawn(move || read_loop(reader, tx));
        let _ = std::thread::Builder::new()
            .name("legate-pty-flush".to_string())
            .spawn(move || flush_loop(rx, bridge, flushed));
        let bridge = Arc::clone(&self.bridge);
        let _ = std::thread::Builder::new()
            .name("legate-pty-wait".to_string())
            .spawn(move || wait_loop(child, bridge, done));
    }
}

fn kill_process_tree(pid: u32, pgid: Option<i32>) {
    if pid == 0 {
        return;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = Command::new("taskkill")
            .args(["/pid", &pid.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        use std::process::{Command, Stdio};
        let group = pgid.unwrap_or(pid as i32);
        let _ = Command::new("kill")
            .args(["-KILL", &format!("-{group}")])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = pid;
    }
    #[cfg(windows)]
    {
        let _ = pgid;
    }
}

enum ReaderMsg {
    Data(String),
    Eof,
}

struct EofOnDrop(Option<Sender<ReaderMsg>>);

impl Drop for EofOnDrop {
    fn drop(&mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(ReaderMsg::Eof);
        }
    }
}

fn read_loop(mut reader: Box<dyn Read + Send>, tx: Sender<ReaderMsg>) {
    let _eof = EofOnDrop(Some(tx.clone()));
    let mut raw = [0u8; 8192];
    let mut carry = Vec::new();
    loop {
        match reader.read(&mut raw) {
            Ok(0) => break,
            Ok(n) => {
                let text = decode_utf8(&mut carry, &raw[..n]);
                if text.is_empty() {
                    continue;
                }
                if tx.send(ReaderMsg::Data(text)).is_err() {
                    break;
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => {
                tracing::debug!("pty read ended: {err}");
                break;
            }
        }
    }
    if !carry.is_empty() {
        let _ = tx.send(ReaderMsg::Data("\u{FFFD}".to_string()));
    }
}

/// Groups reader output so the host is not woken once per byte.
struct Coalescer {
    buf: String,
    started: Option<Instant>,
    delay: Duration,
    max_bytes: usize,
}

impl Coalescer {
    fn new(delay: Duration, max_bytes: usize) -> Self {
        Self {
            buf: String::new(),
            started: None,
            delay,
            max_bytes,
        }
    }

    fn push(&mut self, text: &str, now: Instant) -> Option<String> {
        if text.is_empty() {
            return None;
        }
        if self.buf.is_empty() {
            self.started = Some(now);
        }
        self.buf.push_str(text);
        self.take_due(now)
    }

    fn poll(&mut self, now: Instant) -> Option<String> {
        self.take_due(now)
    }

    fn finish(&mut self) -> Option<String> {
        if self.buf.is_empty() {
            None
        } else {
            self.started = None;
            Some(std::mem::take(&mut self.buf))
        }
    }

    fn time_until(&self, now: Instant) -> Option<Duration> {
        let started = self.started?;
        if self.buf.is_empty() {
            return None;
        }
        Some(
            self.delay
                .saturating_sub(now.saturating_duration_since(started)),
        )
    }

    fn take_due(&mut self, now: Instant) -> Option<String> {
        if self.buf.is_empty() {
            return None;
        }
        let aged = self
            .started
            .is_some_and(|started| now.saturating_duration_since(started) >= self.delay);
        if aged || self.buf.len() >= self.max_bytes {
            self.started = None;
            Some(std::mem::take(&mut self.buf))
        } else {
            None
        }
    }
}

fn flush_loop(rx: Receiver<ReaderMsg>, bridge: Arc<IoBridge>, done: Arc<Done>) {
    let mut coal = Coalescer::new(READER_COALESCE, READER_COALESCE_BYTES);
    loop {
        let now = Instant::now();
        if let Some(chunk) = coal.poll(now) {
            bridge.emit_data(chunk);
            continue;
        }
        let msg = match coal.time_until(now) {
            None => rx.recv().ok(),
            Some(delay) if delay.is_zero() => continue,
            Some(delay) => match rx.recv_timeout(delay) {
                Ok(msg) => Some(msg),
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => None,
            },
        };
        match msg {
            Some(ReaderMsg::Data(text)) => {
                if let Some(chunk) = coal.push(&text, Instant::now()) {
                    bridge.emit_data(chunk);
                }
            }
            Some(ReaderMsg::Eof) | None => {
                if let Some(chunk) = coal.finish() {
                    bridge.emit_data(chunk);
                }
                done.signal();
                break;
            }
        }
    }
}

fn wait_loop(
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
    bridge: Arc<IoBridge>,
    done: Arc<Done>,
) {
    let code = match child.wait() {
        Ok(status) => Some(i64::from(status.exit_code())),
        Err(err) => {
            tracing::error!("pty wait: {err}");
            None
        }
    };
    done.wait_for(Duration::from_millis(500));
    bridge.emit_exit(code);
}

struct Done {
    flag: Mutex<bool>,
    cv: Condvar,
}

impl Default for Done {
    fn default() -> Self {
        Self {
            flag: Mutex::new(false),
            cv: Condvar::new(),
        }
    }
}

impl Done {
    fn signal(&self) {
        *lock(&self.flag) = true;
        self.cv.notify_all();
    }

    fn wait_for(&self, timeout: Duration) {
        let guard = lock(&self.flag);
        if *guard {
            return;
        }
        let waited = self.cv.wait_timeout(guard, timeout);
        let _guard = match waited {
            Ok((guard, _)) => guard,
            Err(err) => err.into_inner().0,
        };
    }
}

struct IoBridge {
    inner: Mutex<IoState>,
}

struct IoState {
    data_cb: Option<Arc<dyn Fn(String) + Send + Sync>>,
    exit_cb: Option<Arc<dyn Fn(Option<i64>) + Send + Sync>>,
    data_q: Vec<String>,
    exit_q: Option<Option<i64>>,
}

impl IoBridge {
    fn new() -> Self {
        Self {
            inner: Mutex::new(IoState {
                data_cb: None,
                exit_cb: None,
                data_q: Vec::new(),
                exit_q: None,
            }),
        }
    }

    fn set_data(&self, cb: Arc<dyn Fn(String) + Send + Sync>) {
        lock(&self.inner).data_cb = Some(cb);
    }

    fn set_exit(&self, cb: Arc<dyn Fn(Option<i64>) + Send + Sync>) {
        lock(&self.inner).exit_cb = Some(cb);
    }

    fn ready(&self) -> bool {
        let io = lock(&self.inner);
        io.data_cb.is_some() && io.exit_cb.is_some()
    }

    fn take_queued(&self) -> QueuedPty {
        let mut io = lock(&self.inner);
        QueuedPty {
            data: std::mem::take(&mut io.data_q),
            exit: io.exit_q.take(),
        }
    }

    fn emit_data(&self, data: String) {
        let cb = lock(&self.inner).data_cb.clone();
        if let Some(cb) = cb {
            let _ = catch_unwind(AssertUnwindSafe(|| cb(data)));
        } else {
            lock(&self.inner).data_q.push(data);
        }
    }

    fn emit_exit(&self, code: Option<i64>) {
        let cb = lock(&self.inner).exit_cb.clone();
        if let Some(cb) = cb {
            let _ = catch_unwind(AssertUnwindSafe(|| cb(code)));
        } else {
            let mut io = lock(&self.inner);
            if io.exit_q.is_none() {
                io.exit_q = Some(code);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalescer_flushes_on_age_or_size() {
        let start = Instant::now();
        let mut coal = Coalescer::new(Duration::from_millis(6), 8);
        assert!(coal.push("ab", start).is_none());
        assert!(coal.poll(start + Duration::from_millis(5)).is_none());
        assert_eq!(
            coal.poll(start + Duration::from_millis(6)).as_deref(),
            Some("ab")
        );
        assert_eq!(
            coal.push("0123456789", start).as_deref(),
            Some("0123456789")
        );
    }
}
