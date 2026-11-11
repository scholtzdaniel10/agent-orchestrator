use std::fs;

use legate_core::SetupState;

use crate::changes::ListedName;
use crate::test_support::temp_project;
use crate::worktrees::Worktrees;

#[tokio::test]
async fn project_git_counts_dirty_changes_and_inactive_worktrees() {
    let (temp, project, wt) = temp_project();
    let _keep = temp;
    let trees = Worktrees::new(&wt);
    let created = trees
        .create(&project, "aaaaaaaa-0000-0000-0000-000000000000", None)
        .await
        .unwrap();
    fs::write(created.path.join("notes.txt"), "alpha\nextra\n").unwrap();
    let quiet = trees
        .create(&project, "bbbbbbbb-0000-0000-0000-000000000000", None)
        .await
        .unwrap();

    let counts = trees
        .project_git(
            &project,
            false,
            &[
                ListedName {
                    id: created.id.clone(),
                    name: "login".to_string(),
                    created_at: 5,
                },
                ListedName {
                    id: quiet.id.clone(),
                    name: "empty".to_string(),
                    created_at: 9,
                },
            ],
            &[&created.id],
            |id| {
                if id == quiet.id {
                    Some(SetupState::Running)
                } else {
                    None
                }
            },
        )
        .await;
    assert!(counts.info.is_repo);
    assert_eq!(counts.info.branch.as_deref(), Some("main"));
    assert_eq!(counts.changes, 2);
    assert!(counts.insertions >= 1);
    let worktrees = counts.worktrees.expect("inactive project lists worktrees");
    assert_eq!(worktrees.len(), 2);
    assert_eq!(worktrees[0].name, "empty");
    assert_eq!(worktrees[0].files, 0);
    assert!(!worktrees[0].working);
    assert_eq!(worktrees[1].name, "login");
    assert_eq!(worktrees[1].files, 1);
    assert!(worktrees[1].working);
    assert_eq!(worktrees[1].branch, "orch/aaaaaaaa");

    let active = trees.project_git(&project, true, &[], &[], |_| None).await;
    assert!(active.worktrees.is_none());
    assert_eq!(active.changes, 2);

    let plain = temp_dir_plain(&project);
    let missing = trees.project_git(&plain, false, &[], &[], |_| None).await;
    assert!(!missing.info.is_repo);
    assert_eq!(missing.dirty, 0);
    assert_eq!(missing.changes, 0);
    assert!(missing.worktrees.is_none());
}

fn temp_dir_plain(project: &std::path::Path) -> std::path::PathBuf {
    let plain = project.parent().unwrap().join("plain-card");
    fs::create_dir_all(&plain).unwrap();
    plain
}
