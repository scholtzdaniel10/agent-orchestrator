//! The worktree gate, a direct lead chat, and a shell tab, against a fake Ollama.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use legate_app::{AppEvent, AppHandle, AppOptions, JobAccess, JobType, TerminalKind};
use legate_core::{JobStatus, ProviderId, Settings};
use legate_providers::{Bin, ClaudeAdapter, CursorAdapter};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::Notify;

struct Server {
    url: String,
    gate: Arc<Notify>,
}

async fn serve(hold_first: bool) -> Server {
    let gate = Arc::new(Notify::new());
    let state = Arc::new(ChatState {
        hold_first,
        held: AtomicBool::new(false),
        gate: Arc::clone(&gate),
    });
    let app = Router::new()
        .route(
            "/api/version",
            get(|| async { Json(json!({"version": "0.9.0"})) }),
        )
        .route(
            "/api/tags",
            get(|| async { Json(json!({"models": [{"name": "coder-agent"}]})) }),
        )
        .route("/api/chat", post(chat))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Server {
        url: format!("http://127.0.0.1:{port}"),
        gate,
    }
}

struct ChatState {
    hold_first: bool,
    held: AtomicBool,
    gate: Arc<Notify>,
}

async fn chat(State(state): State<Arc<ChatState>>, body: Bytes) -> Json<Value> {
    let text = String::from_utf8_lossy(&body);
    let writing = text.contains("write_file") && !text.contains("Tool result:");
    if state.hold_first && !state.held.swap(true, Ordering::SeqCst) {
        state.gate.notified().await;
    }
    let content = if writing {
        r#"{"thought":"write","tool":"write_file","args":{"path":"hello.txt","text":"hello from local\n"}}"#
    } else {
        r#"{"thought":"done","tool":"final_answer","args":{"answer":"hello from local"}}"#
    };
    Json(json!({
        "model": "coder-agent",
        "message": {"role": "assistant", "content": content},
        "done": true
    }))
}

struct Fixture {
    data: PathBuf,
    repo: PathBuf,
    worktrees: PathBuf,
    other: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("legate-app-{}", uuid::Uuid::new_v4()));
        let fixture = Self {
            data: root.join("data"),
            repo: root.join("repo"),
            worktrees: root.join("wt"),
            other: root.join("other"),
        };
        std::fs::create_dir_all(&fixture.data).unwrap();
        std::fs::create_dir_all(&fixture.repo).unwrap();
        std::fs::create_dir_all(&fixture.worktrees).unwrap();
        std::fs::create_dir_all(&fixture.other).unwrap();
        git(&fixture.repo, &["init", "-b", "main"]);
        git(&fixture.repo, &["config", "user.email", "test@example.com"]);
        git(&fixture.repo, &["config", "user.name", "Test"]);
        git(&fixture.repo, &["config", "core.autocrlf", "false"]);
        std::fs::write(fixture.repo.join("README.md"), "seed\n").unwrap();
        git(&fixture.repo, &["add", "README.md"]);
        git(&fixture.repo, &["commit", "-m", "init"]);
        fixture
    }

    fn write_settings(&self, url: &str) {
        let mut settings = Settings::open(self.data.join("settings.json"));
        settings
            .set_prefs(&json!({
                "localUrl": url,
                "localModel": "coder-agent",
                "usageCheck": false
            }))
            .unwrap();
        let projects = vec![
            self.repo.to_string_lossy().into_owned(),
            self.other.to_string_lossy().into_owned(),
        ];
        // `set_project` records the folder and keeps it in the project list.
        settings.set_project(&projects[0]).unwrap();
        let mut raw: Value = serde_json::from_str(
            &std::fs::read_to_string(self.data.join("settings.json")).unwrap(),
        )
        .unwrap();
        raw["projects"] = json!(projects);
        std::fs::write(
            self.data.join("settings.json"),
            serde_json::to_string_pretty(&raw).unwrap(),
        )
        .unwrap();
    }

    fn options(&self) -> AppOptions {
        let missing = Bin::new("legate-missing-cli");
        AppOptions {
            data_dir: Some(self.data.clone()),
            project: Some(self.repo.clone()),
            worktrees_dir: Some(self.worktrees.clone()),
            claude: Some(Arc::new(ClaudeAdapter::new(Some(missing.clone())))),
            cursor: Some(Arc::new(CursorAdapter::new(Some(missing.clone())))),
            grok: Some(Arc::new(CursorAdapter::new(Some(missing)))),
            local: None,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let root = self.data.parent().unwrap_or(Path::new("."));
        let _ = std::fs::remove_dir_all(root);
    }
}

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|err| panic!("git {}: {err}", args.join(" ")));
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn wait_done(
    rx: &mut tokio::sync::broadcast::Receiver<AppEvent>,
    id: &str,
) -> legate_core::JobRecord {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            panic!("timed out waiting for a job update for {id}");
        }
        let event = tokio::time::timeout(left, rx.recv())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for a job update for {id}"))
            .unwrap_or_else(|_| panic!("event channel closed"));
        let AppEvent::Job(job) = event else {
            continue;
        };
        if job.id != id {
            continue;
        }
        if matches!(job.status, JobStatus::Done | JobStatus::Failed) {
            return job;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn edit_job_runs_on_local_merges_and_drops_the_worktree() {
    let server = serve(false).await;
    let fixture = Fixture::new();
    fixture.write_settings(&server.url);
    let app = AppHandle::start(fixture.options()).await.expect("start");

    let plans = app.list_plans().await.expect("plans");
    let local = plans
        .iter()
        .find(|plan| plan.id == ProviderId::Local)
        .expect("local plan");
    assert!(local.available, "local plan: {local:?}");

    let mut rx = app.subscribe();
    let submitted = app
        .submit_job(
            JobType::Debugging,
            "Write hello.txt",
            None,
            Some(JobAccess::Edit),
            None,
        )
        .await
        .expect("submit");
    let finished = wait_done(&mut rx, &submitted.id).await;
    assert_eq!(
        finished.status,
        JobStatus::Done,
        "provider {:?} error {:?} output {}",
        finished.provider,
        finished.error,
        finished.output
    );
    assert_eq!(finished.provider, Some(ProviderId::Local));

    let change_id = finished.change.clone().expect("job change id");
    // A job update also refreshes the change list, and that git read can hold
    // the worktree index for a moment. Ask again until the new file is counted.
    let mut change = None;
    for _ in 0..20 {
        let changes = app.list_changes().await.expect("changes");
        if let Some(found) = changes.into_iter().find(|item| item.id == change_id)
            && !found.files.is_empty()
            && found.insertions > 0
        {
            change = Some(found);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let change = change.unwrap_or_else(|| panic!("change {change_id} never listed a file"));
    let folder = PathBuf::from(&change.path);

    let merged = app.merge_change(&change_id).await.expect("merge");
    assert!(merged.ok, "{}", merged.message);
    assert!(!folder.exists(), "worktree still at {}", folder.display());
    let staged = Command::new("git")
        .args(["diff", "--cached", "--name-only"])
        .current_dir(&fixture.repo)
        .output()
        .unwrap();
    let names = String::from_utf8_lossy(&staged.stdout);
    assert!(
        names
            .lines()
            .any(|line| line.replace('\\', "/") == "hello.txt"),
        "staged names: {names}"
    );

    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn switch_project_is_refused_while_a_job_runs() {
    let server = serve(true).await;
    let fixture = Fixture::new();
    fixture.write_settings(&server.url);
    let app = AppHandle::start(fixture.options()).await.expect("start");
    let mut rx = app.subscribe();
    let job = app
        .submit_job(
            JobType::Debugging,
            "Write hello.txt",
            Some(ProviderId::Local),
            Some(JobAccess::Edit),
            None,
        )
        .await
        .expect("submit");
    let refused = app
        .switch_project(fixture.other.to_string_lossy().as_ref())
        .await
        .expect_err("switch should be refused");
    assert_eq!(refused.to_string(), "Refused while work is running");
    server.gate.notify_one();
    let finished = wait_done(&mut rx, &job.id).await;
    assert_eq!(finished.status, JobStatus::Done, "{:?}", finished.error);
    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn lead_chat_to_local_streams_messages() {
    let server = serve(false).await;
    let fixture = Fixture::new();
    fixture.write_settings(&server.url);
    let app = AppHandle::start(fixture.options()).await.expect("start");
    app.reset_lead(Some(ProviderId::Local))
        .await
        .expect("reset lead");
    let mut rx = app.subscribe();
    let message = app.send_lead("Say hello").await.expect("send");
    let mut streamed = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(50), rx.recv()).await {
            Ok(Ok(AppEvent::Lead { message, .. })) => {
                if message.text.contains("hello from local") {
                    streamed = true;
                    break;
                }
            }
            Ok(Err(_)) => break,
            Err(_) if streamed => break,
            _ => {}
        }
    }
    assert!(
        streamed,
        "lead reply was {:?} and no streamed update contained the answer",
        message.text
    );
    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shell_terminal_echoes_on_subscribe() {
    let server = serve(false).await;
    let fixture = Fixture::new();
    fixture.write_settings(&server.url);
    let app = AppHandle::start(fixture.options()).await.expect("start");
    let mut rx = app.subscribe();
    let info = app
        .open_terminal(TerminalKind::Shell, 80.0, 24.0, None, None)
        .await
        .expect("open shell");
    let mut echoed = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
            Ok(Ok(AppEvent::TerminalData { id, data })) if id == info.id => {
                echoed.push_str(&data);
                // ConPTY asks for the cursor position before the command's output.
                if data.contains("[6n") {
                    let _ = app.write_terminal(&info.id, "\u{1b}[1;1R", 1).await;
                }
                if echoed.contains("legate-shell-ok") {
                    break;
                }
            }
            Ok(Err(_)) => break,
            Err(_) => {}
            _ => {}
        }
    }
    let snapshot = app.terminal_snapshot(&info.id).await.unwrap_or_default();
    let listed = app.list_terminals().await.unwrap_or_default();
    assert!(
        echoed.contains("legate-shell-ok"),
        "terminal {} output was {echoed:?}; snapshot {snapshot:?}; terminals {listed:?}",
        info.id
    );
    app.shutdown().await;
}
