//! Activity log diffs. Port of `flow-events.ts`.
//!
//! The first snapshot has nothing to compare with, so [`observe_flow`] yields no events.
//! [`FlowEvent`] matches the TypeScript shape, including `id`. Diffs themselves are
//! [`FlowChange`] (a `FlowEvent` before an id is assigned).

use std::collections::HashSet;

use legate_core::{
    ChangeSet, JobRecord, JobStatus, JobStep, JobStepStatus, LeadMessage, LeadRole, LeadStatus,
    PlanStatus, ProviderId, StepTool, TerminalInfo, TerminalProvider, TerminalStatus,
};

const STEP_LOG_CAP: usize = 40;

/// Who an activity line is about: the lead, the router, or one plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlowActor {
    Lead,
    Router,
    Plan(ProviderId),
}

impl FlowActor {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lead => "lead",
            Self::Router => "router",
            Self::Plan(ProviderId::Claude) => "claude",
            Self::Plan(ProviderId::Codex) => "codex",
            Self::Plan(ProviderId::Cursor) => "cursor",
            Self::Plan(ProviderId::Grok) => "grok",
            Self::Plan(ProviderId::Local) => "local",
        }
    }
}

impl std::fmt::Display for FlowActor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// `ok`, `bad`, or `warn` on an activity line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowTone {
    Ok,
    Bad,
    Warn,
}

impl FlowTone {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Bad => "bad",
            Self::Warn => "warn",
        }
    }
}

/// One activity-log row, matching `FlowEvent` in `flow-events.ts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowEvent {
    pub id: i64,
    pub at: i64,
    pub actor: FlowActor,
    pub text: String,
    pub result: Option<String>,
    pub tone: Option<FlowTone>,
}

/// A [`FlowEvent`] before the publisher assigns `id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowChange {
    pub at: i64,
    pub actor: FlowActor,
    pub text: String,
    pub result: Option<String>,
    pub tone: Option<FlowTone>,
}

impl FlowChange {
    pub fn with_id(self, id: i64) -> FlowEvent {
        FlowEvent {
            id,
            at: self.at,
            actor: self.actor,
            text: self.text,
            result: self.result,
            tone: self.tone,
        }
    }
}

/// Inputs [`diff_flow`] compares.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowSnapshot {
    pub jobs: Vec<JobRecord>,
    pub plans: Vec<PlanStatus>,
    pub lead: Vec<LeadMessage>,
    pub terminals: Vec<TerminalInfo>,
    pub changes: Vec<ChangeSet>,
}

/// The first snapshot has nothing to compare with, so it yields no events.
pub fn observe_flow(prev: Option<&FlowSnapshot>, next: &FlowSnapshot, now: i64) -> Vec<FlowChange> {
    match prev {
        None => Vec::new(),
        Some(prev) => diff_flow(prev, next, now),
    }
}

pub fn diff_flow(prev: &FlowSnapshot, next: &FlowSnapshot, now: i64) -> Vec<FlowChange> {
    let mut events = Vec::new();
    events.extend(diff_lead(&prev.lead, &next.lead, now));
    events.extend(diff_router(&prev.jobs, &next.jobs, now));
    events.extend(diff_changes(prev, next, now));
    events.extend(diff_workers(prev, next, now));
    events.extend(diff_terminals(&prev.terminals, &next.terminals, now));
    events
}

/// Local `HH:MM` for an epoch millisecond, matching `Date#getHours`.
pub fn format_hours_minutes(ms: i64) -> String {
    let (hours, minutes, _) = local_hms(ms);
    format!("{hours:02}:{minutes:02}")
}

/// Local `HH:MM:SS` for the activity log.
pub fn format_log_time(ms: i64) -> String {
    let (hours, minutes, seconds) = local_hms(ms);
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

fn local_hms(ms: i64) -> (u8, u8, u8) {
    #[cfg(target_arch = "wasm32")]
    {
        let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms as f64));
        (
            date.get_hours() as u8,
            date.get_minutes() as u8,
            date.get_seconds() as u8,
        )
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::OnceLock;

        use time::{OffsetDateTime, UtcOffset};
        static OFFSET: OnceLock<UtcOffset> = OnceLock::new();
        let offset =
            *OFFSET.get_or_init(|| UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC));
        let Ok(ts) = OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000) else {
            return (0, 0, 0);
        };
        let local = ts.to_offset(offset);
        (local.hour(), local.minute(), local.second())
    }
}

fn change(
    now: i64,
    actor: FlowActor,
    text: impl Into<String>,
    tone: Option<FlowTone>,
    result: Option<String>,
) -> FlowChange {
    FlowChange {
        at: now,
        actor,
        text: text.into(),
        result,
        tone,
    }
}

fn diff_lead(prev: &[LeadMessage], next: &[LeadMessage], now: i64) -> Vec<FlowChange> {
    let mut events = Vec::new();
    for message in next {
        if message.role != LeadRole::Lead {
            continue;
        }
        let before = prev.iter().find(|item| item.id == message.id);
        let Some(before) = before else {
            events.push(change(now, FlowActor::Lead, "turn started", None, None));
            continue;
        };
        if before.status != LeadStatus::Streaming {
            continue;
        }
        if message.status == LeadStatus::Done {
            events.push(change(
                now,
                FlowActor::Lead,
                "turn finished",
                Some(FlowTone::Ok),
                Some("ok".into()),
            ));
        } else if message.status == LeadStatus::Error {
            events.push(change(
                now,
                FlowActor::Lead,
                "turn failed",
                Some(FlowTone::Bad),
                None,
            ));
        }
    }
    events
}

fn diff_router(prev: &[JobRecord], next: &[JobRecord], now: i64) -> Vec<FlowChange> {
    let mut events = Vec::new();
    for job in next {
        let before = prev.iter().find(|item| item.id == job.id);
        if before.is_none() {
            events.push(change(now, FlowActor::Router, route_text(job), None, None));
        }
        let Some(before) = before else {
            continue;
        };
        if job.failed_over.len() <= before.failed_over.len() {
            continue;
        }
        let added = &job.failed_over[before.failed_over.len()..];
        for (index, from) in added.iter().enumerate() {
            let to = if index + 1 < added.len() {
                added[index + 1].as_str().to_string()
            } else {
                job.provider
                    .map(|id| id.as_str().to_string())
                    .unwrap_or_else(|| "no plan".into())
            };
            events.push(change(
                now,
                FlowActor::Router,
                format!(
                    "{} failover {} → {to}",
                    job.job_type.as_str(),
                    from.as_str()
                ),
                Some(FlowTone::Warn),
                None,
            ));
        }
    }
    events
}

fn route_text(job: &JobRecord) -> String {
    let target = job
        .provider
        .map(|id| id.as_str().to_string())
        .unwrap_or_else(|| "no plan".into());
    let mut text = format!("{} → {target}", job.job_type.as_str());
    if let Some(reason) = job.reason.as_deref().filter(|reason| !reason.is_empty()) {
        text.push_str(" · ");
        text.push_str(reason);
    }
    if job.edit == Some(true) {
        text.push_str(" · edits");
    }
    text
}

fn file_count(count: usize) -> String {
    format!("{count} {}", if count == 1 { "file" } else { "files" })
}

fn change_actor(jobs: &[JobRecord], id: &str) -> FlowActor {
    let Some(job) = jobs.iter().find(|job| job.change.as_deref() == Some(id)) else {
        return FlowActor::Router;
    };
    job.provider
        .map(FlowActor::Plan)
        .unwrap_or(FlowActor::Router)
}

fn diff_changes(prev: &FlowSnapshot, next: &FlowSnapshot, now: i64) -> Vec<FlowChange> {
    let prev_ids: HashSet<&str> = prev.changes.iter().map(|item| item.id.as_str()).collect();
    let next_ids: HashSet<&str> = next.changes.iter().map(|item| item.id.as_str()).collect();
    let mut events = Vec::new();
    for item in &next.changes {
        if prev_ids.contains(item.id.as_str()) {
            continue;
        }
        events.push(change(
            now,
            change_actor(&next.jobs, &item.id),
            format!("change ready ({})", file_count(item.files.len())),
            Some(FlowTone::Ok),
            None,
        ));
    }
    for item in &prev.changes {
        if next_ids.contains(item.id.as_str()) {
            continue;
        }
        events.push(change(
            now,
            FlowActor::Router,
            format!("change closed ({})", item.id),
            None,
            None,
        ));
    }
    events
}

fn plan_order(prev: &FlowSnapshot, next: &FlowSnapshot) -> Vec<ProviderId> {
    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    let mut add = |id: ProviderId| {
        if seen.insert(id) {
            ids.push(id);
        }
    };
    for plan in &next.plans {
        add(plan.id);
    }
    for plan in &prev.plans {
        add(plan.id);
    }
    for job in &next.jobs {
        if let Some(id) = job.provider {
            add(id);
        }
    }
    for job in &prev.jobs {
        if let Some(id) = job.provider {
            add(id);
        }
    }
    ids
}

fn is_resting(plan: &PlanStatus, now: i64) -> bool {
    plan.resting_until.is_some_and(|until| until > now)
}

fn diff_workers(prev: &FlowSnapshot, next: &FlowSnapshot, now: i64) -> Vec<FlowChange> {
    let mut events = Vec::new();
    for plan_id in plan_order(prev, next) {
        for job in &next.jobs {
            if job.provider != Some(plan_id) {
                continue;
            }
            let before = prev.jobs.iter().find(|item| item.id == job.id);
            let actor = FlowActor::Plan(plan_id);
            if job.status == JobStatus::Running
                && before.is_none_or(|item| item.status != JobStatus::Running)
            {
                events.push(change(
                    now,
                    actor,
                    format!("{} running", job.job_type.as_str()),
                    None,
                    None,
                ));
            }
            events.extend(diff_job_steps(before, job, now));
            if job.status == JobStatus::Done
                && before.is_none_or(|item| item.status != JobStatus::Done)
            {
                events.push(change(
                    now,
                    actor,
                    format!("{} done", job.job_type.as_str()),
                    Some(FlowTone::Ok),
                    Some("ok".into()),
                ));
            }
            if job.status == JobStatus::Failed
                && before.is_none_or(|item| item.status != JobStatus::Failed)
            {
                let result = job
                    .error
                    .as_deref()
                    .unwrap_or("")
                    .chars()
                    .take(40)
                    .collect::<String>();
                events.push(change(
                    now,
                    actor,
                    format!("{} failed", job.job_type.as_str()),
                    Some(FlowTone::Bad),
                    Some(result),
                ));
            }
        }

        let before_plan = prev.plans.iter().find(|plan| plan.id == plan_id);
        let after_plan = next.plans.iter().find(|plan| plan.id == plan_id);
        let (Some(before_plan), Some(after_plan)) = (before_plan, after_plan) else {
            continue;
        };
        let was_resting = is_resting(before_plan, now);
        let now_resting = is_resting(after_plan, now);
        let actor = FlowActor::Plan(plan_id);
        if !was_resting && now_resting && after_plan.resting_until.is_some() {
            let until = after_plan.resting_until.unwrap_or(now);
            events.push(change(
                now,
                actor,
                format!("resting until {}", format_hours_minutes(until)),
                Some(FlowTone::Warn),
                None,
            ));
        } else if was_resting && !now_resting {
            events.push(change(now, actor, "back", Some(FlowTone::Ok), None));
        }
        if before_plan.model != after_plan.model {
            let model = after_plan.model.as_deref().unwrap_or("CLI default");
            events.push(change(now, actor, format!("model → {model}"), None, None));
        }
    }
    events
}

fn eligible_steps(job: &JobRecord) -> Vec<&JobStep> {
    job.steps
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .filter(|step| {
            step.status != JobStepStatus::Running
                && step.tool != StepTool::Say
                && step.tool != StepTool::Think
        })
        .collect()
}

fn step_log_text(step: &JobStep) -> String {
    if let Some(edit) = &step.edit {
        return format!("edited {} +{} −{}", edit.path, edit.added, edit.removed);
    }
    if step.title.is_empty() {
        return String::new();
    }
    let mut chars = step.title.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut body: String = first.to_lowercase().collect();
    body.push_str(chars.as_str());
    body
}

fn diff_job_steps(before: Option<&JobRecord>, job: &JobRecord, now: i64) -> Vec<FlowChange> {
    let Some(provider) = job.provider else {
        return Vec::new();
    };
    let prev_n = before
        .map(|item| eligible_steps(item).len())
        .unwrap_or(0)
        .min(STEP_LOG_CAP);
    let next = eligible_steps(job);
    let next_n = next.len().min(STEP_LOG_CAP);
    let actor = FlowActor::Plan(provider);
    next.iter()
        .take(next_n)
        .skip(prev_n)
        .map(|step| change(now, actor, step_log_text(step), None, None))
        .collect()
}

fn terminal_actor(provider: TerminalProvider) -> FlowActor {
    match provider {
        TerminalProvider::Shell => FlowActor::Router,
        TerminalProvider::Claude => FlowActor::Plan(ProviderId::Claude),
        TerminalProvider::Codex => FlowActor::Plan(ProviderId::Codex),
        TerminalProvider::Cursor => FlowActor::Plan(ProviderId::Cursor),
        TerminalProvider::Grok => FlowActor::Plan(ProviderId::Grok),
        TerminalProvider::Local => FlowActor::Plan(ProviderId::Local),
    }
}

fn diff_terminals(prev: &[TerminalInfo], next: &[TerminalInfo], now: i64) -> Vec<FlowChange> {
    let mut events = Vec::new();
    for terminal in next {
        let before = prev.iter().find(|item| item.id == terminal.id);
        let actor = terminal_actor(terminal.provider);
        if before.is_none() {
            events.push(change(
                now,
                actor,
                format!("terminal opened ({})", terminal.title),
                None,
                None,
            ));
        }
        let ended = terminal.status == TerminalStatus::Exited
            && before.is_none_or(|item| item.status != TerminalStatus::Exited);
        if ended {
            let code = terminal.exit_code;
            let tone = if code == Some(0) {
                FlowTone::Ok
            } else {
                FlowTone::Bad
            };
            let result = match code {
                Some(value) => format!("code {value}"),
                None => "code null".into(),
            };
            events.push(change(
                now,
                actor,
                format!("terminal ended ({})", terminal.title),
                Some(tone),
                Some(result),
            ));
        }
    }
    for terminal in prev {
        if next.iter().any(|item| item.id == terminal.id) {
            continue;
        }
        events.push(change(
            now,
            terminal_actor(terminal.provider),
            format!("terminal closed ({})", terminal.title),
            None,
            None,
        ));
    }
    events
}

#[cfg(test)]
mod tests {
    use super::{
        FlowActor, FlowChange, FlowSnapshot, FlowTone, diff_flow, format_hours_minutes,
        observe_flow,
    };
    use legate_core::{
        ChangeSet, ChangedFile, FileEdit, JobRecord, JobStatus, JobStep, JobStepStatus, JobType,
        LeadMessage, LeadRole, LeadStatus, PlanStatus, ProviderId, StepTool, TerminalInfo,
        TerminalProvider, TerminalStatus, VersionStatus,
    };
    use std::collections::BTreeMap;

    const NOW: i64 = 1_790_928_000_000;

    fn plan(id: ProviderId) -> PlanStatus {
        PlanStatus {
            id,
            available: true,
            problem: None,
            used: 0.2,
            resting_until: None,
            resets_at: None,
            at_risk: false,
            busy: false,
            running: 0,
            queued: 0,
            model: None,
            windows: Vec::new(),
            usage_at: None,
            version: None,
            version_status: VersionStatus::Unknown,
            tested_version: "2.1.292".into(),
        }
    }

    fn job(id: &str, status: JobStatus) -> JobRecord {
        JobRecord {
            id: id.into(),
            job_type: JobType::Planning,
            prompt: "prompt".into(),
            provider: Some(ProviderId::Claude),
            status,
            output: String::new(),
            failed_over: Vec::new(),
            steps: None,
            steps_dropped: None,
            model: None,
            reason: None,
            edit: None,
            access: None,
            change: None,
            group: None,
            lead_message: None,
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

    fn lead(id: &str, role: LeadRole, status: LeadStatus) -> LeadMessage {
        LeadMessage {
            id: id.into(),
            role,
            text: String::new(),
            provider: None,
            model: None,
            status,
            kind: None,
            proposal: None,
            extra: BTreeMap::new(),
        }
    }

    fn terminal(id: &str) -> TerminalInfo {
        TerminalInfo {
            id: id.into(),
            provider: TerminalProvider::Claude,
            title: "claude 1".into(),
            status: TerminalStatus::Running,
            exit_code: None,
            model: None,
            started_at: NOW,
            change: None,
        }
    }

    fn change_set(id: &str) -> ChangeSet {
        ChangeSet {
            id: id.into(),
            branch: format!("orch/{id}"),
            path: id.into(),
            files: vec![ChangedFile {
                path: "notes.txt".into(),
                insertions: Some(1),
                deletions: Some(0),
            }],
            insertions: 1,
            deletions: 0,
            name: None,
            setup: None,
        }
    }

    fn snap() -> FlowSnapshot {
        FlowSnapshot {
            jobs: Vec::new(),
            plans: Vec::new(),
            lead: Vec::new(),
            terminals: Vec::new(),
            changes: Vec::new(),
        }
    }

    fn line(actor: FlowActor, text: &str) -> FlowChange {
        FlowChange {
            at: NOW,
            actor,
            text: text.into(),
            result: None,
            tone: None,
        }
    }

    fn marked(actor: FlowActor, text: &str, tone: FlowTone, result: Option<&str>) -> FlowChange {
        FlowChange {
            at: NOW,
            actor,
            text: text.into(),
            result: result.map(str::to_string),
            tone: Some(tone),
        }
    }

    fn step(id: &str, tool: StepTool, title: &str, status: JobStepStatus) -> JobStep {
        JobStep {
            id: id.into(),
            tool,
            title: title.into(),
            detail: None,
            status,
            started_at: NOW,
            ended_at: Some(NOW + 10),
            edit: None,
        }
    }

    #[test]
    fn a_new_lead_message_starts_a_turn_and_a_user_message_does_not() {
        let mut next = snap();
        next.lead = vec![
            {
                let mut message = lead("u", LeadRole::User, LeadStatus::Done);
                message.text = "hello".into();
                message
            },
            lead("a", LeadRole::Lead, LeadStatus::Streaming),
        ];
        assert_eq!(
            diff_flow(&snap(), &next, NOW),
            vec![line(FlowActor::Lead, "turn started")]
        );
    }

    #[test]
    fn a_lead_turn_finishing_or_failing_is_recorded_once() {
        let streaming = {
            let mut message = lead("a", LeadRole::Lead, LeadStatus::Streaming);
            message.text = "partial".into();
            message
        };
        let done = {
            let mut message = streaming.clone();
            message.status = LeadStatus::Done;
            message.text = "full".into();
            message
        };
        let failed = {
            let mut message = streaming.clone();
            message.status = LeadStatus::Error;
            message.text = "nope".into();
            message
        };
        let still = {
            let mut message = streaming.clone();
            message.text = "more".into();
            message
        };
        let mut prev = snap();
        prev.lead = vec![streaming];
        let mut next_done = snap();
        next_done.lead = vec![done];
        let mut next_failed = snap();
        next_failed.lead = vec![failed];
        let mut next_still = snap();
        next_still.lead = vec![still];
        assert_eq!(
            diff_flow(&prev, &next_done, NOW),
            vec![marked(
                FlowActor::Lead,
                "turn finished",
                FlowTone::Ok,
                Some("ok")
            )]
        );
        assert_eq!(
            diff_flow(&prev, &next_failed, NOW),
            vec![marked(FlowActor::Lead, "turn failed", FlowTone::Bad, None)]
        );
        assert_eq!(diff_flow(&prev, &next_still, NOW), Vec::<FlowChange>::new());
    }

    #[test]
    fn a_new_job_with_a_reason_appends_it_after_the_plan() {
        let mut cursor = job("j1", JobStatus::Queued);
        cursor.provider = Some(ProviderId::Cursor);
        cursor.job_type = JobType::Review;
        cursor.reason = Some("allowance expiring".into());
        let mut unassigned = job("j2", JobStatus::Queued);
        unassigned.provider = None;
        unassigned.job_type = JobType::Refactor;
        unassigned.reason = Some("chosen".into());
        let mut next = snap();
        next.jobs = vec![cursor, unassigned];
        assert_eq!(
            diff_flow(&snap(), &next, NOW),
            vec![
                line(FlowActor::Router, "review → cursor · allowance expiring"),
                line(FlowActor::Router, "refactor → no plan · chosen"),
            ]
        );
    }

    #[test]
    fn a_new_running_job_keeps_the_reason_on_the_router_line_only() {
        let mut running = job("j", JobStatus::Running);
        running.job_type = JobType::Boilerplate;
        running.reason = Some("first choice".into());
        let mut next = snap();
        next.jobs = vec![running];
        next.plans = vec![plan(ProviderId::Claude)];
        assert_eq!(
            diff_flow(&snap(), &next, NOW),
            vec![
                line(FlowActor::Router, "boilerplate → claude · first choice"),
                line(FlowActor::Plan(ProviderId::Claude), "boilerplate running"),
            ]
        );
    }

    #[test]
    fn a_new_job_names_its_plan_or_no_plan() {
        let mut cursor = job("j1", JobStatus::Queued);
        cursor.provider = Some(ProviderId::Cursor);
        cursor.job_type = JobType::Review;
        let mut unassigned = job("j2", JobStatus::Queued);
        unassigned.provider = None;
        unassigned.job_type = JobType::Refactor;
        let mut next = snap();
        next.jobs = vec![cursor, unassigned];
        assert_eq!(
            diff_flow(&snap(), &next, NOW),
            vec![
                line(FlowActor::Router, "review → cursor"),
                line(FlowActor::Router, "refactor → no plan"),
            ]
        );
    }

    #[test]
    fn a_job_becoming_running_is_attributed_to_its_provider() {
        let mut queued = job("j", JobStatus::Queued);
        queued.job_type = JobType::Debugging;
        let mut running = queued.clone();
        running.status = JobStatus::Running;
        let mut prev = snap();
        prev.jobs = vec![queued];
        let mut next = snap();
        next.jobs = vec![running];
        assert_eq!(
            diff_flow(&prev, &next, NOW),
            vec![line(
                FlowActor::Plan(ProviderId::Claude),
                "debugging running"
            )]
        );
    }

    #[test]
    fn a_job_becoming_done_or_failed_records_the_result() {
        let mut running = job("j", JobStatus::Running);
        running.provider = Some(ProviderId::Cursor);
        running.job_type = JobType::Boilerplate;
        let mut done = running.clone();
        done.status = JobStatus::Done;
        let error = "x".repeat(50);
        let mut failed = running.clone();
        failed.status = JobStatus::Failed;
        failed.error = Some(error.clone());
        let mut prev = snap();
        prev.jobs = vec![running];
        let mut next_done = snap();
        next_done.jobs = vec![done];
        let mut next_failed = snap();
        next_failed.jobs = vec![failed];
        assert_eq!(
            diff_flow(&prev, &next_done, NOW),
            vec![marked(
                FlowActor::Plan(ProviderId::Cursor),
                "boilerplate done",
                FlowTone::Ok,
                Some("ok")
            )]
        );
        assert_eq!(
            diff_flow(&prev, &next_failed, NOW),
            vec![marked(
                FlowActor::Plan(ProviderId::Cursor),
                "boilerplate failed",
                FlowTone::Bad,
                Some(&error[..40])
            )]
        );
    }

    #[test]
    fn a_job_whose_failed_over_grew_records_the_handoff() {
        let mut before = job("j", JobStatus::Running);
        before.job_type = JobType::Planning;
        let mut after = job("j", JobStatus::Queued);
        after.provider = Some(ProviderId::Cursor);
        after.failed_over = vec![ProviderId::Claude];
        let mut prev = snap();
        prev.jobs = vec![before];
        let mut next = snap();
        next.jobs = vec![after];
        assert_eq!(
            diff_flow(&prev, &next, NOW),
            vec![marked(
                FlowActor::Router,
                "planning failover claude → cursor",
                FlowTone::Warn,
                None
            )]
        );
    }

    #[test]
    fn a_plan_entering_or_leaving_rest_is_recorded() {
        let until = NOW + 90 * 60 * 1000;
        let awake = plan(ProviderId::Claude);
        let mut resting = awake.clone();
        resting.resting_until = Some(until);
        let mut prev_awake = snap();
        prev_awake.plans = vec![awake.clone()];
        let mut next_resting = snap();
        next_resting.plans = vec![resting.clone()];
        let mut prev_resting = snap();
        prev_resting.plans = vec![resting];
        let mut next_awake = snap();
        next_awake.plans = vec![awake];
        assert_eq!(
            diff_flow(&prev_awake, &next_resting, NOW),
            vec![marked(
                FlowActor::Plan(ProviderId::Claude),
                &format!("resting until {}", format_hours_minutes(until)),
                FlowTone::Warn,
                None
            )]
        );
        assert_eq!(
            diff_flow(&prev_resting, &next_awake, NOW),
            vec![marked(
                FlowActor::Plan(ProviderId::Claude),
                "back",
                FlowTone::Ok,
                None
            )]
        );
    }

    #[test]
    fn a_plan_model_change_names_the_new_model_or_the_cli_default() {
        let mut claude = plan(ProviderId::Claude);
        claude.model = Some("opus".into());
        let mut cursor = plan(ProviderId::Cursor);
        cursor.model = Some("fast".into());
        let mut next_claude = claude.clone();
        next_claude.model = Some("sonnet".into());
        let mut next_cursor = cursor.clone();
        next_cursor.model = None;
        let mut prev = snap();
        prev.plans = vec![claude, cursor];
        let mut next = snap();
        next.plans = vec![next_claude, next_cursor];
        assert_eq!(
            diff_flow(&prev, &next, NOW),
            vec![
                line(FlowActor::Plan(ProviderId::Claude), "model → sonnet"),
                line(FlowActor::Plan(ProviderId::Cursor), "model → CLI default"),
            ]
        );
    }

    #[test]
    fn terminals_opening_ending_and_disappearing_are_recorded() {
        let running = terminal("t1");
        let mut ended_ok = running.clone();
        ended_ok.status = TerminalStatus::Exited;
        ended_ok.exit_code = Some(0);
        let mut ended_bad = running.clone();
        ended_bad.status = TerminalStatus::Exited;
        ended_bad.exit_code = Some(7);
        let mut opened = snap();
        opened.terminals = vec![running.clone()];
        let mut prev = snap();
        prev.terminals = vec![running];
        let mut ok_next = snap();
        ok_next.terminals = vec![ended_ok];
        let mut bad_next = snap();
        bad_next.terminals = vec![ended_bad];
        assert_eq!(
            diff_flow(&snap(), &opened, NOW),
            vec![line(
                FlowActor::Plan(ProviderId::Claude),
                "terminal opened (claude 1)"
            )]
        );
        assert_eq!(
            diff_flow(&prev, &ok_next, NOW),
            vec![marked(
                FlowActor::Plan(ProviderId::Claude),
                "terminal ended (claude 1)",
                FlowTone::Ok,
                Some("code 0")
            )]
        );
        assert_eq!(
            diff_flow(&prev, &bad_next, NOW),
            vec![marked(
                FlowActor::Plan(ProviderId::Claude),
                "terminal ended (claude 1)",
                FlowTone::Bad,
                Some("code 7")
            )]
        );
        assert_eq!(
            diff_flow(&prev, &snap(), NOW),
            vec![line(
                FlowActor::Plan(ProviderId::Claude),
                "terminal closed (claude 1)"
            )]
        );
    }

    #[test]
    fn the_first_snapshot_yields_nothing() {
        let mut running = job("j", JobStatus::Running);
        running.edit = Some(true);
        running.change = Some("abcd1234".into());
        let mut done = lead("a", LeadRole::Lead, LeadStatus::Done);
        done.text = "hi".into();
        let mut resting = plan(ProviderId::Claude);
        resting.resting_until = Some(NOW + 60_000);
        resting.model = Some("opus".into());
        let first = FlowSnapshot {
            jobs: vec![running],
            plans: vec![resting],
            lead: vec![done],
            terminals: vec![terminal("t")],
            changes: vec![change_set("abcd1234")],
        };
        assert_eq!(observe_flow(None, &first, NOW), Vec::<FlowChange>::new());
    }

    #[test]
    fn an_unchanged_snapshot_yields_nothing() {
        let mut running = job("j", JobStatus::Running);
        running.output = "partial".into();
        let mut streaming = lead("a", LeadRole::Lead, LeadStatus::Streaming);
        streaming.text = "partial".into();
        streaming.model = Some("opus".into());
        let mut claude = plan(ProviderId::Claude);
        claude.model = Some("opus".into());
        claude.busy = true;
        let mut ended = terminal("t");
        ended.status = TerminalStatus::Exited;
        ended.exit_code = Some(0);
        let current = FlowSnapshot {
            jobs: vec![running],
            plans: vec![claude],
            lead: vec![streaming],
            terminals: vec![ended],
            changes: vec![change_set("abcd1234")],
        };
        let cloned = current.clone();
        assert_eq!(diff_flow(&current, &cloned, NOW), Vec::<FlowChange>::new());
        let mut edited = current.clone();
        edited.jobs[0].output = "more text".into();
        edited.lead[0].text = "more text".into();
        assert_eq!(diff_flow(&current, &edited, NOW), Vec::<FlowChange>::new());
    }

    #[test]
    fn several_changes_in_one_diff_stay_in_lead_router_plan_then_terminal_order() {
        let until = NOW + 30 * 60 * 1000;
        let streaming = lead("a", LeadRole::Lead, LeadStatus::Streaming);
        let mut debugging = job("j1", JobStatus::Running);
        debugging.job_type = JobType::Debugging;
        let mut refactor = job("j3", JobStatus::Running);
        refactor.job_type = JobType::Refactor;
        let mut claude = plan(ProviderId::Claude);
        claude.model = Some("old".into());
        let cursor = plan(ProviderId::Cursor);
        let mut prev = snap();
        prev.lead = vec![streaming.clone()];
        prev.jobs = vec![debugging.clone(), refactor.clone()];
        prev.plans = vec![claude.clone(), cursor.clone()];
        prev.terminals = vec![terminal("old")];

        let mut finished = streaming.clone();
        finished.status = LeadStatus::Done;
        finished.text = "done".into();
        let mut review = job("j2", JobStatus::Running);
        review.provider = Some(ProviderId::Cursor);
        review.job_type = JobType::Review;
        let mut handed = refactor.clone();
        handed.status = JobStatus::Queued;
        handed.provider = Some(ProviderId::Cursor);
        handed.failed_over = vec![ProviderId::Claude];
        let mut done_debug = debugging.clone();
        done_debug.status = JobStatus::Done;
        let mut resting = claude.clone();
        resting.resting_until = Some(until);
        let mut cursor_model = cursor.clone();
        cursor_model.model = Some("gpt".into());
        let mut opened = terminal("new");
        opened.provider = TerminalProvider::Cursor;
        opened.title = "cursor 1".into();
        let mut next = snap();
        next.lead = vec![finished, lead("b", LeadRole::Lead, LeadStatus::Streaming)];
        next.jobs = vec![done_debug, review, handed];
        next.plans = vec![resting, cursor_model];
        next.terminals = vec![opened];

        let lines: Vec<String> = diff_flow(&prev, &next, NOW)
            .into_iter()
            .map(|event| format!("{}: {}", event.actor, event.text))
            .collect();
        let expected = [
            "lead: turn finished",
            "lead: turn started",
            "router: review → cursor",
            "router: refactor failover claude → cursor",
            "claude: debugging done",
            "claude: resting until PLACE",
            "cursor: review running",
            "cursor: model → gpt",
            "cursor: terminal opened (cursor 1)",
            "claude: terminal closed (claude 1)",
        ]
        .map(str::to_string);
        let mut expected = expected.to_vec();
        expected[5] = format!("claude: resting until {}", format_hours_minutes(until));
        assert_eq!(lines, expected);
    }

    #[test]
    fn an_editing_job_ends_the_router_line_with_edits() {
        let mut running = job("j", JobStatus::Running);
        running.job_type = JobType::Boilerplate;
        running.edit = Some(true);
        running.reason = Some("first choice".into());
        let mut next = snap();
        next.jobs = vec![running];
        next.plans = vec![plan(ProviderId::Claude)];
        assert_eq!(
            diff_flow(&snap(), &next, NOW),
            vec![
                line(
                    FlowActor::Router,
                    "boilerplate → claude · first choice · edits"
                ),
                line(FlowActor::Plan(ProviderId::Claude), "boilerplate running"),
            ]
        );
    }

    #[test]
    fn an_editing_job_without_a_reason_still_ends_with_edits() {
        let mut queued = job("j", JobStatus::Queued);
        queued.provider = None;
        queued.job_type = JobType::Review;
        queued.edit = Some(true);
        let mut next = snap();
        next.jobs = vec![queued];
        assert_eq!(
            diff_flow(&snap(), &next, NOW),
            vec![line(FlowActor::Router, "review → no plan · edits")]
        );
    }

    #[test]
    fn a_change_ready_event_uses_the_provider_of_the_job_that_owns_it() {
        let item = change_set("abcd1234");
        let mut owned = job("j", JobStatus::Done);
        owned.provider = Some(ProviderId::Cursor);
        owned.change = Some("abcd1234".into());
        let mut prev = snap();
        prev.jobs = vec![owned.clone()];
        let mut next = snap();
        next.jobs = vec![owned];
        next.changes = vec![item];
        assert_eq!(
            diff_flow(&prev, &next, NOW),
            vec![marked(
                FlowActor::Plan(ProviderId::Cursor),
                "change ready (1 file)",
                FlowTone::Ok,
                None
            )]
        );
    }

    #[test]
    fn a_change_with_no_matching_job_or_no_provider_is_ready_on_the_router() {
        let mut lone = change_set("aaaaaaaa");
        lone.files = vec![
            ChangedFile {
                path: "a.txt".into(),
                insertions: Some(1),
                deletions: Some(0),
            },
            ChangedFile {
                path: "b.txt".into(),
                insertions: Some(2),
                deletions: Some(1),
            },
        ];
        let unassigned = change_set("bbbbbbbb");
        let mut owned = job("j", JobStatus::Done);
        owned.provider = None;
        owned.change = Some("bbbbbbbb".into());
        let mut next_lone = snap();
        next_lone.changes = vec![lone];
        let mut prev = snap();
        prev.jobs = vec![owned.clone()];
        let mut next = snap();
        next.jobs = vec![owned];
        next.changes = vec![unassigned];
        assert_eq!(
            diff_flow(&snap(), &next_lone, NOW),
            vec![marked(
                FlowActor::Router,
                "change ready (2 files)",
                FlowTone::Ok,
                None
            )]
        );
        assert_eq!(
            diff_flow(&prev, &next, NOW),
            vec![marked(
                FlowActor::Router,
                "change ready (1 file)",
                FlowTone::Ok,
                None
            )]
        );
    }

    #[test]
    fn a_change_that_disappears_is_closed_by_the_router() {
        let staying = change_set("aaaaaaaa");
        let leaving = change_set("bbbbbbbb");
        let mut arriving = change_set("cccccccc");
        arriving.files = vec![
            ChangedFile {
                path: "a.txt".into(),
                insertions: Some(1),
                deletions: Some(0),
            },
            ChangedFile {
                path: "b.txt".into(),
                insertions: Some(1),
                deletions: Some(0),
            },
            ChangedFile {
                path: "c.txt".into(),
                insertions: Some(1),
                deletions: Some(0),
            },
        ];
        let mut owned = job("j", JobStatus::Done);
        owned.change = Some("cccccccc".into());
        let mut prev = snap();
        prev.jobs = vec![owned.clone()];
        prev.changes = vec![staying.clone(), leaving];
        let mut next = snap();
        next.jobs = vec![owned];
        next.changes = vec![staying, arriving];
        let lines: Vec<String> = diff_flow(&prev, &next, NOW)
            .into_iter()
            .map(|event| format!("{}: {}", event.actor, event.text))
            .collect();
        assert_eq!(
            lines,
            vec![
                "claude: change ready (3 files)",
                "router: change closed (bbbbbbbb)",
            ]
        );
    }

    #[test]
    fn finished_steps_that_are_not_say_or_think_appear_in_the_activity_log() {
        let mut running = job("j", JobStatus::Running);
        running.provider = Some(ProviderId::Cursor);
        running.job_type = JobType::Boilerplate;
        let mut with_steps = running.clone();
        let mut edit = step(
            "s1",
            StepTool::Edit,
            "Edited src/app.ts",
            JobStepStatus::Done,
        );
        edit.ended_at = Some(NOW + 10);
        edit.edit = Some(FileEdit {
            path: "src/app.ts".into(),
            added: 4,
            removed: 1,
        });
        let say = step("s2", StepTool::Say, "hello", JobStepStatus::Done);
        let think = step("s3", StepTool::Think, "hmm", JobStepStatus::Done);
        let mut read = step("s4", StepTool::Read, "Read src/app.ts", JobStepStatus::Done);
        read.ended_at = Some(NOW + 20);
        let mut reading = step("s5", StepTool::Read, "Read more.ts", JobStepStatus::Running);
        reading.ended_at = None;
        with_steps.steps = Some(vec![edit, say, think, read, reading]);
        let mut prev = snap();
        prev.jobs = vec![running];
        let mut next = snap();
        next.jobs = vec![with_steps];
        assert_eq!(
            diff_flow(&prev, &next, NOW),
            vec![
                line(
                    FlowActor::Plan(ProviderId::Cursor),
                    "edited src/app.ts +4 −1"
                ),
                line(FlowActor::Plan(ProviderId::Cursor), "read src/app.ts"),
            ]
        );
    }

    #[test]
    fn at_most_40_step_lines_are_kept_per_job() {
        let steps: Vec<JobStep> = (0..45)
            .map(|index| {
                let mut item = step(
                    &format!("s{index}"),
                    StepTool::Read,
                    &format!("Read {index}"),
                    JobStepStatus::Done,
                );
                item.ended_at = Some(NOW + 1);
                item
            })
            .collect();
        let mut before = job("j", JobStatus::Running);
        before.steps = Some(steps[..10].to_vec());
        let mut after = job("j", JobStatus::Running);
        after.steps = Some(steps);
        let mut prev = snap();
        prev.jobs = vec![before];
        let mut next = snap();
        next.jobs = vec![after];
        let lines = diff_flow(&prev, &next, NOW);
        assert_eq!(lines.len(), 30);
        assert_eq!(lines[0].text, "read 10");
        assert_eq!(lines[29].text, "read 39");
    }

    #[test]
    fn a_job_that_is_new_and_already_running_yields_the_router_line_and_the_running_line() {
        let mut running = job("j", JobStatus::Running);
        running.job_type = JobType::Boilerplate;
        let mut next = snap();
        next.jobs = vec![running];
        next.plans = vec![plan(ProviderId::Claude)];
        assert_eq!(
            diff_flow(&snap(), &next, NOW),
            vec![
                line(FlowActor::Router, "boilerplate → claude"),
                line(FlowActor::Plan(ProviderId::Claude), "boilerplate running"),
            ]
        );
    }
}
