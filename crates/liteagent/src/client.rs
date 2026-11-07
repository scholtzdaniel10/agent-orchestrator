//! HTTP backends.
//!
//! llama-server uses the native API (`/apply-template`, `/completion`, `/slots`)
//! because that is what prefix save/restore and grammar sampling need.
//! Ollama uses `/api/chat` with a JSON schema in `format`. Any other
//! OpenAI-compatible server uses `/v1/chat/completions` with tools.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::toolpath::{ToolSpec, parse_step, synthesize_step};

/// Which protocol a [`Backend`] speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerKind {
    /// llama.cpp `llama-server` native routes.
    LlamaServer,
    /// Ollama native `/api/chat`.
    Ollama,
    /// `/v1/chat/completions` with tools.
    OpenAi,
}

/// One completion, with the timings the bench report needs.
#[derive(Debug, Clone)]
pub struct CallStats {
    /// Backend name (`big` or `small`), not the GGUF filename.
    pub model: String,
    /// Prompt tokens actually processed on this call.
    pub prompt_tokens: u64,
    /// Prompt tokens reused from the KV cache.
    pub cached_tokens: u64,
    /// Time to first token, milliseconds. This is the wait before the step.
    pub prompt_ms: f64,
    /// Tokens generated.
    pub gen_tokens: u64,
    /// Generation time, milliseconds.
    pub gen_ms: f64,
    /// Wall time of the HTTP call, milliseconds.
    pub wall_ms: f64,
}

impl CallStats {
    /// Generated tokens per second. Zero when the server reported no gen time.
    pub fn gen_tps(&self) -> f64 {
        if self.gen_ms > 0.0 {
            self.gen_tokens as f64 / (self.gen_ms / 1000.0)
        } else {
            0.0
        }
    }
}

/// One chat message. The agent only appends to this list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// `system`, `user`, or `assistant`.
    pub role: String,
    /// Message text.
    pub content: String,
}

/// A model server. Construct with [`Backend::llama`], [`Backend::ollama`],
/// [`Backend::openai`], or [`Backend::connect`] to auto-detect.
#[derive(Debug, Clone)]
pub struct Backend {
    url: String,
    kind: ServerKind,
    name: String,
    slot: u32,
    cache: bool,
    timeout: Duration,
    num_ctx: Option<u32>,
    model: Option<String>,
    slot_dir: Option<PathBuf>,
    temperature: f64,
    history: Vec<CallStats>,
}

impl Backend {
    /// llama-server at `url` (for example `http://127.0.0.1:8080`).
    pub fn llama(url: impl Into<String>) -> Self {
        Self::build(url.into(), ServerKind::LlamaServer)
    }

    /// Ollama at `url` (for example `http://127.0.0.1:11434`). Set a model with
    /// [`Backend::with_model`]. Pass context length with [`Backend::with_num_ctx`].
    pub fn ollama(url: impl Into<String>) -> Self {
        Self::build(url.into(), ServerKind::Ollama)
    }

    /// Any server that speaks `/v1/chat/completions` with tools.
    pub fn openai(url: impl Into<String>) -> Self {
        Self::build(url.into(), ServerKind::OpenAi)
    }

    /// Probe `url` and pick llama-server, Ollama, or OpenAI-compatible.
    ///
    /// Ollama is `/api/version` with a `version` string. llama-server is `/props`
    /// or `/health`. Anything else that answers `/v1/models` is OpenAI-compatible.
    pub async fn connect(url: impl Into<String>) -> Result<Self> {
        let url = trim_url(url.into());
        let kind = detect(&url).await?;
        Ok(Self::build(url, kind))
    }

    /// Label stored on [`CallStats::model`]. Use `big` and `small` for the cascade.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Model name for Ollama and OpenAI-compatible servers.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Ollama `options.num_ctx`. Ignored by llama-server (set `-c` on the server).
    pub fn with_num_ctx(mut self, num_ctx: u32) -> Self {
        self.num_ctx = Some(num_ctx);
        self
    }

    /// llama-server slot id. The same slot is reused so `cache_prompt` hits.
    pub fn with_slot(mut self, slot: u32) -> Self {
        self.slot = slot;
        self
    }

    /// Folder passed to llama-server as `--slot-save-path`. `None` still tries
    /// restore/save; the server decides whether the file exists.
    pub fn with_slot_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.slot_dir = Some(dir.into());
        self
    }

    /// `false` turns off llama-server `cache_prompt` (the bench baseline).
    pub fn with_cache(mut self, cache: bool) -> Self {
        self.cache = cache;
        self
    }

    /// Per-request timeout. The default is 600 seconds.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Sampling temperature. The default is `0.2`.
    pub fn with_temperature(mut self, temperature: f64) -> Self {
        self.temperature = temperature;
        self
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> ServerKind {
        self.kind
    }

    pub fn cache(&self) -> bool {
        self.cache
    }

    pub(crate) fn slot_dir(&self) -> Option<PathBuf> {
        self.slot_dir.clone()
    }

    /// Every completion issued on this backend, including prefix builds.
    pub fn history(&self) -> &[CallStats] {
        &self.history
    }

    /// `true` when the server's health route returns HTTP 200.
    pub async fn health(&self) -> bool {
        match self.kind {
            ServerKind::LlamaServer => self.status_ok("/health").await,
            ServerKind::Ollama => self.status_ok("/api/version").await,
            ServerKind::OpenAi => {
                self.status_ok("/v1/models").await || self.status_ok("/health").await
            }
        }
    }

    pub(crate) async fn render(&self, messages: &[Message]) -> Result<String> {
        let body = json!({ "messages": messages_json(messages) });
        let out = self
            .post_json("/apply-template", &body, self.timeout)
            .await?;
        match out.get("prompt").and_then(Value::as_str) {
            Some(prompt) => Ok(prompt.to_string()),
            None => Err(Error::Missing {
                url: self.joined("/apply-template"),
                field: "prompt",
            }),
        }
    }

    pub(crate) async fn complete_prompt(
        &mut self,
        prompt: &str,
        schema: Option<&Value>,
        n_predict: u32,
    ) -> Result<(String, CallStats)> {
        let mut body = json!({
            "prompt": prompt,
            "n_predict": n_predict,
            "temperature": self.temperature,
            "cache_prompt": self.cache,
            "id_slot": self.slot,
            "stream": false,
        });
        if let Some(schema) = schema {
            body["json_schema"] = schema.clone();
        }
        let started = Instant::now();
        let out = self.post_json("/completion", &body, self.timeout).await?;
        let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
        let stats = llama_stats(&self.name, &out, wall_ms);
        let text = match out.get("content") {
            Some(Value::String(text)) => text.clone(),
            _ => String::new(),
        };
        self.history.push(stats.clone());
        Ok((text, stats))
    }

    pub(crate) async fn complete_chat(
        &mut self,
        messages: &[Message],
        schema: &Value,
        specs: &[ToolSpec],
        n_predict: u32,
    ) -> Result<(String, CallStats)> {
        let started = Instant::now();
        let (text, stats) = match self.kind {
            ServerKind::Ollama => self.ollama_chat(messages, schema, n_predict).await?,
            ServerKind::OpenAi => self.openai_chat(messages, specs, n_predict).await?,
            ServerKind::LlamaServer => {
                let prompt = self.render(messages).await?;
                return self.complete_prompt(&prompt, Some(schema), n_predict).await;
            }
        };
        let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
        let mut stats = stats;
        stats.wall_ms = wall_ms;
        self.history.push(stats.clone());
        Ok((text, stats))
    }

    pub(crate) async fn slot_save(&self, filename: &str) -> Result<()> {
        let path = format!("/slots/{}?action=save", self.slot);
        self.post_json(&path, &json!({ "filename": filename }), self.timeout)
            .await?;
        Ok(())
    }

    pub(crate) async fn slot_restore(&self, filename: &str) -> Result<()> {
        let path = format!("/slots/{}?action=restore", self.slot);
        self.post_json(&path, &json!({ "filename": filename }), self.timeout)
            .await?;
        Ok(())
    }

    async fn ollama_chat(
        &self,
        messages: &[Message],
        schema: &Value,
        n_predict: u32,
    ) -> Result<(String, CallStats)> {
        let Some(model) = &self.model else {
            return Err(Error::ModelRequired);
        };
        let mut options = json!({
            "temperature": self.temperature,
            "num_predict": n_predict,
        });
        if let Some(num_ctx) = self.num_ctx {
            options["num_ctx"] = json!(num_ctx);
        }
        let body = json!({
            "model": model,
            "messages": messages_json(messages),
            "stream": false,
            "format": schema,
            "options": options,
        });
        let out = self.post_json("/api/chat", &body, self.timeout).await?;
        let text = out
            .pointer("/message/content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        Ok((text, ollama_stats(&self.name, &out)))
    }

    async fn openai_chat(
        &self,
        messages: &[Message],
        specs: &[ToolSpec],
        n_predict: u32,
    ) -> Result<(String, CallStats)> {
        let tools: Vec<Value> = specs
            .iter()
            .map(|spec| {
                json!({
                    "type": "function",
                    "function": {
                        "name": spec.name,
                        "description": spec.description,
                        "parameters": spec.params,
                        "strict": true,
                    }
                })
            })
            .collect();
        let model = self.model.clone().unwrap_or_else(|| "default".to_string());
        let body = json!({
            "model": model,
            "messages": messages_json(messages),
            "temperature": self.temperature,
            "max_tokens": n_predict,
            "tools": tools,
            "tool_choice": "required",
            "stream": false,
        });
        let out = self
            .post_json("/v1/chat/completions", &body, self.timeout)
            .await?;
        Ok((openai_text(&out), openai_stats(&self.name, &out)))
    }

    async fn status_ok(&self, path: &str) -> bool {
        match self
            .request(reqwest::Method::GET, path, None, Duration::from_secs(5))
            .await
        {
            Ok((status, _)) => status == 200,
            Err(_) => false,
        }
    }

    async fn post_json(&self, path: &str, body: &Value, timeout: Duration) -> Result<Value> {
        let (status, text) = self
            .request(reqwest::Method::POST, path, Some(body), timeout)
            .await?;
        let url = self.joined(path);
        if !(200..300).contains(&status) {
            return Err(Error::Status {
                url,
                status,
                body: truncate(&text),
            });
        }
        if text.trim().is_empty() {
            return Ok(json!({}));
        }
        serde_json::from_str(&text).map_err(|source| Error::Json { url, source })
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<(u16, String)> {
        let client = http_client().map_err(Error::Client)?;
        let url = self.joined(path);
        let mut req = client.request(method, &url).timeout(timeout);
        if let Some(body) = body {
            req = req.json(body);
        }
        let response = req.send().await.map_err(|source| Error::Http {
            url: url.clone(),
            source,
        })?;
        let status = response.status().as_u16();
        let text = response.text().await.map_err(|source| Error::Http {
            url: url.clone(),
            source,
        })?;
        Ok((status, text))
    }

    fn joined(&self, path: &str) -> String {
        format!("{}{path}", self.url)
    }

    fn build(url: String, kind: ServerKind) -> Self {
        Self {
            url: trim_url(url),
            kind,
            name: "big".to_string(),
            slot: 0,
            cache: true,
            timeout: Duration::from_secs(600),
            num_ctx: None,
            model: None,
            slot_dir: None,
            temperature: 0.2,
            history: Vec::new(),
        }
    }
}

fn messages_json(messages: &[Message]) -> Value {
    Value::Array(
        messages
            .iter()
            .map(|msg| json!({ "role": msg.role, "content": msg.content }))
            .collect(),
    )
}

/// `cached = tokens_cached - prompt_n - predicted_n`, then clamped at zero.
/// That is the Python client's reading of llama-server timings.
fn llama_stats(name: &str, out: &Value, wall_ms: f64) -> CallStats {
    let timings = out.get("timings");
    let prompt_tokens = json_u64(timings.and_then(|t| t.get("prompt_n")));
    let gen_tokens = json_u64(timings.and_then(|t| t.get("predicted_n")));
    let tokens_cached = json_u64(out.get("tokens_cached"));
    let cached_tokens = tokens_cached
        .saturating_sub(prompt_tokens)
        .saturating_sub(gen_tokens);
    CallStats {
        model: name.to_string(),
        prompt_tokens,
        cached_tokens,
        prompt_ms: json_f64(timings.and_then(|t| t.get("prompt_ms"))),
        gen_tokens,
        gen_ms: json_f64(timings.and_then(|t| t.get("predicted_ms"))),
        wall_ms,
    }
}

fn ollama_stats(name: &str, out: &Value) -> CallStats {
    let cached = json_u64(out.get("prompt_cache_hit_tokens"))
        .max(json_u64(out.get("cached_tokens")))
        .max(json_u64(out.pointer("/prompt_eval_cached")));
    CallStats {
        model: name.to_string(),
        prompt_tokens: json_u64(out.get("prompt_eval_count")),
        cached_tokens: cached,
        prompt_ms: json_f64(out.get("prompt_eval_duration")) / 1_000_000.0,
        gen_tokens: json_u64(out.get("eval_count")),
        gen_ms: json_f64(out.get("eval_duration")) / 1_000_000.0,
        wall_ms: 0.0,
    }
}

fn openai_stats(name: &str, out: &Value) -> CallStats {
    let prompt_total = json_u64(out.pointer("/usage/prompt_tokens"));
    let cached = json_u64(out.pointer("/usage/prompt_tokens_details/cached_tokens"));
    CallStats {
        model: name.to_string(),
        prompt_tokens: prompt_total.saturating_sub(cached),
        cached_tokens: cached,
        prompt_ms: 0.0,
        gen_tokens: json_u64(out.pointer("/usage/completion_tokens")),
        gen_ms: 0.0,
        wall_ms: 0.0,
    }
}

fn openai_text(out: &Value) -> String {
    let Some(message) = out.pointer("/choices/0/message") else {
        return String::new();
    };
    if let Some(content) = message.get("content").and_then(Value::as_str)
        && parse_step(content).is_some()
    {
        return content.to_string();
    }
    let Some(calls) = message.get("tool_calls").and_then(Value::as_array) else {
        return message
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
    };
    let Some(first) = calls.first() else {
        return String::new();
    };
    let name = first
        .pointer("/function/name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let arguments = first
        .pointer("/function/arguments")
        .cloned()
        .unwrap_or(Value::Null);
    let args_json = match arguments {
        Value::String(text) => text,
        other => other.to_string(),
    };
    let thought = message.get("content").and_then(Value::as_str).unwrap_or("");
    if name.is_empty() {
        return thought.to_string();
    }
    synthesize_step(thought, name, &args_json)
}

fn json_u64(value: Option<&Value>) -> u64 {
    let Some(value) = value else {
        return 0;
    };
    if let Some(n) = value.as_u64() {
        return n;
    }
    if let Some(n) = value.as_i64() {
        return n.max(0) as u64;
    }
    if let Some(n) = value.as_f64() {
        if n.is_sign_negative() || !n.is_finite() {
            return 0;
        }
        return n.trunc() as u64;
    }
    0
}

fn json_f64(value: Option<&Value>) -> f64 {
    value.and_then(Value::as_f64).unwrap_or(0.0)
}

fn truncate(text: &str) -> String {
    const MAX: usize = 800;
    let count = text.chars().count();
    if count <= MAX {
        text.to_string()
    } else {
        let head: String = text.chars().take(MAX).collect();
        format!("{head}...")
    }
}

fn trim_url(url: String) -> String {
    url.trim().trim_end_matches('/').to_string()
}

fn http_client() -> std::result::Result<&'static reqwest::Client, String> {
    static CLIENT: OnceLock<std::result::Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(600))
                .build()
                .map_err(|err| err.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
}

async fn detect(url: &str) -> Result<ServerKind> {
    let client = http_client().map_err(Error::Client)?;
    let (version, props, health) = tokio::join!(
        get_json(client, url, "/api/version"),
        get_json(client, url, "/props"),
        get_json(client, url, "/health"),
    );
    if version
        .as_ref()
        .and_then(|v| v.get("version"))
        .and_then(Value::as_str)
        .is_some()
        && !is_llama_props(props.as_ref())
    {
        return Ok(ServerKind::Ollama);
    }
    if is_llama_props(props.as_ref()) || is_llama_health(health.as_ref()) {
        return Ok(ServerKind::LlamaServer);
    }
    if get_json(client, url, "/v1/models").await.is_some() {
        return Ok(ServerKind::OpenAi);
    }
    if version.is_some() || props.is_some() || health.is_some() {
        return Ok(ServerKind::OpenAi);
    }
    Err(Error::Detect {
        url: url.to_string(),
        reason: "no /api/version, /props, /health, or /v1/models response".to_string(),
    })
}

fn is_llama_props(value: Option<&Value>) -> bool {
    let Some(obj) = value.and_then(Value::as_object) else {
        return false;
    };
    obj.contains_key("default_generation_settings")
        || obj.contains_key("total_slots")
        || obj.contains_key("chat_template")
        || obj.contains_key("model_path")
        || obj.contains_key("generation_settings")
}

fn is_llama_health(value: Option<&Value>) -> bool {
    value
        .and_then(|v| v.get("status"))
        .and_then(Value::as_str)
        .is_some()
}

async fn get_json(client: &reqwest::Client, base: &str, path: &str) -> Option<Value> {
    let url = format!("{base}{path}");
    let response = client
        .get(&url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let text = response.text().await.ok()?;
    serde_json::from_str(&text).ok()
}
