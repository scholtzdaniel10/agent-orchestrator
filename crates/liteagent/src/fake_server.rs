//! In-process llama-server, Ollama, and OpenAI stand-ins.
//!
//! Prompt processing is charged per uncached character, same as the Python fake,
//! so cache tests see the same shape of timings without a model.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};

const PP_MS_PER_CHAR: f64 = 0.5;
const GEN_MS_PER_CHAR: f64 = 2.0;

#[derive(Clone, Copy)]
enum Mode {
    Llama,
    Ollama,
    OpenAi,
}

struct Inner {
    script: VecDeque<String>,
    slots: HashMap<u32, String>,
    saved: HashMap<String, String>,
    calls: Vec<FakeCall>,
    bodies: Vec<Recorded>,
}

/// One llama-server `/completion`, in characters standing in for tokens.
#[derive(Debug, Clone)]
pub struct FakeCall {
    pub cached: usize,
    pub new_chars: usize,
    pub schema: bool,
    pub schema_body: Option<Value>,
    pub n_predict: i64,
    pub cache_prompt: bool,
}

/// A POST the fake server saw.
#[derive(Debug, Clone)]
pub struct Recorded {
    pub path: String,
    pub body: Value,
}

type Shared = Arc<Mutex<Inner>>;

pub struct FakeServer {
    url: String,
    state: Shared,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

impl FakeServer {
    pub async fn llama(script: &[&str]) -> Self {
        Self::start(Mode::Llama, script).await
    }

    pub async fn ollama(script: &[&str]) -> Self {
        Self::start(Mode::Ollama, script).await
    }

    pub async fn openai(script: &[&str]) -> Self {
        Self::start(Mode::OpenAi, script).await
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn clear_slots(&self) {
        lock(&self.state).slots.clear();
    }

    pub fn calls(&self) -> Vec<FakeCall> {
        lock(&self.state).calls.clone()
    }

    pub fn bodies(&self) -> Vec<Recorded> {
        lock(&self.state).bodies.clone()
    }

    async fn start(mode: Mode, script: &[&str]) -> Self {
        let state = Arc::new(Mutex::new(Inner {
            script: script.iter().map(|s| (*s).to_string()).collect(),
            slots: HashMap::new(),
            saved: HashMap::new(),
            calls: Vec::new(),
            bodies: Vec::new(),
        }));
        let app = router(mode, Arc::clone(&state));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake server");
        let port = listener.local_addr().expect("local addr").port();
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self {
            url: format!("http://127.0.0.1:{port}"),
            state,
            handle: Some(handle),
        }
    }
}

fn router(mode: Mode, state: Shared) -> Router {
    let mut router = Router::new().fallback(fallback);
    router = match mode {
        Mode::Llama => router
            .route("/health", get(health))
            .route("/props", get(props))
            .route("/apply-template", post(apply_template))
            .route("/completion", post(completion))
            .route("/slots/{id}", post(slots)),
        Mode::Ollama => router
            .route("/api/version", get(ollama_version))
            .route("/api/chat", post(ollama_chat)),
        Mode::OpenAi => router
            .route("/v1/models", get(openai_models))
            .route("/v1/chat/completions", post(openai_chat)),
    };
    router.with_state(state)
}

async fn fallback() -> (StatusCode, Json<Value>) {
    (StatusCode::NOT_FOUND, Json(json!({})))
}

async fn health() -> Json<Value> {
    Json(json!({"status": "ok"}))
}

async fn props() -> Json<Value> {
    Json(json!({
        "default_generation_settings": {"n_predict": 512},
        "total_slots": 1,
        "chat_template": "test"
    }))
}

async fn ollama_version() -> Json<Value> {
    Json(json!({"version": "0.0.0-test"}))
}

async fn openai_models() -> Json<Value> {
    Json(json!({"object": "list", "data": [{"id": "test"}]}))
}

async fn apply_template(State(state): State<Shared>, body: Bytes) -> Json<Value> {
    let value = parse_body(&body);
    record(&state, "/apply-template", &value);
    let messages = value.get("messages").and_then(Value::as_array);
    let mut prompt = String::new();
    if let Some(messages) = messages {
        for message in messages {
            let role = message.get("role").and_then(Value::as_str).unwrap_or("");
            let content = message.get("content").and_then(Value::as_str).unwrap_or("");
            prompt.push_str(&format!("<|{role}|>{content}<|end|>"));
        }
    }
    prompt.push_str("<|assistant|>");
    Json(json!({"prompt": prompt}))
}

async fn completion(State(state): State<Shared>, body: Bytes) -> (StatusCode, Json<Value>) {
    let value = parse_body(&body);
    record(&state, "/completion", &value);
    let mut inner = lock(&state);
    let sid = value.get("id_slot").and_then(Value::as_u64).unwrap_or(0) as u32;
    let prompt = value
        .get("prompt")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let cache = value
        .get("cache_prompt")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let n_predict = value.get("n_predict").and_then(Value::as_i64).unwrap_or(-1);
    let prev = if cache {
        inner.slots.get(&sid).cloned().unwrap_or_default()
    } else {
        String::new()
    };
    let cached = common_chars(&prev, &prompt);
    let new_chars = char_len(&prompt).saturating_sub(cached);
    let content = if n_predict == 0 {
        String::new()
    } else {
        match inner.script.pop_front() {
            Some(line) => line,
            None => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": "script exhausted"})),
                );
            }
        }
    };
    inner.slots.insert(sid, format!("{prompt}{content}"));
    let schema_body = value.get("json_schema").cloned();
    inner.calls.push(FakeCall {
        cached,
        new_chars,
        schema: schema_body.is_some(),
        schema_body,
        n_predict,
        cache_prompt: cache,
    });
    let response = json!({
        "content": content,
        "tokens_cached": char_len(&prompt) + char_len(&content),
        "timings": {
            "prompt_n": new_chars,
            "prompt_ms": new_chars as f64 * PP_MS_PER_CHAR,
            "predicted_n": char_len(&content),
            "predicted_ms": char_len(&content) as f64 * GEN_MS_PER_CHAR,
        }
    });
    (StatusCode::OK, Json(response))
}

async fn slots(
    State(state): State<Shared>,
    Path(id): Path<u32>,
    Query(query): Query<HashMap<String, String>>,
    body: Bytes,
) -> (StatusCode, Json<Value>) {
    let value = parse_body(&body);
    record(&state, &format!("/slots/{id}"), &value);
    let Some(filename) = value
        .get("filename")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": "filename"})));
    };
    let action = query.get("action").map(String::as_str).unwrap_or("");
    let mut inner = lock(&state);
    if action == "save" {
        let text = inner.slots.get(&id).cloned().unwrap_or_default();
        inner.saved.insert(filename.clone(), text);
        return (
            StatusCode::OK,
            Json(json!({"id_slot": id, "filename": filename})),
        );
    }
    if action == "restore" {
        let Some(text) = inner.saved.get(&filename).cloned() else {
            return (StatusCode::BAD_REQUEST, Json(json!({"error": "no file"})));
        };
        inner.slots.insert(id, text);
        return (
            StatusCode::OK,
            Json(json!({"id_slot": id, "filename": filename})),
        );
    }
    (StatusCode::NOT_FOUND, Json(json!({})))
}

async fn ollama_chat(State(state): State<Shared>, body: Bytes) -> (StatusCode, Json<Value>) {
    let value = parse_body(&body);
    record(&state, "/api/chat", &value);
    let mut inner = lock(&state);
    let Some(content) = inner.script.pop_front() else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "script exhausted"})),
        );
    };
    let model = value.get("model").cloned().unwrap_or(Value::Null);
    let response = json!({
        "model": model,
        "message": {"role": "assistant", "content": content},
        "done": true,
        "prompt_eval_count": 12,
        "prompt_eval_duration": 2_000_000,
        "eval_count": char_len(&content),
        "eval_duration": 4_000_000
    });
    (StatusCode::OK, Json(response))
}

async fn openai_chat(State(state): State<Shared>, body: Bytes) -> (StatusCode, Json<Value>) {
    let value = parse_body(&body);
    record(&state, "/v1/chat/completions", &value);
    let mut inner = lock(&state);
    let Some(content) = inner.script.pop_front() else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "script exhausted"})),
        );
    };
    (StatusCode::OK, Json(openai_response(&content)))
}

fn openai_response(content: &str) -> Value {
    let parsed: Value = serde_json::from_str(content).unwrap_or(Value::Null);
    let usage = json!({
        "prompt_tokens": 100,
        "completion_tokens": 20,
        "prompt_tokens_details": {"cached_tokens": 40}
    });
    if let Some(name) = parsed.get("tool").and_then(Value::as_str) {
        let args = parsed.get("args").cloned().unwrap_or(json!({}));
        let thought = parsed.get("thought").and_then(Value::as_str).unwrap_or("");
        let arguments = serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string());
        return json!({
            "choices": [{
                "index": 0,
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "content": thought,
                    "tool_calls": [{
                        "id": "call_0",
                        "type": "function",
                        "function": {"name": name, "arguments": arguments}
                    }]
                }
            }],
            "usage": usage
        });
    }
    json!({
        "choices": [{
            "index": 0,
            "finish_reason": "stop",
            "message": {"role": "assistant", "content": content}
        }],
        "usage": usage
    })
}

fn record(state: &Shared, path: &str, body: &Value) {
    lock(state).bodies.push(Recorded {
        path: path.to_string(),
        body: body.clone(),
    });
}

fn parse_body(body: &Bytes) -> Value {
    if body.is_empty() {
        return json!({});
    }
    serde_json::from_slice(body).unwrap_or(json!({}))
}

fn lock(state: &Shared) -> std::sync::MutexGuard<'_, Inner> {
    state.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn char_len(text: &str) -> usize {
    text.chars().count()
}

fn common_chars(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}
