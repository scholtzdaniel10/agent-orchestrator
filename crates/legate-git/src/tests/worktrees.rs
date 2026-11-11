use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use legate_core::ChangedFile;

use crate::error::GitError;
use crate::gitpath::utf16_len;
use crate::test_support::{Temp, git, git_ok, git_text};
use crate::worktrees::{
    ApplyOutcome, RepoStatus, WINDOWS_LOCK_MESSAGE, Worktrees, fill_status, parse_numstat_z,
    remove_worktree_with_retries,
};

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

fn short_status(project: &Path) -> Vec<String> {
    let mut lines: Vec<String> = git_ok(project, &["status", "--short"])
        .split(['\n', '\r'])
        .map(|line| line.trim_end().to_string())
        .filter(|line| !line.is_empty())
        .collect();
    lines.sort();
    lines
}

#[tokio::test]
async fn a_goals_next_job_starts_from_the_goals_branch_and_folds_back() {
    let (temp, project, wt) = crate::test_support::temp_project();
    let _keep = temp;
    let trees = Worktrees::new(&wt);
    let goal = trees.create(&project, "a0a0a0a1-aaaa", None).await.unwrap();
    fs::write(goal.path.join("notes.txt"), "alpha\nround one\n").unwrap();
    assert!(trees.commit_all(&goal.path, "Goal round 1").await.unwrap());
    assert!(!trees.commit_all(&goal.path, "nothing").await.unwrap());

    let goal_branch = trees.branch_of(&goal.id).unwrap();
    let job = trees
        .create(&project, "b0b0b0b2-bbbb", Some(&goal_branch))
        .await
        .unwrap();
    assert_eq!(read(&job.path.join("notes.txt")), "alpha\nround one\n");
    fs::write(job.path.join("notes.txt"), "alpha\nround one\nround two\n").unwrap();
    fs::write(job.path.join("new.txt"), "new\n").unwrap();

    assert_eq!(
        trees
            .apply_into(&job.id, &goal.path, &goal_branch)
            .await
            .unwrap(),
        ApplyOutcome {
            ok: true,
            empty: false,
            message: "applied".to_string(),
        }
    );
    assert_eq!(
        read(&goal.path.join("notes.txt")),
        "alpha\nround one\nround two\n"
    );
    assert!(goal.path.join("new.txt").exists());

    let idle = trees
        .create(&project, "c0c0c0c3-cccc", Some(&goal_branch))
        .await
        .unwrap();
    let folded = trees
        .apply_into(&idle.id, &goal.path, &goal_branch)
        .await
        .unwrap();
    assert!(folded.ok);
    assert!(folded.empty);
}

#[tokio::test]
async fn status_counts_uncommitted_files_and_upstream_distance() {
    let (temp, project, wt) = crate::test_support::temp_project();
    let root = temp.path().to_path_buf();
    let _keep = temp;
    let trees = Worktrees::new(&wt);
    assert_eq!(
        trees.status(&project).await,
        RepoStatus {
            dirty: 0,
            ahead: 0,
            behind: 0
        }
    );
    fs::write(project.join("notes.txt"), "changed\n").unwrap();
    fs::write(project.join("new.txt"), "new\n").unwrap();
    assert_eq!(
        trees.status(&project).await,
        RepoStatus {
            dirty: 2,
            ahead: 0,
            behind: 0
        }
    );

    let upstream = root.join("upstream.git");
    let upstream_s = upstream.to_string_lossy().into_owned();
    let project_s = project.to_string_lossy().into_owned();
    git_ok(&root, &["clone", "--bare", &project_s, &upstream_s]);
    git_ok(&project, &["remote", "add", "origin", &upstream_s]);
    git_ok(&project, &["fetch", "origin"]);
    git_ok(&project, &["branch", "--set-upstream-to", "origin/main"]);
    git_ok(&project, &["add", "-A"]);
    git_ok(&project, &["commit", "-m", "local"]);
    assert_eq!(
        trees.status(&project).await,
        RepoStatus {
            dirty: 0,
            ahead: 1,
            behind: 0
        }
    );
    assert_eq!(
        trees.status(&root.join("missing")).await,
        RepoStatus {
            dirty: 0,
            ahead: 0,
            behind: 0
        }
    );
}

#[tokio::test]
async fn info_reports_a_repo_a_plain_folder_and_a_missing_folder() {
    let (temp, project, wt) = crate::test_support::temp_project();
    let root = temp.path().to_path_buf();
    let _keep = temp;
    let trees = Worktrees::new(&wt);
    let plain = root.join("plain");
    fs::create_dir(&plain).unwrap();
    let about = trees.info(&project).await;
    assert_eq!(about.path, project.to_string_lossy());
    assert!(about.is_repo);
    assert_eq!(about.branch.as_deref(), Some("main"));
    let plain_info = trees.info(&plain).await;
    assert!(!plain_info.is_repo);
    assert_eq!(plain_info.branch, None);
    let missing = trees.info(&root.join("missing")).await;
    assert!(!missing.is_repo);
    assert_eq!(missing.branch, None);
}

#[tokio::test]
async fn create_makes_a_worktree_and_the_same_job_returns_it() {
    let (temp, project, wt) = crate::test_support::temp_project();
    let root = temp.path().to_path_buf();
    let _keep = temp;
    let trees = Worktrees::new(&wt);
    let job_id = "abcdef12-1111-1111-1111-111111111111";
    let first = trees.create(&project, job_id, None).await.unwrap();
    assert_eq!(first.id, "abcdef12");
    assert_eq!(first.path, wt.join("abcdef12"));
    assert_eq!(first.branch, "orch/abcdef12");
    assert_eq!(
        git_text(&first.path, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "orch/abcdef12"
    );
    let again = trees.create(&project, job_id, None).await.unwrap();
    assert_eq!(again, first);

    let plain = root.join("plain");
    fs::create_dir(&plain).unwrap();
    assert_eq!(
        trees
            .create(&plain, job_id, None)
            .await
            .unwrap_err()
            .to_string(),
        "not a git repository"
    );
    assert_eq!(
        trees
            .create(&project, "zzzzzzzz", None)
            .await
            .unwrap_err()
            .to_string(),
        "invalid change id"
    );
}

#[tokio::test]
async fn create_rejects_a_repository_that_has_no_commits_yet() {
    let temp = Temp::new();
    let project = temp.path().join("empty-repo");
    fs::create_dir_all(&project).unwrap();
    git_ok(&project, &["init", "-b", "main"]);
    let trees = Worktrees::new(temp.path().join("wt"));
    assert_eq!(
        trees
            .create(&project, "abcdef12-1111-1111-1111-111111111111", None)
            .await
            .unwrap_err()
            .to_string(),
        "empty-repo has no commits yet, so Legate cannot make a worktree from it. Make a first commit there, then try again."
    );
}

#[tokio::test]
async fn list_reports_text_and_binary_changes_and_ignores_other_worktrees() {
    let (temp, project, wt) = crate::test_support::temp_project();
    let root = temp.path().to_path_buf();
    let _keep = temp;
    let outside = root.join("outside");
    fs::create_dir(&outside).unwrap();
    let trees = Worktrees::new(&wt);
    let quiet = trees
        .create(&project, "ccccccc1-0000-0000-0000-000000000000", None)
        .await
        .unwrap();
    assert!(trees.list(&project).await.is_empty());

    let outside_s = outside.join("abcdef12").to_string_lossy().into_owned();
    git_ok(
        &project,
        &["worktree", "add", "-b", "orch/abcdef12", &outside_s, "HEAD"],
    );
    fs::write(
        outside.join("abcdef12").join("notes.txt"),
        "alpha\noutside\n",
    )
    .unwrap();
    let side = wt.join("side").to_string_lossy().into_owned();
    git_ok(&project, &["worktree", "add", "-b", "side", &side, "HEAD"]);
    fs::write(wt.join("side").join("notes.txt"), "alpha\nside\n").unwrap();

    fs::write(quiet.path.join("notes.txt"), "alpha\nstill quiet\n").unwrap();
    let git_file = quiet.path.join(".git");
    if git_file.is_dir() {
        fs::remove_dir_all(&git_file).unwrap();
    } else {
        fs::remove_file(&git_file).unwrap();
    }

    let later = trees
        .create(&project, "bbbbbbb2-0000-0000-0000-000000000000", None)
        .await
        .unwrap();
    let earlier = trees
        .create(&project, "aaaaaaa0-0000-0000-0000-000000000000", None)
        .await
        .unwrap();
    fs::write(later.path.join("notes.txt"), "alpha\nours\n").unwrap();
    fs::write(later.path.join("extra.txt"), "one\n").unwrap();
    fs::write(later.path.join("blob.bin"), [0, 1, 2, 0]).unwrap();
    fs::write(earlier.path.join("notes.txt"), "alpha\nfirst\n").unwrap();

    let listed = trees.list(&project).await;
    let ids: Vec<_> = listed.iter().map(|change| change.id.as_str()).collect();
    assert_eq!(ids, ["aaaaaaa0", "bbbbbbb2"]);
    let detailed = &listed[1];
    assert_eq!(detailed.files.len(), 3);
    assert!(detailed.files.contains(&ChangedFile {
        path: "notes.txt".to_string(),
        insertions: Some(1),
        deletions: Some(0),
    }));
    assert!(detailed.files.contains(&ChangedFile {
        path: "extra.txt".to_string(),
        insertions: Some(1),
        deletions: Some(0),
    }));
    assert!(detailed.files.contains(&ChangedFile {
        path: "blob.bin".to_string(),
        insertions: None,
        deletions: None,
    }));
    assert_eq!(detailed.insertions, 2);
    assert_eq!(detailed.deletions, 0);
    assert_eq!(detailed.branch, "orch/bbbbbbb2");

    let plain = root.join("plain");
    fs::create_dir(&plain).unwrap();
    assert!(trees.list(&plain).await.is_empty());
    assert!(trees.list(&root.join("missing")).await.is_empty());
}

#[tokio::test]
async fn diff_contains_the_added_line_and_is_cut_at_the_cap() {
    let (temp, project, wt) = crate::test_support::temp_project();
    let _keep = temp;
    let trees = Worktrees::new(&wt);
    let small = trees
        .create(&project, "abc12345-0000-0000-0000-000000000000", None)
        .await
        .unwrap();
    fs::write(small.path.join("notes.txt"), "alpha\nbeta\n").unwrap();
    let diff = trees.diff(&project, &small.id).await.unwrap();
    assert!(diff.contains("+beta"));
    assert!(utf16_len(&diff) < 400_000);
    assert!(!diff.contains("diff cut at 400000 characters"));

    let huge = trees
        .create(&project, "def67890-0000-0000-0000-000000000000", None)
        .await
        .unwrap();
    fs::write(
        huge.path.join("big.txt"),
        format!("{}\n", "x".repeat(500_000)),
    )
    .unwrap();
    let cut = trees.diff(&project, &huge.id).await.unwrap();
    assert_eq!(utf16_len(&cut), 400_000);
    assert!(cut.ends_with("\u{2026} diff cut at 400000 characters"));
    assert!(cut.starts_with("diff --git"));
}

#[tokio::test]
async fn merge_stages_the_change_without_a_new_commit() {
    let (temp, project, wt) = crate::test_support::temp_project();
    let _keep = temp;
    let trees = Worktrees::new(&wt);
    let created = trees
        .create(&project, "1234abcd-0000-0000-0000-000000000000", None)
        .await
        .unwrap();
    fs::write(created.path.join("notes.txt"), "alpha\nbeta\n").unwrap();
    fs::write(created.path.join("added.txt"), "fresh\n").unwrap();
    let merged = trees.merge(&project, &created.id).await.unwrap();
    assert!(merged.ok);
    assert_eq!(merged.message, "applied to main as staged changes");
    assert_eq!(read(&project.join("notes.txt")), "alpha\nbeta\n");
    assert_eq!(read(&project.join("added.txt")), "fresh\n");
    assert_eq!(short_status(&project), ["A  added.txt", "M  notes.txt"]);
    assert_eq!(git_text(&project, &["rev-list", "--count", "HEAD"]), "1");
    assert!(!created.path.exists());
    assert!(
        !git(
            &project,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                "refs/heads/orch/1234abcd"
            ]
        )
        .status
        .success()
    );
    assert!(trees.list(&project).await.is_empty());
}

#[tokio::test]
async fn merge_refuses_a_conflict_and_an_empty_worktree() {
    let (temp, project, wt) = crate::test_support::temp_project();
    let _keep = temp;
    let trees = Worktrees::new(&wt);
    let created = trees
        .create(&project, "feedface-0000-0000-0000-000000000000", None)
        .await
        .unwrap();
    fs::write(created.path.join("notes.txt"), "alpha\nfrom-worktree\n").unwrap();
    fs::write(project.join("notes.txt"), "other\n").unwrap();
    git_ok(&project, &["add", "notes.txt"]);
    git_ok(&project, &["commit", "-m", "diverge"]);
    let before = fs::read(project.join("notes.txt")).unwrap();
    let count = git_text(&project, &["rev-list", "--count", "HEAD"]);
    let status = git_text(&project, &["status", "--short"]);
    let conflict = trees.merge(&project, &created.id).await.unwrap();
    assert!(!conflict.ok);
    assert!(conflict.message.starts_with("does not apply cleanly: "));
    assert_eq!(fs::read(project.join("notes.txt")).unwrap(), before);
    assert_eq!(git_text(&project, &["rev-list", "--count", "HEAD"]), count);
    assert_eq!(git_text(&project, &["status", "--short"]), status);
    assert!(created.path.exists());
    assert!(
        git(
            &project,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                "refs/heads/orch/feedface"
            ]
        )
        .status
        .success()
    );

    let empty = trees
        .create(&project, "0ddba11a-0000-0000-0000-000000000000", None)
        .await
        .unwrap();
    let refused = trees.merge(&project, &empty.id).await.unwrap();
    assert!(!refused.ok);
    assert_eq!(refused.message, "nothing to merge");
    assert!(empty.path.exists());
}

#[tokio::test]
async fn discard_removes_the_worktree_and_can_be_repeated() {
    let (temp, project, wt) = crate::test_support::temp_project();
    let _keep = temp;
    let trees = Worktrees::new(&wt);
    let created = trees
        .create(&project, "0a1b2c3d-0000-0000-0000-000000000000", None)
        .await
        .unwrap();
    fs::write(created.path.join("notes.txt"), "alpha\nfrom-worktree\n").unwrap();
    let notes = fs::read(project.join("notes.txt")).unwrap();
    trees.discard(&project, &created.id).await.unwrap();
    assert!(!created.path.exists());
    assert!(
        !git(
            &project,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                "refs/heads/orch/0a1b2c3d"
            ]
        )
        .status
        .success()
    );
    assert_eq!(fs::read(project.join("notes.txt")).unwrap(), notes);
    assert_eq!(git_text(&project, &["status", "--short"]), "");
    assert_eq!(git_text(&project, &["rev-list", "--count", "HEAD"]), "1");
    trees.discard(&project, &created.id).await.unwrap();
}

#[tokio::test]
async fn ids_list_every_worktree_including_ones_with_no_file_changes() {
    let (temp, project, wt) = crate::test_support::temp_project();
    let root = temp.path().to_path_buf();
    let _keep = temp;
    let outside = root.join("outside");
    fs::create_dir(&outside).unwrap();
    let trees = Worktrees::new(&wt);
    assert!(trees.ids(&project).await.is_empty());
    let quiet = trees
        .create(&project, "ccccccc1-0000-0000-0000-000000000000", None)
        .await
        .unwrap();
    assert!(trees.list(&project).await.is_empty());
    assert_eq!(trees.ids(&project).await, ["ccccccc1"]);
    assert_eq!(trees.folder("ccccccc1").unwrap(), quiet.path);

    let outside_s = outside.join("abcdef12").to_string_lossy().into_owned();
    git_ok(
        &project,
        &["worktree", "add", "-b", "orch/abcdef12", &outside_s, "HEAD"],
    );
    let side = wt.join("side").to_string_lossy().into_owned();
    git_ok(&project, &["worktree", "add", "-b", "side", &side, "HEAD"]);
    let later = trees
        .create(&project, "bbbbbbb2-0000-0000-0000-000000000000", None)
        .await
        .unwrap();
    fs::write(later.path.join("notes.txt"), "alpha\nours\n").unwrap();
    assert_eq!(trees.ids(&project).await, ["bbbbbbb2", "ccccccc1"]);
    let listed: Vec<_> = trees
        .list(&project)
        .await
        .into_iter()
        .map(|change| change.id)
        .collect();
    assert_eq!(listed, ["bbbbbbb2"]);

    let plain = root.join("plain");
    fs::create_dir(&plain).unwrap();
    assert!(trees.ids(&plain).await.is_empty());
    assert!(trees.ids(&root.join("missing")).await.is_empty());
}

#[tokio::test]
async fn a_bad_id_is_rejected_before_git_runs() {
    let temp = Temp::new();
    let plain = temp.path().join("plain");
    fs::create_dir(&plain).unwrap();
    let trees = Worktrees::new(temp.path().join("wt"));
    for id in ["../../x", "zzzzzzzz"] {
        assert_eq!(
            trees
                .create(&plain, id, None)
                .await
                .unwrap_err()
                .to_string(),
            "invalid change id"
        );
        assert_eq!(
            trees.diff(&plain, id).await.unwrap_err().to_string(),
            "invalid change id"
        );
        assert_eq!(
            trees.merge(&plain, id).await.unwrap_err().to_string(),
            "invalid change id"
        );
        assert_eq!(
            trees.discard(&plain, id).await.unwrap_err().to_string(),
            "invalid change id"
        );
        assert_eq!(
            trees.folder(id).unwrap_err().to_string(),
            "invalid change id"
        );
    }
}

#[tokio::test]
async fn a_plain_folder_with_a_valid_id_is_not_a_change() {
    let (temp, project, _wt) = crate::test_support::temp_project();
    let _keep = temp;
    let wt = project.join("wt");
    let stray = wt.join("deadbeef");
    fs::create_dir_all(&stray).unwrap();
    fs::write(stray.join("stray.txt"), "not a change\n").unwrap();
    let trees = Worktrees::new(&wt);
    assert_eq!(
        trees
            .diff(&project, "deadbeef")
            .await
            .unwrap_err()
            .to_string(),
        "not a change worktree"
    );
    assert_eq!(
        trees
            .merge(&project, "deadbeef")
            .await
            .unwrap_err()
            .to_string(),
        "not a change worktree"
    );
    assert_eq!(git_text(&project, &["diff", "--cached", "--name-only"]), "");
}

#[tokio::test]
async fn merge_from_a_subfolder_still_applies_files_outside_it() {
    let (temp, project, wt) = crate::test_support::temp_project();
    let _keep = temp;
    let sub = project.join("app");
    fs::create_dir(&sub).unwrap();
    fs::write(sub.join("inner.txt"), "inner\n").unwrap();
    git_ok(&project, &["add", "app/inner.txt"]);
    git_ok(&project, &["commit", "-m", "add app"]);
    let trees = Worktrees::new(&wt);
    let made = trees.create(&sub, "abcdef12-job", None).await.unwrap();
    fs::write(
        made.path.join("notes.txt"),
        "alpha\noutside the subfolder\n",
    )
    .unwrap();
    fs::write(
        made.path.join("app").join("inner.txt"),
        "inner\ninside the subfolder\n",
    )
    .unwrap();
    let result = trees.merge(&sub, &made.id).await.unwrap();
    assert!(result.ok);
    assert_eq!(
        read(&project.join("notes.txt")),
        "alpha\noutside the subfolder\n"
    );
    assert_eq!(
        read(&sub.join("inner.txt")),
        "inner\ninside the subfolder\n"
    );
    assert!(!made.path.exists());
}

#[tokio::test]
async fn remove_retries_three_times_then_names_the_windows_lock() {
    let attempts = AtomicUsize::new(0);
    let delays = Mutex::new(Vec::new());
    let err = remove_worktree_with_retries(
        || {
            attempts.fetch_add(1, Ordering::SeqCst);
            async { Err(GitError::new("busy")) }
        },
        || false,
        |ms| {
            delays
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .push(ms);
            async {}
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.to_string(), WINDOWS_LOCK_MESSAGE);
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    assert_eq!(
        *delays.lock().unwrap_or_else(|err| err.into_inner()),
        [300, 300]
    );
}

#[tokio::test]
async fn remove_stops_when_the_folder_is_gone_and_does_not_delay_then() {
    let attempts = AtomicUsize::new(0);
    let delays = Mutex::new(Vec::new());
    remove_worktree_with_retries(
        || {
            let n = attempts.fetch_add(1, Ordering::SeqCst) + 1;
            async move {
                if n < 2 {
                    Err(GitError::new("busy"))
                } else {
                    Ok(())
                }
            }
        },
        || attempts.load(Ordering::SeqCst) >= 2,
        |ms| {
            delays
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .push(ms);
            async {}
        },
    )
    .await
    .unwrap();
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(*delays.lock().unwrap_or_else(|err| err.into_inner()), [300]);
}

#[test]
fn numstat_z_keeps_binary_and_renames() {
    let bytes = b"1\t0\tfresh.txt\0-\t-\tblob.bin\x000\t0\t\0moved.txt\0renamed.txt\0";
    assert_eq!(
        parse_numstat_z(bytes),
        vec![
            ChangedFile {
                path: "fresh.txt".to_string(),
                insertions: Some(1),
                deletions: Some(0),
            },
            ChangedFile {
                path: "blob.bin".to_string(),
                insertions: None,
                deletions: None,
            },
            ChangedFile {
                path: "moved.txt => renamed.txt".to_string(),
                insertions: Some(0),
                deletions: Some(0),
            },
        ]
    );
}

#[test]
fn status_counts_one_record_per_change_including_a_rename() {
    let mut status = RepoStatus {
        dirty: 0,
        ahead: 0,
        behind: 0,
    };
    fill_status(b"# branch.ab +2 -3\0? notes.txt\0", &mut status);
    assert_eq!(
        status,
        RepoStatus {
            dirty: 1,
            ahead: 2,
            behind: 3
        }
    );

    let mut renamed = RepoStatus {
        dirty: 0,
        ahead: 0,
        behind: 0,
    };
    fill_status(
        b"2 R. N... 100644 100644 100644 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb R100 renamed.txt\0moved.txt\0",
        &mut renamed,
    );
    assert_eq!(renamed.dirty, 1);
}
