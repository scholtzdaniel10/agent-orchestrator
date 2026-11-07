//! Local provider against an in-process Ollama and llama-server.
//!
//! No model is loaded. The fake serves the routes liteagent probes and the
//! scripted JSON steps the agent loop expects.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use legate_core::{AgentEvent, FileEdit, JobAccess, RunOptions, StepPhase, StepTool};
use legate_providers::{LimitError, LocalAdapter, LocalConfig, ProviderAdapter, ProviderId};
use serde_json::{Value, json};
use tokio::task::JoinHandle;

const PROMPT: &str = "What does hello.txt say? Answer with its exact contents.";
const OLLAMA_PROMPT_TOKENS: i64 = 12;
const LLAMA_PROMPT_TOKENS: i64 = 5;

#[cfg(windows)]
const SHELL_CMD: &str = "Set-Content -Path shell-marker.txt -Value legate-ok";
#[cfg(not(windows))]
const SHELL_CMD: &str = "printf legate-ok > shell-marker.txt";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Ollama,
    Llama,
}

struct Hit {
    path: String,
    body: Value,
}

struct Inner {
    mode: Mode,
    script: VecDeque<String>,
    hits: Vec<Hit>,
    version_hits: u32,
    hang_chat: bool,
    fail_body: Option<String>,
    tags: Vec<String>,
    build_info: String,
    released: Arc<AtomicBool>,
}

type Shared = Arc<Mutex<Inner>>;

struct UntilDrop(Arc<AtomicBool>);

impl Drop for UntilDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

struct Fake {
    url: String,
    state: Shared,
    handle: Option<JoinHandle<()>>,
}

impl Drop for Fake {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

impl Fake {
    async fn ollama(script: &[&str]) -> Self {
        Self::start(Mode::Ollama, script).await
    }

    async fn llama(script: &[&str]) -> Self {
        Self::start(Mode::Llama, script).await
    }

    fn url(&self) -> &str {
        &self.url
    }

    fn set_hang(&self, hang: bool) {
        lock(&self.state).hang_chat = hang;
    }

    fn set_tags(&self, tags: &[&str]) {
        lock(&self.state).tags = tags.iter().map(|tag| (*tag).to_string()).collect();
    }

    fn fail_chat(&self, body: &str) {
        lock(&self.state).fail_body = Some(body.to_string());
    }

    fn version_hits(&self) -> u32 {
        lock(&self.state).version_hits
    }

    fn bodies(&self, path: &str) -> Vec<Value> {
        lock(&self.state)
            .hits
            .iter()
            .filter(|hit| hit.path == path)
            .map(|hit| hit.body.clone())
            .collect()
    }

    fn saw(&self, path: &str) -> bool {
        lock(&self.state).hits.iter().any(|hit| hit.path == path)
    }

    fn released(&self) -> bool {
        lock(&self.state).released.load(Ordering::SeqCst)
    }

    async fn start(mode: Mode, script: &[&str]) -> Self {
        let state = Arc::new(Mutex::new(Inner {
            mode,
            script: script.iter().map(|line| (*line).to_string()).collect(),
            hits: Vec::new(),
            version_hits: 0,
            hang_chat: false,
            fail_body: None,
            tags: vec!["coder-agent:latest".to_string()],
            build_info: "b4242-test".to_string(),
            released: Arc::new(AtomicBool::new(false)),
        }));
        let app = router(Arc::clone(&state));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake server");
        let port = listener.local_addr().expect("local addr").port();
        let url = format!("http://127.0.0.1:{port}");
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let fake = Self {
            url: url.clone(),
            state,
            handle: Some(handle),
        };
        let path = match mode {
            Mode::Ollama => "/api/version",
            Mode::Llama => "/health",
        };
        for _ in 0..50 {
            if probe(&fake.url, path).await {
                fake.reset_counts();
                return fake;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("fake server did not start at {url}");
    }

    fn reset_counts(&self) {
        let mut inner = lock(&self.state);
        inner.version_hits = 0;
        inner.hits.clear();
    }
}

fn router(state: Shared) -> Router {
    Router::new()
        .route("/api/version", get(ollama_version))
        .route("/api/tags", get(ollama_tags))
        .route("/api/chat", post(ollama_chat))
        .route("/health", get(health))
        .route("/props", get(props))
        .route("/apply-template", post(apply_template))
        .route("/completion", post(completion))
        .fallback(fallback)
        .with_state(state)
}

async fn fallback() -> Response {
    StatusCode::NOT_FOUND.into_response()
}

async fn ollama_version(State(state): State<Shared>) -> Response {
    let mut inner = lock(&state);
    if inner.mode != Mode::Ollama {
        return StatusCode::NOT_FOUND.into_response();
    }
    inner.version_hits = inner.version_hits.saturating_add(1);
    Json(json!({"version": "0.0.0-test"})).into_response()
}

async fn ollama_tags(State(state): State<Shared>) -> Response {
    let inner = lock(&state);
    if inner.mode != Mode::Ollama {
        return StatusCode::NOT_FOUND.into_response();
    }
    let models: Vec<Value> = inner
        .tags
        .iter()
        .map(|name| json!({"name": name, "model": name}))
        .collect();
    Json(json!({"models": models})).into_response()
}

async fn health(State(state): State<Shared>) -> Response {
    if lock(&state).mode != Mode::Llama {
        return StatusCode::NOT_FOUND.into_response();
    }
    Json(json!({"status": "ok"})).into_response()
}

async fn props(State(state): State<Shared>) -> Response {
    let inner = lock(&state);
    if inner.mode != Mode::Llama {
        return StatusCode::NOT_FOUND.into_response();
    }
    Json(json!({
        "default_generation_settings": {"n_predict": 512},
        "total_slots": 1,
        "build_info": inner.build_info,
    }))
    .into_response()
}

async fn ollama_chat(State(state): State<Shared>, body: Bytes) -> Response {
    if lock(&state).mode != Mode::Ollama {
        return StatusCode::NOT_FOUND.into_response();
    }
    let value = parse_body(&body);
    if lock(&state).hang_chat {
        let released = Arc::clone(&lock(&state).released);
        let guard = UntilDrop(released);
        {
            let mut inner = lock(&state);
            inner.hits.push(Hit {
                path: "/api/chat".to_string(),
                body: value,
            });
        }
        std::future::pending::<()>().await;
        drop(guard);
        return StatusCode::OK.into_response();
    }
    let (content, fail) = {
        let mut inner = lock(&state);
        inner.hits.push(Hit {
            path: "/api/chat".to_string(),
            body: value.clone(),
        });
        if let Some(fail) = inner.fail_body.clone() {
            (None, Some(fail))
        } else {
            (inner.script.pop_front(), None)
        }
    };
    if let Some(fail) = fail {
        return (StatusCode::BAD_REQUEST, fail).into_response();
    }
    let Some(content) = content else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "script exhausted"})),
        )
            .into_response();
    };
    let model = value.get("model").cloned().unwrap_or(Value::Null);
    Json(json!({
        "model": model,
        "message": {"role": "assistant", "content": content},
        "done": true,
        "prompt_eval_count": OLLAMA_PROMPT_TOKENS,
        "prompt_eval_duration": 2_000_000,
        "eval_count": content.chars().count() as u64,
        "eval_duration": 4_000_000
    }))
    .into_response()
}

async fn apply_template(State(state): State<Shared>, body: Bytes) -> Response {
    if lock(&state).mode != Mode::Llama {
        return StatusCode::NOT_FOUND.into_response();
    }
    let value = parse_body(&body);
    lock(&state).hits.push(Hit {
        path: "/apply-template".to_string(),
        body: value.clone(),
    });
    let mut prompt = String::new();
    if let Some(messages) = value.get("messages").and_then(Value::as_array) {
        for message in messages {
            let role = message.get("role").and_then(Value::as_str).unwrap_or("");
            let content = message.get("content").and_then(Value::as_str).unwrap_or("");
            prompt.push_str(role);
            prompt.push('\n');
            prompt.push_str(content);
            prompt.push('\n');
        }
    }
    Json(json!({"prompt": prompt})).into_response()
}

async fn completion(State(state): State<Shared>, body: Bytes) -> Response {
    if lock(&state).mode != Mode::Llama {
        return StatusCode::NOT_FOUND.into_response();
    }
    let value = parse_body(&body);
    let n_predict = value.get("n_predict").and_then(Value::as_i64).unwrap_or(-1);
    let content = {
        let mut inner = lock(&state);
        inner.hits.push(Hit {
            path: "/completion".to_string(),
            body: value,
        });
        if n_predict == 0 {
            Some(String::new())
        } else {
            inner.script.pop_front()
        }
    };
    let Some(content) = content else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "script exhausted"})),
        )
            .into_response();
    };
    let generated = if n_predict == 0 {
        0
    } else {
        content.chars().count() as u64
    };
    let prompt_n = if n_predict == 0 {
        0
    } else {
        u64::try_from(LLAMA_PROMPT_TOKENS).unwrap_or(0)
    };
    Json(json!({
        "content": content,
        "tokens_cached": prompt_n + generated,
        "timings": {
            "prompt_n": prompt_n,
            "prompt_ms": 1.0,
            "predicted_n": generated,
            "predicted_ms": 1.0
        }
    }))
    .into_response()
}

fn parse_body(body: &Bytes) -> Value {
    if body.is_empty() {
        return json!({});
    }
    serde_json::from_slice(body).unwrap_or_else(|_| json!({}))
}

fn lock(state: &Shared) -> std::sync::MutexGuard<'_, Inner> {
    state.lock().unwrap_or_else(|poison| poison.into_inner())
}

async fn probe(url: &str, path: &str) -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let Some(addr) = url.strip_prefix("http://") else {
        return false;
    };
    let connect = tokio::time::timeout(
        Duration::from_millis(200),
        tokio::net::TcpStream::connect(addr),
    );
    let Ok(Ok(mut stream)) = connect.await else {
        return false;
    };
    let req = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    if stream.write_all(req.as_bytes()).await.is_err() {
        return false;
    }
    let mut buf = [0u8; 128];
    match tokio::time::timeout(Duration::from_millis(500), stream.read(&mut buf)).await {
        Ok(Ok(n)) if n > 0 => String::from_utf8_lossy(&buf[..n]).contains("200"),
        _ => false,
    }
}

fn j_tool(thought: &str, tool: &str, args_body: &str) -> String {
    format!(r#"{{"thought": "{thought}", "tool": "{tool}", "args": {args_body}}}"#)
}

fn j_final(answer: &str) -> String {
    j_tool(
        "done",
        "final_answer",
        &format!(r#"{{"answer": "{answer}"}}"#),
    )
}

struct Temp {
    root: PathBuf,
}

impl Temp {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root =
            std::env::temp_dir().join(format!("legate-local-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn adapter(url: &str, model: &str, small: Option<&str>, num_ctx: u32) -> LocalAdapter {
    LocalAdapter::new(LocalConfig {
        url: url.to_string(),
        model: model.to_string(),
        small_model: small.map(str::to_string),
        num_ctx,
    })
}

fn access_opts(access: JobAccess) -> RunOptions {
    RunOptions {
        access: Some(access),
        resume: Some("sess-old".to_string()),
        ..RunOptions::default()
    }
}

async fn collect(
    run: legate_providers::RunHandle,
) -> (Vec<AgentEvent>, legate_providers::ProcessExit) {
    let collect = async {
        let mut run = run;
        let mut events = Vec::new();
        while let Some(event) = run.events.recv().await {
            events.push(event);
        }
        let exit = run.exit().await;
        (events, exit)
    };
    match tokio::time::timeout(Duration::from_secs(30), collect).await {
        Ok(pair) => pair,
        Err(_) => panic!("local run timed out"),
    }
}

fn tool_names(body: &Value) -> Vec<String> {
    body.pointer("/format/oneOf")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|branch| {
            branch
                .pointer("/properties/tool/const")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect()
}

fn system_text(body: &Value) -> String {
    body.pointer("/messages/0/content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn expect_step(
    event: &AgentEvent,
) -> (
    &str,
    StepPhase,
    StepTool,
    &str,
    Option<&str>,
    Option<bool>,
    Option<&FileEdit>,
) {
    match event {
        AgentEvent::Step {
            id,
            phase,
            tool,
            title,
            detail,
            ok,
            edit,
        } => (
            id.as_str(),
            *phase,
            *tool,
            title.as_str(),
            detail.as_deref(),
            *ok,
            edit.as_ref(),
        ),
        other => panic!("expected a step, got {other:?}"),
    }
}

fn result_text(events: &[AgentEvent]) -> &str {
    events
        .iter()
        .find_map(|event| match event {
            AgentEvent::Result { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .unwrap_or("")
}

fn expected_tokens(mode: Mode, lines: &[&str]) -> i64 {
    let prompt = match mode {
        Mode::Ollama => OLLAMA_PROMPT_TOKENS,
        Mode::Llama => LLAMA_PROMPT_TOKENS,
    };
    let generated: i64 = lines.iter().map(|line| line.chars().count() as i64).sum();
    prompt * lines.len() as i64 + generated
}

async fn assert_read_roundtrip(mode: Mode) {
    let read = j_tool("read", "read_file", r#"{"path": "hello.txt"}"#);
    let done = j_final("hi there");
    let script = [read.as_str(), done.as_str()];
    let fake = match mode {
        Mode::Ollama => Fake::ollama(&script).await,
        Mode::Llama => Fake::llama(&script).await,
    };
    let temp = Temp::new();
    std::fs::write(temp.path().join("hello.txt"), "hi there").unwrap();
    let model = "coder-agent";
    let local = adapter(fake.url(), model, None, 8192);
    assert_eq!(local.id(), ProviderId::Local);
    assert!(local.parse_event(r#"{"type":"result"}"#).is_none());
    let run = local.run("job-1", PROMPT, temp.path(), &access_opts(JobAccess::Read));
    let (events, exit) = collect(run).await;
    assert_eq!(exit.code, Some(0), "stderr {}", exit.stderr);
    assert!(exit.stderr.is_empty());
    assert!(!exit.unreadable);
    assert_eq!(events.len(), 5, "{events:?}");

    match &events[0] {
        AgentEvent::Init { session_id, model } => {
            assert_eq!(session_id, "job-1");
            assert_eq!(model.as_deref(), Some("coder-agent"));
        }
        other => panic!("init, got {other:?}"),
    }
    let start = expect_step(&events[1]);
    assert_eq!(start.0, "1");
    assert_eq!(start.1, StepPhase::Start);
    assert_eq!(start.2, StepTool::Read);
    assert_eq!(start.3, "Read hello.txt");
    assert_eq!(start.4, None);
    assert_eq!(start.5, None);
    assert!(start.6.is_none());
    let end = expect_step(&events[2]);
    assert_eq!(end.0, "1");
    assert_eq!(end.1, StepPhase::End);
    assert_eq!(end.2, StepTool::Read);
    assert_eq!(end.3, "Read hello.txt");
    assert_eq!(end.4, Some("hi there"));
    assert_eq!(end.5, Some(true));
    match &events[3] {
        AgentEvent::Text { text } => assert_eq!(text, "hi there"),
        other => panic!("text, got {other:?}"),
    }
    match &events[4] {
        AgentEvent::Result {
            ok,
            text,
            session_id,
            duration_ms,
            cost_usd,
            tokens,
        } => {
            assert!(*ok);
            assert_eq!(text, "hi there");
            assert_eq!(session_id.as_deref(), Some("job-1"));
            assert!(duration_ms.is_some_and(|ms| ms.is_finite() && ms >= 0.0));
            assert!(cost_usd.is_none());
            assert_eq!(*tokens, Some(expected_tokens(mode, &script)));
        }
        other => panic!("result, got {other:?}"),
    }
    assert!(!result_text(&events).contains("sess-old"));

    if mode == Mode::Ollama {
        let chats = fake.bodies("/api/chat");
        assert_eq!(chats.len(), 2, "{chats:?}");
        assert_eq!(chats[0]["model"], "coder-agent");
        assert_eq!(chats[0]["options"]["num_ctx"], 8192);
        assert_eq!(chats[0]["messages"][1]["content"], PROMPT);
        assert_eq!(
            chats[0]["messages"][0]["content"],
            chats[1]["messages"][0]["content"]
        );
        let names = tool_names(&chats[0]);
        assert!(names.contains(&"read_file".to_string()));
        assert!(!names.contains(&"write_file".to_string()));
        assert!(!names.contains(&"shell".to_string()));
        assert!(!names.contains(&"ask_bigger_model".to_string()));
    }
}

#[tokio::test]
async fn ollama_maps_a_read_and_the_final_answer() {
    assert_read_roundtrip(Mode::Ollama).await;
}

#[tokio::test]
async fn llama_server_maps_a_read_and_the_final_answer() {
    assert_read_roundtrip(Mode::Llama).await;
}

#[tokio::test]
async fn llama_server_version_comes_from_props_and_health_signs_in() {
    let fake = Fake::llama(&[]).await;
    let local = adapter(fake.url(), "local-gguf", None, 65536);
    assert!(local.is_installed().await);
    assert!(local.is_signed_in().await);
    assert_eq!(local.version().await.as_deref(), Some("b4242-test"));
    assert!(local.list_models().await.is_empty());
}

#[tokio::test]
async fn ollama_tags_sign_in_and_version() {
    let fake = Fake::ollama(&[]).await;
    fake.set_tags(&["coder-agent:latest", "tiny:latest"]);
    let local = adapter(fake.url(), "coder-agent", None, 65536);
    assert!(local.is_installed().await);
    assert_eq!(local.version().await.as_deref(), Some("0.0.0-test"));
    assert!(local.is_signed_in().await);
    let names: Vec<String> = local
        .list_models()
        .await
        .into_iter()
        .map(|model| model.id)
        .collect();
    assert_eq!(
        names,
        vec!["coder-agent:latest".to_string(), "tiny:latest".to_string()]
    );

    fake.set_tags(&["other:latest"]);
    assert!(!local.is_signed_in().await);
    local.set_config(LocalConfig {
        model: "coder-agent:latest".to_string(),
        ..local.config()
    });
    fake.set_tags(&["coder-agent:latest"]);
    assert!(local.is_signed_in().await);
    local.set_config(LocalConfig {
        model: "coder".to_string(),
        ..local.config()
    });
    assert!(!local.is_signed_in().await);
}

#[tokio::test]
async fn write_file_maps_to_an_edit_with_line_counts() {
    let first = j_tool(
        "write",
        "write_file",
        r#"{"path": "note.txt", "text": "one\ntwo"}"#,
    );
    let first_done = j_final("wrote");
    let second = j_tool(
        "rewrite",
        "write_file",
        r#"{"path": "note.txt", "text": "only"}"#,
    );
    let second_done = j_final("rewrote");
    let script = [
        first.as_str(),
        first_done.as_str(),
        second.as_str(),
        second_done.as_str(),
    ];
    let fake = Fake::ollama(&script).await;
    let temp = Temp::new();
    let local = adapter(fake.url(), "coder-agent", None, 65536);

    let run = local.run(
        "job-w",
        "write note.txt",
        temp.path(),
        &access_opts(JobAccess::Edit),
    );
    let (events, exit) = collect(run).await;
    assert_eq!(exit.code, Some(0), "{}", exit.stderr);
    let end = expect_step(&events[2]);
    assert_eq!(end.2, StepTool::Edit);
    assert_eq!(end.3, "Edited note.txt");
    assert_eq!(end.5, Some(true));
    let edit = end.6.expect("edit");
    assert_eq!(edit.path, "note.txt");
    assert_eq!(edit.added, 2);
    assert_eq!(edit.removed, 0);
    let start = expect_step(&events[1]);
    assert_eq!(start.6.map(|edit| (edit.added, edit.removed)), Some((2, 0)));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("note.txt")).unwrap(),
        "one\ntwo"
    );

    let run = local.run(
        "job-w2",
        "rewrite note.txt",
        temp.path(),
        &access_opts(JobAccess::Edit),
    );
    let (events, exit) = collect(run).await;
    assert_eq!(exit.code, Some(0), "{}", exit.stderr);
    let end = expect_step(&events[2]);
    let edit = end.6.expect("edit");
    assert_eq!(edit.added, 1);
    assert_eq!(edit.removed, 2);
    assert_eq!(
        std::fs::read_to_string(temp.path().join("note.txt")).unwrap(),
        "only"
    );
}

#[tokio::test]
async fn calculator_maps_to_other_and_list_files_maps_to_read() {
    let calc = j_tool("math", "calculator", r#"{"expression": "6*7"}"#);
    let calc_done = j_final("42");
    let list = j_tool("list", "list_files", r#"{"folder": "."}"#);
    let list_done = j_final("listed");
    let script = [
        calc.as_str(),
        calc_done.as_str(),
        list.as_str(),
        list_done.as_str(),
    ];
    let fake = Fake::ollama(&script).await;
    let temp = Temp::new();
    std::fs::write(temp.path().join("hello.txt"), "hi there").unwrap();
    let local = adapter(fake.url(), "coder-agent", None, 65536);

    let run = local.run("job-c", "6*7", temp.path(), &access_opts(JobAccess::Read));
    let (events, exit) = collect(run).await;
    assert_eq!(exit.code, Some(0), "{}", exit.stderr);
    let end = expect_step(&events[2]);
    assert_eq!(end.2, StepTool::Other);
    assert_eq!(end.3, "calculator");
    assert_eq!(end.4, Some("42"));
    assert_eq!(end.5, Some(true));

    let run = local.run("job-l", "list", temp.path(), &access_opts(JobAccess::Read));
    let (events, exit) = collect(run).await;
    assert_eq!(exit.code, Some(0), "{}", exit.stderr);
    let end = expect_step(&events[2]);
    assert_eq!(end.2, StepTool::Read);
    assert_eq!(end.3, "Read .");
    assert_eq!(end.4, Some("hello.txt"));
    assert_eq!(end.5, Some(true));
}

#[tokio::test]
async fn access_levels_expose_read_edit_and_shell_tools() {
    let done = j_final("ok");
    let script = [done.as_str(), done.as_str(), done.as_str()];
    let fake = Fake::ollama(&script).await;
    let temp = Temp::new();
    let local = adapter(fake.url(), "coder-agent", None, 65536);
    for access in [JobAccess::Read, JobAccess::Edit, JobAccess::Full] {
        let run = local.run("job-a", "hi", temp.path(), &access_opts(access));
        let (_events, exit) = collect(run).await;
        assert_eq!(exit.code, Some(0), "{access:?} {}", exit.stderr);
    }
    let chats = fake.bodies("/api/chat");
    assert_eq!(chats.len(), 3);
    assert_tools(&tool_names(&chats[0]), JobAccess::Read);
    assert_tools(&tool_names(&chats[1]), JobAccess::Edit);
    assert_tools(&tool_names(&chats[2]), JobAccess::Full);
    assert_eq!(chats[0]["options"]["num_ctx"], 65536);
}

fn assert_tools(names: &[String], access: JobAccess) {
    assert!(names.iter().any(|name| name == "calculator"), "{names:?}");
    assert!(names.iter().any(|name| name == "list_files"), "{names:?}");
    assert!(names.iter().any(|name| name == "read_file"), "{names:?}");
    assert!(names.iter().any(|name| name == "final_answer"), "{names:?}");
    assert!(!names.iter().any(|name| name == "write_note"), "{names:?}");
    assert!(
        !names.iter().any(|name| name == "ask_bigger_model"),
        "{names:?}"
    );
    let write = names.iter().any(|name| name == "write_file");
    let shell = names.iter().any(|name| name == "shell");
    match access {
        JobAccess::Read => {
            assert!(!write, "{names:?}");
            assert!(!shell, "{names:?}");
        }
        JobAccess::Edit => {
            assert!(write, "{names:?}");
            assert!(!shell, "{names:?}");
        }
        JobAccess::Full => {
            assert!(write, "{names:?}");
            assert!(shell, "{names:?}");
        }
    }
}

#[tokio::test]
async fn full_access_shell_runs_in_the_work_folder() {
    let call = j_tool("run", "shell", &format!(r#"{{"command": "{SHELL_CMD}"}}"#));
    let done = j_final("ran");
    let script = [call.as_str(), done.as_str()];
    let fake = Fake::ollama(&script).await;
    let temp = Temp::new();
    let local = adapter(fake.url(), "coder-agent", None, 65536);
    let run = local.run(
        "job-s",
        "run it",
        temp.path(),
        &access_opts(JobAccess::Full),
    );
    let (events, exit) = collect(run).await;
    assert_eq!(exit.code, Some(0), "{}", exit.stderr);
    let start = expect_step(&events[1]);
    assert_eq!(start.2, StepTool::Shell);
    assert_eq!(start.1, StepPhase::Start);
    assert!(start.3.contains(SHELL_CMD), "{}", start.3);
    assert_eq!(start.4, Some(SHELL_CMD));
    let end = expect_step(&events[2]);
    assert_eq!(end.2, StepTool::Shell);
    assert_eq!(end.5, Some(true), "{events:?}");
    let marker = std::fs::read_to_string(temp.path().join("shell-marker.txt"))
        .expect("shell did not write shell-marker.txt in the work folder");
    assert!(marker.contains("legate-ok"), "{marker:?}");
}

#[tokio::test]
async fn write_file_stays_inside_the_work_folder() {
    let call = j_tool(
        "escape",
        "write_file",
        r#"{"path": "../outside.txt", "text": "nope"}"#,
    );
    let done = j_final("blocked");
    let script = [call.as_str(), done.as_str()];
    let fake = Fake::ollama(&script).await;
    let temp = Temp::new();
    let work = temp.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let local = adapter(fake.url(), "coder-agent", None, 65536);
    let run = local.run("job-e", "escape", &work, &access_opts(JobAccess::Edit));
    let (events, exit) = collect(run).await;
    assert_eq!(exit.code, Some(0), "{}", exit.stderr);
    let end = expect_step(&events[2]);
    assert_eq!(end.2, StepTool::Edit);
    assert_eq!(end.5, Some(false), "{events:?}");
    assert!(end.6.is_none(), "an escaped path has no edit counts");
    let detail = end.4.unwrap_or("");
    assert!(detail.contains("outside"), "{detail}");
    assert!(!temp.path().join("outside.txt").exists());
    assert!(!work.join("outside.txt").exists());
}

#[tokio::test]
async fn kill_cancels_a_hung_request() {
    let fake = Fake::ollama(&[]).await;
    fake.set_hang(true);
    let temp = Temp::new();
    let local = adapter(fake.url(), "coder-agent", None, 65536);
    let run = local.run("job-k", "hang", temp.path(), &access_opts(JobAccess::Read));
    tokio::time::timeout(Duration::from_secs(5), async {
        while !fake.saw("/api/chat") {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("chat request did not start");
    let started = Instant::now();
    run.kill();
    let exit = tokio::time::timeout(Duration::from_secs(2), run.exit())
        .await
        .expect("kill did not finish the run");
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_ne!(exit.code, Some(0));
    tokio::time::timeout(Duration::from_secs(2), async {
        let mut run = run;
        while run.events.recv().await.is_some() {}
    })
    .await
    .expect("event stream stayed open after kill");
    tokio::time::timeout(Duration::from_secs(2), async {
        while !fake.released() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("hung request was not cancelled");
}

#[tokio::test]
async fn unreachable_server_is_not_installed_and_is_a_limit_error() {
    let local = adapter("http://127.0.0.1:1", "coder-agent", None, 65536);
    assert!(!local.is_installed().await);
    assert!(!local.is_signed_in().await);
    assert!(local.version().await.is_none());
    assert!(local.list_models().await.is_empty());
    let temp = Temp::new();
    let run = local.run("job-down", "hi", temp.path(), &access_opts(JobAccess::Read));
    let (events, exit) = tokio::time::timeout(Duration::from_secs(15), collect(run))
        .await
        .expect("unreachable run hung");
    assert_eq!(exit.code, Some(1));
    assert!(
        local.is_limit_error(LimitError::Text(&exit.stderr)),
        "{}",
        exit.stderr
    );
    let limit = events.iter().find_map(|event| match event {
        AgentEvent::Limit { message, .. } => Some(message.as_str()),
        _ => None,
    });
    let Some(message) = limit else {
        panic!("missing limit event: {events:?}");
    };
    assert!(local.is_limit_error(LimitError::Event(&AgentEvent::Limit {
        resets_at: None,
        message: message.to_string(),
    })));
    assert!(!local.is_limit_error(LimitError::Text("syntax error in the model output")));
    assert!(!local.is_limit_error(LimitError::Event(&AgentEvent::Text {
        text: "connection refused".to_string(),
    })));
    assert!(local.is_limit_error(LimitError::Text(
        "request (9000 tokens) exceeds the available context size (8192 tokens)"
    )));
}

#[tokio::test]
async fn context_overflow_from_the_server_is_a_limit_error() {
    let done = j_final("unused");
    let fake = Fake::ollama(&[done.as_str()]).await;
    fake.fail_chat("the request exceeds the available context size (8192 tokens)");
    let temp = Temp::new();
    let local = adapter(fake.url(), "coder-agent", None, 65536);
    let run = local.run("job-ctx", "hi", temp.path(), &access_opts(JobAccess::Read));
    let (events, exit) = collect(run).await;
    assert_eq!(exit.code, Some(1));
    assert!(
        local.is_limit_error(LimitError::Text(&exit.stderr)),
        "{}",
        exit.stderr
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AgentEvent::Limit { .. }))
    );
}

#[tokio::test]
async fn reuses_one_backend_and_keeps_the_system_prompt_identical() {
    let done = j_final("ok");
    let script = [done.as_str(), done.as_str()];
    let fake = Fake::ollama(&script).await;
    let first = Temp::new();
    let second = Temp::new();
    let local = adapter(fake.url(), "coder-agent", None, 32768);
    for (id, dir) in [("job-r1", first.path()), ("job-r2", second.path())] {
        let run = local.run(id, "same", dir, &access_opts(JobAccess::Read));
        let (_events, exit) = collect(run).await;
        assert_eq!(exit.code, Some(0), "{}", exit.stderr);
    }
    assert_eq!(fake.version_hits(), 1, "detect should run once per url");
    let chats = fake.bodies("/api/chat");
    assert_eq!(chats.len(), 2);
    let system = system_text(&chats[0]);
    assert_eq!(system, system_text(&chats[1]));
    let first_name = first.path().file_name().unwrap().to_string_lossy();
    let second_name = second.path().file_name().unwrap().to_string_lossy();
    assert!(!system.contains(first_name.as_ref()));
    assert!(!system.contains(second_name.as_ref()));
    assert_eq!(chats[0]["options"]["num_ctx"], 32768);
    assert_eq!(chats[1]["options"]["num_ctx"], 32768);
}

#[tokio::test]
async fn small_model_is_asked_first() {
    let done = j_final("from-small");
    let fake = Fake::ollama(&[done.as_str()]).await;
    let temp = Temp::new();
    let local = adapter(fake.url(), "coder-agent", Some("coder-small"), 65536);
    let run = local.run(
        "job-small",
        "hi",
        temp.path(),
        &access_opts(JobAccess::Read),
    );
    let (events, exit) = collect(run).await;
    assert_eq!(exit.code, Some(0), "{}", exit.stderr);
    assert_eq!(result_text(&events), "from-small");
    assert_eq!(fake.version_hits(), 1);
    let chats = fake.bodies("/api/chat");
    assert_eq!(
        chats.len(),
        1,
        "the small model's final answer should end the job"
    );
    assert_eq!(chats[0]["model"], "coder-small");
    assert!(
        tool_names(&chats[0])
            .iter()
            .any(|name| name == "ask_bigger_model")
    );
}

#[tokio::test]
async fn run_options_model_overrides_the_configured_model() {
    let done = j_final("ok");
    let fake = Fake::ollama(&[done.as_str()]).await;
    let temp = Temp::new();
    let local = adapter(fake.url(), "coder-agent", None, 65536);
    let mut opts = access_opts(JobAccess::Read);
    opts.model = Some("override-model".to_string());
    let run = local.run("job-m", "hi", temp.path(), &opts);
    let (events, exit) = collect(run).await;
    assert_eq!(exit.code, Some(0), "{}", exit.stderr);
    match &events[0] {
        AgentEvent::Init { model, .. } => assert_eq!(model.as_deref(), Some("override-model")),
        other => panic!("init, got {other:?}"),
    }
    assert_eq!(fake.bodies("/api/chat")[0]["model"], "override-model");
}

#[tokio::test]
#[ignore = "requires Ollama at 127.0.0.1:11434 with coder-agent:latest already pulled"]
async fn real_ollama_reads_hello_txt() {
    let local = LocalAdapter::new(LocalConfig {
        model: "coder-agent:latest".to_string(),
        ..LocalConfig::default()
    });
    assert!(
        local.is_installed().await,
        "ollama did not answer /api/version"
    );
    assert!(
        local.is_signed_in().await,
        "coder-agent:latest is not in /api/tags"
    );
    let temp = Temp::new();
    std::fs::write(temp.path().join("hello.txt"), "hi there").unwrap();
    let run = local.run(
        "smoke-1",
        PROMPT,
        temp.path(),
        &RunOptions {
            access: Some(JobAccess::Read),
            ..RunOptions::default()
        },
    );
    let (events, exit) = tokio::time::timeout(Duration::from_secs(240), collect(run))
        .await
        .expect("smoke timed out");
    for event in &events {
        println!("{event:?}");
    }
    println!("exit code={:?} stderr={}", exit.code, exit.stderr);
    assert_eq!(exit.code, Some(0), "{}", exit.stderr);
    assert!(
        result_text(&events).contains("hi there"),
        "result was {:?}",
        result_text(&events)
    );
}
