//! Git fields of a sidebar project card: branch, waiting changes, dirty files, upstream.

use std::collections::HashSet;
use std::path::Path;

use legate_core::{ProjectInfo, ProjectWorktree, SetupState};

use crate::changes::{ListedName, merge_listed_changes};
use crate::gitpath::path_string;
use crate::worktrees::Worktrees;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectGitCounts {
    pub info: ProjectInfo,
    pub dirty: i64,
    pub ahead: i64,
    pub behind: i64,
    pub changes: i64,
    pub insertions: i64,
    pub deletions: i64,
    /// Present when this project is not the open one, matching `describeProject`.
    pub worktrees: Option<Vec<ProjectWorktree>>,
}

impl Worktrees {
    /// Branch, change totals, dirty files, and ahead/behind. Chats and running jobs stay with the caller.
    pub async fn project_git<F>(
        &self,
        path: &Path,
        active: bool,
        names: &[ListedName],
        working_ids: &[&str],
        setup: F,
    ) -> ProjectGitCounts
    where
        F: Fn(&str) -> Option<SetupState>,
    {
        let info = self.info(path).await;
        if !info.is_repo {
            return empty(info);
        }
        let (status, (listed, ids)) = tokio::join!(self.status(path), self.refresh_changes(path));
        let root = self.root.clone();
        let merged = merge_listed_changes(
            &listed,
            &ids,
            names,
            |id| path_string(&root.join(id)),
            &setup,
        );
        let mut changes = 0i64;
        let mut insertions = 0i64;
        let mut deletions = 0i64;
        for change in &merged {
            changes += 1;
            insertions += change.insertions;
            deletions += change.deletions;
        }
        let worktrees = if active {
            None
        } else {
            let working: HashSet<&str> = working_ids.iter().copied().collect();
            Some(
                merged
                    .into_iter()
                    .map(|change| ProjectWorktree {
                        working: working.contains(change.id.as_str()),
                        name: change.name.unwrap_or_default(),
                        files: i64::try_from(change.files.len()).unwrap_or(i64::MAX),
                        id: change.id,
                        branch: change.branch,
                        insertions: change.insertions,
                        deletions: change.deletions,
                    })
                    .collect(),
            )
        };
        ProjectGitCounts {
            info,
            dirty: status.dirty,
            ahead: status.ahead,
            behind: status.behind,
            changes,
            insertions,
            deletions,
            worktrees,
        }
    }
}

fn empty(info: ProjectInfo) -> ProjectGitCounts {
    ProjectGitCounts {
        info,
        dirty: 0,
        ahead: 0,
        behind: 0,
        changes: 0,
        insertions: 0,
        deletions: 0,
        worktrees: None,
    }
}
