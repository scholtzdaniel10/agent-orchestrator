//! Branch status and source control actions.

use std::path::Path;

use legate_core::{BranchStatus, FileEntry};

use crate::cmd::{CmdOut, CmdRun, ProcessRun, strings};
use crate::error::GitError;
use crate::files::parse_status_v2;
use crate::history::parse_numstat;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrTarget {
    pub title: String,
    pub base: String,
    pub head: String,
}

pub struct SourceControl<R: CmdRun> {
    run: R,
}

impl SourceControl<ProcessRun> {
    pub fn new() -> Self {
        Self::with(ProcessRun::command())
    }
}

impl Default for SourceControl<ProcessRun> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: CmdRun> SourceControl<R> {
    pub fn with(run: R) -> Self {
        Self { run }
    }

    pub async fn status(&self, dir: &Path) -> Result<BranchStatus, GitError> {
        let out = self
            .checked(
                dir,
                &[
                    "status",
                    "--porcelain=v2",
                    "-z",
                    "--branch",
                    "--no-ahead-behind",
                    "--untracked-files=all",
                ],
            )
            .await?;
        let mut branch = String::new();
        let mut unborn = false;
        let mut upstream = None;
        let mut upstream_exists = false;
        for header in out
            .stdout
            .split('\0')
            .take_while(|row| row.starts_with("# "))
        {
            if let Some(name) = header.strip_prefix("# branch.head ") {
                branch = name.to_string();
            } else if let Some(name) = header.strip_prefix("# branch.upstream ") {
                upstream = Some(name.to_string());
            } else if header.starts_with("# branch.ab ") {
                upstream_exists = true;
            } else if header == "# branch.oid (initial)" {
                unborn = true;
            }
        }
        let uncommitted = parse_status_v2(out.stdout_z())
            .into_iter()
            .map(|(path, status)| FileEntry { path, status })
            .collect();
        let detached = branch == "(detached)";
        let base = self.base(dir).await;
        let mut status = BranchStatus {
            branch,
            upstream: upstream.filter(|_| !detached),
            base,
            ahead: 0,
            behind: 0,
            uncommitted,
            commits: 0,
            insertions: 0,
            deletions: 0,
        };
        if unborn {
            return Ok(status);
        }
        if !detached && upstream_exists {
            let counts = self
                .checked(
                    dir,
                    &[
                        "rev-list",
                        "--left-right",
                        "--count",
                        "@{upstream}...HEAD",
                        "--",
                    ],
                )
                .await?;
            let (behind, ahead) = counts
                .stdout
                .trim()
                .split_once('\t')
                .ok_or_else(|| GitError::new("Invalid Git upstream counts."))?;
            status.behind = count(behind)?;
            status.ahead = count(ahead)?;
        }
        if let Some(base) = &status.base {
            let range = format!("refs/remotes/{base}..HEAD");
            let commits = self
                .checked(dir, &["rev-list", "--count", &range, "--"])
                .await?;
            status.commits = count(commits.stdout.trim())?;
            let range = format!("refs/remotes/{base}...HEAD");
            let diff = self
                .checked(dir, &["diff", "--numstat", &range, "--"])
                .await?;
            for file in parse_numstat(&diff.stdout) {
                status.insertions += file.added.unwrap_or_default();
                status.deletions += file.removed.unwrap_or_default();
            }
        }
        Ok(status)
    }

    pub async fn commit_all(&self, dir: &Path, message: &str) -> Result<(), GitError> {
        if message.trim().is_empty() {
            return Err(GitError::new("Write a commit message first."));
        }
        self.checked(dir, &["add", "-A"]).await?;
        let diff = self.git(dir, &["diff", "--cached", "--quiet", "--"]).await;
        match diff.code {
            0 => return Err(GitError::new("There is nothing to commit.")),
            1 if diff.stderr.is_empty() => {}
            _ => return Err(command_error(&diff)),
        }
        self.checked(dir, &["commit", "-m", message]).await?;
        Ok(())
    }

    pub async fn push(&self, dir: &Path) -> Result<(), GitError> {
        let branch = self
            .branch(dir)
            .await?
            .ok_or_else(|| GitError::new("Check out a branch before pushing."))?;
        if self.has_upstream(dir, &branch).await? {
            self.checked(dir, &["push"]).await?;
        } else {
            let reference = format!("refs/heads/{branch}");
            self.checked(dir, &["push", "--set-upstream", "origin", &reference])
                .await?;
        }
        Ok(())
    }

    pub async fn pr_target(&self, dir: &Path) -> Result<PrTarget, GitError> {
        let head = self
            .branch(dir)
            .await?
            .ok_or_else(|| GitError::new("Publish the branch first."))?;
        if !self.has_upstream(dir, &head).await? {
            return Err(GitError::new("Publish the branch first."));
        }
        let base = self
            .base(dir)
            .await
            .ok_or_else(|| GitError::new("This repository has no origin to open it on."))?;
        let base = base.strip_prefix("origin/").unwrap_or(&base).to_string();
        let log = self.checked(dir, &["log", "-1", "--format=%s"]).await?;
        Ok(PrTarget {
            title: log.stdout.trim_end().to_string(),
            base,
            head,
        })
    }

    async fn branch(&self, dir: &Path) -> Result<Option<String>, GitError> {
        let out = self.git(dir, &["symbolic-ref", "--quiet", "HEAD"]).await;
        match out.code {
            0 => Ok(out
                .stdout
                .trim()
                .strip_prefix("refs/heads/")
                .map(str::to_owned)),
            1 if out.stderr.is_empty() => Ok(None),
            _ => Err(command_error(&out)),
        }
    }

    async fn has_upstream(&self, dir: &Path, branch: &str) -> Result<bool, GitError> {
        let reference = format!("refs/heads/{branch}");
        let out = self
            .checked(dir, &["for-each-ref", "--format=%(upstream)", &reference])
            .await?;
        Ok(!out.stdout.trim().is_empty())
    }

    async fn base(&self, dir: &Path) -> Option<String> {
        let out = self
            .git(
                dir,
                &["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"],
            )
            .await;
        let target = (out.code == 0)
            .then_some(out.stdout.trim())
            .filter(|name| name.starts_with("refs/remotes/origin/"));
        for reference in target
            .into_iter()
            .chain(["refs/remotes/origin/main", "refs/remotes/origin/master"])
        {
            let exists = self
                .git(dir, &["show-ref", "--verify", "--quiet", reference])
                .await;
            if exists.code == 0 {
                return reference.strip_prefix("refs/remotes/").map(str::to_owned);
            }
        }
        None
    }

    async fn checked(&self, dir: &Path, args: &[&str]) -> Result<CmdOut, GitError> {
        let out = self.git(dir, args).await;
        if out.code == 0 {
            Ok(out)
        } else {
            Err(command_error(&out))
        }
    }

    async fn git(&self, dir: &Path, args: &[&str]) -> CmdOut {
        self.run
            .run("git".into(), strings(args), Some(dir.to_path_buf()))
            .await
    }
}

fn count(text: &str) -> Result<i64, GitError> {
    text.parse()
        .map_err(|err| GitError::new(format!("Invalid Git count: {err}")))
}

fn command_error(out: &CmdOut) -> GitError {
    let line = out
        .stderr
        .lines()
        .next()
        .filter(|line| !line.trim().is_empty());
    GitError::new(line.unwrap_or("Git command failed."))
}
