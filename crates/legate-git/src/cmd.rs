//! Async process spawn. A worker thread never waits on a child with blocking IO.

use std::future::Future;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

pub(crate) const MAX_BUFFER: usize = 32 * 1024 * 1024;
pub(crate) const GIT_TIMEOUT: Duration = Duration::from_secs(60);
pub(crate) const COMMAND_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmdOut {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
    /// Process bytes for `-z` parsers. `None` when a test runner built a text result.
    stdout_raw: Option<Vec<u8>>,
}

impl CmdOut {
    pub fn new(code: i32, stdout: impl Into<String>, stderr: impl Into<String>) -> Self {
        Self {
            code,
            stdout: stdout.into(),
            stderr: stderr.into(),
            stdout_raw: None,
        }
    }

    pub fn ok(stdout: impl Into<String>) -> Self {
        Self::new(0, stdout, "")
    }

    pub fn fail(code: i32, stderr: impl Into<String>) -> Self {
        Self::new(code, "", stderr)
    }

    pub(crate) fn from_raw(code: i32, stdout: Vec<u8>, stderr: Vec<u8>) -> Self {
        Self {
            code,
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
            stdout_raw: Some(stdout),
        }
    }

    /// NUL-separated stdout. Raw process bytes when the runner kept them.
    pub fn stdout_z(&self) -> &[u8] {
        self.stdout_raw.as_deref().unwrap_or(self.stdout.as_bytes())
    }
}

pub trait CmdRun: Send + Sync {
    fn run(
        &self,
        command: String,
        args: Vec<String>,
        cwd: Option<PathBuf>,
    ) -> impl Future<Output = CmdOut> + Send;
}

#[derive(Debug, Clone, Copy)]
pub struct ProcessRun {
    timeout: Duration,
    max_buffer: usize,
}

impl ProcessRun {
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            max_buffer: MAX_BUFFER,
        }
    }

    pub fn git() -> Self {
        Self::new(GIT_TIMEOUT)
    }

    pub fn command() -> Self {
        Self::new(COMMAND_TIMEOUT)
    }
}

impl CmdRun for ProcessRun {
    fn run(
        &self,
        command: String,
        args: Vec<String>,
        cwd: Option<PathBuf>,
    ) -> impl Future<Output = CmdOut> + Send {
        let timeout = self.timeout;
        let max_buffer = self.max_buffer;
        async move {
            let raw = spawn_raw(&command, &args, cwd.as_deref(), timeout, max_buffer).await;
            CmdOut::from_raw(raw.code, raw.stdout, raw.stderr)
        }
    }
}

pub struct FnRun<F> {
    f: Arc<F>,
}

impl<F> Clone for FnRun<F> {
    fn clone(&self) -> Self {
        Self {
            f: Arc::clone(&self.f),
        }
    }
}

impl<F, Fut> FnRun<F>
where
    F: Fn(String, Vec<String>, Option<PathBuf>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = CmdOut> + Send + 'static,
{
    pub fn new(f: F) -> Self {
        Self { f: Arc::new(f) }
    }
}

impl<F, Fut> CmdRun for FnRun<F>
where
    F: Fn(String, Vec<String>, Option<PathBuf>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = CmdOut> + Send + 'static,
{
    fn run(
        &self,
        command: String,
        args: Vec<String>,
        cwd: Option<PathBuf>,
    ) -> impl Future<Output = CmdOut> + Send {
        let f = Arc::clone(&self.f);
        async move { f(command, args, cwd).await }
    }
}

#[derive(Debug)]
pub(crate) struct RawOut {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl RawOut {
    fn fail(message: &str) -> Self {
        Self {
            code: 1,
            stdout: Vec::new(),
            stderr: message.as_bytes().to_vec(),
        }
    }

    pub(crate) fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).trim().to_string()
    }
}

pub(crate) async fn spawn_raw(
    command: &str,
    args: &[String],
    cwd: Option<&std::path::Path>,
    timeout: Duration,
    max_buffer: usize,
) -> RawOut {
    let mut cmd = Command::new(command);
    cmd.args(args);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    cmd.kill_on_drop(true);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    #[cfg(windows)]
    {
        cmd.creation_flags(0x0800_0000);
    }

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => return RawOut::fail(&err.to_string()),
    };
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill().await;
        return RawOut::fail("missing pipe");
    };
    let out_handle = tokio::spawn(read_capped(stdout, max_buffer));
    let err_handle = tokio::spawn(read_capped(stderr, max_buffer));

    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(err)) => {
            out_handle.abort();
            err_handle.abort();
            return RawOut::fail(&err.to_string());
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            out_handle.abort();
            err_handle.abort();
            return RawOut::fail("timed out");
        }
    };

    let stdout = match out_handle.await {
        Ok(Ok(buf)) => buf,
        Ok(Err(err)) => return RawOut::fail(&err.to_string()),
        Err(_) => Vec::new(),
    };
    let stderr = match err_handle.await {
        Ok(Ok(buf)) => buf,
        Ok(Err(err)) => err.to_string().into_bytes(),
        Err(_) => Vec::new(),
    };
    RawOut {
        code: status.code().unwrap_or(1),
        stdout,
        stderr,
    }
}

async fn read_capped<R>(mut pipe: R, cap: usize) -> std::io::Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    let mut buf = Vec::new();
    let mut tmp = [0u8; 16 * 1024];
    loop {
        let n = pipe.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        if buf.len().saturating_add(n) > cap {
            return Err(std::io::Error::new(
                std::io::ErrorKind::OutOfMemory,
                "stdout maxBuffer length exceeded",
            ));
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    Ok(buf)
}

pub(crate) fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_string()).collect()
}
