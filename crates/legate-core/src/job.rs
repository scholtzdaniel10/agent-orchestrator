//! A saved job. The TypeScript type lives on the orchestrator; the store persists it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{FileEdit, JobAccess, JobType, ProviderId, StepTool};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Queued,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStepStatus {
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStep {
    pub id: String,
    pub tool: StepTool,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub status: JobStepStatus,
    pub started_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit: Option<FileEdit>,
}

/// One job as the database stores it. Columns overlay `owner_id`, `heartbeat_at`, and cancel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobRecord {
    pub id: String,
    #[serde(rename = "type")]
    pub job_type: JobType,
    pub prompt: String,
    #[serde(default)]
    pub provider: Option<ProviderId>,
    pub status: JobStatus,
    #[serde(default)]
    pub output: String,
    #[serde(default)]
    pub failed_over: Vec<ProviderId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<Vec<JobStep>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps_dropped: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<JobAccess>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heartbeat_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_requested: Option<bool>,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}
