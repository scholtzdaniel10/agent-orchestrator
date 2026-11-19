use std::fs;
use std::path::Path;
use std::time::Duration;

use legate_core::{FileEntry, FileStatus};

use super::files::files_repo;
use crate::test_support::{git_ok, git_text};
use crate::{ProcessRun, SourceControl};

fn add_origin(temp: &Path, project: &Path) {
    let origin = temp.join("origin.git");
    let origin = origin.to_str().unwrap();
    git_ok(temp, &["init", "-q", "--bare", "-b", "main", origin]);
    git_ok(project, &["remote", "add", "origin", origin]);
    git_ok(project, &["config", "push.default", "simple"]);
}

#[tokio::test]
async fn status_lists_uncommitted_and_branch() {
    let (temp, project) = files_repo();
    add_origin(temp.path(), &project);
    git_ok(&project, &["push", "origin", "main"]);
    git_ok(&project, &["switch", "-c", "topic"]);
    git_ok(&project, &["push", "-u", "origin", "topic"]);
    fs::write(project.join("app.txt"), "new\nsecond line\n").unwrap();
    git_ok(&project, &["commit", "-am", "Update app"]);
    fs::write(project.join("app.txt"), "working copy\n").unwrap();
    fs::write(project.join("notes.txt"), "notes\n").unwrap();

    let status = SourceControl::new().status(&project).await.unwrap();
    assert_eq!(status.branch, "topic");
    assert_eq!(status.upstream.as_deref(), Some("origin/topic"));
    assert_eq!(status.base.as_deref(), Some("origin/main"));
    assert_eq!((status.ahead, status.behind), (1, 0));
    assert_eq!(
        (status.commits, status.insertions, status.deletions),
        (1, 2, 1)
    );
    assert_eq!(
        status.uncommitted,
        [
            FileEntry {
                path: "app.txt".into(),
                status: FileStatus::Modified,
            },
            FileEntry {
                path: "notes.txt".into(),
                status: FileStatus::Untracked,
            },
        ]
    );
}

#[tokio::test]
async fn status_without_origin_has_no_base_or_upstream() {
    let (_temp, project) = files_repo();
    let status = SourceControl::new().status(&project).await.unwrap();
    assert_eq!(status.branch, "main");
    assert_eq!(status.base, None);
    assert_eq!(status.upstream, None);
    assert_eq!((status.ahead, status.behind, status.commits), (0, 0, 0));
    assert_eq!((status.insertions, status.deletions), (0, 0));
}

#[tokio::test]
async fn commit_all_commits_everything() {
    let (_temp, project) = files_repo();
    fs::write(project.join("app.txt"), "changed\n").unwrap();
    fs::write(project.join("notes.txt"), "new\n").unwrap();
    fs::remove_file(project.join("gone.txt")).unwrap();
    let source = SourceControl::new();
    source.commit_all(&project, "Update files").await.unwrap();
    assert!(
        source
            .status(&project)
            .await
            .unwrap()
            .uncommitted
            .is_empty()
    );
    assert_eq!(
        git_text(&project, &["log", "-1", "--format=%s"]),
        "Update files"
    );
}

#[tokio::test]
async fn commit_all_rejects_blank_and_clean() {
    let (_temp, project) = files_repo();
    let source = SourceControl::new();
    for (message, expected) in [
        (" \t\n", "Write a commit message first."),
        ("Update files", "There is nothing to commit."),
    ] {
        let err = source.commit_all(&project, message).await.unwrap_err();
        assert_eq!(err.to_string(), expected);
    }
}

#[tokio::test]
async fn push_publishes_then_tracks() {
    let (temp, project) = files_repo();
    add_origin(temp.path(), &project);
    let source = SourceControl::new();
    source.push(&project).await.unwrap();
    let status = source.status(&project).await.unwrap();
    assert_eq!(status.upstream.as_deref(), Some("origin/main"));
    assert_eq!(status.ahead, 0);

    fs::write(project.join("app.txt"), "new\n").unwrap();
    source.commit_all(&project, "Update app").await.unwrap();
    assert_eq!(source.status(&project).await.unwrap().ahead, 1);

    git_ok(&project, &["remote", "rename", "origin", "published"]);
    git_ok(
        &project,
        &["update-ref", "-d", "refs/remotes/published/main"],
    );
    source.push(&project).await.unwrap();
    let status = source.status(&project).await.unwrap();
    assert_eq!(status.upstream.as_deref(), Some("published/main"));
    assert_eq!(status.ahead, 0);
}

#[tokio::test]
async fn push_detached_head_refuses() {
    let (_temp, project) = files_repo();
    git_ok(&project, &["switch", "--detach"]);
    let err = SourceControl::new().push(&project).await.unwrap_err();
    assert_eq!(err.to_string(), "Check out a branch before pushing.");
}

#[tokio::test]
async fn push_to_missing_remote_fails_fast() {
    let (temp, project) = files_repo();
    let missing = temp.path().join("missing.git");
    git_ok(
        &project,
        &["remote", "add", "origin", missing.to_str().unwrap()],
    );
    let timeout = Duration::from_secs(10);
    let source = SourceControl::with(ProcessRun::new(timeout));
    let err = tokio::time::timeout(timeout, source.push(&project))
        .await
        .expect("push should fail before the timeout")
        .unwrap_err();
    assert!(!err.to_string().trim().is_empty());
    assert_ne!(err.to_string(), "timed out");
}

#[tokio::test]
async fn pr_target_needs_upstream_and_base() {
    let (temp, project) = files_repo();
    add_origin(temp.path(), &project);
    git_ok(&project, &["switch", "-c", "topic"]);
    let source = SourceControl::new();
    let err = source.pr_target(&project).await.unwrap_err();
    assert_eq!(err.to_string(), "Publish the branch first.");

    source.push(&project).await.unwrap();
    let err = source.pr_target(&project).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        "This repository has no origin to open it on."
    );

    git_ok(&project, &["push", "origin", "main:master"]);
    let target = source.pr_target(&project).await.unwrap();
    assert_eq!(target.base, "master");

    git_ok(&project, &["push", "origin", "main", "main:trunk"]);
    git_ok(
        &project,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/trunk",
        ],
    );
    let target = source.pr_target(&project).await.unwrap();
    assert_eq!(target.title, "first");
    assert_eq!(target.base, "trunk");
    assert_eq!(target.head, "topic");
}
