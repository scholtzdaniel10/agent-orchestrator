//! Worktrees, diffs, history, and GitHub.
//!
//! Git commands go through the git CLI. A refresh uses one
//! `git status --porcelain=v2 -z`, one `git worktree list --porcelain`,
//! and one `git diff --numstat -z` per worktree. Independent commands run
//! concurrently, and a worker thread never blocks on a child process.

mod changes;
mod cmd;
mod error;
mod files;
mod fresh;
mod github;
mod gitpath;
mod guard;
mod history;
mod project;
mod setup;
mod worktrees;

pub use changes::{ListedName, merge_listed_changes};
pub use cmd::{CmdOut, CmdRun, FnRun, ProcessRun};
pub use error::GitError;
pub use files::{FileStat, FileSys, Files, OUTSIDE_PROJECT, OsFs};
pub use fresh::{EnvMap, merged_path, refresh_path, registry_path};
pub use github::{Github, OpenedPr};
pub use guard::{
    GuardJob, JOB_STALE_MS, WORKTREE_BUSY_MESSAGE, assert_worktree_idle, worktree_has_active_job,
};
pub use history::{
    Commit, CommitDetail, CommitFile, CommitPage, History, PAGE, parse_log, parse_numstat,
    require_hash,
};
pub use project::ProjectGitCounts;
pub use setup::{
    SetupTracker, parse_setup_commands, parse_worktree_name, parse_worktree_name_value,
};
pub use worktrees::{
    ApplyOutcome, MergeOutcome, RepoStatus, WINDOWS_LOCK_MESSAGE, WorktreeLoc, Worktrees,
    remove_worktree_with_retries,
};

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod tests;
