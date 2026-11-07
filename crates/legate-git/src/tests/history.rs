use std::fs;
use std::path::Path;

use crate::history::{History, PAGE, parse_log, parse_numstat, require_hash};
use crate::test_support::{Temp, git_ok};

#[test]
fn parse_log_reads_fields_parents_and_refs_and_skips_junk() {
    let field = '\u{1f}';
    let record = '\u{1e}';
    let hash = "a".repeat(40);
    let parent = "b".repeat(40);
    let out = format!(
        "{hash}{field}aaaaaaa{field}{parent}{field}Ada{field}1700000000{field}HEAD -> main, tag: v1{field}Fix it{record}\nnot a commit{record}"
    );
    let commits = parse_log(&out);
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].hash, hash);
    assert_eq!(commits[0].short, "aaaaaaa");
    assert_eq!(commits[0].parents, vec![parent]);
    assert_eq!(commits[0].author, "Ada");
    assert_eq!(commits[0].time, 1_700_000_000_000);
    assert_eq!(commits[0].subject, "Fix it");
    assert_eq!(
        commits[0].refs,
        vec!["HEAD -> main".to_string(), "tag: v1".to_string()]
    );
}

#[test]
fn parse_numstat_keeps_counts_and_marks_binary_files() {
    let files = parse_numstat("3\t1\tsrc/a.ts\n-\t-\timg.png\n\n");
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].path, "src/a.ts");
    assert_eq!(files[0].added, Some(3));
    assert_eq!(files[0].removed, Some(1));
    assert_eq!(files[1].path, "img.png");
    assert_eq!(files[1].added, None);
    assert_eq!(files[1].removed, None);
}

#[test]
fn only_commit_ids_pass() {
    assert_eq!(require_hash("abc1234").unwrap(), "abc1234");
    assert_eq!(
        require_hash("--all").unwrap_err().to_string(),
        "Not a commit id."
    );
    assert_eq!(
        require_hash("HEAD").unwrap_err().to_string(),
        "Not a commit id."
    );
}

fn history_repo() -> (Temp, std::path::PathBuf) {
    let temp = Temp::new();
    let dir = temp.path().to_path_buf();
    git_ok(&dir, &["init", "-b", "main"]);
    git_ok(&dir, &["config", "user.name", "Ada"]);
    git_ok(&dir, &["config", "user.email", "ada@example.invalid"]);
    git_ok(&dir, &["config", "commit.gpgsign", "false"]);
    git_ok(&dir, &["config", "core.autocrlf", "false"]);
    git_ok(&dir, &["config", "color.ui", "false"]);
    (temp, dir)
}

fn commit(dir: &Path, file: &str, text: &str, message: &str) {
    fs::write(dir.join(file), text).unwrap();
    git_ok(dir, &["add", "-A"]);
    git_ok(dir, &["commit", "-m", message]);
}

#[tokio::test]
async fn empty_repository_has_no_commits_and_a_plain_folder_is_refused() {
    let (temp, dir) = history_repo();
    let _keep = temp;
    let history = History::new();
    let page = history.log(&dir, 0, PAGE).await.unwrap();
    assert!(page.commits.is_empty());
    assert!(!page.more);

    let plain_temp = Temp::new();
    let err = history
        .log(plain_temp.path(), 0, PAGE)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("not a git repository"));
}

#[tokio::test]
async fn log_pages_newest_first_detail_lists_files_and_file_diff_shows_one_file() {
    let (temp, dir) = history_repo();
    let _keep = temp;
    commit(&dir, "a.txt", "one\n", "first");
    commit(&dir, "a.txt", "one\ntwo\n", "second");
    commit(&dir, "b.txt", "bee\n", "third\n\nwith a body");
    let history = History::new();

    let all = history.log(&dir, 0, PAGE).await.unwrap();
    let subjects: Vec<_> = all
        .commits
        .iter()
        .map(|commit| commit.subject.as_str())
        .collect();
    assert_eq!(subjects, ["third", "second", "first"]);
    assert!(!all.more);
    assert!(
        all.commits[0]
            .refs
            .iter()
            .any(|item| item == "HEAD -> main")
    );
    assert!(all.commits[2].parents.is_empty());

    let page = history.log(&dir, 1, 1).await.unwrap();
    assert_eq!(
        page.commits
            .iter()
            .map(|commit| commit.subject.as_str())
            .collect::<Vec<_>>(),
        ["second"]
    );
    assert!(page.more);

    let third = &all.commits[0].hash;
    let detail = history.detail(&dir, third).await.unwrap();
    assert_eq!(detail.message, "third\n\nwith a body");
    assert_eq!(detail.author, "Ada");
    assert_eq!(detail.files.len(), 1);
    assert_eq!(detail.files[0].path, "b.txt");
    assert_eq!(detail.files[0].added, Some(1));
    assert_eq!(detail.files[0].removed, Some(0));

    let second = &all.commits[1].hash;
    let diff = history.file_diff(&dir, second, "a.txt").await.unwrap();
    assert!(diff.contains("+two"));
}
