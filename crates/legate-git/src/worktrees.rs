//! Job worktrees: `orch/<id>` branches under one root folder.

use std::path::{Path, PathBuf};

use futures::future::join_all;
use legate_core::{ChangeSet, ChangedFile, ProjectInfo};

use crate::cmd::{self, RawOut, strings};
use crate::error::GitError;
use crate::gitpath::{
    canonicalize_path, cap_diff, git_path, is_inside, normalize_abs, path_string, read_until_nul,
};

pub const WINDOWS_LOCK_MESSAGE: &str =
    "Windows still has files open in this worktree. Close programs using it and try again.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoStatus {
    pub dirty: i64,
    pub ahead: i64,
    pub behind: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeLoc {
    pub id: String,
    pub path: PathBuf,
    pub branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyOutcome {
    pub ok: bool,
    pub empty: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeOutcome {
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ListedWorktree {
    pub path: String,
    pub branch: Option<String>,
}

pub struct Worktrees {
    pub root: PathBuf,
}

impl Worktrees {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Uncommitted files and distance from upstream. Zeros when `project` is not a repo.
    pub async fn status(&self, project: &Path) -> RepoStatus {
        let mut result = RepoStatus {
            dirty: 0,
            ahead: 0,
            behind: 0,
        };
        let raw = run_git(
            &strings(&["status", "--porcelain=v2", "-z", "--branch"]),
            project,
        )
        .await;
        if raw.code != 0 {
            return result;
        }
        fill_status(&raw.stdout, &mut result);
        result
    }

    /// Never fails. A missing folder or a non-repo is `is_repo: false`.
    pub async fn info(&self, project: &Path) -> ProjectInfo {
        let path = path_string(project);
        let inside = git_ok(&strings(&["rev-parse", "--is-inside-work-tree"]), project).await;
        let is_repo = matches!(inside, Ok(text) if text.trim() == "true");
        if !is_repo {
            return ProjectInfo {
                path,
                is_repo: false,
                branch: None,
            };
        }
        match git_ok(
            &strings(&["symbolic-ref", "--quiet", "--short", "HEAD"]),
            project,
        )
        .await
        {
            Ok(name) => {
                let name = name.trim();
                ProjectInfo {
                    path,
                    is_repo: true,
                    branch: if name.is_empty() {
                        None
                    } else {
                        Some(name.to_string())
                    },
                }
            }
            Err(_) => ProjectInfo {
                path,
                is_repo: true,
                branch: None,
            },
        }
    }

    /// The worktree for a job, on its own branch from `base` (`HEAD` when omitted).
    pub async fn create(
        &self,
        project: &Path,
        job_id: &str,
        base: Option<&str>,
    ) -> Result<WorktreeLoc, GitError> {
        let base = base.unwrap_or("HEAD");
        let located = self.locate(&id_from_job(job_id)?)?;
        let about = self.info(project).await;
        if !about.is_repo {
            return Err(GitError::new("not a git repository"));
        }
        if located.path.exists() || self.branch_exists(project, &located.branch).await {
            return Ok(located);
        }
        if base == "HEAD"
            && git_ok(
                &strings(&["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]),
                project,
            )
            .await
            .is_err()
        {
            let name = project
                .file_name()
                .map(git_os)
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "repository".to_string());
            return Err(GitError::new(format!(
                "{name} has no commits yet, so Legate cannot make a worktree from it. Make a first commit there, then try again."
            )));
        }
        std::fs::create_dir_all(&self.root)?;
        let project_s = path_string(project);
        let path_s = path_string(&located.path);
        let added = git_ok(
            &strings(&[
                "-C",
                &project_s,
                "worktree",
                "add",
                "-b",
                &located.branch,
                &path_s,
                base,
            ]),
            project,
        )
        .await;
        if let Err(err) = added {
            if located.path.exists() || self.branch_exists(project, &located.branch).await {
                return Ok(located);
            }
            return Err(err);
        }
        Ok(located)
    }

    pub fn branch_of(&self, id: &str) -> Result<String, GitError> {
        Ok(self.locate(id)?.branch)
    }

    pub fn folder(&self, id: &str) -> Result<PathBuf, GitError> {
        Ok(self.locate(id)?.path)
    }

    /// Applies what worktree `from_id` changed since it left `base` into `target`, staged.
    pub async fn apply_into(
        &self,
        from_id: &str,
        target: &Path,
        base: &str,
    ) -> Result<ApplyOutcome, GitError> {
        let located = self.locate(from_id)?;
        self.assert_worktree(&located).await?;
        let dir = std::env::temp_dir().join(unique_temp("orch-fold-"));
        std::fs::create_dir_all(&dir)?;
        let result = self.apply_into_dir(&located, target, base, &dir).await;
        let _ = std::fs::remove_dir_all(&dir);
        result
    }

    async fn apply_into_dir(
        &self,
        located: &WorktreeLoc,
        target: &Path,
        base: &str,
        dir: &Path,
    ) -> Result<ApplyOutcome, GitError> {
        git_ok(&strings(&["add", "-A"]), &located.path).await?;
        let from = git_ok(
            &strings(&["merge-base", base, &located.branch]),
            &located.path,
        )
        .await?;
        let from = from.trim();
        let patch = git_bytes(
            &strings(&["diff", "--cached", "--binary", from]),
            &located.path,
        )
        .await?;
        if patch.is_empty() {
            return Ok(ApplyOutcome {
                ok: true,
                empty: true,
                message: "no changes".to_string(),
            });
        }
        let file = dir.join("change.diff");
        std::fs::write(&file, &patch)?;
        if let Err(err) = git_ok(
            &strings(&["apply", "--index", "--check", &path_string(&file)]),
            target,
        )
        .await
        {
            return Ok(ApplyOutcome {
                ok: false,
                empty: false,
                message: format!("does not apply cleanly: {}", first_error_line(&err)),
            });
        }
        git_ok(&strings(&["apply", "--index", &path_string(&file)]), target).await?;
        Ok(ApplyOutcome {
            ok: true,
            empty: false,
            message: "applied".to_string(),
        })
    }

    /// Commits everything in `dir`. False when there is nothing staged.
    pub async fn commit_all(&self, dir: &Path, message: &str) -> Result<bool, GitError> {
        git_ok(&strings(&["add", "-A"]), dir).await?;
        let staged = git_ok(&strings(&["diff", "--cached", "--name-only"]), dir).await?;
        if staged.trim().is_empty() {
            return Ok(false);
        }
        let mut args = Vec::new();
        if git_ok(&strings(&["config", "user.email"]), dir)
            .await
            .is_err()
        {
            args.extend(strings(&[
                "-c",
                "user.name=Legate",
                "-c",
                "user.email=legate@localhost",
            ]));
        }
        args.extend(strings(&["commit", "--no-verify", "-q", "-m", message]));
        git_ok(&args, dir).await?;
        Ok(true)
    }

    /// Every orch worktree id for this project, including ones with no changed files.
    pub async fn ids(&self, project: &Path) -> Vec<String> {
        let about = self.info(project).await;
        if !about.is_repo {
            return Vec::new();
        }
        let mut ids = self.orch_ids(project).await;
        ids.sort();
        ids
    }

    pub async fn list(&self, project: &Path) -> Vec<ChangeSet> {
        let about = self.info(project).await;
        if !about.is_repo {
            return Vec::new();
        }
        let Some(entries) = self.porcelain(project).await else {
            return Vec::new();
        };
        let selected = self.selected(project, &entries);
        let mut changes = self.summarize_many(project, selected).await;
        changes.sort_by(|a, b| a.id.cmp(&b.id));
        changes
    }

    pub async fn diff(&self, project: &Path, id: &str) -> Result<String, GitError> {
        let located = self.locate(id)?;
        self.assert_worktree(&located).await?;
        let add_args = strings(&["add", "-A"]);
        let (added, base) = tokio::join!(
            git_ok(&add_args, &located.path),
            self.merge_base(project, &located.branch),
        );
        added?;
        let base = base?;
        let text = git_ok(&strings(&["diff", "--cached", &base]), &located.path).await?;
        Ok(cap_diff(&text))
    }

    pub async fn merge(&self, project: &Path, id: &str) -> Result<MergeOutcome, GitError> {
        let located = self.locate(id)?;
        self.assert_worktree(&located).await?;
        let add_args = strings(&["add", "-A"]);
        let (added, base) = tokio::join!(
            git_ok(&add_args, &located.path),
            self.merge_base(project, &located.branch),
        );
        added?;
        let base = base?;
        let patch = git_bytes(
            &strings(&["diff", "--cached", "--binary", &base]),
            &located.path,
        )
        .await?;
        if patch.is_empty() {
            return Ok(MergeOutcome {
                ok: false,
                message: "nothing to merge".to_string(),
            });
        }
        let dir = std::env::temp_dir().join(unique_temp("orch-apply-"));
        std::fs::create_dir_all(&dir)?;
        let file = dir.join("change.diff");
        let wrote = std::fs::write(&file, &patch);
        let outcome = if let Err(err) = wrote {
            Err(GitError::from(err))
        } else {
            self.apply_merge(project, &located, &file).await
        };
        let _ = std::fs::remove_dir_all(&dir);
        outcome
    }

    async fn apply_merge(
        &self,
        project: &Path,
        located: &WorktreeLoc,
        file: &Path,
    ) -> Result<MergeOutcome, GitError> {
        let top = git_ok(&strings(&["rev-parse", "--show-toplevel"]), project).await?;
        let top = PathBuf::from(top.trim());
        let file_s = path_string(file);
        if let Err(err) = git_ok(&strings(&["apply", "--index", "--check", &file_s]), &top).await {
            return Ok(MergeOutcome {
                ok: false,
                message: format!("does not apply cleanly: {}", first_error_line(&err)),
            });
        }
        git_ok(&strings(&["apply", "--index", &file_s]), &top).await?;
        let about = self.info(project).await;
        let where_ = about.branch.unwrap_or_else(|| "the project".to_string());
        self.discard(project, &located.id).await?;
        Ok(MergeOutcome {
            ok: true,
            message: format!("applied to {where_} as staged changes"),
        })
    }

    pub async fn discard(&self, project: &Path, id: &str) -> Result<(), GitError> {
        let located = self.locate(id)?;
        let project_s = path_string(project);
        let path_s = path_string(&located.path);
        let branch = located.branch.clone();
        remove_worktree_with_retries(
            || {
                let args = strings(&["-C", &project_s, "worktree", "remove", "--force", &path_s]);
                let project = project.to_path_buf();
                async move { git_ok(&args, &project).await.map(|_| ()) }
            },
            || !located.path.exists(),
            |ms| async move {
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            },
        )
        .await?;
        git_allow_missing(
            &strings(&["-C", &project_s, "branch", "-D", &branch]),
            project,
        )
        .await?;
        git_allow_missing(&strings(&["-C", &project_s, "worktree", "prune"]), project).await?;
        Ok(())
    }

    /// One porcelain read, then one numstat per orch worktree, for a project-card refresh.
    pub(crate) async fn refresh_changes(&self, project: &Path) -> (Vec<ChangeSet>, Vec<String>) {
        let Some(entries) = self.porcelain(project).await else {
            return (Vec::new(), Vec::new());
        };
        let selected = self.selected(project, &entries);
        let mut ids = selected
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        ids.sort();
        let mut listed = self.summarize_many(project, selected).await;
        listed.sort_by(|a, b| a.id.cmp(&b.id));
        (listed, ids)
    }

    fn locate(&self, id: &str) -> Result<WorktreeLoc, GitError> {
        if !is_change_id(id) {
            return Err(GitError::new("invalid change id"));
        }
        let path = self.root.join(id);
        if !self.path_inside_root(&path) {
            return Err(GitError::new("invalid change id"));
        }
        Ok(WorktreeLoc {
            id: id.to_string(),
            path,
            branch: format!("orch/{id}"),
        })
    }

    async fn assert_worktree(&self, located: &WorktreeLoc) -> Result<(), GitError> {
        let current = match git_ok(
            &strings(&["symbolic-ref", "--quiet", "--short", "HEAD"]),
            &located.path,
        )
        .await
        {
            Ok(text) => text.trim().to_string(),
            Err(_) => String::new(),
        };
        if current != located.branch {
            return Err(GitError::new("not a change worktree"));
        }
        Ok(())
    }

    fn path_inside_root(&self, target: &Path) -> bool {
        match (canonicalize_path(&self.root), canonicalize_path(target)) {
            (Some(root), Some(target)) => is_inside(&root, &target, false),
            _ => is_inside(&normalize_abs(&self.root), &normalize_abs(target), false),
        }
    }

    async fn branch_exists(&self, project: &Path, branch: &str) -> bool {
        let project_s = path_string(project);
        let reference = format!("refs/heads/{branch}");
        git_ok(
            &strings(&[
                "-C",
                &project_s,
                "rev-parse",
                "--verify",
                "--quiet",
                &reference,
            ]),
            project,
        )
        .await
        .is_ok()
    }

    async fn merge_base(&self, project: &Path, branch: &str) -> Result<String, GitError> {
        let project_s = path_string(project);
        let base = git_ok(
            &strings(&["-C", &project_s, "merge-base", "HEAD", branch]),
            project,
        )
        .await?;
        let base = base.trim();
        if base.is_empty() {
            return Err(GitError::new("no merge base"));
        }
        Ok(base.to_string())
    }

    async fn porcelain(&self, project: &Path) -> Option<Vec<ListedWorktree>> {
        let project_s = path_string(project);
        let raw = run_git(
            &strings(&["-C", &project_s, "worktree", "list", "--porcelain"]),
            project,
        )
        .await;
        if raw.code != 0 {
            return None;
        }
        Some(parse_worktrees(&raw.stdout))
    }

    async fn orch_ids(&self, project: &Path) -> Vec<String> {
        let Some(entries) = self.porcelain(project).await else {
            return Vec::new();
        };
        self.selected(project, &entries)
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    }

    fn selected(&self, project: &Path, entries: &[ListedWorktree]) -> Vec<(String, PathBuf)> {
        let mut out = Vec::new();
        for entry in entries {
            let Some(branch) = entry.branch.as_deref() else {
                continue;
            };
            let Some(id) = orch_id(branch) else {
                continue;
            };
            let listed = resolve_listed(project, &entry.path);
            if !self.path_inside_root(&listed) {
                continue;
            }
            out.push((id.to_string(), listed));
        }
        out
    }

    async fn summarize_many(
        &self,
        project: &Path,
        selected: Vec<(String, PathBuf)>,
    ) -> Vec<ChangeSet> {
        let futs = selected
            .into_iter()
            .map(|(id, path)| self.summarize(project, id, path));
        let results = join_all(futs).await;
        results
            .into_iter()
            .filter_map(Result::ok)
            .flatten()
            .collect()
    }

    async fn summarize(
        &self,
        project: &Path,
        id: String,
        listed_path: PathBuf,
    ) -> Result<Option<ChangeSet>, GitError> {
        let branch = format!("orch/{id}");
        let add_args = strings(&["add", "-A"]);
        let (added, base) = tokio::join!(
            git_ok(&add_args, &listed_path),
            self.merge_base(project, &branch),
        );
        added?;
        let base = base?;
        let raw = run_git(
            &strings(&["diff", "--cached", "--numstat", "-z", &base]),
            &listed_path,
        )
        .await;
        if raw.code != 0 {
            return Err(GitError::new(raw.stderr_text()));
        }
        let files = parse_numstat_z(&raw.stdout);
        if files.is_empty() {
            return Ok(None);
        }
        let mut insertions = 0i64;
        let mut deletions = 0i64;
        for file in &files {
            if let Some(added) = file.insertions {
                insertions += added;
            }
            if let Some(removed) = file.deletions {
                deletions += removed;
            }
        }
        Ok(Some(ChangeSet {
            id: id.clone(),
            branch,
            path: path_string(&self.root.join(&id)),
            files,
            insertions,
            deletions,
            name: None,
            setup: None,
        }))
    }
}

pub async fn remove_worktree_with_retries<R, Fut, D, DFut>(
    mut remove: R,
    mut folder_gone: impl FnMut() -> bool,
    mut delay: D,
) -> Result<(), GitError>
where
    R: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<(), GitError>>,
    D: FnMut(u64) -> DFut,
    DFut: std::future::Future<Output = ()>,
{
    for attempt in 0..3 {
        let _ = remove().await;
        if folder_gone() {
            return Ok(());
        }
        if attempt < 2 {
            delay(300).await;
        }
    }
    Err(GitError::new(WINDOWS_LOCK_MESSAGE))
}

fn id_from_job(job_id: &str) -> Result<String, GitError> {
    let id: String = job_id.chars().take(8).collect();
    if !is_change_id(&id) {
        return Err(GitError::new("invalid change id"));
    }
    Ok(id)
}

pub(crate) fn is_change_id(id: &str) -> bool {
    id.len() == 8
        && id
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn orch_id(branch: &str) -> Option<&str> {
    let rest = branch.strip_prefix("refs/heads/orch/")?;
    if is_change_id(rest) { Some(rest) } else { None }
}

fn resolve_listed(project: &Path, listed: &str) -> PathBuf {
    let path = PathBuf::from(listed);
    if path.is_absolute() {
        path
    } else {
        normalize_abs(&project.join(listed))
    }
}

pub(crate) fn parse_worktrees(bytes: &[u8]) -> Vec<ListedWorktree> {
    let mut listed = Vec::new();
    let mut current: Option<ListedWorktree> = None;
    for line in split_lines(bytes) {
        if let Some(path) = line.strip_prefix(b"worktree ") {
            if let Some(prev) = current.take() {
                listed.push(prev);
            }
            current = Some(ListedWorktree {
                path: git_path(path),
                branch: None,
            });
        } else if let Some(branch) = line.strip_prefix(b"branch ")
            && let Some(current) = current.as_mut()
        {
            current.branch = Some(git_path(branch));
        }
    }
    if let Some(current) = current {
        listed.push(current);
    }
    listed
}

fn split_lines(bytes: &[u8]) -> Vec<&[u8]> {
    let mut lines = Vec::new();
    let mut start = 0usize;
    for (idx, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            let mut end = idx;
            if end > start && bytes[end - 1] == b'\r' {
                end -= 1;
            }
            lines.push(&bytes[start..end]);
            start = idx + 1;
        }
    }
    if start < bytes.len() {
        let mut end = bytes.len();
        if end > start && bytes[end - 1] == b'\r' {
            end -= 1;
        }
        lines.push(&bytes[start..end]);
    }
    lines
}

pub(crate) fn fill_status(bytes: &[u8], status: &mut RepoStatus) {
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == 0 {
            index += 1;
            continue;
        }
        if bytes[index] == b'#' {
            let (line, next) = read_until_nul(bytes, index);
            if let Some((ahead, behind)) = parse_branch_ab(line) {
                status.ahead = ahead;
                status.behind = behind;
            }
            index = next;
            continue;
        }
        let kind = bytes[index];
        let (_record, next) = read_until_nul(bytes, index);
        index = next;
        if kind == b'2' {
            let (_orig, next) = read_until_nul(bytes, index);
            index = next;
        }
        status.dirty += 1;
    }
}

fn parse_branch_ab(line: &[u8]) -> Option<(i64, i64)> {
    let text = std::str::from_utf8(line).ok()?;
    let rest = text.strip_prefix("# branch.ab +")?;
    let (ahead, behind) = rest.split_once(" -")?;
    Some((ahead.parse().ok()?, behind.parse().ok()?))
}

pub(crate) fn parse_numstat_z(bytes: &[u8]) -> Vec<ChangedFile> {
    let mut files = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        let (record, next) = read_until_nul(bytes, index);
        index = next;
        if record.is_empty() {
            continue;
        }
        let Some((added, removed, path)) = split_numstat(record) else {
            continue;
        };
        if path.is_empty() {
            let (old, next) = read_until_nul(bytes, index);
            index = next;
            let (new, next) = read_until_nul(bytes, index);
            index = next;
            if old.is_empty() && new.is_empty() {
                continue;
            }
            push_numstat(
                &mut files,
                added,
                removed,
                format!("{} => {}", git_path(old), git_path(new)),
            );
        } else {
            push_numstat(&mut files, added, removed, git_path(path));
        }
    }
    files
}

fn split_numstat(record: &[u8]) -> Option<(&[u8], &[u8], &[u8])> {
    let tab = record.iter().position(|&byte| byte == b'\t')?;
    if tab == 0 {
        return None;
    }
    let tab2_rel = record[tab + 1..].iter().position(|&byte| byte == b'\t')?;
    let tab2 = tab + 1 + tab2_rel;
    Some((&record[..tab], &record[tab + 1..tab2], &record[tab2 + 1..]))
}

fn push_numstat(files: &mut Vec<ChangedFile>, added: &[u8], removed: &[u8], path: String) {
    let Ok(added) = std::str::from_utf8(added) else {
        return;
    };
    let Ok(removed) = std::str::from_utf8(removed) else {
        return;
    };
    if added == "-" || removed == "-" {
        files.push(ChangedFile {
            path,
            insertions: None,
            deletions: None,
        });
        return;
    }
    let Ok(insertions) = added.parse::<i64>() else {
        return;
    };
    let Ok(deletions) = removed.parse::<i64>() else {
        return;
    };
    files.push(ChangedFile {
        path,
        insertions: Some(insertions),
        deletions: Some(deletions),
    });
}

fn first_error_line(err: &GitError) -> String {
    let text = err.to_string();
    crate::gitpath::nonempty_line(&text)
        .unwrap_or(&text)
        .to_string()
}

fn git_os(name: &std::ffi::OsStr) -> String {
    name.to_string_lossy().into_owned()
}

fn unique_temp(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}{n}")
}

async fn run_git(args: &[String], cwd: &Path) -> RawOut {
    cmd::spawn_raw("git", args, Some(cwd), cmd::GIT_TIMEOUT, cmd::MAX_BUFFER).await
}

async fn git_ok(args: &[String], cwd: &Path) -> Result<String, GitError> {
    let raw = run_git(args, cwd).await;
    if raw.code != 0 {
        return Err(GitError::new(raw.stderr_text()));
    }
    Ok(String::from_utf8_lossy(&raw.stdout).into_owned())
}

async fn git_bytes(args: &[String], cwd: &Path) -> Result<Vec<u8>, GitError> {
    let raw = run_git(args, cwd).await;
    if raw.code != 0 {
        return Err(GitError::new(raw.stderr_text()));
    }
    Ok(raw.stdout)
}

async fn git_allow_missing(args: &[String], cwd: &Path) -> Result<(), GitError> {
    if let Err(err) = git_ok(args, cwd).await {
        let message = err.to_string().to_lowercase();
        if message.contains("not found")
            || message.contains("not a working tree")
            || message.contains("no such")
            || message.contains("does not exist")
            || message.contains("invalid reference")
        {
            return Ok(());
        }
        return Err(err);
    }
    Ok(())
}
