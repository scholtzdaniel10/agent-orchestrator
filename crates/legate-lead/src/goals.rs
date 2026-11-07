//! A goal is a piece of work the lead drives in its own worktree, round by round.

use std::collections::HashSet;
use std::rc::{Rc, Weak};

use async_trait::async_trait;
use legate_core::{JobAccess, JobRecord, JobStatus, JobType};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::LeadError;
use crate::text::{ellipsis_tail, first_line, fresh_id, indent, split_lines};

pub const MAX_JOBS_PER_ROUND: usize = 3;
const OUTPUT_TAIL: usize = 6000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GoalStatus {
    Planning,
    Waiting,
    Running,
    Checking,
    Done,
    Stopped,
    Failed,
}

impl GoalStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Planning => "planning",
            Self::Waiting => "waiting",
            Self::Running => "running",
            Self::Checking => "checking",
            Self::Done => "done",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedJob {
    #[serde(rename = "type")]
    pub job_type: JobType,
    pub access: JobAccess,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalCheck {
    pub code: Option<i32>,
    pub output: String,
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalRound {
    pub n: i64,
    pub summary: String,
    pub jobs: Vec<PlannedJob>,
    pub job_ids: Vec<String>,
    pub problems: Vec<String>,
    pub check: Option<GoalCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalProposal {
    pub summary: String,
    pub jobs: Vec<PlannedJob>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalSource {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Goal {
    pub id: String,
    pub project: String,
    pub text: String,
    pub check: Option<String>,
    pub max_rounds: i64,
    pub quick: bool,
    pub worktree: String,
    pub status: GoalStatus,
    pub rounds: Vec<GoalRound>,
    pub proposal: Option<GoalProposal>,
    pub question: Option<String>,
    pub notes: Vec<String>,
    pub outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<GoalSource>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickJob {
    pub job_type: JobType,
    pub access: JobAccess,
}

#[derive(Debug, Clone)]
pub struct NewGoal {
    pub project: String,
    pub text: String,
    pub check: Option<String>,
    pub max_rounds: Option<f64>,
    pub quick: Option<QuickJob>,
    pub source: Option<GoalSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedTree {
    pub id: String,
    pub path: String,
    pub branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyResult {
    pub ok: bool,
    pub empty: bool,
    pub message: String,
}

pub struct GoalExtra {
    pub goal: String,
    pub base: String,
    pub project: String,
}

pub trait GoalStore {
    fn save_goal(
        &self,
        id: &str,
        project: &str,
        updated_at: i64,
        data: &str,
    ) -> Result<(), LeadError>;
    fn goals(&self, project: &str) -> Result<Vec<String>, LeadError>;
    fn goal(&self, id: &str) -> Result<Option<String>, LeadError>;
    fn delete_goal(&self, id: &str) -> Result<(), LeadError>;
}

impl GoalStore for legate_store::Store {
    fn save_goal(
        &self,
        id: &str,
        project: &str,
        updated_at: i64,
        data: &str,
    ) -> Result<(), LeadError> {
        Ok(legate_store::Store::save_goal(
            self, id, project, updated_at, data,
        )?)
    }

    fn goals(&self, project: &str) -> Result<Vec<String>, LeadError> {
        Ok(legate_store::Store::goals(self, project, None)?)
    }

    fn goal(&self, id: &str) -> Result<Option<String>, LeadError> {
        Ok(legate_store::Store::goal(self, id)?)
    }

    fn delete_goal(&self, id: &str) -> Result<(), LeadError> {
        Ok(legate_store::Store::delete_goal(self, id)?)
    }
}

#[async_trait(?Send)]
pub trait GoalWorktrees {
    async fn create(&self, project: &str, id: &str) -> Result<CreatedTree, String>;
    fn folder(&self, id: &str) -> String;
    fn branch_of(&self, id: &str) -> String;
    async fn apply_into(
        &self,
        from_id: &str,
        target: &str,
        base: &str,
    ) -> Result<ApplyResult, String>;
    async fn commit_all(&self, dir: &str, message: &str) -> Result<bool, String>;
    async fn discard(&self, project: &str, id: &str) -> Result<(), String>;
}

#[async_trait(?Send)]
pub trait GoalPlanner {
    async fn plan(&self, goal: &Goal, folder: &str, prompt: &str) -> Result<String, String>;
}

#[async_trait(?Send)]
pub trait GoalChecks {
    async fn run(&self, command: &str, folder: &str) -> Result<(Option<i32>, String), String>;
}

pub type JobLookup = Rc<dyn Fn(&str) -> Option<JobRecord>>;
pub type NameWorktree = Rc<dyn Fn(&str, &str, &str)>;
pub type GoalListener = Rc<dyn Fn(&Goal)>;

pub struct Goals {
    store: Rc<dyn GoalStore>,
    worktrees: Rc<dyn GoalWorktrees>,
    submit: Rc<dyn Fn(PlannedJob, GoalExtra) -> Result<JobRecord, LeadError>>,
    job: JobLookup,
    cancel: Rc<dyn Fn(&str)>,
    planner: Rc<dyn GoalPlanner>,
    checks: Rc<dyn GoalChecks>,
    ceiling: Rc<dyn Fn() -> JobAccess>,
    name_worktree: Option<NameWorktree>,
    on_update: Option<GoalListener>,
    now: Rc<dyn Fn() -> i64>,
    finishing: std::cell::RefCell<HashSet<String>>,
    this: Weak<Goals>,
}

pub struct GoalDeps {
    pub store: Rc<dyn GoalStore>,
    pub worktrees: Rc<dyn GoalWorktrees>,
    pub submit: Rc<dyn Fn(PlannedJob, GoalExtra) -> Result<JobRecord, LeadError>>,
    pub job: JobLookup,
    pub cancel: Rc<dyn Fn(&str)>,
    pub planner: Rc<dyn GoalPlanner>,
    pub checks: Rc<dyn GoalChecks>,
    pub ceiling: Rc<dyn Fn() -> JobAccess>,
    pub name_worktree: Option<NameWorktree>,
    pub on_update: Option<GoalListener>,
    pub now: Option<Rc<dyn Fn() -> i64>>,
}

impl Goals {
    pub fn new(deps: GoalDeps) -> Rc<Self> {
        Rc::new_cyclic(|this| Self {
            store: deps.store,
            worktrees: deps.worktrees,
            submit: deps.submit,
            job: deps.job,
            cancel: deps.cancel,
            planner: deps.planner,
            checks: deps.checks,
            ceiling: deps.ceiling,
            name_worktree: deps.name_worktree,
            on_update: deps.on_update,
            now: deps.now.unwrap_or_else(|| Rc::new(system_now)),
            finishing: std::cell::RefCell::new(HashSet::new()),
            this: this.clone(),
        })
    }

    pub fn list(&self, project: &str) -> Result<Vec<Goal>, LeadError> {
        let mut out = Vec::new();
        for raw in self.store.goals(project)? {
            if let Some(goal) = parse_goal(&raw) {
                out.push(goal);
            }
        }
        Ok(out)
    }

    pub fn get(&self, id: &str) -> Result<Option<Goal>, LeadError> {
        Ok(self.store.goal(id)?.as_deref().and_then(parse_goal))
    }

    pub async fn create(self: &Rc<Self>, input: NewGoal) -> Result<Goal, LeadError> {
        let text = input.text.trim().to_string();
        if text.is_empty() {
            return Err(LeadError::message("Say what the goal is."));
        }
        let id = fresh_id();
        let created = self
            .worktrees
            .create(&input.project, &id)
            .await
            .map_err(LeadError::message)?;
        let now = (self.now)();
        let check = input.check.unwrap_or_default().trim().to_string();
        let mut goal = Goal {
            id: id.clone(),
            project: input.project.clone(),
            text: text.clone(),
            check: if check.is_empty() { None } else { Some(check) },
            max_rounds: clamp_rounds(input.max_rounds.unwrap_or(5.0)),
            quick: input.quick.is_some(),
            worktree: created.id.clone(),
            status: GoalStatus::Planning,
            rounds: Vec::new(),
            proposal: None,
            question: None,
            notes: Vec::new(),
            outcome: None,
            source: input.source,
            created_at: now,
            updated_at: now,
        };
        if let Some(name) = &self.name_worktree {
            name(
                &created.id,
                &input.project,
                &format!("Goal: {}", first_line(&text, 40)),
            );
        }
        if let Some(quick) = input.quick {
            goal.status = GoalStatus::Waiting;
            goal.proposal = Some(GoalProposal {
                summary: "Your task".to_string(),
                jobs: vec![PlannedJob {
                    job_type: quick.job_type,
                    access: quick.access,
                    prompt: text,
                }],
            });
            self.save(&mut goal)?;
            return self.approve(&id, None);
        }
        self.save(&mut goal)?;
        self.spawn_plan(id);
        Ok(goal)
    }

    pub fn approve(&self, id: &str, jobs: Option<Vec<PlannedJob>>) -> Result<Goal, LeadError> {
        let mut goal = self.require(id)?;
        let Some(proposal) = goal.proposal.clone() else {
            return Err(LeadError::message("There is no proposed round to approve."));
        };
        if goal.status != GoalStatus::Waiting {
            return Err(LeadError::message("There is no proposed round to approve."));
        }
        let chosen: Vec<PlannedJob> = jobs
            .unwrap_or(proposal.jobs)
            .into_iter()
            .filter(|job| !job.prompt.trim().is_empty())
            .collect();
        if chosen.is_empty() {
            return Err(LeadError::message("A round needs at least one job."));
        }
        let ceiling = (self.ceiling)();
        let base = self.worktrees.branch_of(&goal.worktree);
        let mut round = GoalRound {
            n: goal.rounds.len() as i64 + 1,
            summary: proposal.summary,
            jobs: chosen.clone(),
            job_ids: Vec::new(),
            problems: Vec::new(),
            check: None,
        };
        goal.proposal = None;
        goal.status = GoalStatus::Running;
        goal.outcome = None;
        goal.rounds.push(round.clone());
        self.save(&mut goal)?;
        for planned in &chosen {
            let record = (self.submit)(
                PlannedJob {
                    job_type: planned.job_type,
                    access: cap_access(planned.access, ceiling),
                    prompt: planned.prompt.clone(),
                },
                GoalExtra {
                    goal: goal.id.clone(),
                    base: base.clone(),
                    project: goal.project.clone(),
                },
            )?;
            round.job_ids.push(record.id);
        }
        if let Some(stored) = goal.rounds.last_mut() {
            stored.job_ids = round.job_ids;
        }
        self.save(&mut goal)?;
        self.on_job(None, Some(&goal.id))?;
        Ok(goal)
    }

    pub fn answer(&self, id: &str, text: &str) -> Result<Goal, LeadError> {
        let mut goal = self.require(id)?;
        if matches!(
            goal.status,
            GoalStatus::Planning | GoalStatus::Running | GoalStatus::Checking
        ) {
            return Err(LeadError::message("Wait for this round to finish."));
        }
        let note = text.trim();
        if !note.is_empty() {
            goal.notes.push(note.to_string());
        }
        goal.question = None;
        goal.proposal = None;
        if goal.status != GoalStatus::Waiting {
            goal.max_rounds = goal.max_rounds.max(goal.rounds.len() as i64 + 1);
        }
        self.save(&mut goal)?;
        self.spawn_plan(id.to_string());
        Ok(goal)
    }

    pub fn stop(&self, id: &str) -> Result<Goal, LeadError> {
        let mut goal = self.require(id)?;
        if goal.status == GoalStatus::Running
            && let Some(round) = goal.rounds.last()
        {
            for job_id in round.job_ids.clone() {
                if let Some(job) = (self.job)(&job_id)
                    && matches!(job.status, JobStatus::Queued | JobStatus::Running)
                {
                    (self.cancel)(&job_id);
                }
            }
        }
        goal.status = GoalStatus::Stopped;
        goal.proposal = None;
        goal.outcome = Some("Stopped by you. The work so far is in Changes.".to_string());
        self.save(&mut goal)?;
        Ok(goal)
    }

    pub async fn remove(&self, id: &str) -> Result<(), LeadError> {
        let goal = self.require(id)?;
        if matches!(
            goal.status,
            GoalStatus::Planning | GoalStatus::Running | GoalStatus::Checking
        ) {
            return Err(LeadError::message("Stop the goal first."));
        }
        self.store.delete_goal(id)?;
        let _ = self.worktrees.discard(&goal.project, &goal.worktree).await;
        Ok(())
    }

    pub fn on_job(&self, job: Option<&JobRecord>, goal_id: Option<&str>) -> Result<(), LeadError> {
        let Some(id) = goal_id.or_else(|| job.and_then(|job| job.goal.as_deref())) else {
            return Ok(());
        };
        let Some(goal) = self.get(id)? else {
            return Ok(());
        };
        if goal.status != GoalStatus::Running {
            return Ok(());
        }
        let Some(round) = goal.rounds.last() else {
            return Ok(());
        };
        if let Some(job) = job
            && !round.job_ids.iter().any(|job_id| job_id == &job.id)
        {
            return Ok(());
        }
        let ended = round.job_ids.iter().all(|job_id| match (self.job)(job_id) {
            None => true,
            Some(record) => matches!(record.status, JobStatus::Done | JobStatus::Failed),
        });
        if ended {
            self.spawn_finish(id.to_string());
        }
        Ok(())
    }

    pub fn resume(&self, project: &str) -> Result<(), LeadError> {
        for mut goal in self.list(project)? {
            if goal.status == GoalStatus::Running {
                self.on_job(None, Some(&goal.id))?;
            } else if matches!(goal.status, GoalStatus::Planning | GoalStatus::Checking) {
                goal.status = GoalStatus::Waiting;
                goal.question = Some(
                    "Legate closed while this goal was working. Plan the next round again?"
                        .to_string(),
                );
                self.save(&mut goal)?;
            }
        }
        Ok(())
    }

    fn spawn_plan(&self, id: String) {
        let Some(this) = self.this.upgrade() else {
            return;
        };
        tokio::task::spawn_local(async move {
            this.plan(&id).await;
        });
    }

    fn spawn_finish(&self, id: String) {
        let Some(this) = self.this.upgrade() else {
            return;
        };
        tokio::task::spawn_local(async move {
            this.finish_round(&id).await;
        });
    }

    async fn plan(&self, id: &str) {
        let Ok(mut goal) = self.require(id) else {
            return;
        };
        goal.status = GoalStatus::Planning;
        goal.question = None;
        if self.save(&mut goal).is_err() {
            return;
        }
        let folder = self.worktrees.folder(&goal.worktree);
        let prompt = plan_prompt(&goal);
        let text = match self.planner.plan(&goal, &folder, &prompt).await {
            Ok(text) => text,
            Err(err) => {
                if let Ok(mut fresh) = self.require(id) {
                    fresh.status = GoalStatus::Waiting;
                    fresh.question = Some(format!("The lead could not plan: {err}. Try again?"));
                    let _ = self.save(&mut fresh);
                }
                return;
            }
        };
        let Ok(mut fresh) = self.require(id) else {
            return;
        };
        if fresh.status != GoalStatus::Planning {
            return;
        }
        match parse_plan(&text) {
            None => {
                fresh.status = GoalStatus::Waiting;
                fresh.question = Some(format!(
                    "The lead's answer did not include a plan: \"{}\". Try again, or tell it what to do.",
                    first_line(&text, 160)
                ));
            }
            Some(ParsedPlan::Done { summary }) => {
                fresh.status = GoalStatus::Done;
                fresh.outcome = Some(if summary.is_empty() {
                    "The lead judges the goal met. Review it in Changes.".to_string()
                } else {
                    summary
                });
            }
            Some(ParsedPlan::Ask { question }) => {
                fresh.status = GoalStatus::Waiting;
                fresh.question = Some(question);
            }
            Some(ParsedPlan::Jobs { summary, jobs }) => {
                fresh.status = GoalStatus::Waiting;
                fresh.proposal = Some(GoalProposal { summary, jobs });
            }
        }
        let _ = self.save(&mut fresh);
    }

    async fn finish_round(&self, id: &str) {
        if !self.finishing.borrow_mut().insert(id.to_string()) {
            return;
        }
        if let Err(err) = self.finish_round_inner(id).await {
            tracing::error!("{err}");
        }
        self.finishing.borrow_mut().remove(id);
    }

    async fn finish_round_inner(&self, id: &str) -> Result<(), LeadError> {
        let mut goal = self.require(id)?;
        if goal.status != GoalStatus::Running {
            return Ok(());
        }
        let Some(mut round) = goal.rounds.last().cloned() else {
            return Ok(());
        };
        let folder = self.worktrees.folder(&goal.worktree);
        let base = self.worktrees.branch_of(&goal.worktree);
        round.problems.clear();
        for job_id in round.job_ids.clone() {
            let job = (self.job)(&job_id);
            let label = first_line(
                job.as_ref()
                    .map(|job| job.prompt.as_str())
                    .unwrap_or(&job_id),
                60,
            );
            if job
                .as_ref()
                .is_none_or(|job| job.status == JobStatus::Failed)
            {
                let why = job
                    .as_ref()
                    .and_then(|job| job.error.clone())
                    .unwrap_or_else(|| "the job is gone".to_string());
                round.problems.push(format!("{label}: {why}"));
            }
            let Some(job) = job else {
                continue;
            };
            if job.change.is_none() {
                continue;
            }
            if job.status == JobStatus::Done
                && let Some(change) = &job.change
            {
                match self.worktrees.apply_into(change, &folder, &base).await {
                    Ok(result) if !result.ok => {
                        round.problems.push(format!("{label}: {}", result.message))
                    }
                    Err(err) => round.problems.push(format!("{label}: {err}")),
                    Ok(_) => {}
                }
            }
            if let Some(change) = &job.change {
                let _ = self.worktrees.discard(&goal.project, change).await;
            }
        }
        let _ = self
            .worktrees
            .commit_all(
                &folder,
                &format!("Goal round {}: {}", round.n, round.summary),
            )
            .await;
        if goal.check.is_some() {
            goal.status = GoalStatus::Checking;
            if let Some(stored) = goal.rounds.last_mut() {
                *stored = round.clone();
            }
            self.save(&mut goal)?;
            let command = goal.check.clone().unwrap_or_default();
            let result = match self.checks.run(&command, &folder).await {
                Ok(result) => result,
                Err(err) => (None, err),
            };
            round.check = Some(GoalCheck {
                code: result.0,
                output: ellipsis_tail(&result.1, OUTPUT_TAIL),
                at: (self.now)(),
            });
        }
        if let Some(stored) = goal.rounds.last_mut() {
            *stored = round.clone();
        }
        let plan_next = decide(&mut goal, &round);
        self.save(&mut goal)?;
        if plan_next {
            self.spawn_plan(id.to_string());
        }
        Ok(())
    }

    fn require(&self, id: &str) -> Result<Goal, LeadError> {
        self.get(id)?
            .ok_or_else(|| LeadError::message("That goal is gone."))
    }

    fn save(&self, goal: &mut Goal) -> Result<(), LeadError> {
        goal.updated_at = (self.now)();
        let data =
            serde_json::to_string(goal).map_err(|err| LeadError::message(err.to_string()))?;
        self.store
            .save_goal(&goal.id, &goal.project, goal.updated_at, &data)?;
        if let Some(on_update) = &self.on_update {
            on_update(goal);
        }
        Ok(())
    }
}

fn decide(goal: &mut Goal, round: &GoalRound) -> bool {
    if round
        .check
        .as_ref()
        .is_some_and(|check| check.code == Some(0))
    {
        goal.status = GoalStatus::Done;
        goal.outcome = Some(format!(
            "The check passed after round {}. Review the goal in Changes.",
            round.n
        ));
        return false;
    }
    if goal.quick && goal.check.is_none() {
        goal.status = GoalStatus::Done;
        goal.outcome = Some("Done. Review it in Changes.".to_string());
        return false;
    }
    let previous = goal
        .rounds
        .len()
        .checked_sub(2)
        .and_then(|index| goal.rounds.get(index));
    if let (Some(check), Some(previous)) = (&round.check, previous)
        && let Some(earlier) = &previous.check
        && earlier.code != Some(0)
        && failure_key(&earlier.output) == failure_key(&check.output)
    {
        goal.status = GoalStatus::Waiting;
        goal.question = Some(
            "The check failed the same way twice. Tell the lead what to try, or stop the goal."
                .to_string(),
        );
        return false;
    }
    if goal.rounds.len() as i64 >= goal.max_rounds {
        goal.status = GoalStatus::Stopped;
        goal.outcome = Some(format!(
            "Reached the limit of {} rounds. The work so far is in Changes; answer to give it more.",
            goal.max_rounds
        ));
        return false;
    }
    goal.status = GoalStatus::Planning;
    true
}

fn cap_access(access: JobAccess, ceiling: JobAccess) -> JobAccess {
    let rank = |access: JobAccess| match access {
        JobAccess::Read => 0,
        JobAccess::Edit => 1,
        JobAccess::Full => 2,
    };
    let capped = if rank(access) > rank(ceiling) {
        ceiling
    } else {
        access
    };
    if capped == JobAccess::Read {
        JobAccess::Edit
    } else {
        capped
    }
}

fn clamp_rounds(value: f64) -> i64 {
    if !value.is_finite() {
        return 5;
    }
    (value.round() as i64).clamp(1, 20)
}

pub fn plan_prompt(goal: &Goal) -> String {
    let mut lines = vec![
        "You plan the next round of work towards a goal in Legate. This folder is the goal's own worktree with every earlier round committed. Read the code as you need; do not change anything.".to_string(),
        String::new(),
        format!("Goal: {}", goal.text),
        if let Some(check) = &goal.check {
            format!("Done when this command exits 0 in this folder: {check}")
        } else {
            "Done when you judge the goal met.".to_string()
        },
        format!(
            "This is round {} of at most {}.",
            goal.rounds.len() + 1,
            goal.max_rounds
        ),
    ];
    if !goal.rounds.is_empty() {
        lines.push(String::new());
        lines.push("Earlier rounds:".to_string());
        for round in &goal.rounds {
            lines.push(format!("Round {}: {}", round.n, round.summary));
            for job in &round.jobs {
                lines.push(format!(
                    "- {}, {}: {}",
                    job.job_type.as_str(),
                    job.access.as_str(),
                    first_line(&job.prompt, 200)
                ));
            }
            for problem in &round.problems {
                lines.push(format!("  problem: {problem}"));
            }
            if let Some(check) = &round.check {
                let code = check
                    .code
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "null".to_string());
                lines.push(format!("  check exited {code}:"));
                lines.push(indent(&ellipsis_tail(&check.output, 1500)));
            }
        }
    }
    if !goal.notes.is_empty() {
        lines.push(String::new());
        lines.push("From the person:".to_string());
        for note in &goal.notes {
            lines.push(format!("- {note}"));
        }
    }
    lines.push(String::new());
    lines.push("Answer with exactly one JSON block, one of:".to_string());
    lines.push("```json".to_string());
    lines.push(
        "{\"decision\": \"jobs\", \"summary\": \"<one line: what this round does>\", \"jobs\": [{\"type\": \"refactor\", \"access\": \"edit\", \"prompt\": \"<what to do>\"}]}"
            .to_string(),
    );
    lines.push("{\"decision\": \"done\", \"summary\": \"<why the goal is met>\"}".to_string());
    lines.push(
        "{\"decision\": \"ask\", \"question\": \"<what you need from the person>\"}".to_string(),
    );
    lines.push("```".to_string());
    lines.push(format!(
        "Rules: at most {MAX_JOBS_PER_ROUND} jobs; jobs in one round must not edit the same files; each prompt stands alone (the worker sees nothing else) and names the files; keep rounds small; type is planning, debugging, review, refactor or boilerplate; access is edit, or full only when the job must run commands such as tests."
    ));
    lines.join("\n")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedPlan {
    Jobs {
        summary: String,
        jobs: Vec<PlannedJob>,
    },
    Done {
        summary: String,
    },
    Ask {
        question: String,
    },
}

pub fn parse_plan(text: &str) -> Option<ParsedPlan> {
    let fenced = fenced_blocks(text);
    let candidates = if fenced.is_empty() {
        let start = text.find('{')?;
        let end = text.rfind('}')?;
        if end < start {
            return None;
        }
        vec![text[start..=end].to_string()]
    } else {
        fenced.into_iter().rev().collect()
    };
    for candidate in candidates {
        let Ok(value) = serde_json::from_str::<Value>(candidate.trim()) else {
            continue;
        };
        if let Some(plan) = read_plan(&value) {
            return Some(plan);
        }
    }
    None
}

fn fenced_blocks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("```") {
        let mut after = &rest[start + 3..];
        if let Some(stripped) = after.strip_prefix("json") {
            after = stripped;
        }
        after = after.trim_start_matches([' ', '\t', '\n', '\r', '\u{000c}', '\u{000b}']);
        let Some(end) = after.find("```") else {
            break;
        };
        out.push(after[..end].to_string());
        rest = &after[end + 3..];
    }
    out
}

fn read_plan(value: &Value) -> Option<ParsedPlan> {
    let record = value.as_object()?;
    let summary = record
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    match record.get("decision").and_then(Value::as_str)? {
        "done" => Some(ParsedPlan::Done { summary }),
        "ask" => {
            let question = record
                .get("question")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            if question.is_empty() {
                None
            } else {
                Some(ParsedPlan::Ask { question })
            }
        }
        "jobs" => {
            let raw = record.get("jobs")?.as_array()?;
            let mut jobs = Vec::new();
            for item in raw {
                let Some(job) = item.as_object() else {
                    continue;
                };
                let prompt = job
                    .get("prompt")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if prompt.is_empty() {
                    continue;
                }
                let job_type = job
                    .get("type")
                    .and_then(Value::as_str)
                    .and_then(JobType::parse)
                    .unwrap_or(JobType::Refactor);
                let access = match job
                    .get("access")
                    .and_then(Value::as_str)
                    .and_then(JobAccess::parse)
                {
                    Some(access) if access != JobAccess::Read => access,
                    _ => JobAccess::Edit,
                };
                jobs.push(PlannedJob {
                    job_type,
                    access,
                    prompt,
                });
            }
            if jobs.is_empty() {
                return None;
            }
            jobs.truncate(MAX_JOBS_PER_ROUND);
            Some(ParsedPlan::Jobs {
                summary: if summary.is_empty() {
                    "Next round".to_string()
                } else {
                    summary
                },
                jobs,
            })
        }
        _ => None,
    }
}

pub fn failure_key(output: &str) -> String {
    let lines: Vec<&str> = split_lines(output)
        .into_iter()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let start = lines.len().saturating_sub(15);
    let joined = lines[start..].join("\n");
    let chars: Vec<char> = joined.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index].is_ascii_digit() {
            let mut next = index + 1;
            while next < chars.len() && chars[next].is_ascii_digit() {
                next += 1;
            }
            if next < chars.len() && chars[next] == '.' {
                let mut fraction = next + 1;
                if fraction < chars.len() && chars[fraction].is_ascii_digit() {
                    while fraction < chars.len() && chars[fraction].is_ascii_digit() {
                        fraction += 1;
                    }
                    next = fraction;
                }
            }
            out.push('#');
            index = next;
        } else {
            out.push(chars[index].to_ascii_lowercase());
            index += 1;
        }
    }
    out
}

fn parse_goal(raw: &str) -> Option<Goal> {
    let value: Value = serde_json::from_str(raw).ok()?;
    let object = value.as_object()?;
    if !object.get("id").is_some_and(Value::is_string)
        || !object.get("rounds").is_some_and(Value::is_array)
    {
        return None;
    }
    serde_json::from_value(value).ok()
}

pub(crate) fn system_now() -> i64 {
    legate_router::system_now_ms()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use async_trait::async_trait;
    use legate_core::{JobAccess, JobRecord, JobStatus, JobType};

    use super::{
        ApplyResult, CreatedTree, Goal, GoalChecks, GoalDeps, GoalExtra, GoalProposal, GoalStatus,
        GoalStore, GoalWorktrees, Goals, NewGoal, ParsedPlan, PlannedJob, QuickJob, failure_key,
        parse_plan, plan_prompt,
    };
    use crate::error::LeadError;

    struct Mem {
        rows: RefCell<HashMap<String, (String, i64, String)>>,
    }

    impl GoalStore for Mem {
        fn save_goal(
            &self,
            id: &str,
            project: &str,
            updated_at: i64,
            data: &str,
        ) -> Result<(), LeadError> {
            self.rows.borrow_mut().insert(
                id.to_string(),
                (project.to_string(), updated_at, data.to_string()),
            );
            Ok(())
        }

        fn goals(&self, project: &str) -> Result<Vec<String>, LeadError> {
            let mut rows: Vec<_> = self
                .rows
                .borrow()
                .values()
                .filter(|row| row.0 == project)
                .cloned()
                .collect();
            rows.sort_by_key(|row| std::cmp::Reverse(row.1));
            Ok(rows.into_iter().map(|row| row.2).collect())
        }

        fn goal(&self, id: &str) -> Result<Option<String>, LeadError> {
            Ok(self.rows.borrow().get(id).map(|row| row.2.clone()))
        }

        fn delete_goal(&self, id: &str) -> Result<(), LeadError> {
            self.rows.borrow_mut().remove(id);
            Ok(())
        }
    }

    struct Trees {
        applied: RefCell<Vec<String>>,
        commits: RefCell<Vec<String>>,
        discarded: RefCell<Vec<String>>,
    }

    #[async_trait(?Send)]
    impl GoalWorktrees for Trees {
        async fn create(&self, _project: &str, id: &str) -> Result<CreatedTree, String> {
            Ok(CreatedTree {
                id: format!("g-{}", &id[..4.min(id.len())]),
                path: "/wt/goal".into(),
                branch: "orch/goal".into(),
            })
        }

        fn folder(&self, _id: &str) -> String {
            "/wt/goal".into()
        }

        fn branch_of(&self, _id: &str) -> String {
            "orch/goal".into()
        }

        async fn apply_into(
            &self,
            from_id: &str,
            _target: &str,
            _base: &str,
        ) -> Result<ApplyResult, String> {
            self.applied.borrow_mut().push(from_id.to_string());
            Ok(ApplyResult {
                ok: true,
                empty: false,
                message: "applied".into(),
            })
        }

        async fn commit_all(&self, _dir: &str, message: &str) -> Result<bool, String> {
            self.commits.borrow_mut().push(message.to_string());
            Ok(true)
        }

        async fn discard(&self, _project: &str, id: &str) -> Result<(), String> {
            self.discarded.borrow_mut().push(id.to_string());
            Ok(())
        }
    }

    struct Planner {
        replies: RefCell<Vec<String>>,
        plans: RefCell<Vec<String>>,
    }

    #[async_trait(?Send)]
    impl super::GoalPlanner for Planner {
        async fn plan(&self, _goal: &Goal, _folder: &str, prompt: &str) -> Result<String, String> {
            self.plans.borrow_mut().push(prompt.to_string());
            self.replies
                .borrow_mut()
                .pop()
                .ok_or_else(|| "no reply".to_string())
        }
    }

    struct Checks {
        checks: RefCell<Vec<(Option<i32>, String)>>,
    }

    #[async_trait(?Send)]
    impl GoalChecks for Checks {
        async fn run(
            &self,
            _command: &str,
            _folder: &str,
        ) -> Result<(Option<i32>, String), String> {
            Ok(self
                .checks
                .borrow_mut()
                .pop()
                .unwrap_or((Some(0), "ok".to_string())))
        }
    }

    type Submitted = Rc<RefCell<Vec<(PlannedJob, String, String, String)>>>;

    struct Harness {
        goals: Rc<Goals>,
        jobs: Rc<RefCell<HashMap<String, JobRecord>>>,
        submitted: Submitted,
        trees: Rc<Trees>,
        planner: Rc<Planner>,
    }

    fn drive(fut: impl std::future::Future<Output = ()> + 'static) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move { tokio::task::LocalSet::new().run_until(fut).await });
    }

    async fn flush() {
        for _ in 0..30 {
            tokio::task::yield_now().await;
        }
    }

    fn record(id: &str, job: &PlannedJob, extra: &GoalExtra) -> JobRecord {
        JobRecord {
            id: id.to_string(),
            job_type: job.job_type,
            prompt: job.prompt.clone(),
            provider: None,
            status: JobStatus::Running,
            output: String::new(),
            failed_over: Vec::new(),
            steps: None,
            steps_dropped: None,
            model: None,
            reason: None,
            edit: None,
            access: Some(job.access),
            change: None,
            group: None,
            lead_message: None,
            routine: None,
            title: None,
            session_id: None,
            goal: Some(extra.goal.clone()),
            base: Some(extra.base.clone()),
            error: None,
            owner_id: None,
            heartbeat_at: None,
            cancel_requested: None,
            extra: Default::default(),
        }
    }

    fn harness(
        replies: Vec<String>,
        checks: Vec<(Option<i32>, String)>,
        ceiling: JobAccess,
    ) -> Harness {
        let jobs = Rc::new(RefCell::new(HashMap::new()));
        let submitted = Rc::new(RefCell::new(Vec::new()));
        let jobs_submit = Rc::clone(&jobs);
        let submitted_submit = Rc::clone(&submitted);
        let next = Rc::new(RefCell::new(0usize));
        let trees = Rc::new(Trees {
            applied: RefCell::new(Vec::new()),
            commits: RefCell::new(Vec::new()),
            discarded: RefCell::new(Vec::new()),
        });
        let planner = Rc::new(Planner {
            replies: RefCell::new(replies),
            plans: RefCell::new(Vec::new()),
        });
        // Replies are shifted from the front in TS. Vec::pop takes the back, so reverse.
        planner.replies.borrow_mut().reverse();
        let goals = Goals::new(GoalDeps {
            store: Rc::new(Mem {
                rows: RefCell::new(HashMap::new()),
            }),
            worktrees: trees.clone(),
            submit: Rc::new(move |job, extra| {
                *next.borrow_mut() += 1;
                let id = format!("job-{}", *next.borrow());
                let row = record(&id, &job, &extra);
                jobs_submit.borrow_mut().insert(id, row.clone());
                submitted_submit
                    .borrow_mut()
                    .push((job, extra.goal, extra.base, extra.project));
                Ok(row)
            }),
            job: {
                let jobs = Rc::clone(&jobs);
                Rc::new(move |id| jobs.borrow().get(id).cloned())
            },
            cancel: {
                let jobs = Rc::clone(&jobs);
                Rc::new(move |id| {
                    if let Some(job) = jobs.borrow_mut().get_mut(id) {
                        job.status = JobStatus::Failed;
                    }
                })
            },
            planner: planner.clone(),
            checks: Rc::new(Checks {
                checks: RefCell::new({
                    let mut rows = checks;
                    rows.reverse();
                    rows
                }),
            }),
            ceiling: Rc::new(move || ceiling),
            name_worktree: None,
            on_update: None,
            now: Some(Rc::new(|| 1)),
        });
        Harness {
            goals,
            jobs,
            submitted,
            trees,
            planner,
        }
    }

    fn finish(h: &Harness, id: &str, status: JobStatus, change: Option<&str>) {
        let mut jobs = h.jobs.borrow_mut();
        let job = jobs.get_mut(id).unwrap();
        job.status = status;
        if let Some(change) = change {
            job.change = Some(change.to_string());
        }
        if status == JobStatus::Failed {
            job.error = Some("boom".to_string());
        }
        let snap = job.clone();
        drop(jobs);
        h.goals.on_job(Some(&snap), None).unwrap();
    }

    fn jobs_json(prompt: &str, access: &str) -> String {
        format!(
            "Here is the plan.\n```json\n{}\n```",
            serde_json::json!({
                "decision": "jobs",
                "summary": format!("do {prompt}"),
                "jobs": [{ "type": "refactor", "access": access, "prompt": prompt }]
            })
        )
    }

    #[test]
    fn parse_plan_reads_jobs_done_and_ask() {
        match parse_plan(&jobs_json("fix login", "edit")).unwrap() {
            ParsedPlan::Jobs { summary, jobs } => {
                assert_eq!(summary, "do fix login");
                assert_eq!(jobs[0].job_type, JobType::Refactor);
                assert_eq!(jobs[0].access, JobAccess::Edit);
                assert_eq!(jobs[0].prompt, "fix login");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            parse_plan(r#"{"decision":"done","summary":"all green"}"#),
            Some(ParsedPlan::Done {
                summary: "all green".into()
            })
        );
        assert_eq!(
            parse_plan("```json\n{\"decision\":\"ask\",\"question\":\"Which file?\"}\n```"),
            Some(ParsedPlan::Ask {
                question: "Which file?".into()
            })
        );
        assert!(parse_plan("I think we should refactor.").is_none());
        assert!(parse_plan(r#"{"decision":"jobs","jobs":[]}"#).is_none());
        let many = parse_plan(
            &serde_json::json!({
                "decision": "jobs",
                "jobs": (1..=5).map(|n| serde_json::json!({
                    "type": "nope",
                    "access": "read",
                    "prompt": format!("p{n}")
                })).collect::<Vec<_>>()
            })
            .to_string(),
        );
        let ParsedPlan::Jobs { jobs, .. } = many.unwrap() else {
            panic!("jobs");
        };
        assert_eq!(
            jobs.iter()
                .map(|job| job.prompt.as_str())
                .collect::<Vec<_>>(),
            vec!["p1", "p2", "p3"]
        );
        assert!(
            jobs.iter()
                .all(|job| job.job_type == JobType::Refactor && job.access == JobAccess::Edit)
        );
    }

    #[test]
    fn failure_key_ignores_numbers_such_as_timings() {
        assert_eq!(
            failure_key("FAIL auth.test.ts\nexpected 1 got 2\n(12ms)"),
            failure_key("FAIL auth.test.ts\nexpected 3 got 4\n(80ms)")
        );
        assert_ne!(
            failure_key("FAIL auth.test.ts"),
            failure_key("FAIL files.test.ts")
        );
    }

    #[test]
    fn a_quick_task_runs_the_prompt_and_ends_when_the_check_passes() {
        drive(async {
            let h = harness(
                Vec::new(),
                vec![(Some(0), "all passed".into())],
                JobAccess::Full,
            );
            let goal = h
                .goals
                .create(NewGoal {
                    project: "/p".into(),
                    text: "Fix the off-by-one in pager.ts".into(),
                    check: Some("pnpm test -- pager".into()),
                    max_rounds: None,
                    quick: Some(QuickJob {
                        job_type: JobType::Debugging,
                        access: JobAccess::Edit,
                    }),
                    source: None,
                })
                .await
                .unwrap();
            assert!(h.planner.plans.borrow().is_empty());
            assert_eq!(goal.status, GoalStatus::Running);
            {
                let submitted = h.submitted.borrow();
                assert_eq!(submitted[0].1, goal.id);
                assert_eq!(submitted[0].2, "orch/goal");
                assert_eq!(submitted[0].3, "/p");
            }
            finish(&h, "job-1", JobStatus::Done, Some("change-1"));
            flush().await;
            assert_eq!(h.trees.applied.borrow().as_slice(), ["change-1"]);
            assert_eq!(h.trees.discarded.borrow().as_slice(), ["change-1"]);
            assert_eq!(
                h.trees.commits.borrow().as_slice(),
                ["Goal round 1: Your task"]
            );
            let done = h.goals.get(&goal.id).unwrap().unwrap();
            assert_eq!(done.status, GoalStatus::Done);
            assert_eq!(done.rounds[0].check.as_ref().unwrap().code, Some(0));
            assert_eq!(done.rounds[0].check.as_ref().unwrap().output, "all passed");
        });
    }

    #[test]
    fn a_quick_task_without_a_check_is_done_after_its_one_job() {
        drive(async {
            let h = harness(Vec::new(), Vec::new(), JobAccess::Full);
            let goal = h
                .goals
                .create(NewGoal {
                    project: "/p".into(),
                    text: "Rename getUser to loadUser".into(),
                    check: None,
                    max_rounds: None,
                    quick: Some(QuickJob {
                        job_type: JobType::Refactor,
                        access: JobAccess::Edit,
                    }),
                    source: None,
                })
                .await
                .unwrap();
            finish(&h, "job-1", JobStatus::Done, Some("change-1"));
            flush().await;
            assert_eq!(
                h.goals.get(&goal.id).unwrap().unwrap().status,
                GoalStatus::Done
            );
        });
    }

    #[test]
    fn a_planned_goal_waits_and_stops_when_the_check_fails_the_same_way_twice() {
        drive(async {
            let h = harness(
                vec![
                    jobs_json("make login keep the session", "edit"),
                    jobs_json("fix the failing test", "edit"),
                ],
                vec![
                    (
                        Some(1),
                        "FAIL session.test.ts\nexpected token 1 (12ms)".into(),
                    ),
                    (
                        Some(1),
                        "FAIL session.test.ts\nexpected token 2 (40ms)".into(),
                    ),
                ],
                JobAccess::Full,
            );
            let goal = h
                .goals
                .create(NewGoal {
                    project: "/p".into(),
                    text: "Keep the session".into(),
                    check: Some("pnpm test".into()),
                    max_rounds: None,
                    quick: None,
                    source: None,
                })
                .await
                .unwrap();
            flush().await;
            let now = h.goals.get(&goal.id).unwrap().unwrap();
            assert_eq!(now.status, GoalStatus::Waiting);
            assert_eq!(
                now.proposal.as_ref().unwrap().jobs[0].prompt,
                "make login keep the session"
            );
            assert!(h.submitted.borrow().is_empty());
            h.goals.approve(&goal.id, None).unwrap();
            finish(&h, "job-1", JobStatus::Done, Some("change-1"));
            flush().await;
            let now = h.goals.get(&goal.id).unwrap().unwrap();
            assert_eq!(now.status, GoalStatus::Waiting);
            assert_eq!(
                now.proposal.as_ref().unwrap().summary,
                "do fix the failing test"
            );
            {
                let plans = h.planner.plans.borrow();
                assert!(plans[1].contains("check exited 1"));
                assert!(plans[1].contains("FAIL session.test.ts"));
            }
            h.goals
                .approve(
                    &goal.id,
                    Some(vec![PlannedJob {
                        job_type: JobType::Debugging,
                        access: JobAccess::Full,
                        prompt: "my own fix".into(),
                    }]),
                )
                .unwrap();
            assert_eq!(h.submitted.borrow()[1].0.prompt, "my own fix");
            finish(&h, "job-2", JobStatus::Done, Some("change-2"));
            flush().await;
            let now = h.goals.get(&goal.id).unwrap().unwrap();
            assert_eq!(now.status, GoalStatus::Waiting);
            assert!(
                now.question
                    .as_deref()
                    .unwrap()
                    .contains("failed the same way twice")
            );
            assert!(now.proposal.is_none());
        });
    }

    #[test]
    fn the_round_limit_stops_a_goal_and_an_answer_gives_one_more_round() {
        drive(async {
            let h = harness(
                vec![jobs_json("first", "edit"), jobs_json("second", "edit")],
                vec![(Some(1), "one error".into())],
                JobAccess::Full,
            );
            let goal = h
                .goals
                .create(NewGoal {
                    project: "/p".into(),
                    text: "Goal".into(),
                    check: Some("make".into()),
                    max_rounds: Some(1.0),
                    quick: None,
                    source: None,
                })
                .await
                .unwrap();
            flush().await;
            h.goals.approve(&goal.id, None).unwrap();
            finish(&h, "job-1", JobStatus::Done, Some("c1"));
            flush().await;
            let stopped = h.goals.get(&goal.id).unwrap().unwrap();
            assert_eq!(stopped.status, GoalStatus::Stopped);
            assert!(
                stopped
                    .outcome
                    .as_deref()
                    .unwrap()
                    .contains("limit of 1 rounds")
            );
            h.goals.answer(&goal.id, "Try the other file").unwrap();
            flush().await;
            let again = h.goals.get(&goal.id).unwrap().unwrap();
            assert_eq!(again.status, GoalStatus::Waiting);
            assert_eq!(again.max_rounds, 2);
            assert!(h.planner.plans.borrow()[1].contains("Try the other file"));
        });
    }

    #[test]
    fn an_answer_without_a_plan_a_failed_planner_and_stop_leave_the_goal_readable() {
        drive(async {
            let h = harness(
                vec!["I would refactor things.".into()],
                Vec::new(),
                JobAccess::Full,
            );
            let goal = h
                .goals
                .create(NewGoal {
                    project: "/p".into(),
                    text: "Goal".into(),
                    check: None,
                    max_rounds: None,
                    quick: None,
                    source: None,
                })
                .await
                .unwrap();
            flush().await;
            assert!(
                h.goals
                    .get(&goal.id)
                    .unwrap()
                    .unwrap()
                    .question
                    .unwrap()
                    .contains("did not include a plan")
            );
            h.goals.answer(&goal.id, "Plan it").unwrap();
            flush().await;
            assert!(
                h.goals
                    .get(&goal.id)
                    .unwrap()
                    .unwrap()
                    .question
                    .unwrap()
                    .contains("could not plan")
            );
            assert_eq!(h.goals.stop(&goal.id).unwrap().status, GoalStatus::Stopped);
            h.goals.remove(&goal.id).await.unwrap();
            assert!(h.goals.get(&goal.id).unwrap().is_none());
        });
    }

    #[test]
    fn jobs_are_capped_at_the_ceiling_and_read_becomes_edit() {
        drive(async {
            let h = harness(
                vec![jobs_json("run the tests", "full")],
                Vec::new(),
                JobAccess::Edit,
            );
            let goal = h
                .goals
                .create(NewGoal {
                    project: "/p".into(),
                    text: "Goal".into(),
                    check: None,
                    max_rounds: None,
                    quick: None,
                    source: None,
                })
                .await
                .unwrap();
            flush().await;
            h.goals.approve(&goal.id, None).unwrap();
            assert_eq!(h.submitted.borrow()[0].0.access, JobAccess::Edit);
            let _ = GoalProposal {
                summary: String::new(),
                jobs: Vec::new(),
            };
        });
    }

    #[test]
    fn a_failed_job_is_reported_to_the_lead_as_a_problem() {
        drive(async {
            let h = harness(
                vec![jobs_json("one", "edit"), jobs_json("two", "edit")],
                vec![(Some(1), "x".into())],
                JobAccess::Full,
            );
            let goal = h
                .goals
                .create(NewGoal {
                    project: "/p".into(),
                    text: "Goal".into(),
                    check: Some("make".into()),
                    max_rounds: None,
                    quick: None,
                    source: None,
                })
                .await
                .unwrap();
            flush().await;
            h.goals.approve(&goal.id, None).unwrap();
            finish(&h, "job-1", JobStatus::Failed, None);
            flush().await;
            let stored = h.goals.get(&goal.id).unwrap().unwrap();
            assert!(stored.rounds[0].problems[0].contains("boom"));
            assert!(plan_prompt(&stored).contains("problem: one: boom"));
        });
    }
}
