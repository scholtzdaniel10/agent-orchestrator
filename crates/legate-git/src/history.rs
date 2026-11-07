//! Commit history. Read-only: these calls never change the repository.

use std::path::Path;

use crate::cmd::{CmdOut, CmdRun};
use crate::error::GitError;
use crate::gitpath::{cap_show_diff, nonempty_line};

pub const PAGE: i64 = 100;

const FIELD: char = '\u{1f}';
const RECORD: char = '\u{1e}';

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub short: String,
    pub parents: Vec<String>,
    pub author: String,
    /// Author time, in milliseconds.
    pub time: i64,
    pub subject: String,
    /// Branch, tag, and HEAD names, as `git log --format=%D` prints them.
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitFile {
    pub path: String,
    /// `None` for a binary file.
    pub added: Option<i64>,
    pub removed: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitDetail {
    pub hash: String,
    pub author: String,
    pub email: String,
    pub time: i64,
    pub message: String,
    pub files: Vec<CommitFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitPage {
    pub commits: Vec<Commit>,
    pub more: bool,
}

pub struct History<R> {
    run: R,
}

impl History<crate::cmd::ProcessRun> {
    pub fn new() -> Self {
        Self {
            run: crate::cmd::ProcessRun::command(),
        }
    }
}

impl Default for History<crate::cmd::ProcessRun> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: CmdRun> History<R> {
    pub fn with_run(run: R) -> Self {
        Self { run }
    }

    /// Commits on HEAD, and on its upstream when there is one, newest first.
    pub async fn log(&self, dir: &Path, skip: i64, limit: i64) -> Result<CommitPage, GitError> {
        let inside = self
            .git(
                vec!["rev-parse".into(), "--is-inside-work-tree".into()],
                dir,
            )
            .await;
        if inside.code != 0 {
            return Err(GitError::new("This folder is not a git repository."));
        }
        let head = self
            .git(
                vec![
                    "rev-parse".into(),
                    "--verify".into(),
                    "--quiet".into(),
                    "HEAD".into(),
                ],
                dir,
            )
            .await;
        if head.code != 0 {
            return Ok(CommitPage {
                commits: Vec::new(),
                more: false,
            });
        }
        let upstream = self
            .git(
                vec![
                    "rev-parse".into(),
                    "--verify".into(),
                    "--quiet".into(),
                    "@{upstream}".into(),
                ],
                dir,
            )
            .await;
        let mut args = vec![
            "log".to_string(),
            "--date-order".to_string(),
            format!("--format=%H{FIELD}%h{FIELD}%P{FIELD}%an{FIELD}%at{FIELD}%D{FIELD}%s{RECORD}"),
            format!("--skip={}", skip.max(0)),
            "-n".to_string(),
            (limit.max(1).saturating_add(1)).to_string(),
        ];
        if upstream.code == 0 {
            args.push("HEAD".to_string());
            args.push("@{upstream}".to_string());
        } else {
            args.push("HEAD".to_string());
        }
        args.push("--".to_string());
        let out = self.git(args, dir).await;
        if out.code != 0 {
            return Err(failed(&out.stderr, "git log failed."));
        }
        let commits = parse_log(&out.stdout);
        let more = commits.len() as i64 > limit;
        let commits = if more {
            let keep = usize::try_from(limit).unwrap_or(0);
            commits.into_iter().take(keep).collect()
        } else {
            commits
        };
        Ok(CommitPage { commits, more })
    }

    pub async fn detail(&self, dir: &Path, hash: &str) -> Result<CommitDetail, GitError> {
        let id = require_hash(hash)?;
        let format = format!("--format=%H{FIELD}%an{FIELD}%ae{FIELD}%at{FIELD}%B");
        let head_args = vec!["show".into(), "-s".into(), format, id.clone(), "--".into()];
        let stat_args = vec![
            "show".into(),
            "--diff-merges=first-parent".into(),
            "--no-renames".into(),
            "--numstat".into(),
            "--format=".into(),
            id,
            "--".into(),
        ];
        let (head, stat) = tokio::join!(self.git(head_args, dir), self.git(stat_args, dir));
        if head.code != 0 {
            return Err(failed(&head.stderr, "That commit was not found."));
        }
        if stat.code != 0 {
            return Err(failed(&stat.stderr, "git show failed."));
        }
        let mut parts = head.stdout.split(FIELD);
        let full = parts.next().unwrap_or("").trim().to_string();
        let author = parts.next().unwrap_or("").to_string();
        let email = parts.next().unwrap_or("").to_string();
        let time = parts.next().unwrap_or("0");
        let message = join_field(parts).trim().to_string();
        Ok(CommitDetail {
            hash: full,
            author,
            email,
            time: time.trim().parse::<i64>().unwrap_or(0).saturating_mul(1000),
            message,
            files: parse_numstat(&stat.stdout),
        })
    }

    /// One file's diff in a commit, against its first parent.
    pub async fn file_diff(&self, dir: &Path, hash: &str, path: &str) -> Result<String, GitError> {
        let id = require_hash(hash)?;
        let out = self
            .git(
                vec![
                    "show".into(),
                    "--diff-merges=first-parent".into(),
                    "--no-renames".into(),
                    "--format=".into(),
                    id,
                    "--".into(),
                    path.to_string(),
                ],
                dir,
            )
            .await;
        if out.code != 0 {
            return Err(failed(&out.stderr, "git show failed."));
        }
        Ok(cap_show_diff(&out.stdout))
    }

    async fn git(&self, args: Vec<String>, dir: &Path) -> CmdOut {
        self.run
            .run("git".to_string(), args, Some(dir.to_path_buf()))
            .await
    }
}

pub fn parse_log(out: &str) -> Vec<Commit> {
    let mut commits = Vec::new();
    for record in out.split(RECORD) {
        let text = trim_start_ws(record);
        if text.is_empty() {
            continue;
        }
        let mut parts = text.split(FIELD);
        let hash = parts.next().unwrap_or("");
        let short = parts.next().unwrap_or("");
        let parents = parts.next().unwrap_or("");
        let author = parts.next().unwrap_or("");
        let time = parts.next().unwrap_or("0");
        let refs = parts.next().unwrap_or("");
        let subject = join_field(parts);
        if !is_commit_hash(hash) {
            continue;
        }
        commits.push(Commit {
            hash: hash.to_string(),
            short: short.to_string(),
            parents: parents
                .split(' ')
                .filter(|parent| !parent.is_empty())
                .map(str::to_string)
                .collect(),
            author: author.to_string(),
            time: time.trim().parse::<i64>().unwrap_or(0).saturating_mul(1000),
            subject: subject.trim().to_string(),
            refs: refs
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect(),
        });
    }
    commits
}

pub fn parse_numstat(out: &str) -> Vec<CommitFile> {
    let mut files = Vec::new();
    for line in out.split(['\n', '\r']) {
        let Some((added, rest)) = line.split_once('\t') else {
            continue;
        };
        let Some((removed, path)) = rest.split_once('\t') else {
            continue;
        };
        if path.is_empty() {
            continue;
        }
        if !is_count(added) || !is_count(removed) {
            continue;
        }
        files.push(CommitFile {
            path: path.to_string(),
            added: count_or_null(added),
            removed: count_or_null(removed),
        });
    }
    files
}

pub fn require_hash(hash: &str) -> Result<String, GitError> {
    if !is_commit_hash(hash) {
        return Err(GitError::new("Not a commit id."));
    }
    Ok(hash.to_string())
}

fn is_commit_hash(hash: &str) -> bool {
    (7..=40).contains(&hash.len())
        && hash
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_count(text: &str) -> bool {
    text == "-" || text.bytes().all(|byte| byte.is_ascii_digit())
}

fn count_or_null(text: &str) -> Option<i64> {
    if text == "-" { None } else { text.parse().ok() }
}

fn join_field<'a>(parts: impl Iterator<Item = &'a str>) -> String {
    let mut out = String::new();
    for (index, part) in parts.enumerate() {
        if index > 0 {
            out.push(FIELD);
        }
        out.push_str(part);
    }
    out
}

fn trim_start_ws(text: &str) -> &str {
    text.trim_start_matches(|ch: char| ch.is_whitespace())
}

fn failed(stderr: &str, fallback: &str) -> GitError {
    GitError::new(nonempty_line(stderr).unwrap_or(fallback))
}
