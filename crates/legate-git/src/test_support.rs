use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use crate::cmd::{CmdOut, CmdRun};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
}

pub struct Temp {
    dir: tempfile::TempDir,
}

impl Temp {
    pub fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("tempdir"),
        }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}

pub struct Spy<F> {
    calls: Arc<Mutex<Vec<Call>>>,
    handle: Arc<F>,
}

impl<F> Clone for Spy<F> {
    fn clone(&self) -> Self {
        Self {
            calls: Arc::clone(&self.calls),
            handle: Arc::clone(&self.handle),
        }
    }
}

impl<F> Spy<F> {
    pub fn new(handle: F) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            handle: Arc::new(handle),
        }
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }
}

impl<F, Fut> CmdRun for Spy<F>
where
    F: Fn(String, Vec<String>, Option<PathBuf>) -> Fut + Send + Sync,
    Fut: Future<Output = CmdOut> + Send,
{
    fn run(
        &self,
        command: String,
        args: Vec<String>,
        cwd: Option<PathBuf>,
    ) -> impl Future<Output = CmdOut> + Send {
        self.calls
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .push(Call {
                command: command.clone(),
                args: args.clone(),
                cwd: cwd.clone(),
            });
        let handle = Arc::clone(&self.handle);
        handle(command, args, cwd)
    }
}

pub fn git(cwd: &Path, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(cwd);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd.output().expect("spawn git")
}

pub fn git_ok(cwd: &Path, args: &[&str]) -> String {
    let out = git(cwd, args);
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub fn git_text(cwd: &Path, args: &[&str]) -> String {
    git_ok(cwd, args).trim().to_string()
}

pub fn init_repo(dir: &Path, notes: &str) {
    std::fs::create_dir_all(dir).expect("mkdir");
    git_ok(dir, &["init", "-b", "main"]);
    git_ok(dir, &["config", "user.name", "test"]);
    git_ok(dir, &["config", "user.email", "test@example.invalid"]);
    git_ok(dir, &["config", "commit.gpgsign", "false"]);
    git_ok(dir, &["config", "core.autocrlf", "false"]);
    git_ok(dir, &["config", "color.ui", "false"]);
    std::fs::write(dir.join("notes.txt"), notes).expect("notes");
    git_ok(dir, &["add", "notes.txt"]);
    git_ok(dir, &["commit", "-m", "init"]);
}

pub fn temp_project() -> (Temp, PathBuf, PathBuf) {
    let temp = Temp::new();
    let project = temp.path().join("project");
    let wt = temp.path().join("wt");
    init_repo(&project, "alpha\n");
    (temp, project, wt)
}
