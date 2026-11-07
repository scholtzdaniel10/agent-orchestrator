//! Shared HTTP client and in-memory backend for the bridge tests.

#![allow(dead_code)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use legate_bridge::{
    BackendError, Bridge, BridgeBackend, BridgeTool, NewJob, ToolConfig, ToolError, WorkerSnapshot,
    start_bridge,
};
use legate_core::{JobAccess, JobRecord, JobStatus, LeadProposal, ProviderId};
use reqwest::header::HeaderMap;
use reqwest::{Client, Method};
use serde_json::{Map, Value, json};
use tokio::sync::Notify;
use uuid::Uuid;

pub fn client() -> Client {
    Client::builder()
        .http1_only()
        .no_proxy()
        .pool_max_idle_per_host(0)
        .timeout(Duration::from_secs(10))
        .build()
        .expect("client")
}

pub struct Reply {
    pub status: u16,
    pub headers: HeaderMap,
    pub text: String,
}

pub async fn exchange(
    client: &Client,
    method: Method,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<String>,
) -> Reply {
    let mut req = client.request(method, url);
    for (name, value) in headers {
        req = req.header(*name, *value);
    }
    if let Some(body) = body {
        req = req.body(body);
    }
    let response = req.send().await.expect("response");
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let text = response.text().await.expect("body");
    Reply {
        status,
        headers,
        text,
    }
}

pub fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

pub fn header<'a>(reply: &'a Reply, name: &str) -> Option<&'a str> {
    reply
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
}

pub async fn authed(client: &Client, bridge: &Bridge, body: &str, extra: &[(&str, &str)]) -> Reply {
    let mut headers = vec![
        ("authorization", bearer(&bridge.info().token)),
        ("content-type", "application/json".to_string()),
    ];
    for (name, value) in extra {
        headers.retain(|(key, _)| !key.eq_ignore_ascii_case(name));
        headers.push((name, (*value).to_string()));
    }
    let view: Vec<(&str, &str)> = headers
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    exchange(
        client,
        Method::POST,
        &bridge.info().url,
        &view,
        Some(body.to_string()),
    )
    .await
}

pub fn json_body(reply: &Reply) -> Value {
    serde_json::from_str(&reply.text).expect("json")
}

pub async fn rpc(client: &Client, bridge: &Bridge, message: Value) -> Reply {
    let mut body = Map::new();
    body.insert("jsonrpc".to_string(), json!("2.0"));
    if let Some(object) = message.as_object() {
        for (key, value) in object {
            body.insert(key.clone(), value.clone());
        }
    }
    authed(
        client,
        bridge,
        &serde_json::to_string(&Value::Object(body)).unwrap(),
        &[],
    )
    .await
}

pub async fn call_ok(client: &Client, bridge: &Bridge, name: &str, args: Value) -> Value {
    let reply = rpc(
        client,
        bridge,
        json!({"id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}}),
    )
    .await;
    assert_eq!(reply.status, 200, "{}", reply.text);
    let body = json_body(&reply);
    assert!(body.get("error").is_none(), "{body}");
    let result = &body["result"];
    assert!(result.get("isError").is_none(), "{result}");
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
}

pub async fn call_err(client: &Client, bridge: &Bridge, name: &str, args: Value) -> String {
    let reply = rpc(
        client,
        bridge,
        json!({"id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}}),
    )
    .await;
    assert_eq!(reply.status, 200, "{}", reply.text);
    let body = json_body(&reply);
    assert!(body.get("error").is_none(), "{body}");
    let result = &body["result"];
    assert_eq!(result["isError"], json!(true), "{result}");
    result["content"][0]["text"].as_str().unwrap().to_string()
}

pub async fn start_orch(fake: Arc<Fake>, config: ToolConfig) -> Bridge {
    start_bridge(legate_bridge::orchestrator_tools(fake, config))
        .await
        .expect("bridge")
}

pub fn echo_tools() -> Vec<BridgeTool> {
    let echo_schema = json!({
        "type": "object",
        "properties": { "text": { "type": "string" } },
        "additionalProperties": false
    });
    vec![
        BridgeTool::new(
            "echo",
            "Returns a string or an object",
            echo_schema,
            |args| async move {
                if let Some(Value::String(text)) = args.get("text") {
                    Ok(Value::String(text.clone()))
                } else {
                    Ok(json!({"ok": true}))
                }
            },
        ),
        BridgeTool::new(
            "boom",
            "Always throws",
            json!({"type": "object"}),
            |_args| async move { Err(ToolError::new("boom")) },
        ),
    ]
}

pub fn port_of(bridge: &Bridge) -> u16 {
    bridge
        .info()
        .url
        .trim_start_matches("http://127.0.0.1:")
        .trim_end_matches("/mcp")
        .parse()
        .expect("port")
}

struct Script {
    text: String,
    fail: bool,
    hold: bool,
}

struct Stored {
    record: JobRecord,
    text: String,
    fail: bool,
}

struct Inner {
    max_access: JobAccess,
    lead_turn: Option<String>,
    workers: Vec<WorkerSnapshot>,
    jobs: Vec<Stored>,
    scripts: VecDeque<Script>,
    max_running: usize,
    released: bool,
    proposals: Vec<(String, LeadProposal)>,
}

/// Records tool side effects and plays back scripted job results.
pub struct Fake {
    inner: Mutex<Inner>,
    notify: Notify,
}

impl Fake {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                max_access: JobAccess::Read,
                lead_turn: None,
                workers: Vec::new(),
                jobs: Vec::new(),
                scripts: VecDeque::new(),
                max_running: usize::MAX,
                released: false,
                proposals: Vec::new(),
            }),
            notify: Notify::new(),
        })
    }

    pub fn set_max_access(&self, access: JobAccess) {
        lock(&self.inner).max_access = access;
    }

    pub fn set_lead_turn(&self, turn: Option<String>) {
        lock(&self.inner).lead_turn = turn;
    }

    pub fn set_workers(&self, workers: Vec<WorkerSnapshot>) {
        lock(&self.inner).workers = workers;
    }

    pub fn set_max_running(&self, max_running: usize) {
        lock(&self.inner).max_running = max_running;
    }

    pub fn push_ok(&self, text: &str) {
        self.push(text, false, false);
    }

    pub fn push_ok_held(&self, text: &str) {
        self.push(text, false, true);
    }

    pub fn push_fail(&self, text: &str) {
        self.push(text, true, false);
    }

    fn push(&self, text: &str, fail: bool, hold: bool) {
        lock(&self.inner).scripts.push_back(Script {
            text: text.to_string(),
            fail,
            hold,
        });
    }

    pub fn release(&self) {
        {
            let mut inner = lock(&self.inner);
            inner.released = true;
            for job in &mut inner.jobs {
                finish(job);
            }
        }
        self.notify.notify_waiters();
    }

    pub fn job(&self, id: &str) -> JobRecord {
        lock(&self.inner)
            .jobs
            .iter()
            .find(|job| job.record.id == id)
            .unwrap_or_else(|| panic!("missing {id}"))
            .record
            .clone()
    }

    pub fn jobs(&self) -> Vec<JobRecord> {
        lock(&self.inner)
            .jobs
            .iter()
            .map(|job| job.record.clone())
            .collect()
    }

    pub fn proposals(&self) -> Vec<(String, LeadProposal)> {
        lock(&self.inner).proposals.clone()
    }

    fn lookup(&self, id: &str) -> Option<JobRecord> {
        lock(&self.inner)
            .jobs
            .iter()
            .find(|job| job.record.id == id)
            .map(|job| job.record.clone())
    }
}

fn finish(job: &mut Stored) {
    if is_terminal(job.record.status) {
        return;
    }
    if job.fail {
        job.record.status = JobStatus::Failed;
        job.record.error = Some(job.text.clone());
    } else {
        job.record.status = JobStatus::Done;
        job.record.output = job.text.clone();
    }
}

fn is_terminal(status: JobStatus) -> bool {
    status == JobStatus::Done || status == JobStatus::Failed
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

#[async_trait]
#[allow(clippy::unused_async)]
impl BridgeBackend for Fake {
    async fn max_access(&self) -> JobAccess {
        lock(&self.inner).max_access
    }

    async fn lead_turn(&self) -> Option<String> {
        lock(&self.inner).lead_turn.clone()
    }

    async fn workers(&self) -> Result<Vec<WorkerSnapshot>, BackendError> {
        Ok(lock(&self.inner).workers.clone())
    }

    async fn send_job(&self, job: NewJob) -> Result<JobRecord, BackendError> {
        let mut inner = lock(&self.inner);
        let script = inner.scripts.pop_front().unwrap_or(Script {
            text: String::new(),
            fail: false,
            hold: false,
        });
        let running = inner
            .jobs
            .iter()
            .filter(|stored| stored.record.status == JobStatus::Running)
            .count();
        let open = script.hold && !inner.released;
        let status = if running >= inner.max_running {
            JobStatus::Queued
        } else if open {
            JobStatus::Running
        } else if script.fail {
            JobStatus::Failed
        } else {
            JobStatus::Done
        };
        let id = Uuid::new_v4().to_string();
        let mut record = blank_record(&id, &job, status);
        if status == JobStatus::Done {
            record.output = script.text.clone();
        } else if status == JobStatus::Failed {
            record.error = Some(script.text.clone());
        }
        inner.jobs.push(Stored {
            record: record.clone(),
            text: script.text,
            fail: script.fail,
        });
        Ok(record)
    }

    async fn list_jobs(&self) -> Result<Vec<JobRecord>, BackendError> {
        Ok(self.jobs())
    }

    async fn get_job(&self, id: &str) -> Result<Option<JobRecord>, BackendError> {
        Ok(self.lookup(id))
    }

    async fn wait_for_job(
        &self,
        id: &str,
        wait: Duration,
    ) -> Result<Option<JobRecord>, BackendError> {
        let deadline = Instant::now() + wait;
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            match self.lookup(id) {
                Some(job) if is_terminal(job.status) => return Ok(Some(job)),
                None => return Ok(None),
                Some(_) => {}
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(self.lookup(id));
            }
            tokio::select! {
                () = notified => {}
                () = tokio::time::sleep(remaining) => return Ok(self.lookup(id)),
            }
        }
    }

    async fn propose_jobs(&self, turn: String, proposal: LeadProposal) -> Result<(), BackendError> {
        lock(&self.inner).proposals.push((turn, proposal));
        Ok(())
    }
}

fn blank_record(id: &str, job: &NewJob, status: JobStatus) -> JobRecord {
    let edits = job.access != JobAccess::Read;
    JobRecord {
        id: id.to_string(),
        job_type: job.job_type,
        prompt: job.prompt.clone(),
        provider: Some(ProviderId::Claude),
        status,
        output: String::new(),
        failed_over: Vec::new(),
        steps: None,
        steps_dropped: None,
        model: None,
        reason: None,
        edit: edits.then_some(true),
        access: Some(job.access),
        change: edits.then(|| id.chars().take(8).collect()),
        group: None,
        lead_message: job.lead_message.clone(),
        routine: None,
        title: None,
        session_id: None,
        goal: None,
        base: None,
        error: None,
        owner_id: None,
        heartbeat_at: None,
        cancel_requested: None,
        extra: BTreeMap::new(),
    }
}
