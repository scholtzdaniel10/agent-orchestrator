//! The lead's tools, matching `src/core/bridge/tools.ts`.
//!
//! Descriptions and input schemas are the TypeScript `JSON.stringify` text.
//! Length checks use UTF-16 code units, which is what JavaScript's `length`
//! and `slice` count.

use std::sync::Arc;
use std::time::Duration;

use legate_core::{
    JobAccess, JobRecord, JobStatus, JobType, LeadProposal, ProposalStatus, ProposedJob, ProviderId,
};
use serde::Serialize;
use serde_json::{Map, Value};
use time::format_description::FormatItem;
use time::macros::format_description;

use crate::backend::{BridgeBackend, NewJob, WorkerSnapshot};
use crate::error::ToolError;
use crate::server::BridgeTool;

/// `list_workers`
pub const LIST_WORKERS: &str = "list_workers";
/// `send_job`
pub const SEND_JOB: &str = "send_job";
/// `get_status`
pub const GET_STATUS: &str = "get_status";
/// `get_result`
pub const GET_RESULT: &str = "get_result";
/// `propose_jobs`
pub const PROPOSE_JOBS: &str = "propose_jobs";

/// Description of [`LIST_WORKERS`], byte-identical to `tools.ts`.
pub const LIST_WORKERS_DESCRIPTION: &str = "Who can run jobs right now, how much allowance each has left, and the access ceiling the person set (read, edit, or full). A job cannot be given more than that.";
/// Description of [`SEND_JOB`], byte-identical to `tools.ts`.
pub const SEND_JOB_DESCRIPTION: &str = "Hand one job to a worker and return its id immediately. The router chooses the worker. The prompt must be self-contained because the worker sees nothing else. Access is read (files only), edit (change files in a separate worktree the user reviews and merges), or full (edit plus commands, without asking). The person sets a ceiling; a request above it is lowered. Pass access as that string, or the old edit boolean (true means edit).";
/// Description of [`GET_STATUS`], byte-identical to `tools.ts`.
pub const GET_STATUS_DESCRIPTION: &str = "Return every job on the board, oldest first, or one job when id is set. Includes jobs the user started directly. Prompts are cut to 120 characters. Outputs are not included.";
/// Description of [`GET_RESULT`], byte-identical to `tools.ts`.
pub const GET_RESULT_DESCRIPTION: &str = "Wait until a job is done or failed, or until the wait ends, then return its status and output (last 6000 characters). If it is still queued or running, the current status is returned so the caller can ask again.";
/// Description of [`PROPOSE_JOBS`], byte-identical to `tools.ts`.
pub const PROPOSE_JOBS_DESCRIPTION: &str = "Propose up to 4 jobs for the person to run. Nothing starts until they press Run, and they may edit or remove jobs first. Each job needs a short title (a few words), a type, an access level (read, edit or full; above the ceiling it is lowered), and a self-contained prompt, because the worker sees nothing else. Leave worker out to let the router choose, or name claude, cursor or grok. Call it once per turn with the whole plan and a one-line summary.";

/// JSON schema for [`LIST_WORKERS`].
pub const LIST_WORKERS_SCHEMA: &str =
    r#"{"type":"object","properties":{},"additionalProperties":false}"#;
/// JSON schema for [`SEND_JOB`].
pub const SEND_JOB_SCHEMA: &str = r#"{"type":"object","additionalProperties":false,"required":["type","prompt"],"properties":{"type":{"type":"string","enum":["planning","debugging","review","refactor","boilerplate"]},"prompt":{"type":"string","minLength":1,"maxLength":20000},"edit":{"type":"boolean"},"access":{"type":"string","enum":["read","edit","full"]}}}"#;
/// JSON schema for [`GET_STATUS`].
pub const GET_STATUS_SCHEMA: &str =
    r#"{"type":"object","additionalProperties":false,"properties":{"id":{"type":"string"}}}"#;
/// JSON schema for [`GET_RESULT`].
pub const GET_RESULT_SCHEMA: &str = r#"{"type":"object","additionalProperties":false,"required":["id"],"properties":{"id":{"type":"string"},"wait_seconds":{"type":"number","minimum":0,"maximum":300}}}"#;
/// JSON schema for [`PROPOSE_JOBS`].
pub const PROPOSE_JOBS_SCHEMA: &str = r#"{"type":"object","additionalProperties":false,"required":["summary","jobs"],"properties":{"summary":{"type":"string","minLength":1,"maxLength":600},"jobs":{"type":"array","minItems":1,"maxItems":4,"items":{"type":"object","additionalProperties":false,"required":["title","type","prompt"],"properties":{"title":{"type":"string","minLength":1,"maxLength":80},"type":{"type":"string","enum":["planning","debugging","review","refactor","boilerplate"]},"access":{"type":"string","enum":["read","edit","full"]},"worker":{"type":"string","enum":["claude","cursor","grok"]},"prompt":{"type":"string","minLength":1,"maxLength":20000}}}}}}"#;

const MAX_PROMPT: usize = 20_000;
const MAX_OUTPUT: usize = 6_000;
const MAX_PROPOSED_JOBS: usize = 4;
const MAX_TITLE: usize = 80;
const MAX_SUMMARY: usize = 600;
const STATUS_PROMPT_UNITS: usize = 120;
const DEFAULT_WAIT_SECONDS: f64 = 120.0;
const MAX_WAIT_SECONDS: f64 = 300.0;

const ISO_MS: &[FormatItem<'static>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z");

const JOB_FIELDS: &[&str] = &["title", "type", "access", "worker", "prompt"];

/// Which lead tools to publish.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ToolConfig {
    /// When true, `propose_jobs` replaces `send_job`.
    pub propose: bool,
}

/// The lead's tools, wired to `backend`.
///
/// With [`ToolConfig::propose`], the model gets `propose_jobs` instead of
/// `send_job`, so nothing starts until the person runs the plan.
pub fn orchestrator_tools(backend: Arc<dyn BridgeBackend>, config: ToolConfig) -> Vec<BridgeTool> {
    let mut tools = Vec::with_capacity(4);
    tools.push(raw_tool(
        LIST_WORKERS,
        LIST_WORKERS_DESCRIPTION,
        LIST_WORKERS_SCHEMA,
        backend.clone(),
        list_workers,
    ));
    if config.propose {
        tools.push(raw_tool(
            PROPOSE_JOBS,
            PROPOSE_JOBS_DESCRIPTION,
            PROPOSE_JOBS_SCHEMA,
            backend.clone(),
            propose_jobs_tool,
        ));
    } else {
        tools.push(raw_tool(
            SEND_JOB,
            SEND_JOB_DESCRIPTION,
            SEND_JOB_SCHEMA,
            backend.clone(),
            send_job,
        ));
    }
    tools.push(raw_tool(
        GET_STATUS,
        GET_STATUS_DESCRIPTION,
        GET_STATUS_SCHEMA,
        backend.clone(),
        get_status,
    ));
    tools.push(raw_tool(
        GET_RESULT,
        GET_RESULT_DESCRIPTION,
        GET_RESULT_SCHEMA,
        backend,
        get_result,
    ));
    tools
}

/// A plan the person edited, checked like the lead's own proposal.
pub fn proposed_jobs(value: &Value) -> Result<Vec<ProposedJob>, ToolError> {
    let Some(items) = value.as_array() else {
        return Err(ToolError::new("jobs must be a non-empty list"));
    };
    if items.is_empty() {
        return Err(ToolError::new("jobs must be a non-empty list"));
    }
    if items.len() > MAX_PROPOSED_JOBS {
        return Err(ToolError::new(format!(
            "propose at most {MAX_PROPOSED_JOBS} jobs"
        )));
    }
    items
        .iter()
        .enumerate()
        .map(|(index, item)| proposed_job(item, index))
        .collect()
}

fn raw_tool<F, Fut>(
    name: &'static str,
    description: &'static str,
    schema: &'static str,
    backend: Arc<dyn BridgeBackend>,
    call: F,
) -> BridgeTool
where
    F: Fn(Arc<dyn BridgeBackend>, Map<String, Value>) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<Value, ToolError>> + Send + 'static,
{
    BridgeTool::from_raw(name, description, schema, move |args| {
        let backend = backend.clone();
        call(backend, args)
    })
}

async fn list_workers(
    backend: Arc<dyn BridgeBackend>,
    args: Map<String, Value>,
) -> Result<Value, ToolError> {
    reject_unknown(&args, &[])?;
    let max_access = backend.max_access().await;
    let workers = backend.workers().await?;
    let mut listed = Vec::with_capacity(workers.len());
    for worker in workers {
        listed.push(worker_json(worker)?);
    }
    render(&WorkersOut {
        max_access,
        workers: listed,
    })
}

async fn send_job(
    backend: Arc<dyn BridgeBackend>,
    args: Map<String, Value>,
) -> Result<Value, ToolError> {
    reject_unknown(&args, &["type", "prompt", "edit", "access"])?;
    let job_type = match args
        .get("type")
        .and_then(Value::as_str)
        .and_then(JobType::parse)
    {
        Some(job_type) => job_type,
        None => {
            return Err(ToolError::new(
                "type must be planning, debugging, review, refactor, or boilerplate",
            ));
        }
    };
    let Some(prompt) = args.get("prompt").and_then(Value::as_str) else {
        return Err(ToolError::new("prompt must be a non-empty string"));
    };
    if js_len(prompt) > MAX_PROMPT {
        return Err(ToolError::new("prompt must be at most 20000 characters"));
    }
    if prompt.trim().is_empty() {
        return Err(ToolError::new("prompt must be a non-empty string"));
    }
    if let Some(edit) = args.get("edit")
        && !edit.is_boolean()
    {
        return Err(ToolError::new("edit must be a boolean"));
    }
    if let Some(access) = args.get("access")
        && access.as_str().and_then(JobAccess::parse).is_none()
    {
        return Err(ToolError::new("access must be read, edit, or full"));
    }
    let requested = requested_access(&args);
    let max_access = backend.max_access().await;
    let granted = if access_rank(requested) > access_rank(max_access) {
        max_access
    } else {
        requested
    };
    let record = backend
        .send_job(NewJob {
            job_type,
            prompt: prompt.to_string(),
            access: granted,
            lead_message: backend.lead_turn().await,
        })
        .await?;
    let note = (granted != requested).then(|| {
        format!(
            "Access lowered to {}: the person allows {}.",
            granted.as_str(),
            access_label(granted)
        )
    });
    render(&SendOut {
        id: &record.id,
        provider: record.provider,
        status: record.status,
        note,
    })
}

async fn get_status(
    backend: Arc<dyn BridgeBackend>,
    args: Map<String, Value>,
) -> Result<Value, ToolError> {
    reject_unknown(&args, &["id"])?;
    let Some(id) = args.get("id") else {
        let jobs = backend.list_jobs().await?;
        let views: Vec<StatusView> = jobs.iter().map(status_view).collect();
        return render(&views);
    };
    let Some(id) = id.as_str() else {
        return Err(ToolError::new("id must be a string"));
    };
    let Some(job) = backend.get_job(id).await? else {
        return Err(ToolError::new(format!("Unknown job id: {id}")));
    };
    render(&status_view(&job))
}

async fn get_result(
    backend: Arc<dyn BridgeBackend>,
    args: Map<String, Value>,
) -> Result<Value, ToolError> {
    reject_unknown(&args, &["id", "wait_seconds"])?;
    let Some(id) = args.get("id").and_then(Value::as_str) else {
        return Err(ToolError::new("id must be a string"));
    };
    let Some(current) = backend.get_job(id).await? else {
        return Err(ToolError::new(format!("Unknown job id: {id}")));
    };
    let wait = wait_duration(args.get("wait_seconds"));
    let job = if wait.is_zero() || is_terminal(current.status) {
        current
    } else {
        match backend.wait_for_job(id, wait).await? {
            Some(job) => job,
            None => return Err(ToolError::new(format!("Unknown job id: {id}"))),
        }
    };
    render(&result_view(&job))
}

async fn propose_jobs_tool(
    backend: Arc<dyn BridgeBackend>,
    args: Map<String, Value>,
) -> Result<Value, ToolError> {
    reject_unknown(&args, &["summary", "jobs"])?;
    let Some(turn) = backend.lead_turn().await else {
        return Err(ToolError::new("propose_jobs only works during a lead turn"));
    };
    let Some(summary) = args.get("summary").and_then(Value::as_str) else {
        return Err(ToolError::new("summary must be a non-empty string"));
    };
    if summary.trim().is_empty() {
        return Err(ToolError::new("summary must be a non-empty string"));
    }
    let Some(items) = args.get("jobs").and_then(Value::as_array) else {
        return Err(ToolError::new("jobs must be a non-empty list"));
    };
    if items.is_empty() {
        return Err(ToolError::new("jobs must be a non-empty list"));
    }
    if items.len() > MAX_PROPOSED_JOBS {
        return Err(ToolError::new(format!(
            "propose at most {MAX_PROPOSED_JOBS} jobs"
        )));
    }
    let max_access = backend.max_access().await;
    let mut lowered = Vec::new();
    let mut jobs = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let mut job = proposed_job(item, index)?;
        if access_rank(job.access) > access_rank(max_access) {
            lowered.push(job.title.clone());
            job.access = max_access;
        }
        jobs.push(job);
    }
    let count = jobs.len();
    backend
        .propose_jobs(
            turn,
            LeadProposal {
                summary: js_prefix(summary.trim(), MAX_SUMMARY),
                jobs,
                status: ProposalStatus::Waiting,
                job_ids: None,
            },
        )
        .await?;
    let note = if lowered.is_empty() {
        None
    } else {
        Some(format!(
            "Access lowered to {} (the person allows {}) for: {}.",
            max_access.as_str(),
            access_label(max_access),
            lowered.join(", ")
        ))
    };
    render(&ProposeOut {
        status: "waiting for the person to press Run",
        jobs: count,
        note,
    })
}

fn proposed_job(raw: &Value, index: usize) -> Result<ProposedJob, ToolError> {
    let where_at = format!("jobs[{index}]");
    let Some(item) = raw.as_object() else {
        return Err(ToolError::new(format!("{where_at} must be an object")));
    };
    for key in item.keys() {
        if !JOB_FIELDS.contains(&key.as_str()) {
            return Err(ToolError::new(format!(
                "{where_at}: unexpected argument: {key}"
            )));
        }
    }
    let title = match item.get("title").and_then(Value::as_str) {
        Some(title) if !title.trim().is_empty() => js_prefix(title.trim(), MAX_TITLE),
        _ => {
            return Err(ToolError::new(format!(
                "{where_at}.title must be a non-empty string"
            )));
        }
    };
    let Some(job_type) = item
        .get("type")
        .and_then(Value::as_str)
        .and_then(JobType::parse)
    else {
        return Err(ToolError::new(format!(
            "{where_at}.type must be planning, debugging, review, refactor, or boilerplate"
        )));
    };
    if let Some(access) = item.get("access")
        && access.as_str().and_then(JobAccess::parse).is_none()
    {
        return Err(ToolError::new(format!(
            "{where_at}.access must be read, edit, or full"
        )));
    }
    if let Some(worker) = item.get("worker")
        && worker.as_str().and_then(lead_worker).is_none()
    {
        return Err(ToolError::new(format!(
            "{where_at}.worker must be claude, cursor, or grok"
        )));
    }
    let Some(prompt) = item.get("prompt").and_then(Value::as_str) else {
        return Err(ToolError::new(format!(
            "{where_at}.prompt must be a non-empty string"
        )));
    };
    if prompt.trim().is_empty() {
        return Err(ToolError::new(format!(
            "{where_at}.prompt must be a non-empty string"
        )));
    }
    if js_len(prompt) > MAX_PROMPT {
        return Err(ToolError::new(format!(
            "{where_at}.prompt must be at most 20000 characters"
        )));
    }
    let access = item
        .get("access")
        .and_then(Value::as_str)
        .and_then(JobAccess::parse)
        .unwrap_or(JobAccess::Read);
    Ok(ProposedJob {
        title,
        job_type,
        access,
        worker: item
            .get("worker")
            .and_then(Value::as_str)
            .and_then(lead_worker),
        prompt: prompt.to_string(),
    })
}

fn worker_json(worker: WorkerSnapshot) -> Result<WorkerOut, ToolError> {
    Ok(WorkerOut {
        provider: worker.id,
        available: worker.available,
        headroom: rounded_number(worker.headroom),
        resting_until: worker.resting_until.map(iso_millis).transpose()?,
        busy: worker.busy,
        queued: worker.queued,
    })
}

fn status_view(job: &JobRecord) -> StatusView<'_> {
    StatusView {
        id: &job.id,
        job_type: job.job_type,
        provider: job.provider,
        status: job.status,
        failed_over: &job.failed_over,
        prompt: js_prefix(&job.prompt, STATUS_PROMPT_UNITS),
    }
}

fn result_view(job: &JobRecord) -> ResultView<'_> {
    ResultView {
        id: &job.id,
        status: job.status,
        provider: job.provider,
        failed_over: &job.failed_over,
        output: js_suffix(&job.output, MAX_OUTPUT),
        truncated: js_len(&job.output) > MAX_OUTPUT,
        edit: job.edit == Some(true),
        change: job.change.as_deref(),
        error: job.error.as_deref(),
    }
}

fn requested_access(args: &Map<String, Value>) -> JobAccess {
    if let Some(access) = args
        .get("access")
        .and_then(Value::as_str)
        .and_then(JobAccess::parse)
    {
        return access;
    }
    if args.get("edit") == Some(&Value::Bool(true)) {
        JobAccess::Edit
    } else {
        JobAccess::Read
    }
}

fn wait_duration(value: Option<&Value>) -> Duration {
    let Some(Value::Number(number)) = value else {
        return Duration::from_secs_f64(DEFAULT_WAIT_SECONDS);
    };
    let Some(seconds) = number.as_f64() else {
        return Duration::from_secs_f64(DEFAULT_WAIT_SECONDS);
    };
    if !seconds.is_finite() {
        return Duration::from_secs_f64(DEFAULT_WAIT_SECONDS);
    }
    Duration::from_secs_f64(seconds.clamp(0.0, MAX_WAIT_SECONDS))
}

fn is_terminal(status: JobStatus) -> bool {
    status == JobStatus::Done || status == JobStatus::Failed
}

fn access_rank(access: JobAccess) -> u8 {
    match access {
        JobAccess::Read => 0,
        JobAccess::Edit => 1,
        JobAccess::Full => 2,
    }
}

fn access_label(access: JobAccess) -> &'static str {
    match access {
        JobAccess::Read => "read only",
        JobAccess::Edit => "edit files",
        JobAccess::Full => "do anything",
    }
}

/// `local` is a provider in the Rust core, but the lead may only name these three.
fn lead_worker(value: &str) -> Option<ProviderId> {
    match value {
        "claude" => Some(ProviderId::Claude),
        "cursor" => Some(ProviderId::Cursor),
        "grok" => Some(ProviderId::Grok),
        _ => None,
    }
}

fn reject_unknown(args: &Map<String, Value>, allowed: &[&str]) -> Result<(), ToolError> {
    for key in args.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(ToolError::new(format!("unexpected argument: {key}")));
        }
    }
    Ok(())
}

fn render(value: &impl Serialize) -> Result<Value, ToolError> {
    let text = serde_json::to_string(value).map_err(|err| ToolError::new(err.to_string()))?;
    Ok(Value::String(text))
}

/// `Math.round` (half towards +∞), then two decimal places, then JavaScript's
/// number text (`1`, not `1.0`).
fn rounded_number(value: f64) -> Value {
    if !value.is_finite() {
        return Value::Null;
    }
    let cents_f = js_round(value * 100.0);
    if !cents_f.is_finite() || !(i64::MIN as f64..=i64::MAX as f64).contains(&cents_f) {
        return Value::Null;
    }
    let cents = cents_f as i64;
    if cents.rem_euclid(100) == 0 {
        return Value::Number((cents / 100).into());
    }
    let negative = cents < 0;
    let cents = cents.unsigned_abs();
    let whole = cents / 100;
    let frac = cents % 100;
    let text = if frac.is_multiple_of(10) {
        format!("{}{whole}.{}", sign(negative), frac / 10)
    } else {
        format!("{}{whole}.{frac:02}", sign(negative))
    };
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

fn sign(negative: bool) -> &'static str {
    if negative { "-" } else { "" }
}

fn js_round(value: f64) -> f64 {
    if !value.is_finite() {
        return value;
    }
    let floor = value.floor();
    let diff = value - floor;
    if diff >= 0.5 { floor + 1.0 } else { floor }
}

fn iso_millis(ms: i64) -> Result<String, ToolError> {
    let nanos = i128::from(ms)
        .checked_mul(1_000_000)
        .ok_or_else(|| ToolError::new("resting_until is out of range"))?;
    let dt = time::OffsetDateTime::from_unix_timestamp_nanos(nanos)
        .map_err(|_| ToolError::new("resting_until is out of range"))?;
    dt.format(ISO_MS)
        .map_err(|_| ToolError::new("resting_until is out of range"))
}

fn js_len(text: &str) -> usize {
    text.encode_utf16().count()
}

fn js_prefix(text: &str, units: usize) -> String {
    if units == 0 {
        return String::new();
    }
    let mut taken = 0usize;
    for (index, ch) in text.char_indices() {
        let width = ch.len_utf16();
        if taken + width > units {
            return text[..index].to_string();
        }
        taken += width;
        if taken == units {
            let end = index + ch.len_utf8();
            return text[..end].to_string();
        }
    }
    text.to_string()
}

fn js_suffix(text: &str, units: usize) -> String {
    let total = js_len(text);
    if total <= units {
        return text.to_string();
    }
    let skip = total - units;
    let mut seen = 0usize;
    for (index, ch) in text.char_indices() {
        if seen >= skip {
            return text[index..].to_string();
        }
        let width = ch.len_utf16();
        if seen + width > skip {
            let rest = index + ch.len_utf8();
            return text[rest..].to_string();
        }
        seen += width;
    }
    String::new()
}

#[derive(Serialize)]
struct WorkersOut {
    max_access: JobAccess,
    workers: Vec<WorkerOut>,
}

#[derive(Serialize)]
struct WorkerOut {
    provider: ProviderId,
    available: bool,
    headroom: Value,
    resting_until: Option<String>,
    busy: bool,
    queued: u64,
}

#[derive(Serialize)]
struct SendOut<'a> {
    id: &'a str,
    provider: Option<ProviderId>,
    status: JobStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

#[derive(Serialize)]
struct StatusView<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    job_type: JobType,
    provider: Option<ProviderId>,
    status: JobStatus,
    failed_over: &'a [ProviderId],
    prompt: String,
}

#[derive(Serialize)]
struct ResultView<'a> {
    id: &'a str,
    status: JobStatus,
    provider: Option<ProviderId>,
    failed_over: &'a [ProviderId],
    output: String,
    truncated: bool,
    edit: bool,
    change: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
}

#[derive(Serialize)]
struct ProposeOut {
    status: &'static str,
    jobs: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{
        GET_RESULT_SCHEMA, GET_STATUS_SCHEMA, LIST_WORKERS_SCHEMA, PROPOSE_JOBS_SCHEMA,
        SEND_JOB_SCHEMA, iso_millis, rounded_number,
    };
    use serde_json::json;

    #[test]
    fn schemas_match_the_typescript_shapes() {
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(LIST_WORKERS_SCHEMA).unwrap(),
            json!({"type": "object", "properties": {}, "additionalProperties": false})
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(SEND_JOB_SCHEMA).unwrap(),
            json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["type", "prompt"],
                "properties": {
                    "type": {"type": "string", "enum": ["planning", "debugging", "review", "refactor", "boilerplate"]},
                    "prompt": {"type": "string", "minLength": 1, "maxLength": 20000},
                    "edit": {"type": "boolean"},
                    "access": {"type": "string", "enum": ["read", "edit", "full"]}
                }
            })
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(GET_STATUS_SCHEMA).unwrap(),
            json!({"type": "object", "additionalProperties": false, "properties": {"id": {"type": "string"}}})
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(GET_RESULT_SCHEMA).unwrap(),
            json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["id"],
                "properties": {
                    "id": {"type": "string"},
                    "wait_seconds": {"type": "number", "minimum": 0, "maximum": 300}
                }
            })
        );
        let propose: serde_json::Value = serde_json::from_str(PROPOSE_JOBS_SCHEMA).unwrap();
        assert_eq!(propose["required"], json!(["summary", "jobs"]));
        assert_eq!(propose["properties"]["jobs"]["maxItems"], json!(4));
        assert_eq!(
            propose["properties"]["jobs"]["items"]["properties"]["worker"]["enum"],
            json!(["claude", "cursor", "grok"])
        );
    }

    #[test]
    fn round2_and_iso_match_javascript() {
        assert_eq!(serde_json::to_string(&rounded_number(1.0)).unwrap(), "1");
        assert_eq!(serde_json::to_string(&rounded_number(0.0)).unwrap(), "0");
        assert_eq!(serde_json::to_string(&rounded_number(0.5)).unwrap(), "0.5");
        assert_eq!(
            serde_json::to_string(&rounded_number(1.25)).unwrap(),
            "1.25"
        );
        assert_eq!(
            iso_millis(1_700_000_060_000).unwrap(),
            "2023-11-14T22:14:20.000Z"
        );
    }
}
