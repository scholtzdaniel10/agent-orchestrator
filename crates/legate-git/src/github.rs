//! GitHub CLI: auth, repo list, clone, and opening a pull request on a new branch.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use legate_core::GithubRepo;
use serde_json::Value;

use crate::cmd::{CmdOut, CmdRun, ProcessRun};
use crate::error::GitError;
use crate::gitpath::{utf16_len, utf16_prefix, utf16_suffix};

const CACHE: Duration = Duration::from_millis(60_000);

pub struct Github<R> {
    run: R,
    hex: Arc<dyn Fn() -> String + Send + Sync>,
    cache: Mutex<Option<AvailCache>>,
}

struct AvailCache {
    at: Instant,
    value: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedPr {
    pub url: String,
    pub branch: String,
}

impl Github<ProcessRun> {
    pub fn new() -> Self {
        Self::with(ProcessRun::command(), random_hex)
    }
}

impl Default for Github<ProcessRun> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: CmdRun> Github<R> {
    pub fn with(run: R, hex: impl Fn() -> String + Send + Sync + 'static) -> Self {
        Self {
            run,
            hex: Arc::new(hex),
            cache: Mutex::new(None),
        }
    }

    pub async fn available(&self) -> bool {
        if let Some(cache) = lock(&self.cache).as_ref()
            && cache.at.elapsed() < CACHE
        {
            return cache.value;
        }
        let result = self
            .run
            .run("gh".into(), vec!["auth".into(), "status".into()], None)
            .await;
        let value = result.code == 0;
        *lock(&self.cache) = Some(AvailCache {
            at: Instant::now(),
            value,
        });
        value
    }

    pub async fn repos(&self) -> Vec<GithubRepo> {
        let result = self
            .run
            .run(
                "gh".into(),
                vec![
                    "repo".into(),
                    "list".into(),
                    "--limit".into(),
                    "100".into(),
                    "--json".into(),
                    "nameWithOwner,description,isPrivate,updatedAt".into(),
                ],
                None,
            )
            .await;
        if result.code != 0 {
            return Vec::new();
        }
        let Ok(parsed) = serde_json::from_str::<Value>(&result.stdout) else {
            return Vec::new();
        };
        let Some(rows) = parsed.as_array() else {
            return Vec::new();
        };
        rows.iter().filter_map(parse_repo).collect()
    }

    pub async fn clone(
        &self,
        name_with_owner: &str,
        parent_dir: &Path,
    ) -> Result<PathBuf, GitError> {
        assert_name(name_with_owner)?;
        let repo_name = name_with_owner
            .split_once('/')
            .map(|(_, repo)| repo)
            .unwrap_or(name_with_owner);
        let target = parent_dir.join(repo_name);
        if target.exists() {
            return Err(GitError::new(
                "a folder with that name already exists there",
            ));
        }
        let result = self
            .run
            .run(
                "gh".into(),
                vec![
                    "repo".into(),
                    "clone".into(),
                    name_with_owner.into(),
                    path_arg(&target),
                ],
                None,
            )
            .await;
        if result.code != 0 {
            return Err(GitError::new(stderr_tail(&result.stderr)));
        }
        Ok(target)
    }

    pub async fn create_pr(
        &self,
        cwd: &Path,
        title: &str,
        body: &str,
        base: Option<&str>,
        head: Option<&str>,
    ) -> Result<String, GitError> {
        let mut args = vec![
            "pr".to_string(),
            "create".into(),
            "--title".into(),
            title.into(),
            "--body".into(),
            body.into(),
        ];
        if let Some(base) = base {
            args.push("--base".into());
            args.push(base.into());
        }
        if let Some(head) = head {
            args.push("--head".into());
            args.push(head.into());
        }
        let result = self.git_at("gh", args, Some(cwd)).await;
        if result.code != 0 {
            return Err(GitError::new(stderr_tail(&result.stderr)));
        }
        for line in result.stdout.split(['\n', '\r']) {
            if line.starts_with("https://") {
                return Ok(line.trim().to_string());
            }
        }
        Err(GitError::new("no pull request URL in output"))
    }

    pub async fn open_pr(
        &self,
        project: &Path,
        title: &str,
        body: &str,
    ) -> Result<OpenedPr, GitError> {
        if title.starts_with('-') {
            return Err(GitError::new("invalid title"));
        }
        if !self.available().await {
            return Err(GitError::new("GitHub CLI is not available"));
        }
        let inside = self
            .git(
                vec!["rev-parse".into(), "--is-inside-work-tree".into()],
                project,
            )
            .await;
        if inside.code != 0 || inside.stdout.trim() != "true" {
            return Err(GitError::new("not a git repository"));
        }
        let origin = self
            .git(
                vec!["remote".into(), "get-url".into(), "origin".into()],
                project,
            )
            .await;
        if origin.code != 0 {
            return Err(GitError::new("no origin remote"));
        }
        let head = self
            .git(
                vec!["rev-parse".into(), "--abbrev-ref".into(), "HEAD".into()],
                project,
            )
            .await;
        let base = head.stdout.trim().to_string();
        if head.code != 0 || base.is_empty() || base == "HEAD" {
            return Err(GitError::new("HEAD is detached"));
        }
        let staged = self
            .git(
                vec!["diff".into(), "--cached".into(), "--quiet".into()],
                project,
            )
            .await;
        if staged.code == 0 {
            return Err(GitError::new("nothing is staged"));
        }
        if staged.code != 1 {
            return Err(GitError::new(stderr_tail(&staged.stderr)));
        }

        let branch = format!("orch/pr-{}-{}", pr_slug(title), (self.hex)());
        let created = self
            .git(vec!["switch".into(), "-c".into(), branch.clone()], project)
            .await;
        if created.code != 0 {
            return Err(GitError::new(stderr_tail(&created.stderr)));
        }

        let mut committed = false;
        let outcome = self
            .commit_push_pr(project, title, body, &base, &branch, &mut committed)
            .await;
        self.switch_back(project, &base, &branch, committed).await?;
        outcome
    }

    async fn commit_push_pr(
        &self,
        project: &Path,
        title: &str,
        body: &str,
        base: &str,
        branch: &str,
        committed: &mut bool,
    ) -> Result<OpenedPr, GitError> {
        let mut commit_args = vec!["commit".to_string(), "-m".into(), title.into()];
        if !body.is_empty() {
            commit_args.push("-m".into());
            commit_args.push(body.into());
        }
        let commit = self.git(commit_args, project).await;
        if commit.code != 0 {
            return Err(GitError::new(stderr_tail(&commit.stderr)));
        }
        *committed = true;
        let push = self
            .git(
                vec!["push".into(), "-u".into(), "origin".into(), branch.into()],
                project,
            )
            .await;
        if push.code != 0 {
            return Err(GitError::new(format!(
                "Could not push ({}). Your change is committed on branch {branch}.",
                last_line(&push.stderr)
            )));
        }
        match self
            .create_pr(project, title, body, Some(base), Some(branch))
            .await
        {
            Ok(url) => Ok(OpenedPr {
                url,
                branch: branch.to_string(),
            }),
            Err(err) => Err(GitError::new(format!(
                "Could not open the pull request ({}). Your change is pushed on branch {branch}.",
                last_line(&err.to_string())
            ))),
        }
    }

    async fn switch_back(
        &self,
        project: &Path,
        base: &str,
        branch: &str,
        committed: bool,
    ) -> Result<(), GitError> {
        let back = self.git(vec!["switch".into(), base.into()], project).await;
        if back.code == 0 {
            if !committed {
                let _ = self
                    .git(vec!["branch".into(), "-D".into(), branch.into()], project)
                    .await;
            }
            Ok(())
        } else {
            Err(GitError::new(format!(
                "Could not switch back to {base} ({}). You are on branch {branch}.",
                last_line(&back.stderr)
            )))
        }
    }

    async fn git(&self, args: Vec<String>, cwd: &Path) -> CmdOut {
        self.git_at("git", args, Some(cwd)).await
    }

    async fn git_at(&self, command: &str, args: Vec<String>, cwd: Option<&Path>) -> CmdOut {
        self.run
            .run(command.to_string(), args, cwd.map(Path::to_path_buf))
            .await
    }
}

fn parse_repo(value: &Value) -> Option<GithubRepo> {
    let row = value.as_object()?;
    let name = row.get("nameWithOwner")?.as_str()?;
    if name.is_empty() {
        return None;
    }
    Some(GithubRepo {
        name_with_owner: name.to_string(),
        description: row
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        is_private: row.get("isPrivate").and_then(Value::as_bool) == Some(true),
        updated_at: row
            .get("updatedAt")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    })
}

fn assert_name(name: &str) -> Result<(), GitError> {
    let Some((owner, repo)) = name.split_once('/') else {
        return Err(invalid_name());
    };
    if name.matches('/').count() != 1
        || owner.is_empty()
        || repo.is_empty()
        || owner.starts_with('-')
        || repo.starts_with('-')
        || !owner.chars().all(is_name_char)
        || !repo.chars().all(is_name_char)
        || repo.rsplit(['/', '\\']).next() != Some(repo)
    {
        return Err(invalid_name());
    }
    Ok(())
}

fn is_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-')
}

fn invalid_name() -> GitError {
    GitError::new("invalid repository name")
}

fn stderr_tail(stderr: &str) -> String {
    let trimmed = stderr.trim();
    if utf16_len(trimmed) <= 500 {
        trimmed.to_string()
    } else {
        utf16_suffix(trimmed, 500)
    }
}

fn last_line(text: &str) -> String {
    let line = text
        .split(['\n', '\r'])
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or("no details");
    if utf16_len(line) <= 200 {
        line.to_string()
    } else {
        utf16_prefix(line, 200)
    }
}

fn pr_slug(title: &str) -> String {
    let lower = title.to_lowercase();
    let mut dashed = String::new();
    let mut prev_dash = false;
    for ch in lower.chars() {
        if ch.is_ascii_alphanumeric() {
            dashed.push(ch);
            prev_dash = false;
        } else if !prev_dash {
            dashed.push('-');
            prev_dash = true;
        }
    }
    let cut = utf16_prefix(dashed.trim_matches('-'), 40);
    let cut = cut.trim_end_matches('-');
    if cut.is_empty() {
        "change".to_string()
    } else {
        cut.to_string()
    }
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn random_hex() -> String {
    let bytes = *uuid::Uuid::new_v4().as_bytes();
    format!("{:02x}{:02x}{:02x}", bytes[0], bytes[1], bytes[2])
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}
