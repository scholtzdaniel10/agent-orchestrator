use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use legate_core::{FileContent, FileStatus};

use crate::cmd::{CmdOut, CmdRun, ProcessRun};
use crate::files::{FileStat, FileSys, Files, OUTSIDE_PROJECT, OsFs};
use crate::test_support::{Spy, Temp, git_ok};

struct Flat {
    root: PathBuf,
    names: Vec<String>,
    size: u64,
    bytes: Vec<u8>,
}

impl FileSys for Flat {
    fn realpath(&self, target: &Path) -> Option<PathBuf> {
        Some(target.to_path_buf())
    }

    fn read_file(&self, _target: &Path) -> io::Result<Vec<u8>> {
        Ok(self.bytes.clone())
    }

    fn stat(&self, _target: &Path) -> io::Result<FileStat> {
        Ok(FileStat {
            is_file: true,
            is_dir: false,
            size: self.size,
        })
    }

    fn readdir(&self, dir: &Path) -> io::Result<Vec<String>> {
        if dir == self.root {
            Ok(self.names.clone())
        } else {
            Ok(Vec::new())
        }
    }
}

fn files_repo() -> (Temp, PathBuf) {
    let temp = Temp::new();
    let project = temp.path().join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    let project_arg = project.to_string_lossy().into_owned();
    git_ok(temp.path(), &["init", "-q", "-b", "main", &project_arg]);
    git_ok(&project, &["config", "user.email", "dev@example.com"]);
    git_ok(&project, &["config", "user.name", "Dev"]);
    git_ok(&project, &["config", "core.autocrlf", "false"]);
    git_ok(&project, &["config", "commit.gpgsign", "false"]);
    git_ok(&project, &["config", "color.ui", "false"]);
    fs::write(project.join("kept.txt"), "one\n").unwrap();
    fs::write(project.join("app.txt"), "old\n").unwrap();
    fs::write(project.join("src").join("lib.ts"), "export {}\n").unwrap();
    fs::write(project.join("gone.txt"), "bye\n").unwrap();
    fs::write(project.join("moved.txt"), "move me\n").unwrap();
    git_ok(&project, &["add", "-A"]);
    git_ok(&project, &["commit", "-q", "-m", "first"]);
    (temp, project)
}

#[allow(clippy::type_complexity)]
fn recording() -> Spy<
    impl Fn(
        String,
        Vec<String>,
        Option<PathBuf>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = CmdOut> + Send>>
    + Send
    + Sync,
> {
    Spy::new(
        |command: String,
         args: Vec<String>,
         cwd: Option<PathBuf>|
         -> std::pin::Pin<Box<dyn std::future::Future<Output = CmdOut> + Send>> {
            Box::pin(async move { ProcessRun::command().run(command, args, cwd).await })
        },
    )
}

fn status_of(entries: &[legate_core::FileEntry], path: &str) -> Option<FileStatus> {
    entries
        .iter()
        .find(|row| row.path == path)
        .map(|row| row.status)
}

#[tokio::test]
async fn list_returns_every_git_file_with_status_sorted_by_path() {
    let (_temp, project) = files_repo();
    fs::write(project.join("app.txt"), "new\n").unwrap();
    fs::write(project.join("fresh.txt"), "added\n").unwrap();
    git_ok(&project, &["add", "fresh.txt"]);
    fs::write(project.join("notes.txt"), "untracked\n").unwrap();
    fs::remove_file(project.join("gone.txt")).unwrap();
    git_ok(&project, &["mv", "moved.txt", "renamed.txt"]);

    let listed = Files::new().list(&project).await;
    assert!(!listed.truncated);
    let mut paths: Vec<_> = listed.entries.iter().map(|row| row.path.clone()).collect();
    let sorted = {
        let mut copy = paths.clone();
        copy.sort();
        copy
    };
    paths.sort();
    assert_eq!(paths, sorted);
    assert_eq!(
        status_of(&listed.entries, "kept.txt"),
        Some(FileStatus::Clean)
    );
    assert_eq!(
        status_of(&listed.entries, "app.txt"),
        Some(FileStatus::Modified)
    );
    assert_eq!(
        status_of(&listed.entries, "fresh.txt"),
        Some(FileStatus::Added)
    );
    assert_eq!(
        status_of(&listed.entries, "notes.txt"),
        Some(FileStatus::Untracked)
    );
    assert_eq!(
        status_of(&listed.entries, "gone.txt"),
        Some(FileStatus::Deleted)
    );
    assert_eq!(
        status_of(&listed.entries, "renamed.txt"),
        Some(FileStatus::Renamed)
    );
    assert_eq!(status_of(&listed.entries, "moved.txt"), None);
    assert_eq!(
        status_of(&listed.entries, "src/lib.ts"),
        Some(FileStatus::Clean)
    );
}

#[tokio::test]
async fn list_marks_a_merge_conflict_as_conflicted() {
    let (_temp, project) = files_repo();
    git_ok(&project, &["checkout", "-q", "-b", "other"]);
    fs::write(project.join("app.txt"), "theirs\n").unwrap();
    git_ok(&project, &["add", "app.txt"]);
    git_ok(&project, &["commit", "-q", "-m", "theirs"]);
    git_ok(&project, &["checkout", "-q", "main"]);
    fs::write(project.join("app.txt"), "ours\n").unwrap();
    git_ok(&project, &["add", "app.txt"]);
    git_ok(&project, &["commit", "-q", "-m", "ours"]);
    let merge = crate::test_support::git(&project, &["merge", "other"]);
    assert!(!merge.status.success());

    let listed = Files::new().list(&project).await;
    assert_eq!(
        status_of(&listed.entries, "app.txt"),
        Some(FileStatus::Conflicted)
    );
}

#[tokio::test]
async fn list_walks_a_folder_that_is_not_a_git_repository_and_skips_junk() {
    let (temp, _project) = files_repo();
    let folder = temp.path().join("plain");
    fs::create_dir_all(folder.join("src").join("node_modules")).unwrap();
    fs::create_dir_all(folder.join("node_modules")).unwrap();
    fs::create_dir_all(folder.join(".hidden")).unwrap();
    fs::write(folder.join("readme.md"), "hi\n").unwrap();
    fs::write(folder.join("src").join("app.ts"), "export {}\n").unwrap();
    fs::write(folder.join(".env"), "SECRET=1\n").unwrap();
    fs::write(folder.join("node_modules").join("pkg.js"), "nope\n").unwrap();
    fs::write(
        folder.join("src").join("node_modules").join("pkg.js"),
        "nope\n",
    )
    .unwrap();
    fs::write(folder.join(".hidden").join("x.txt"), "nope\n").unwrap();
    fs::write(folder.join("src").join(".keep"), "ok\n").unwrap();

    let listed = Files::new().list(&folder).await;
    assert!(!listed.truncated);
    let paths: Vec<_> = listed.entries.iter().map(|row| row.path.as_str()).collect();
    assert_eq!(paths, ["readme.md", "src/.keep", "src/app.ts"]);
    assert!(
        listed
            .entries
            .iter()
            .all(|row| row.status == FileStatus::Clean)
    );
}

#[tokio::test]
async fn list_sets_truncated_when_git_reports_more_than_20000_files() {
    let names: Vec<String> = (0..20_001)
        .map(|index| format!("f{index:05}.txt"))
        .collect();
    let stdout = format!("{}\0", names.join("\0"));
    let spy = Spy::new(
        move |_command: String, args: Vec<String>, _cwd: Option<PathBuf>| {
            let stdout = stdout.clone();
            std::future::ready(match args.first().map(String::as_str) {
                Some("rev-parse") => CmdOut::new(0, "true\n", ""),
                Some("ls-files") => CmdOut::new(0, stdout, ""),
                Some("status") => CmdOut::ok(""),
                _ => CmdOut::fail(1, "unexpected"),
            })
        },
    );
    let (_temp, project) = files_repo();
    let listed = Files::with(spy, OsFs).list(&project).await;
    assert!(listed.truncated);
    assert_eq!(listed.entries.len(), 20_000);
}

#[tokio::test]
async fn list_sets_truncated_when_a_non_git_walk_hits_20000_files() {
    let (_temp, project) = files_repo();
    let names: Vec<String> = (0..20_001)
        .map(|index| format!("f{index:05}.txt"))
        .collect();
    let fake = Flat {
        root: project.clone(),
        names,
        size: 0,
        bytes: Vec::new(),
    };
    let spy =
        Spy::new(|_command, _args, _cwd| std::future::ready(CmdOut::fail(1, "not a git repo")));
    let listed = Files::with(spy, fake).list(&project).await;
    assert!(listed.truncated);
    assert_eq!(listed.entries.len(), 20_000);
}

#[tokio::test]
async fn read_returns_text_binary_too_large_and_missing() {
    let (_temp, project) = files_repo();
    fs::write(project.join("notes.txt"), "hello\n").unwrap();
    fs::write(project.join("pic.bin"), [0x00, 0x01, 0x02, 0xff]).unwrap();
    let files = Files::new();
    assert_eq!(
        files.read(&project, "notes.txt").await.unwrap(),
        FileContent::Text {
            text: "hello\n".to_string(),
            bytes: 6
        }
    );
    assert_eq!(
        files.read(&project, "pic.bin").await.unwrap(),
        FileContent::Binary { bytes: 4 }
    );
    assert_eq!(
        files.read(&project, "nope.txt").await.unwrap(),
        FileContent::Missing
    );

    let fake = Flat {
        root: project.clone(),
        names: Vec::new(),
        size: 1_000_001,
        bytes: vec![b'a'; 1_000_001],
    };
    let oversized = Files::with(ProcessRun::command(), fake);
    assert_eq!(
        oversized.read(&project, "kept.txt").await.unwrap(),
        FileContent::TooLarge { bytes: 1_000_001 }
    );
}

#[tokio::test]
async fn diff_against_head_is_empty_when_unchanged_and_shows_the_edit() {
    let (_temp, project) = files_repo();
    let files = Files::new();
    assert_eq!(files.diff(&project, "kept.txt").await.unwrap(), "");
    fs::write(project.join("app.txt"), "new\n").unwrap();
    let text = files.diff(&project, "app.txt").await.unwrap();
    assert!(text.contains("-old"));
    assert!(text.contains("+new"));
}

#[tokio::test]
async fn diff_of_an_untracked_file_adds_every_line_via_no_index() {
    let (_temp, project) = files_repo();
    fs::write(project.join("notes.txt"), "alpha\nbeta\n").unwrap();
    let spy = recording();
    let text = Files::with(spy.clone(), OsFs)
        .diff(&project, "notes.txt")
        .await
        .unwrap();
    assert!(text.contains("+alpha"));
    assert!(text.contains("+beta"));
    let diff_call = spy
        .calls()
        .into_iter()
        .find(|call| {
            call.args.first().map(String::as_str) == Some("diff")
                && call.args.iter().any(|arg| arg == "--no-index")
        })
        .unwrap();
    assert_eq!(
        diff_call.args,
        ["diff", "--no-index", "--", "/dev/null", "notes.txt"]
    );
}

#[tokio::test]
async fn paths_are_passed_to_git_after_double_dash() {
    let (_temp, project) = files_repo();
    let spy = recording();
    Files::with(spy.clone(), OsFs)
        .diff(&project, "kept.txt")
        .await
        .unwrap();
    let calls = spy.calls();
    let diff_call = calls
        .iter()
        .find(|call| {
            call.args.first().map(String::as_str) == Some("diff")
                && call.args.iter().any(|arg| arg == "HEAD")
        })
        .unwrap();
    assert_eq!(diff_call.args, ["diff", "HEAD", "--", "kept.txt"]);
    let ls = calls
        .iter()
        .find(|call| call.args.first().map(String::as_str) == Some("ls-files"))
        .unwrap();
    let dash = ls.args.iter().position(|arg| arg == "--").unwrap();
    assert_eq!(ls.args[dash + 1], "kept.txt");
}

#[tokio::test]
async fn refuses_an_absolute_path() {
    let (temp, project) = files_repo();
    let spy = recording();
    let files = Files::with(spy.clone(), OsFs);
    let outside = temp
        .path()
        .join("outside.txt")
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        files
            .read(&project, &outside)
            .await
            .unwrap_err()
            .to_string(),
        OUTSIDE_PROJECT
    );
    assert_eq!(
        files
            .diff(&project, "/etc/passwd")
            .await
            .unwrap_err()
            .to_string(),
        OUTSIDE_PROJECT
    );
    assert!(spy.calls().is_empty());
}

#[tokio::test]
async fn refuses_dotdot_segments_even_when_they_stay_inside() {
    let (_temp, project) = files_repo();
    let files = Files::new();
    assert_eq!(
        files
            .read(&project, "src/../kept.txt")
            .await
            .unwrap_err()
            .to_string(),
        OUTSIDE_PROJECT
    );
    assert_eq!(
        files
            .diff(&project, "../project/kept.txt")
            .await
            .unwrap_err()
            .to_string(),
        OUTSIDE_PROJECT
    );
}

#[tokio::test]
async fn refuses_a_windows_drive_prefix() {
    let (_temp, project) = files_repo();
    let files = Files::new();
    assert_eq!(
        files
            .read(&project, "C:kept.txt")
            .await
            .unwrap_err()
            .to_string(),
        OUTSIDE_PROJECT
    );
    assert_eq!(
        files
            .read(&project, r"C:\Windows\notepad.exe")
            .await
            .unwrap_err()
            .to_string(),
        OUTSIDE_PROJECT
    );
    assert_eq!(
        files
            .diff(&project, "D:/secret.txt")
            .await
            .unwrap_err()
            .to_string(),
        OUTSIDE_PROJECT
    );
}

#[tokio::test]
async fn refuses_a_unc_prefix() {
    let (_temp, project) = files_repo();
    let files = Files::new();
    assert_eq!(
        files
            .read(&project, r"\\server\share\file.txt")
            .await
            .unwrap_err()
            .to_string(),
        OUTSIDE_PROJECT
    );
    assert_eq!(
        files
            .diff(&project, "//server/share/file.txt")
            .await
            .unwrap_err()
            .to_string(),
        OUTSIDE_PROJECT
    );
}

#[tokio::test]
async fn refuses_a_file_inside_a_linked_folder_outside_the_root() {
    let (temp, project) = files_repo();
    let outside = temp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret.txt"), "nope\n").unwrap();
    let link = project.join("link");
    link_dir(&outside, &link);
    let files = Files::new();
    assert_eq!(
        files
            .read(&project, "link/secret.txt")
            .await
            .unwrap_err()
            .to_string(),
        OUTSIDE_PROJECT
    );
    assert_eq!(
        files
            .diff(&project, "link/secret.txt")
            .await
            .unwrap_err()
            .to_string(),
        OUTSIDE_PROJECT
    );
    assert!(files.resolve(&project, "kept.txt").is_ok());
}

fn link_dir(target: &Path, link: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::fs::symlink_dir;
        use std::os::windows::process::CommandExt;
        if symlink_dir(target, link).is_ok() {
            return;
        }
        let status = Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .creation_flags(0x0800_0000)
            .status()
            .expect("mklink");
        assert!(status.success(), "junction");
    }
    #[cfg(not(windows))]
    {
        std::os::unix::fs::symlink(target, link).expect("symlink");
    }
}
