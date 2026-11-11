use std::fs;
use std::future::ready;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use legate_core::GithubRepo;

use crate::cmd::{CmdOut, CmdRun, ProcessRun};
use crate::github::{Github, OpenedPr};
use crate::test_support::{Call, Spy, Temp, git_ok, git_text};

const HEX: &str = "a1b2c3";
const BRANCH: &str = "orch/pr-fix-login-a1b2c3";
const PR_URL: &str = "https://github.com/acme/demo/pull/7";

fn repo_path() -> PathBuf {
    PathBuf::from("/repo")
}

fn at(command: &str, args: &[&str], cwd: Option<&Path>) -> Call {
    Call {
        command: command.to_string(),
        args: args.iter().map(|arg| (*arg).to_string()).collect(),
        cwd: cwd.map(Path::to_path_buf),
    }
}

fn canned(command: &str, args: &[String]) -> CmdOut {
    if command == "gh" && args.first().map(String::as_str) == Some("auth") {
        return CmdOut::ok("");
    }
    if command == "git"
        && args.first().map(String::as_str) == Some("rev-parse")
        && args.get(1).map(String::as_str) == Some("--is-inside-work-tree")
    {
        return CmdOut::new(0, "true\n", "");
    }
    if command == "git" && args.first().map(String::as_str) == Some("remote") {
        return CmdOut::new(0, "git@github.com:acme/r.git\n", "");
    }
    if command == "git"
        && args.first().map(String::as_str) == Some("rev-parse")
        && args.iter().any(|arg| arg == "--abbrev-ref")
    {
        return CmdOut::new(0, "main\n", "");
    }
    if command == "git" && args.first().map(String::as_str) == Some("diff") {
        return CmdOut::fail(1, "");
    }
    if command == "gh" && args.first().map(String::as_str) == Some("pr") {
        return CmdOut::new(0, "https://github.com/acme/r/pull/9\n", "");
    }
    CmdOut::ok("")
}

fn scripted(
    override_fn: impl Fn(&str, &[String]) -> Option<CmdOut> + Send + Sync + 'static,
) -> Spy<impl Fn(String, Vec<String>, Option<PathBuf>) -> std::future::Ready<CmdOut> + Send + Sync>
{
    Spy::new(
        move |command: String, args: Vec<String>, _cwd: Option<PathBuf>| {
            let out = override_fn(&command, &args).unwrap_or_else(|| canned(&command, &args));
            ready(out)
        },
    )
}

#[tokio::test]
async fn available_is_true_when_auth_exits_0_and_caches() {
    let n = std::sync::atomic::AtomicUsize::new(0);
    let spy = Spy::new(move |_command, _args, _cwd| {
        let seen = n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        ready(CmdOut::new(if seen == 0 { 0 } else { 1 }, "", ""))
    });
    let client = Github::with(spy.clone(), || HEX.to_string());
    assert!(client.available().await);
    assert!(client.available().await);
    let calls = spy.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].args, ["auth", "status"]);
}

#[tokio::test]
async fn available_is_false_when_auth_fails() {
    let spy = Spy::new(|_command, _args, _cwd| ready(CmdOut::fail(1, "not logged in")));
    let client = Github::with(spy, || HEX.to_string());
    assert!(!client.available().await);
}

#[tokio::test]
async fn repos_parses_rows_and_skips_junk() {
    let stdout = serde_json::json!([
        {
            "nameWithOwner": "acme/one",
            "description": "first",
            "isPrivate": true,
            "updatedAt": "2024-01-01T00:00:00Z"
        },
        { "nameWithOwner": 12 },
        null,
        "nope",
        {
            "nameWithOwner": "acme/two",
            "description": null,
            "isPrivate": false,
            "updatedAt": "2024-02-01T00:00:00Z"
        },
        { "description": "missing name" }
    ])
    .to_string();
    let spy = Spy::new(move |_command, _args, _cwd| {
        let stdout = stdout.clone();
        ready(CmdOut::new(0, stdout, ""))
    });
    let client = Github::with(spy.clone(), || HEX.to_string());
    assert_eq!(
        client.repos().await,
        vec![
            GithubRepo {
                name_with_owner: "acme/one".to_string(),
                description: "first".to_string(),
                is_private: true,
                updated_at: "2024-01-01T00:00:00Z".to_string(),
            },
            GithubRepo {
                name_with_owner: "acme/two".to_string(),
                description: String::new(),
                is_private: false,
                updated_at: "2024-02-01T00:00:00Z".to_string(),
            },
        ]
    );
    assert_eq!(
        spy.calls()[0].args,
        [
            "repo",
            "list",
            "--limit",
            "100",
            "--json",
            "nameWithOwner,description,isPrivate,updatedAt"
        ]
    );
}

#[tokio::test]
async fn repos_returns_empty_on_bad_json_or_nonzero_exit() {
    let bad = Github::with(
        Spy::new(|_c, _a, _d| ready(CmdOut::new(0, "{", ""))),
        || HEX.to_string(),
    );
    assert!(bad.repos().await.is_empty());
    let failed = Github::with(
        Spy::new(|_c, _a, _d| ready(CmdOut::new(1, "[]", "err"))),
        || HEX.to_string(),
    );
    assert!(failed.repos().await.is_empty());
    let not_array = Github::with(
        Spy::new(|_c, _a, _d| ready(CmdOut::new(0, "{}", ""))),
        || HEX.to_string(),
    );
    assert!(not_array.repos().await.is_empty());
}

#[tokio::test]
async fn clone_rejects_bad_names_and_an_existing_folder() {
    let temp = Temp::new();
    let root = temp.path().to_path_buf();
    let spy = Spy::new(|_c, _a, _d| ready(CmdOut::ok("")));
    let client = Github::with(spy.clone(), || HEX.to_string());
    for name in ["bad name", "-org/repo", "org/-repo", "../x/y"] {
        assert_eq!(
            client.clone(name, &root).await.unwrap_err().to_string(),
            "invalid repository name"
        );
    }
    assert!(spy.calls().is_empty());

    fs::create_dir(root.join("taken")).unwrap();
    assert_eq!(
        client
            .clone("acme/taken", &root)
            .await
            .unwrap_err()
            .to_string(),
        "a folder with that name already exists there"
    );
    assert!(spy.calls().is_empty());

    let target = client.clone("acme/fresh", &root).await.unwrap();
    let fresh = root.join("fresh").to_string_lossy().into_owned();
    assert_eq!(target, root.join("fresh"));
    assert_eq!(
        spy.calls()[0].args,
        ["repo", "clone", "acme/fresh", fresh.as_str()]
    );

    let fail = Github::with(
        Spy::new(|_c, _a, _d| ready(CmdOut::fail(1, format!("x{}", "e".repeat(510))))),
        || HEX.to_string(),
    );
    assert_eq!(
        fail.clone("acme/x", &root).await.unwrap_err().to_string(),
        "e".repeat(500)
    );
}

#[tokio::test]
async fn create_pr_returns_the_url_and_surfaces_failure() {
    let spy = Spy::new(|_c, _a, _d| {
        ready(CmdOut::new(
            0,
            "Creating pull request\nhttps://github.com/acme/r/pull/3\nDone\n",
            "",
        ))
    });
    let client = Github::with(spy.clone(), || HEX.to_string());
    let repo = repo_path();
    assert_eq!(
        client
            .create_pr(&repo, "Title", "Body", None, None)
            .await
            .unwrap(),
        "https://github.com/acme/r/pull/3"
    );
    assert_eq!(
        spy.calls()[0],
        at(
            "gh",
            &["pr", "create", "--title", "Title", "--body", "Body"],
            Some(&repo)
        )
    );

    let fail = Github::with(
        Spy::new(|_c, _a, _d| ready(CmdOut::fail(1, "no commits"))),
        || HEX.to_string(),
    );
    assert_eq!(
        fail.create_pr(&repo, "t", "b", None, None)
            .await
            .unwrap_err()
            .to_string(),
        "no commits"
    );
}

#[tokio::test]
async fn create_pr_passes_optional_base_and_head() {
    let spy =
        Spy::new(|_c, _a, _d| ready(CmdOut::new(0, "https://github.com/acme/r/pull/4\n", "")));
    let client = Github::with(spy.clone(), || HEX.to_string());
    let repo = repo_path();
    assert_eq!(
        client
            .create_pr(&repo, "T", "B", Some("main"), Some("head-b"))
            .await
            .unwrap(),
        "https://github.com/acme/r/pull/4"
    );
    assert_eq!(
        spy.calls()[0].args,
        [
            "pr", "create", "--title", "T", "--body", "B", "--base", "main", "--head", "head-b"
        ]
    );
}

#[tokio::test]
async fn open_pr_happy_path_uses_the_exact_command_order() {
    let spy = scripted(|_, _| None);
    let client = Github::with(spy.clone(), || HEX.to_string());
    let repo = repo_path();
    assert_eq!(
        client
            .open_pr(&repo, "Fix login", "what changed")
            .await
            .unwrap(),
        OpenedPr {
            url: "https://github.com/acme/r/pull/9".to_string(),
            branch: BRANCH.to_string(),
        }
    );
    assert_eq!(
        spy.calls(),
        vec![
            at("gh", &["auth", "status"], None),
            at("git", &["rev-parse", "--is-inside-work-tree"], Some(&repo)),
            at("git", &["remote", "get-url", "origin"], Some(&repo)),
            at("git", &["rev-parse", "--abbrev-ref", "HEAD"], Some(&repo)),
            at("git", &["diff", "--cached", "--quiet"], Some(&repo)),
            at("git", &["switch", "-c", BRANCH], Some(&repo)),
            at(
                "git",
                &["commit", "-m", "Fix login", "-m", "what changed"],
                Some(&repo)
            ),
            at("git", &["push", "-u", "origin", BRANCH], Some(&repo)),
            at(
                "gh",
                &[
                    "pr",
                    "create",
                    "--title",
                    "Fix login",
                    "--body",
                    "what changed",
                    "--base",
                    "main",
                    "--head",
                    BRANCH
                ],
                Some(&repo)
            ),
            at("git", &["switch", "main"], Some(&repo)),
        ]
    );
    assert!(
        spy.calls()
            .iter()
            .all(|call| call.args.first().map(String::as_str) != Some("add"))
    );
}

#[tokio::test]
async fn open_pr_omits_the_second_commit_message_when_the_body_is_empty() {
    let spy = scripted(|_, _| None);
    let client = Github::with(spy.clone(), || HEX.to_string());
    client.open_pr(&repo_path(), "Fix login", "").await.unwrap();
    let commit = spy
        .calls()
        .into_iter()
        .find(|call| call.args.first().map(String::as_str) == Some("commit"))
        .unwrap();
    assert_eq!(commit.args, ["commit", "-m", "Fix login"]);
}

#[tokio::test]
async fn open_pr_refuses_when_gh_is_not_available() {
    let spy = scripted(|command, args| {
        if command == "gh" && args.first().map(String::as_str) == Some("auth") {
            Some(CmdOut::fail(1, "no auth"))
        } else {
            None
        }
    });
    let client = Github::with(spy.clone(), || HEX.to_string());
    assert_eq!(
        client
            .open_pr(&repo_path(), "Fix login", "")
            .await
            .unwrap_err()
            .to_string(),
        "GitHub CLI is not available"
    );
    assert!(
        spy.calls()
            .iter()
            .all(|call| call.command == "gh"
                && call.args.first().map(String::as_str) == Some("auth"))
    );
}

#[tokio::test]
async fn open_pr_refuses_when_the_folder_is_not_a_git_repo() {
    let spy = scripted(|command, args| {
        if command == "git"
            && args.first().map(String::as_str) == Some("rev-parse")
            && args.get(1).map(String::as_str) == Some("--is-inside-work-tree")
        {
            Some(CmdOut::fail(128, "not a git repository"))
        } else {
            None
        }
    });
    let client = Github::with(spy.clone(), || HEX.to_string());
    assert_eq!(
        client
            .open_pr(&repo_path(), "Fix login", "")
            .await
            .unwrap_err()
            .to_string(),
        "not a git repository"
    );
    assert!(
        spy.calls()
            .iter()
            .all(|call| call.args.first().map(String::as_str) != Some("switch"))
    );
}

#[tokio::test]
async fn open_pr_refuses_when_there_is_no_origin() {
    let spy = scripted(|command, args| {
        if command == "git" && args.first().map(String::as_str) == Some("remote") {
            Some(CmdOut::fail(2, "no such remote"))
        } else {
            None
        }
    });
    let client = Github::with(spy.clone(), || HEX.to_string());
    assert_eq!(
        client
            .open_pr(&repo_path(), "Fix login", "")
            .await
            .unwrap_err()
            .to_string(),
        "no origin remote"
    );
    assert!(
        spy.calls()
            .iter()
            .all(|call| call.args.first().map(String::as_str) != Some("switch"))
    );
}

#[tokio::test]
async fn open_pr_refuses_when_head_is_detached() {
    let spy = scripted(|command, args| {
        if command == "git" && args.iter().any(|arg| arg == "--abbrev-ref") {
            Some(CmdOut::new(0, "HEAD\n", ""))
        } else {
            None
        }
    });
    let client = Github::with(spy.clone(), || HEX.to_string());
    assert_eq!(
        client
            .open_pr(&repo_path(), "Fix login", "")
            .await
            .unwrap_err()
            .to_string(),
        "HEAD is detached"
    );
    assert!(
        spy.calls()
            .iter()
            .all(|call| call.args.first().map(String::as_str) != Some("switch"))
    );
}

#[tokio::test]
async fn open_pr_refuses_when_nothing_is_staged() {
    let spy = scripted(|command, args| {
        if command == "git" && args.first().map(String::as_str) == Some("diff") {
            Some(CmdOut::ok(""))
        } else {
            None
        }
    });
    let client = Github::with(spy.clone(), || HEX.to_string());
    assert_eq!(
        client
            .open_pr(&repo_path(), "Fix login", "")
            .await
            .unwrap_err()
            .to_string(),
        "nothing is staged"
    );
    assert!(
        spy.calls()
            .iter()
            .all(|call| call.args.first().map(String::as_str) != Some("switch"))
    );
}

#[tokio::test]
async fn open_pr_commit_failure_rolls_the_branch_back() {
    let spy = scripted(|command, args| {
        if command == "git" && args.first().map(String::as_str) == Some("commit") {
            Some(CmdOut::fail(1, format!("x{}", "e".repeat(510))))
        } else {
            None
        }
    });
    let client = Github::with(spy.clone(), || HEX.to_string());
    assert_eq!(
        client
            .open_pr(&repo_path(), "Fix login", "")
            .await
            .unwrap_err()
            .to_string(),
        "e".repeat(500)
    );
    let git_args: Vec<Vec<String>> = spy
        .calls()
        .into_iter()
        .filter(|call| call.command == "git")
        .map(|call| call.args)
        .collect();
    assert_eq!(
        git_args,
        vec![
            vec!["rev-parse", "--is-inside-work-tree"],
            vec!["remote", "get-url", "origin"],
            vec!["rev-parse", "--abbrev-ref", "HEAD"],
            vec!["diff", "--cached", "--quiet"],
            vec!["switch", "-c", BRANCH],
            vec!["commit", "-m", "Fix login"],
            vec!["switch", "main"],
            vec!["branch", "-D", BRANCH],
        ]
        .into_iter()
        .map(|args| args.into_iter().map(str::to_string).collect())
        .collect::<Vec<Vec<String>>>()
    );
    assert!(
        spy.calls()
            .iter()
            .all(|call| call.args.first().map(String::as_str) != Some("push"))
    );
}

#[tokio::test]
async fn open_pr_push_failure_keeps_the_branch_and_switches_back() {
    let spy = scripted(|command, args| {
        if command == "git" && args.first().map(String::as_str) == Some("push") {
            Some(CmdOut::fail(1, "rejected"))
        } else {
            None
        }
    });
    let client = Github::with(spy.clone(), || HEX.to_string());
    let repo = repo_path();
    assert_eq!(
        client
            .open_pr(&repo, "Fix login", "")
            .await
            .unwrap_err()
            .to_string(),
        format!("Could not push (rejected). Your change is committed on branch {BRANCH}.")
    );
    let calls = spy.calls();
    assert!(
        calls
            .iter()
            .any(|call| call.args.first().map(String::as_str) == Some("push"))
    );
    assert!(
        calls
            .iter()
            .all(|call| call.args.first().map(String::as_str) != Some("branch"))
    );
    assert_eq!(
        calls.last().unwrap(),
        &at("git", &["switch", "main"], Some(&repo))
    );
}

#[tokio::test]
async fn open_pr_create_failure_keeps_the_pushed_branch() {
    let spy = scripted(|command, args| {
        if command == "gh" && args.first().map(String::as_str) == Some("pr") {
            Some(CmdOut::fail(1, "nope"))
        } else {
            None
        }
    });
    let client = Github::with(spy.clone(), || HEX.to_string());
    let repo = repo_path();
    assert_eq!(
        client
            .open_pr(&repo, "Fix login", "")
            .await
            .unwrap_err()
            .to_string(),
        format!(
            "Could not open the pull request (nope). Your change is pushed on branch {BRANCH}."
        )
    );
    let calls = spy.calls();
    assert!(
        calls
            .iter()
            .all(|call| call.args.first().map(String::as_str) != Some("branch"))
    );
    assert_eq!(
        calls.last().unwrap(),
        &at("git", &["switch", "main"], Some(&repo))
    );
}

#[tokio::test]
async fn open_pr_slug_falls_back_trims_and_caps_at_40() {
    let cases = [
        ("!!!".to_string(), format!("orch/pr-change-{HEX}")),
        (
            "  MixED  Case!! ".to_string(),
            format!("orch/pr-mixed-case-{HEX}"),
        ),
        (
            "Foo_Bar baz".to_string(),
            format!("orch/pr-foo-bar-baz-{HEX}"),
        ),
        ("a".repeat(50), format!("orch/pr-{}-{HEX}", "a".repeat(40))),
        (
            format!("{}-zzz", "a".repeat(39)),
            format!("orch/pr-{}-{HEX}", "a".repeat(39)),
        ),
    ];
    for (title, branch) in cases {
        let spy = scripted(|_, _| None);
        let client = Github::with(spy.clone(), || HEX.to_string());
        client.open_pr(&repo_path(), &title, "").await.unwrap();
        let created = spy
            .calls()
            .into_iter()
            .find(|call| {
                call.args.first().map(String::as_str) == Some("switch")
                    && call.args.get(1).map(String::as_str) == Some("-c")
            })
            .unwrap();
        assert_eq!(created.args[2], branch);
    }
}

#[tokio::test]
async fn open_pr_rejects_a_title_that_starts_with_a_dash() {
    let spy = scripted(|_, _| None);
    let client = Github::with(spy.clone(), || HEX.to_string());
    assert_eq!(
        client
            .open_pr(&repo_path(), "-m", "body")
            .await
            .unwrap_err()
            .to_string(),
        "invalid title"
    );
    assert_eq!(
        client
            .open_pr(&repo_path(), "--output=evil", "")
            .await
            .unwrap_err()
            .to_string(),
        "invalid title"
    );
    assert!(spy.calls().is_empty());
}

#[allow(clippy::type_complexity)]
fn real_or(
    override_fn: impl Fn(&str, &[String]) -> Option<CmdOut> + Send + Sync + 'static,
) -> Spy<
    impl Fn(
        String,
        Vec<String>,
        Option<PathBuf>,
    ) -> Pin<Box<dyn std::future::Future<Output = CmdOut> + Send>>
    + Send
    + Sync,
> {
    Spy::new(
        move |command: String,
              args: Vec<String>,
              cwd: Option<PathBuf>|
              -> Pin<Box<dyn std::future::Future<Output = CmdOut> + Send>> {
            let forced = override_fn(&command, &args);
            Box::pin(async move {
                if let Some(out) = forced {
                    return out;
                }
                if command == "gh" {
                    if args.first().map(String::as_str) == Some("auth") {
                        return CmdOut::ok("");
                    }
                    return CmdOut::new(0, format!("{PR_URL}\n"), "");
                }
                ProcessRun::command().run(command, args, cwd).await
            })
        },
    )
}

struct PrRepo {
    _temp: Temp,
    project: PathBuf,
    origin: PathBuf,
    root: PathBuf,
}

fn pr_repo() -> PrRepo {
    let temp = Temp::new();
    let root = temp.path().to_path_buf();
    let origin = root.join("origin.git");
    let project = root.join("project");
    let origin_s = origin.to_string_lossy().into_owned();
    let project_s = project.to_string_lossy().into_owned();
    git_ok(&root, &["init", "-q", "--bare", "-b", "main", &origin_s]);
    git_ok(&root, &["init", "-q", "-b", "main", &project_s]);
    git_ok(&project, &["config", "user.email", "dev@example.com"]);
    git_ok(&project, &["config", "user.name", "Dev"]);
    git_ok(&project, &["config", "core.autocrlf", "false"]);
    git_ok(&project, &["config", "commit.gpgsign", "false"]);
    git_ok(&project, &["config", "color.ui", "false"]);
    fs::write(project.join("kept.txt"), "one\n").unwrap();
    fs::write(project.join("app.txt"), "old\n").unwrap();
    git_ok(&project, &["add", "-A"]);
    git_ok(&project, &["commit", "-q", "-m", "first"]);
    git_ok(&project, &["remote", "add", "origin", &origin_s]);
    git_ok(&project, &["push", "-q", "-u", "origin", "main"]);
    fs::write(project.join("app.txt"), "new\n").unwrap();
    git_ok(&project, &["add", "app.txt"]);
    fs::write(project.join("kept.txt"), "one\nmine\n").unwrap();
    fs::write(project.join("notes.txt"), "untracked\n").unwrap();
    PrRepo {
        _temp: temp,
        project,
        origin,
        root,
    }
}

fn other_work_is_intact(project: &Path) {
    assert_eq!(
        fs::read_to_string(project.join("kept.txt")).unwrap(),
        "one\nmine\n"
    );
    assert_eq!(
        fs::read_to_string(project.join("notes.txt")).unwrap(),
        "untracked\n"
    );
}

#[tokio::test]
async fn commits_only_the_staged_change_on_a_new_branch() {
    let repo = pr_repo();
    let client = Github::with(real_or(|_, _| None), || "abc123".to_string());
    let result = client
        .open_pr(&repo.project, "Fix the login page", "Why it broke.")
        .await
        .unwrap();
    assert_eq!(
        result,
        OpenedPr {
            url: PR_URL.to_string(),
            branch: "orch/pr-fix-the-login-page-abc123".to_string(),
        }
    );
    assert_eq!(
        git_text(&repo.project, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "main"
    );
    other_work_is_intact(&repo.project);
    assert_eq!(
        fs::read_to_string(repo.project.join("app.txt")).unwrap(),
        "old\n"
    );
    assert_eq!(
        git_text(&repo.project, &["diff", "--cached", "--name-only"]),
        ""
    );
    assert_eq!(
        git_text(
            &repo.origin,
            &["show", &format!("{}:app.txt", result.branch)]
        ),
        "new"
    );
    assert_eq!(
        git_text(
            &repo.origin,
            &["log", "-1", "--format=%s|%b", &result.branch]
        ),
        "Fix the login page|Why it broke."
    );
    assert_eq!(
        git_text(
            &repo.origin,
            &["show", "--name-only", "--format=", &result.branch]
        ),
        "app.txt"
    );
    assert_eq!(
        git_text(
            &repo.origin,
            &["show", &format!("{}:kept.txt", result.branch)]
        ),
        "one"
    );
}

#[tokio::test]
async fn a_failed_commit_leaves_the_change_staged_and_removes_the_branch() {
    let repo = pr_repo();
    let client = Github::with(
        real_or(|command, args| {
            if command == "git" && args.first().map(String::as_str) == Some("commit") {
                Some(CmdOut::fail(1, "hook refused"))
            } else {
                None
            }
        }),
        || "abc123".to_string(),
    );
    assert_eq!(
        client
            .open_pr(&repo.project, "Fix it", "")
            .await
            .unwrap_err()
            .to_string(),
        "hook refused"
    );
    assert_eq!(
        git_text(&repo.project, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "main"
    );
    assert_eq!(
        git_text(&repo.project, &["diff", "--cached", "--name-only"]),
        "app.txt"
    );
    assert_eq!(
        fs::read_to_string(repo.project.join("app.txt")).unwrap(),
        "new\n"
    );
    assert_eq!(git_text(&repo.project, &["branch", "--list", "orch/*"]), "");
    other_work_is_intact(&repo.project);
}

#[tokio::test]
async fn a_failed_push_keeps_the_commit_on_the_local_branch() {
    let repo = pr_repo();
    let gone = repo.root.join("gone.git");
    let gone_s = gone.to_string_lossy().into_owned();
    git_ok(&repo.project, &["remote", "set-url", "origin", &gone_s]);
    let client = Github::with(real_or(|_, _| None), || "abc123".to_string());
    let branch = "orch/pr-fix-it-abc123";
    let err = client
        .open_pr(&repo.project, "Fix it", "")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains(&format!("Your change is committed on branch {branch}.")));
    assert_eq!(
        git_text(&repo.project, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "main"
    );
    assert_eq!(
        git_text(&repo.project, &["show", &format!("{branch}:app.txt")]),
        "new"
    );
    assert!(!gone.exists());
    other_work_is_intact(&repo.project);
}

#[tokio::test]
async fn a_failed_pr_create_says_the_branch_is_pushed() {
    let repo = pr_repo();
    let client = Github::with(
        real_or(|command, args| {
            if command == "gh" && args.first().map(String::as_str) == Some("pr") {
                Some(CmdOut::fail(1, "GraphQL: No commits between main and head"))
            } else {
                None
            }
        }),
        || "abc123".to_string(),
    );
    assert_eq!(
        client
            .open_pr(&repo.project, "Fix it", "")
            .await
            .unwrap_err()
            .to_string(),
        "Could not open the pull request (GraphQL: No commits between main and head). Your change is pushed on branch orch/pr-fix-it-abc123."
    );
    assert_eq!(
        git_text(&repo.project, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "main"
    );
    other_work_is_intact(&repo.project);
}
