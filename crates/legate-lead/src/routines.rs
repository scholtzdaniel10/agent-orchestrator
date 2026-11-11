//! Saved prompts that start a job, including ones that follow a finished job.

use std::rc::Rc;

use legate_core::{JobAccess, JobRecord, JobStatus, JobType, ProviderId};
use legate_store::RoutineRow;
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::LeadError;
use crate::ports::{JobRequest, JobSink};
use crate::text::fresh_id;

const NAME_MAX: usize = 40;
const PROMPT_MAX: usize = 8_000;
const ID_MAX: usize = 64;
const PER_PROJECT: usize = 30;
const FOLLOW_PROMPT: usize = 300;

/// `auto` lets the router pick. A named plan is only Claude, Cursor, or Grok.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutineWorker {
    Auto,
    Plan(ProviderId),
}

impl Serialize for RoutineWorker {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RoutineWorker {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value == "auto" {
            return Ok(Self::Auto);
        }
        ProviderId::parse(&value)
            .map(Self::Plan)
            .ok_or_else(|| de::Error::custom("unknown worker"))
    }
}

impl RoutineWorker {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Plan(id) => id.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineAfter {
    pub job_type: JobType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Routine {
    pub id: String,
    pub name: String,
    pub prompt: String,
    #[serde(rename = "type")]
    pub job_type: JobType,
    pub worker: RoutineWorker,
    pub access: JobAccess,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<RoutineAfter>,
}

pub trait RoutineStore {
    fn list_routines(&self, project: &str) -> Result<Vec<RoutineRow>, LeadError>;
    fn save_routine(&self, row: &RoutineRow) -> Result<(), LeadError>;
    fn delete_routine(&self, id: &str) -> Result<(), LeadError>;
}

impl RoutineStore for legate_store::Store {
    fn list_routines(&self, project: &str) -> Result<Vec<RoutineRow>, LeadError> {
        Ok(legate_store::Store::list_routines(self, project)?)
    }

    fn save_routine(&self, row: &RoutineRow) -> Result<(), LeadError> {
        Ok(legate_store::Store::save_routine(self, row)?)
    }

    fn delete_routine(&self, id: &str) -> Result<(), LeadError> {
        Ok(legate_store::Store::delete_routine(self, id)?)
    }
}

pub struct Routines {
    store: Rc<dyn RoutineStore>,
    jobs: Rc<dyn JobSink>,
    project: Rc<dyn Fn() -> String>,
    id: Rc<dyn Fn() -> String>,
    seen_done: std::cell::RefCell<std::collections::HashSet<String>>,
}

impl Routines {
    pub fn new(
        store: Rc<dyn RoutineStore>,
        jobs: Rc<dyn JobSink>,
        project: Rc<dyn Fn() -> String>,
        id: Option<Rc<dyn Fn() -> String>>,
    ) -> Self {
        Self {
            store,
            jobs,
            project,
            id: id.unwrap_or_else(|| Rc::new(fresh_id)),
            seen_done: std::cell::RefCell::new(std::collections::HashSet::new()),
        }
    }

    pub fn list(&self) -> Result<Vec<Routine>, LeadError> {
        let mut out = Vec::new();
        for row in self.store.list_routines(&self.project_dir())? {
            if let Some(routine) = routine_from_row(&row) {
                out.push(routine);
            }
        }
        Ok(out)
    }

    pub fn save(&self, input: &Value) -> Result<Routine, LeadError> {
        let routine = parse_routine(input, self.id.as_ref())?;
        let project = self.project_dir();
        let rows = self.store.list_routines(&project)?;
        let existing = rows.iter().find(|row| row.id == routine.id);
        if existing.is_none() && rows.len() >= PER_PROJECT {
            return Err(LeadError::message(
                "This project already has 30 routines. Delete one before adding another.",
            ));
        }
        let position = existing
            .map(|row| row.position)
            .unwrap_or_else(|| rows.iter().map(|row| row.position).max().unwrap_or(-1) + 1);
        self.store.save_routine(&RoutineRow {
            id: routine.id.clone(),
            project,
            position,
            data: serialize_routine(&routine),
        })?;
        Ok(routine)
    }

    pub fn remove(&self, id: &str) -> Result<(), LeadError> {
        if id.is_empty() || id.len() > ID_MAX {
            return Err(LeadError::message("invalid routine id"));
        }
        self.store.delete_routine(id)
    }

    pub fn run(&self, id: &str) -> Result<JobRecord, LeadError> {
        let routine = self
            .list()?
            .into_iter()
            .find(|item| item.id == id)
            .ok_or_else(|| {
                LeadError::message("That routine is gone. Refresh the list and try again.")
            })?;
        self.submit_routine(&routine, &routine.prompt)
    }

    pub fn on_job_finished(&self, job: &JobRecord) {
        if job.status != JobStatus::Done || job.routine.is_some() {
            return;
        }
        if !self.seen_done.borrow_mut().insert(job.id.clone()) {
            return;
        }
        let snippet = prefix(&job.prompt, FOLLOW_PROMPT);
        let suffix = format!("\n\nThis follows the job: \"{snippet}\"");
        let Ok(routines) = self.list() else {
            return;
        };
        for routine in routines {
            let Some(after) = &routine.after else {
                continue;
            };
            if after.job_type != job.job_type {
                continue;
            }
            let prompt = format!("{}{suffix}", routine.prompt);
            if let Err(err) = self.submit_routine(&routine, &prompt) {
                tracing::error!("{err}");
            }
        }
    }

    fn submit_routine(&self, routine: &Routine, prompt: &str) -> Result<JobRecord, LeadError> {
        let provider = match routine.worker {
            RoutineWorker::Auto => None,
            RoutineWorker::Plan(id) => Some(id),
        };
        self.jobs.submit(JobRequest {
            job_type: routine.job_type,
            prompt: prompt.to_string(),
            provider,
            access: Some(routine.access),
            routine: Some(routine.name.clone()),
            ..JobRequest::default()
        })
    }

    fn project_dir(&self) -> String {
        (self.project)()
    }
}

pub fn parse_routine(input: &Value, make_id: &dyn Fn() -> String) -> Result<Routine, LeadError> {
    let Some(row) = input.as_object() else {
        return Err(LeadError::message("invalid routine"));
    };
    let id = match row.get("id") {
        None | Some(Value::Null) => make_id(),
        Some(Value::String(text)) if text.is_empty() => make_id(),
        Some(Value::String(text)) => text.clone(),
        Some(_) => return Err(LeadError::message("invalid routine id")),
    };
    if id.is_empty() || id.len() > ID_MAX {
        return Err(LeadError::message("invalid routine id"));
    }
    let name_raw = match row.get("name") {
        Some(Value::String(text)) => text,
        _ => {
            return Err(LeadError::message(
                "Name must be 1 to 40 characters on one line.",
            ));
        }
    };
    if name_raw.contains(['\r', '\n']) {
        return Err(LeadError::message(
            "Name must be 1 to 40 characters on one line.",
        ));
    }
    let name = name_raw.trim();
    if name.is_empty() || name.chars().count() > NAME_MAX {
        return Err(LeadError::message(
            "Name must be 1 to 40 characters on one line.",
        ));
    }
    let prompt = match row.get("prompt") {
        Some(Value::String(text)) => text.clone(),
        _ => return Err(LeadError::message("Prompt must be 1 to 8000 characters.")),
    };
    if prompt.is_empty() || prompt.chars().count() > PROMPT_MAX {
        return Err(LeadError::message("Prompt must be 1 to 8000 characters."));
    }
    let job_type = match row
        .get("type")
        .and_then(Value::as_str)
        .and_then(JobType::parse)
    {
        Some(job_type) => job_type,
        None => return Err(LeadError::message("unknown job type")),
    };
    let worker = parse_worker(row.get("worker"))?;
    let access = match row
        .get("access")
        .and_then(Value::as_str)
        .and_then(JobAccess::parse)
    {
        Some(access) => access,
        None => return Err(LeadError::message("access must be read, edit, or full")),
    };
    let after = match row.get("after") {
        None | Some(Value::Null) => None,
        Some(Value::Object(after)) => {
            let job_type = match after
                .get("jobType")
                .and_then(Value::as_str)
                .and_then(JobType::parse)
            {
                Some(job_type) => job_type,
                None => return Err(LeadError::message("unknown job type")),
            };
            Some(RoutineAfter { job_type })
        }
        Some(_) => return Err(LeadError::message("invalid after")),
    };
    Ok(Routine {
        id,
        name: name.to_string(),
        prompt,
        job_type,
        worker,
        access,
        after,
    })
}

fn parse_worker(value: Option<&Value>) -> Result<RoutineWorker, LeadError> {
    let Some(Value::String(text)) = value else {
        return Err(LeadError::message(
            "worker must be auto, claude, cursor, or grok",
        ));
    };
    match text.as_str() {
        "auto" => Ok(RoutineWorker::Auto),
        "claude" => Ok(RoutineWorker::Plan(ProviderId::Claude)),
        "cursor" => Ok(RoutineWorker::Plan(ProviderId::Cursor)),
        "grok" => Ok(RoutineWorker::Plan(ProviderId::Grok)),
        _ => Err(LeadError::message(
            "worker must be auto, claude, cursor, or grok",
        )),
    }
}

fn serialize_routine(routine: &Routine) -> String {
    let mut data = Map::new();
    data.insert("name".to_string(), Value::String(routine.name.clone()));
    data.insert("prompt".to_string(), Value::String(routine.prompt.clone()));
    data.insert(
        "type".to_string(),
        Value::String(routine.job_type.as_str().to_string()),
    );
    data.insert(
        "worker".to_string(),
        Value::String(routine.worker.as_str().to_string()),
    );
    data.insert(
        "access".to_string(),
        Value::String(routine.access.as_str().to_string()),
    );
    if let Some(after) = &routine.after {
        let mut object = Map::new();
        object.insert(
            "jobType".to_string(),
            Value::String(after.job_type.as_str().to_string()),
        );
        data.insert("after".to_string(), Value::Object(object));
    }
    Value::Object(data).to_string()
}

fn routine_from_row(row: &RoutineRow) -> Option<Routine> {
    let parsed: Value = serde_json::from_str(&row.data).ok()?;
    let Value::Object(mut object) = parsed else {
        return None;
    };
    object.insert("id".to_string(), Value::String(row.id.clone()));
    parse_routine(&Value::Object(object), &fresh_id).ok()
}

fn prefix(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Plans the TypeScript lead can name. `local` is not one of them.
pub(crate) fn chat_plan(value: Option<&str>) -> Option<ProviderId> {
    match value.and_then(ProviderId::parse) {
        Some(id @ (ProviderId::Claude | ProviderId::Cursor | ProviderId::Grok)) => Some(id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use legate_core::{JobAccess, JobRecord, JobStatus, JobType, ProviderId};
    use legate_store::RoutineRow;
    use serde_json::{Value, json};

    use super::{Routine, RoutineStore, RoutineWorker, Routines, parse_routine};
    use crate::error::LeadError;
    use crate::ports::{EmptyJobs, JobRequest, JobSink};

    struct MemStore {
        rows: RefCell<Vec<RoutineRow>>,
    }

    impl RoutineStore for MemStore {
        fn list_routines(&self, project: &str) -> Result<Vec<RoutineRow>, LeadError> {
            let mut rows: Vec<_> = self
                .rows
                .borrow()
                .iter()
                .filter(|row| row.project == project)
                .cloned()
                .collect();
            rows.sort_by_key(|row| row.position);
            Ok(rows)
        }

        fn save_routine(&self, row: &RoutineRow) -> Result<(), LeadError> {
            let mut rows = self.rows.borrow_mut();
            if let Some(found) = rows.iter_mut().find(|item| item.id == row.id) {
                *found = row.clone();
            } else {
                rows.push(row.clone());
            }
            Ok(())
        }

        fn delete_routine(&self, id: &str) -> Result<(), LeadError> {
            self.rows.borrow_mut().retain(|row| row.id != id);
            Ok(())
        }
    }

    struct Rec {
        calls: RefCell<Vec<JobRequest>>,
        jobs: RefCell<Vec<JobRecord>>,
    }

    impl JobSink for Rec {
        fn submit(&self, request: JobRequest) -> Result<JobRecord, LeadError> {
            self.calls.borrow_mut().push(request.clone());
            let job = job_of(&request, self.jobs.borrow().len() + 1);
            self.jobs.borrow_mut().push(job.clone());
            Ok(job)
        }

        fn list(&self) -> Vec<JobRecord> {
            self.jobs.borrow().clone()
        }

        fn get(&self, id: &str) -> Option<JobRecord> {
            self.jobs.borrow().iter().find(|job| job.id == id).cloned()
        }

        fn cancel(&self, _id: &str) -> Result<(), LeadError> {
            Ok(())
        }

        fn on_update(&self, _callback: Rc<dyn Fn(JobRecord)>) -> Box<dyn FnOnce()> {
            Box::new(|| {})
        }
    }

    fn job_of(request: &JobRequest, n: usize) -> JobRecord {
        JobRecord {
            id: format!("job-{n}"),
            job_type: request.job_type,
            prompt: request.prompt.clone(),
            provider: request.provider.or(Some(ProviderId::Claude)),
            status: JobStatus::Queued,
            output: String::new(),
            failed_over: Vec::new(),
            steps: None,
            steps_dropped: None,
            model: None,
            reason: None,
            edit: None,
            access: request.access,
            change: None,
            group: None,
            lead_message: None,
            routine: request.routine.clone(),
            title: None,
            session_id: None,
            goal: None,
            base: None,
            error: None,
            owner_id: None,
            heartbeat_at: None,
            cancel_requested: None,
            extra: Default::default(),
        }
    }

    fn blank(id: &str, status: JobStatus, job_type: JobType, prompt: &str) -> JobRecord {
        JobRecord {
            id: id.to_string(),
            job_type,
            prompt: prompt.to_string(),
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
            extra: Default::default(),
        }
    }

    struct Harness {
        routines: Routines,
        store: Rc<MemStore>,
        rec: Rc<Rec>,
    }

    fn harness(id: Option<Rc<dyn Fn() -> String>>) -> Harness {
        let store = Rc::new(MemStore {
            rows: RefCell::new(Vec::new()),
        });
        let rec = Rc::new(Rec {
            calls: RefCell::new(Vec::new()),
            jobs: RefCell::new(Vec::new()),
        });
        let routines = Routines::new(
            store.clone(),
            rec.clone(),
            Rc::new(|| "/proj".to_string()),
            id,
        );
        Harness {
            routines,
            store,
            rec,
        }
    }

    fn sample() -> Value {
        json!({
            "id": "r1",
            "name": "Review it",
            "prompt": "review the last change",
            "type": "review",
            "worker": "auto",
            "access": "read"
        })
    }

    fn err(value: &Value) -> String {
        parse_routine(value, &|| "id".to_string())
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn save_validates_lists_in_order_and_remove_deletes() {
        let h = harness(Some(Rc::new(|| "generated".to_string())));
        let first = h
            .routines
            .save(&json!({
                "name": "  One  ",
                "prompt": "do one",
                "type": "planning",
                "worker": "claude",
                "access": "edit"
            }))
            .unwrap();
        assert_eq!(first.id, "generated");
        assert_eq!(first.name, "One");
        assert_eq!(first.prompt, "do one");
        assert_eq!(first.job_type, JobType::Planning);
        assert_eq!(first.worker, RoutineWorker::Plan(ProviderId::Claude));
        assert_eq!(first.access, JobAccess::Edit);
        assert!(first.after.is_none());

        let mut second_in = sample();
        second_in["id"] = json!("r2");
        second_in["name"] = json!("Two");
        second_in["after"] = json!({ "jobType": "debugging" });
        let second = h.routines.save(&second_in).unwrap();
        let listed = h.routines.list().unwrap();
        assert_eq!(
            listed
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["generated", "r2"]
        );
        assert_eq!(listed[1], second);
        assert_eq!(h.store.rows.borrow()[0].project, "/proj");
        assert_eq!(h.store.rows.borrow()[0].position, 0);
        assert_eq!(h.store.rows.borrow()[1].position, 1);

        let mut renamed = sample();
        renamed["id"] = json!("r2");
        renamed["name"] = json!("Two renamed");
        renamed["prompt"] = json!("later");
        renamed["after"] = json!({ "jobType": "debugging" });
        h.routines.save(&renamed).unwrap();
        let listed = h.routines.list().unwrap();
        assert_eq!(listed[1].name, "Two renamed");
        assert_eq!(listed[1].prompt, "later");
        assert_eq!(h.store.rows.borrow()[1].position, 1);

        h.routines.remove("r2").unwrap();
        assert_eq!(
            h.routines
                .list()
                .unwrap()
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["generated"]
        );
        assert_eq!(
            h.routines.remove("").unwrap_err().to_string(),
            "invalid routine id"
        );
    }

    #[test]
    fn save_refuses_a_31st_routine_on_the_same_project() {
        let h = harness(Some(Rc::new(|| panic!("should not generate"))));
        for i in 0..30 {
            let mut row = sample();
            row["id"] = json!(format!("r{i}"));
            row["name"] = json!(format!("n{i}"));
            h.routines.save(&row).unwrap();
        }
        let mut extra = sample();
        extra["id"] = json!("r30");
        extra["name"] = json!("too many");
        assert_eq!(
            h.routines.save(&extra).unwrap_err().to_string(),
            "This project already has 30 routines. Delete one before adding another."
        );
        let mut updated = sample();
        updated["id"] = json!("r0");
        updated["name"] = json!("updated");
        h.routines.save(&updated).unwrap();
        assert_eq!(h.routines.list().unwrap()[0].name, "updated");
    }

    #[test]
    fn parse_routine_rejects_bad_fields() {
        assert_eq!(err(&Value::Null), "invalid routine");
        let mut row = sample();
        row["name"] = json!("");
        assert_eq!(err(&row), "Name must be 1 to 40 characters on one line.");
        row["name"] = json!("a\nb");
        assert_eq!(err(&row), "Name must be 1 to 40 characters on one line.");
        row["name"] = json!("x".repeat(41));
        assert_eq!(err(&row), "Name must be 1 to 40 characters on one line.");
        let mut row = sample();
        row["prompt"] = json!("");
        assert_eq!(err(&row), "Prompt must be 1 to 8000 characters.");
        row["prompt"] = json!("x".repeat(8001));
        assert_eq!(err(&row), "Prompt must be 1 to 8000 characters.");
        let mut row = sample();
        row["type"] = json!("nope");
        assert_eq!(err(&row), "unknown job type");
        row = sample();
        row["worker"] = json!("both");
        assert_eq!(err(&row), "worker must be auto, claude, cursor, or grok");
        row = sample();
        row["worker"] = json!("local");
        assert_eq!(err(&row), "worker must be auto, claude, cursor, or grok");
        row = sample();
        row["access"] = json!("write");
        assert_eq!(err(&row), "access must be read, edit, or full");
        row = sample();
        row["after"] = json!({ "jobType": "nope" });
        assert_eq!(err(&row), "unknown job type");
        let _ = Routine {
            id: "r".into(),
            name: "n".into(),
            prompt: "p".into(),
            job_type: JobType::Review,
            worker: RoutineWorker::Auto,
            access: JobAccess::Read,
            after: None,
        };
    }

    #[test]
    fn run_submits_the_saved_prompt_and_marks_the_job() {
        let h = harness(None);
        let mut row = sample();
        row["worker"] = json!("cursor");
        row["access"] = json!("full");
        row["type"] = json!("refactor");
        h.routines.save(&row).unwrap();
        let job = h.routines.run("r1").unwrap();
        assert_eq!(job.job_type, JobType::Refactor);
        assert_eq!(job.prompt, "review the last change");
        assert_eq!(job.provider, Some(ProviderId::Cursor));
        assert_eq!(job.routine.as_deref(), Some("Review it"));
        let calls = h.rec.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].job_type, JobType::Refactor);
        assert_eq!(calls[0].prompt, "review the last change");
        assert_eq!(calls[0].provider, Some(ProviderId::Cursor));
        assert_eq!(calls[0].access, Some(JobAccess::Full));
        assert_eq!(calls[0].routine.as_deref(), Some("Review it"));
        assert_eq!(
            h.routines.run("missing").unwrap_err().to_string(),
            "That routine is gone. Refresh the list and try again."
        );
    }

    #[test]
    fn run_with_auto_does_not_name_a_provider() {
        let h = harness(None);
        h.routines.save(&sample()).unwrap();
        h.routines.run("r1").unwrap();
        assert!(h.rec.calls.borrow()[0].provider.is_none());
        let _ = EmptyJobs;
    }

    #[test]
    fn on_job_finished_runs_matching_routines_once() {
        let h = harness(None);
        let mut follow = sample();
        follow["id"] = json!("after-review");
        follow["name"] = json!("Follow review");
        follow["type"] = json!("refactor");
        follow["prompt"] = json!("clean it up");
        follow["after"] = json!({ "jobType": "review" });
        h.routines.save(&follow).unwrap();
        let mut skip = sample();
        skip["id"] = json!("after-plan");
        skip["name"] = json!("Skip");
        skip["prompt"] = json!("nope");
        skip["after"] = json!({ "jobType": "planning" });
        h.routines.save(&skip).unwrap();
        let mut manual = sample();
        manual["id"] = json!("manual");
        manual["name"] = json!("Manual");
        manual["prompt"] = json!("only click");
        h.routines.save(&manual).unwrap();
        let finished = blank(
            "orig",
            JobStatus::Done,
            JobType::Review,
            "please review this",
        );
        h.routines.on_job_finished(&finished);
        h.routines.on_job_finished(&finished);
        let calls = h.rec.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].job_type, JobType::Refactor);
        assert!(calls[0].provider.is_none());
        assert_eq!(calls[0].access, Some(JobAccess::Read));
        assert_eq!(calls[0].routine.as_deref(), Some("Follow review"));
        assert_eq!(
            calls[0].prompt,
            "clean it up\n\nThis follows the job: \"please review this\""
        );
    }

    #[test]
    fn a_finished_job_started_by_a_routine_does_not_trigger_another() {
        let h = harness(None);
        let mut row = sample();
        row["after"] = json!({ "jobType": "review" });
        h.routines.save(&row).unwrap();
        let mut job = blank("from-routine", JobStatus::Done, JobType::Review, "go");
        job.routine = Some("Review it".to_string());
        h.routines.on_job_finished(&job);
        assert!(h.rec.calls.borrow().is_empty());
    }

    #[test]
    fn a_failed_job_does_not_trigger_routines() {
        let h = harness(None);
        let mut row = sample();
        row["after"] = json!({ "jobType": "review" });
        h.routines.save(&row).unwrap();
        h.routines
            .on_job_finished(&blank("bad", JobStatus::Failed, JobType::Review, "go"));
        assert!(h.rec.calls.borrow().is_empty());
    }

    #[test]
    fn the_follow_on_prompt_uses_the_first_300_characters() {
        let h = harness(None);
        let mut row = sample();
        row["prompt"] = json!("next");
        row["after"] = json!({ "jobType": "planning" });
        h.routines.save(&row).unwrap();
        let long = "a".repeat(400);
        h.routines
            .on_job_finished(&blank("long", JobStatus::Done, JobType::Planning, &long));
        assert_eq!(
            h.rec.calls.borrow()[0].prompt,
            format!("next\n\nThis follows the job: \"{}\"", "a".repeat(300))
        );
    }

    #[test]
    fn corrupt_stored_rows_are_skipped() {
        let h = harness(None);
        h.store
            .save_routine(&RoutineRow {
                id: "bad".to_string(),
                project: "/proj".to_string(),
                position: 0,
                data: "{not-json".to_string(),
            })
            .unwrap();
        h.store
            .save_routine(&RoutineRow {
                id: "ok".to_string(),
                project: "/proj".to_string(),
                position: 1,
                data: serde_json::json!({
                    "name": "Good",
                    "prompt": "go",
                    "type": "review",
                    "worker": "auto",
                    "access": "read"
                })
                .to_string(),
            })
            .unwrap();
        assert_eq!(
            h.routines
                .list()
                .unwrap()
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["ok"]
        );
    }
}
