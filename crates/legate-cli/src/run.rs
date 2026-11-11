//! Command dispatch from `src/cli/run.ts`.

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use legate_core::{ChangeSet, JobAccess, JobRecord, JobStatus, JobType, ProviderId};
use legate_git::{GitError, GuardJob, ListedName, assert_worktree_idle};
use legate_providers::TESTED_VERSIONS;
use tokio::sync::{Notify, mpsc};

use crate::args::{self, Parsed};
use crate::drive::{Drive, DriveError, GoalRequest, JobRequest, Listener};
use crate::format::{
    JobCells, ProposalJob, age_ms, change_cells, change_footer, check_report, footer_line,
    goal_cells, header_from_job, job_cells, json_job, one_sentence, plan_line, proposal_text,
    status_word, step_line, table,
};
use crate::help::{self, usage_for};
use crate::tui;

const LOOKUP: i64 = 1000;

enum CliError {
    /// The command already printed its usage or result.
    Code(i32),
    Immediate(i32),
    Message(String),
}

impl From<DriveError> for CliError {
    fn from(err: DriveError) -> Self {
        Self::Message(err.message)
    }
}

impl From<GitError> for CliError {
    fn from(err: GitError) -> Self {
        Self::Message(err.to_string())
    }
}

struct RunFlags {
    json: bool,
    quiet: bool,
    verbose: bool,
}

pub async fn run<D: Drive>(argv: &[String], drive: &D) -> i32 {
    match run_inner(argv, drive).await {
        Ok(code) => code,
        Err(CliError::Code(code) | CliError::Immediate(code)) => code,
        Err(CliError::Message(text)) => {
            drive.stderr(&format!("{}\n", one_sentence(&text)));
            1
        }
    }
}

async fn run_inner<D: Drive>(argv: &[String], drive: &D) -> Result<i32, CliError> {
    if let Some(early) = help::early_exit(argv, drive.version(), drive.tty()) {
        if early.stderr {
            drive.stderr(&early.text);
        } else {
            drive.stdout(&early.text);
        }
        return Ok(early.code);
    }
    let parsed = match args::parse_args(argv) {
        Ok(parsed) => parsed,
        Err(err) => {
            drive.stderr(&format!(
                "{}\n{}\n",
                err.message,
                usage_for(err.command.as_deref())
            ));
            return Ok(2);
        }
    };
    dispatch(parsed, drive).await
}

async fn dispatch<D: Drive>(parsed: Parsed, drive: &D) -> Result<i32, CliError> {
    let Some(command) = parsed.command.as_deref() else {
        return open_ui(parsed, drive).await;
    };
    match command {
        "ui" => open_ui(parsed, drive).await,
        "status" | "plans" => {
            if unexpected(&parsed.args, command, drive) {
                return Ok(2);
            }
            status(drive).await
        }
        "job" => submit_job(parsed, drive).await,
        "jobs" => list_jobs(parsed, drive),
        "show" => show_job(parsed, drive),
        "changes" => {
            if unexpected(&parsed.args, "changes", drive) {
                return Ok(2);
            }
            print_changes(drive).await
        }
        "diff" => diff_change(parsed, drive).await,
        "merge" => merge_change(parsed, drive).await,
        "discard" => discard_change(parsed, drive).await,
        "pr" => open_pr(parsed, drive).await,
        "worktree" => worktree(parsed, drive).await,
        "routine" => routine(parsed, drive).await,
        "goal" => run_goal(parsed, drive).await,
        "goals" => list_goals(parsed, drive).await,
        _ => {
            drive.stderr(&format!(
                "Unknown command \"{command}\".\n{}\n",
                usage_for(None)
            ));
            Ok(2)
        }
    }
}

async fn open_ui<D: Drive>(parsed: Parsed, drive: &D) -> Result<i32, CliError> {
    if !drive.tty() {
        drive.stderr(&format!("Missing command.\n{}\n", usage_for(None)));
        return Ok(2);
    }
    if unexpected(&parsed.args, "ui", drive) {
        return Ok(2);
    }
    Ok(tui::open(drive).await)
}

fn tested_of(id: ProviderId) -> &'static str {
    match id {
        ProviderId::Claude => TESTED_VERSIONS.claude,
        ProviderId::Codex => TESTED_VERSIONS.codex,
        ProviderId::Cursor => TESTED_VERSIONS.cursor,
        ProviderId::Grok => TESTED_VERSIONS.grok,
        ProviderId::Local => "",
    }
}

async fn status<D: Drive>(drive: &D) -> Result<i32, CliError> {
    drive.init().await?;
    let now = drive.now();
    let plans = drive.plans().await?;
    let mut lines: Vec<String> = plans
        .iter()
        .map(|plan| plan_line(plan, tested_of(plan.id), now, drive.color()))
        .collect();
    lines.push(drive.project());
    let active: Vec<_> = drive
        .jobs_newest(LOOKUP)
        .into_iter()
        .filter(|row| matches!(row.job.status, JobStatus::Running | JobStatus::Queued))
        .collect();
    if !active.is_empty() {
        let rows: Vec<Vec<String>> = active.iter().map(|row| job_row(now, row)).collect();
        lines.push(table(&rows));
    }
    drive.stdout(&format!("{}\n", lines.join("\n")));
    Ok(0)
}

async fn submit_job<D: Drive>(parsed: Parsed, drive: &D) -> Result<i32, CliError> {
    if parsed.args.len() > 1 {
        unexpected(&parsed.args[1..], "job", drive);
        return Ok(2);
    }
    let type_name = parsed.flags.type_name.as_deref().unwrap_or("planning");
    let Some(job_type) = JobType::parse(type_name) else {
        drive.stderr(&format!(
            "Unknown job type \"{type_name}\".\n{}\n",
            usage_for(Some("job"))
        ));
        return Ok(2);
    };
    let Some(worker) = parse_worker(parsed.flags.worker.as_deref()) else {
        drive.stderr(&format!(
            "Worker must be auto, claude, codex, cursor, grok, or local.\n{}\n",
            usage_for(Some("job"))
        ));
        return Ok(2);
    };
    let access_name = parsed.flags.access.as_deref().unwrap_or("read");
    let Some(access) = JobAccess::parse(access_name) else {
        drive.stderr(&format!(
            "Access must be read, edit, or full.\n{}\n",
            usage_for(Some("job"))
        ));
        return Ok(2);
    };
    let Some(prompt) = read_prompt(drive, parsed.args.first().map(String::as_str)).await else {
        drive.stderr(&format!("Missing prompt.\n{}\n", usage_for(Some("job"))));
        return Ok(2);
    };
    if prompt.trim().is_empty() {
        drive.stderr(&format!("Empty prompt.\n{}\n", usage_for(Some("job"))));
        return Ok(2);
    }
    drive.init().await?;
    follow(
        drive,
        Begin::Job(JobRequest {
            job_type,
            prompt: prompt.trim().to_string(),
            provider: worker,
            access,
        }),
        run_flags(&parsed),
    )
    .await
}

fn list_jobs<D: Drive>(parsed: Parsed, drive: &D) -> Result<i32, CliError> {
    if unexpected(&parsed.args, "jobs", drive) {
        return Ok(2);
    }
    let Some(limit) = parse_limit(parsed.flags.limit.as_deref()) else {
        drive.stderr(&format!(
            "Limit must be a positive whole number.\n{}\n",
            usage_for(Some("jobs"))
        ));
        return Ok(2);
    };
    let now = drive.now();
    let rows = drive.jobs_newest(limit);
    if rows.is_empty() {
        drive.stderr("No jobs yet. Start one with: legate job \"<what to do>\"\n");
        return Ok(0);
    }
    let cells: Vec<Vec<String>> = rows.iter().map(|row| job_row(now, row)).collect();
    drive.stdout(&format!("{}\n", table(&cells)));
    Ok(0)
}

fn show_job<D: Drive>(parsed: Parsed, drive: &D) -> Result<i32, CliError> {
    let Some(token) = parsed.args.first() else {
        drive.stderr(&format!("Missing job id.\n{}\n", usage_for(Some("show"))));
        return Ok(2);
    };
    if unexpected(&parsed.args[1..], "show", drive) {
        return Ok(2);
    }
    let rows = drive.jobs_newest(LOOKUP);
    let items: Vec<legate_core::IdItem> = rows
        .iter()
        .map(|row| legate_core::IdItem::new(row.job.id.clone()))
        .collect();
    let hit = match legate_core::resolve_id(token, &items, "Job") {
        legate_core::IdHit::Found { id } => id,
        legate_core::IdHit::Rejected { message } => {
            drive.stderr(&format!("{message}\n"));
            return Ok(2);
        }
    };
    let Some(row) = rows.into_iter().find(|item| item.job.id == hit) else {
        drive.stderr(&format!("No job matches \"{token}\".\n"));
        return Ok(2);
    };
    let job = row.job;
    let mut lines = vec![header_from_job(&job, drive.color())];
    for step in job.steps.iter().flatten() {
        if let Some(line) = step_line(step, true, drive.color()) {
            lines.push(line);
        }
    }
    if !job.output.is_empty() {
        lines.push(job.output.clone());
    }
    if job.status == JobStatus::Failed {
        lines.push(format!(
            "failed: {}",
            job.error.as_deref().unwrap_or("failed")
        ));
    }
    drive.stdout(&format!("{}\n", lines.join("\n")));
    Ok(0)
}

async fn print_changes<D: Drive>(drive: &D) -> Result<i32, CliError> {
    let changes = list_merged(drive).await?;
    if changes.is_empty() {
        drive.stderr("No changes waiting. Edits from jobs and worktrees show up here.\n");
        return Ok(0);
    }
    let rows: Vec<Vec<String>> = changes.iter().map(change_row).collect();
    drive.stdout(&format!("{}\n", table(&rows)));
    Ok(0)
}

async fn diff_change<D: Drive>(parsed: Parsed, drive: &D) -> Result<i32, CliError> {
    let hit = one_change(&parsed, "diff", drive).await?;
    let text = drive.diff(&hit).await?;
    if text.is_empty() || text.ends_with('\n') {
        drive.stdout(&text);
    } else {
        drive.stdout(&format!("{text}\n"));
    }
    Ok(0)
}

async fn merge_change<D: Drive>(parsed: Parsed, drive: &D) -> Result<i32, CliError> {
    let hit = one_change(&parsed, "merge", drive).await?;
    idle(drive, &hit)?;
    let result = drive.merge(&hit).await?;
    if result.ok {
        drive.delete_worktree_name(&hit);
    }
    if result.ok {
        drive.stdout(&format!("{}\n", result.message));
        Ok(0)
    } else {
        drive.stderr(&format!("{}\n", result.message));
        Ok(1)
    }
}

async fn discard_change<D: Drive>(parsed: Parsed, drive: &D) -> Result<i32, CliError> {
    let hit = one_change(&parsed, "discard", drive).await?;
    if !parsed.flags.yes {
        if !drive.stdin_is_tty() {
            drive.stderr("Refusing to discard without --yes when input is not a terminal.\n");
            return Ok(2);
        }
        let answer = drive.ask(&format!("Discard {hit}? [y/N] ")).await;
        let answer = answer.trim().to_lowercase();
        if answer != "y" && answer != "yes" {
            drive.stdout(&format!("Kept {hit}.\n"));
            return Ok(0);
        }
    }
    idle(drive, &hit)?;
    drive.discard(&hit).await?;
    drive.delete_worktree_name(&hit);
    drive.stdout(&format!("Discarded {hit}.\n"));
    Ok(0)
}

async fn open_pr<D: Drive>(parsed: Parsed, drive: &D) -> Result<i32, CliError> {
    let Some(title) = parsed.args.first() else {
        drive.stderr(&format!("Missing title.\n{}\n", usage_for(Some("pr"))));
        return Ok(2);
    };
    if unexpected(&parsed.args[1..], "pr", drive) {
        return Ok(2);
    }
    let trimmed = title.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 120 || title_breaks(title) {
        drive.stderr(&format!(
            "Title must be 1 to 120 characters on one line.\n{}\n",
            usage_for(Some("pr"))
        ));
        return Ok(2);
    }
    let body = parsed.flags.body.clone().unwrap_or_default();
    if body.chars().count() > 4000 {
        drive.stderr(&format!(
            "Description must be at most 4000 characters.\n{}\n",
            usage_for(Some("pr"))
        ));
        return Ok(2);
    }
    let result = drive.open_pr(trimmed, &body).await?;
    drive.stdout(&format!("{}\n", result.url));
    Ok(0)
}

async fn worktree<D: Drive>(parsed: Parsed, drive: &D) -> Result<i32, CliError> {
    match parsed.sub.as_deref() {
        Some("list") => {
            if unexpected(&parsed.args, "worktree list", drive) {
                return Ok(2);
            }
            print_changes(drive).await
        }
        Some("path") => worktree_path(parsed, drive).await,
        Some("create") => worktree_create(parsed, drive).await,
        _ => {
            drive.stderr(&format!(
                "Unknown command \"worktree {}\".\n{}\n",
                parsed.sub.as_deref().unwrap_or(""),
                usage_for(Some("worktree"))
            ));
            Ok(2)
        }
    }
}

async fn worktree_path<D: Drive>(parsed: Parsed, drive: &D) -> Result<i32, CliError> {
    let hit = one_change(&parsed, "worktree path", drive).await?;
    drive.stdout(&format!("{}\n", drive.folder(&hit)));
    Ok(0)
}

async fn worktree_create<D: Drive>(parsed: Parsed, drive: &D) -> Result<i32, CliError> {
    if parsed.args.len() > 1 {
        unexpected(&parsed.args[1..], "worktree create", drive);
        return Ok(2);
    }
    let mut label = String::new();
    if let Some(given) = parsed.args.first() {
        let Some(name) = worktree_name(given) else {
            drive.stderr(&format!(
                "Name must be 1 to 40 characters on one line.\n{}\n",
                usage_for(Some("worktree create"))
            ));
            return Ok(2);
        };
        label = name;
    }
    let about = drive.project_info().await?;
    if !about.is_repo {
        drive.stderr("This folder is not a git repository, so it cannot have worktrees.\n");
        return Ok(1);
    }
    let created = drive.create_tree(&uuid::Uuid::new_v4().to_string()).await?;
    drive.set_worktree_name(ListedName {
        id: created.id.clone(),
        name: label,
        created_at: drive.now(),
    });
    drive.stdout(&format!("{}  {}\n", created.id, created.path));
    Ok(0)
}

async fn routine<D: Drive>(parsed: Parsed, drive: &D) -> Result<i32, CliError> {
    match parsed.sub.as_deref() {
        Some("list") => {
            if unexpected(&parsed.args, "routine list", drive) {
                return Ok(2);
            }
            let routines = drive.routines().await?;
            let rows: Vec<Vec<String>> = routines
                .iter()
                .map(|item| {
                    vec![
                        item.name.clone(),
                        item.job_type.as_str().to_string(),
                        item.worker.as_str().to_string(),
                        item.access.as_str().to_string(),
                    ]
                })
                .collect();
            if rows.is_empty() {
                drive.stderr("No routines yet. Make one in the app: panel → Routines.\n");
                return Ok(0);
            }
            drive.stdout(&format!("{}\n", table(&rows)));
            Ok(0)
        }
        Some("run") => routine_run(parsed, drive).await,
        _ => {
            drive.stderr(&format!(
                "Unknown command \"routine {}\".\n{}\n",
                parsed.sub.as_deref().unwrap_or(""),
                usage_for(Some("routine"))
            ));
            Ok(2)
        }
    }
}

async fn routine_run<D: Drive>(parsed: Parsed, drive: &D) -> Result<i32, CliError> {
    let name = parsed.args.first().map(|item| item.trim()).unwrap_or("");
    if name.is_empty() {
        drive.stderr(&format!(
            "Missing routine name.\n{}\n",
            usage_for(Some("routine run"))
        ));
        return Ok(2);
    }
    if unexpected(&parsed.args[1..], "routine run", drive) {
        return Ok(2);
    }
    let matches: Vec<_> = drive
        .routines()
        .await?
        .into_iter()
        .filter(|item| item.name == name)
        .collect();
    if matches.is_empty() {
        drive.stderr(&format!("No routine named \"{name}\".\n"));
        return Ok(2);
    }
    if matches.len() > 1 {
        let ids = matches
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>()
            .join(" and ");
        drive.stderr(&format!("Routine name \"{name}\" matches {ids}.\n"));
        return Ok(2);
    }
    let routine_id = matches[0].id.clone();
    drive.init().await?;
    follow(drive, Begin::Routine(routine_id), run_flags(&parsed)).await
}

async fn run_goal<D: Drive>(parsed: Parsed, drive: &D) -> Result<i32, CliError> {
    if parsed.args.len() > 1 {
        unexpected(&parsed.args[1..], "goal", drive);
        return Ok(2);
    }
    let Some(text) = parsed.args.first() else {
        drive.stderr(&format!("Missing goal.\n{}\n", usage_for(Some("goal"))));
        return Ok(2);
    };
    if text.trim().is_empty() {
        drive.stderr(&format!(
            "Say what the goal is.\n{}\n",
            usage_for(Some("goal"))
        ));
        return Ok(2);
    }
    let Some(rounds) = parse_rounds(parsed.flags.rounds.as_deref()) else {
        drive.stderr(&format!(
            "Rounds must be a whole number from 1 to 20.\n{}\n",
            usage_for(Some("goal"))
        ));
        return Ok(2);
    };
    drive.init().await?;
    follow_goal(
        drive,
        parsed.flags.yes,
        GoalRequest {
            project: drive.project(),
            text: text.trim().to_string(),
            check: parsed.flags.check.clone(),
            max_rounds: f64::from(rounds),
        },
    )
    .await
}

async fn list_goals<D: Drive>(parsed: Parsed, drive: &D) -> Result<i32, CliError> {
    if unexpected(&parsed.args, "goals", drive) {
        return Ok(2);
    }
    let mut rows = drive.goals().await?;
    rows.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    if rows.is_empty() {
        drive.stderr("No goals yet. Start one with: legate goal \"<what you want>\"\n");
        return Ok(0);
    }
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|goal| {
            goal_cells(
                goal.status.as_str(),
                goal.rounds.len(),
                goal.max_rounds,
                &goal.text,
            )
        })
        .collect();
    drive.stdout(&format!("{}\n", table(&cells)));
    Ok(0)
}

fn job_row(now: i64, row: &crate::drive::JobRow) -> Vec<String> {
    job_cells(JobCells {
        id: &row.job.id,
        status: status_word(row.job.status),
        provider: row.job.provider.map(|id| id.as_str()),
        job_type: row.job.job_type.as_str(),
        access: crate::format::access_word(&row.job),
        age_ms: age_ms(now, row.created_at, &row.job),
        prompt: &row.job.prompt,
    })
}

fn change_row(change: &ChangeSet) -> Vec<String> {
    change_cells(
        &change.id,
        change.name.as_deref(),
        change.files.len(),
        change.insertions,
        change.deletions,
    )
}

async fn list_merged<D: Drive>(drive: &D) -> Result<Vec<ChangeSet>, CliError> {
    let listed = drive.changes().await?;
    let ids = drive.change_ids().await?;
    let names = drive.worktree_names();
    Ok(legate_git::merge_listed_changes(
        &listed,
        &ids,
        &names,
        |id| drive.folder(id),
        |_| None,
    ))
}

async fn one_change<D: Drive>(parsed: &Parsed, key: &str, drive: &D) -> Result<String, CliError> {
    let Some(token) = parsed.args.first() else {
        let missing = if key == "worktree path" {
            "Missing change id or name."
        } else {
            "Missing change id."
        };
        drive.stderr(&format!("{missing}\n{}\n", usage_for(Some(key))));
        return Err(CliError::Code(2));
    };
    if unexpected(&parsed.args[1..], key, drive) {
        return Err(CliError::Code(2));
    }
    let changes = list_merged(drive).await?;
    let items: Vec<legate_core::IdItem> = changes
        .iter()
        .map(|change| match change.name.as_deref() {
            Some(name) if !name.is_empty() => legate_core::IdItem::named(change.id.clone(), name),
            _ => legate_core::IdItem::new(change.id.clone()),
        })
        .collect();
    match legate_core::resolve_id(token, &items, "Change") {
        legate_core::IdHit::Found { id } => Ok(id),
        legate_core::IdHit::Rejected { message } => {
            drive.stderr(&format!("{message}\n"));
            Err(CliError::Code(2))
        }
    }
}

fn idle<D: Drive>(drive: &D, id: &str) -> Result<(), CliError> {
    let jobs: Vec<GuardJob> = drive
        .jobs_newest(LOOKUP)
        .iter()
        .map(|row| GuardJob {
            status: status_word(row.job.status).to_string(),
            change: row.job.change.clone(),
            owner_id: row.job.owner_id.clone(),
            heartbeat_at: row.job.heartbeat_at,
        })
        .collect();
    assert_worktree_idle(&jobs, id, drive.now())?;
    Ok(())
}

async fn read_prompt<D: Drive>(drive: &D, arg: Option<&str>) -> Option<String> {
    if let Some(arg) = arg
        && arg != "-"
    {
        return Some(arg.to_string());
    }
    if arg.is_none() && drive.stdin_is_tty() {
        return None;
    }
    Some(drive.read_stdin().await)
}

fn parse_worker(value: Option<&str>) -> Option<Option<ProviderId>> {
    match value {
        None | Some("auto") => Some(None),
        Some(worker) => ProviderId::parse(worker).map(Some),
    }
}

fn parse_limit(value: Option<&str>) -> Option<i64> {
    match value {
        None => Some(20),
        Some(value) => parse_positive(value),
    }
}

fn parse_rounds(value: Option<&str>) -> Option<u32> {
    let rounds = match value {
        None => 5,
        Some(value) => parse_positive(value)? as u32,
    };
    if (1..=20).contains(&rounds) {
        Some(rounds)
    } else {
        None
    }
}

fn parse_positive(value: &str) -> Option<i64> {
    let mut chars = value.chars();
    let first = chars.next()?;
    if !first.is_ascii_digit() || first == '0' {
        return None;
    }
    let rest: String = chars.collect();
    if rest.len() > 8 || !rest.bytes().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

fn worktree_name(value: &str) -> Option<String> {
    if value.chars().any(|ch| ch == '\r' || ch == '\n') {
        return None;
    }
    let trimmed = value.trim();
    let len = trimmed.chars().count();
    if (1..=40).contains(&len) {
        Some(trimmed.to_string())
    } else {
        None
    }
}

fn title_breaks(value: &str) -> bool {
    value.chars().any(|ch| ch == '\r' || ch == '\n')
}

fn run_flags(parsed: &Parsed) -> RunFlags {
    RunFlags {
        json: parsed.flags.json,
        quiet: parsed.flags.quiet,
        verbose: parsed.flags.verbose,
    }
}

fn unexpected<D: Drive>(args: &[String], key: &str, drive: &D) -> bool {
    let Some(arg) = args.first() else {
        return false;
    };
    drive.stderr(&format!(
        "Unexpected argument \"{arg}\".\n{}\n",
        usage_for(Some(key))
    ));
    true
}

fn write_block<D: Drive>(drive: &D, text: &str) {
    if text.is_empty() {
        return;
    }
    if text.ends_with('\n') {
        drive.stdout(text);
    } else {
        drive.stdout(&format!("{text}\n"));
    }
}

enum Begin {
    Job(JobRequest),
    Routine(String),
}

struct JobLive {
    current: Option<String>,
    queue: VecDeque<JobRecord>,
    latest: Option<JobRecord>,
    header_done: bool,
    settled: bool,
    sigints: u32,
    printed: HashSet<String>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

async fn follow<D: Drive>(drive: &D, begin: Begin, flags: RunFlags) -> Result<i32, CliError> {
    let color = drive.color() && !flags.json;
    let started = drive.now();
    let notify = Arc::new(Notify::new());
    let live = Arc::new(Mutex::new(JobLive {
        current: None,
        queue: VecDeque::new(),
        latest: None,
        header_done: false,
        settled: false,
        sigints: 0,
        printed: HashSet::new(),
    }));
    let (sig_tx, mut sig_rx) = mpsc::unbounded_channel();
    drive.on_sigint(Arc::new(move || {
        let _ = sig_tx.send(());
    }));
    let listen = live.clone();
    let listen_notify = notify.clone();
    let on_job: Listener<JobRecord> = Arc::new(move |job| {
        let mut state = lock(&listen);
        state.queue.push_back(job);
        listen_notify.notify_one();
    });
    let _unsub = drive.on_job(on_job);

    let job = match begin {
        Begin::Job(request) => drive.submit(request).await?,
        Begin::Routine(id) => drive.run_routine(&id)?,
    };
    {
        let mut state = lock(&live);
        state.current = Some(job.id.clone());
    }
    pump_job(drive, &live, &flags, color);
    // The submit return is also a snapshot, the way `handle(job)` is.
    render_job(drive, &live, &flags, color, job);

    let final_job = loop {
        if lock(&live).settled {
            break lock(&live).latest.clone();
        }
        let notified = notify.notified();
        tokio::pin!(notified);
        tokio::select! {
            _ = &mut notified => pump_job(drive, &live, &flags, color),
            Some(()) = sig_rx.recv() => {
                if let Err(err) = on_job_sigint(drive, &live, &flags, color).await {
                    drive.clear_sigint();
                    return Err(err);
                }
            }
        }
    };
    drive.clear_sigint();
    let Some(final_job) = final_job else {
        return Ok(1);
    };
    let elapsed = drive.now() - started;
    if flags.json {
        drive.stdout(&json_job(&final_job));
    } else {
        finish_text(drive, &final_job, &flags, color, elapsed).await?;
    }
    if lock(&live).sigints > 0 {
        return Ok(130);
    }
    Ok(if final_job.status == JobStatus::Done {
        0
    } else {
        1
    })
}

fn pump_job<D: Drive>(drive: &D, live: &Mutex<JobLive>, flags: &RunFlags, color: bool) {
    let pending: Vec<JobRecord> = {
        let mut state = lock(live);
        state.queue.drain(..).collect()
    };
    for job in pending {
        render_job(drive, live, flags, color, job);
    }
}

fn render_job<D: Drive>(
    drive: &D,
    live: &Mutex<JobLive>,
    flags: &RunFlags,
    color: bool,
    job: JobRecord,
) {
    let mut lines = Vec::new();
    let mut terminal_job = None;
    {
        let mut state = lock(live);
        let Some(current) = state.current.clone() else {
            state.queue.push_back(job);
            return;
        };
        if job.id != current || state.settled {
            return;
        }
        state.latest = Some(job.clone());
        if !flags.quiet && !flags.json {
            let pending: Vec<_> = job
                .steps
                .iter()
                .flatten()
                .filter(|step| {
                    !state.printed.contains(&step.id)
                        && step_line(step, flags.verbose, false).is_some()
                })
                .cloned()
                .collect();
            let terminal = matches!(job.status, JobStatus::Done | JobStatus::Failed);
            if !state.header_done && (job.model.is_some() || terminal || !pending.is_empty()) {
                state.header_done = true;
                lines.push(format!("{}\n", header_from_job(&job, color)));
            }
            for step in pending {
                state.printed.insert(step.id.clone());
                if let Some(line) = step_line(&step, flags.verbose, color) {
                    lines.push(format!("{line}\n"));
                }
            }
        }
        if matches!(job.status, JobStatus::Done | JobStatus::Failed) {
            state.settled = true;
            terminal_job = Some(job);
        }
    }
    for line in lines {
        drive.stdout(&line);
    }
    let _ = terminal_job;
}

async fn on_job_sigint<D: Drive>(
    drive: &D,
    live: &Mutex<JobLive>,
    flags: &RunFlags,
    color: bool,
) -> Result<(), CliError> {
    let (sigints, current) = {
        let mut state = lock(live);
        state.sigints += 1;
        (state.sigints, state.current.clone())
    };
    if sigints >= 2 || current.is_none() {
        lock(live).settled = true;
        drive.exit_now(130);
        return Err(CliError::Immediate(130));
    }
    let Some(id) = current else {
        return Ok(());
    };
    match drive.cancel(&id).await {
        Ok(job) => {
            // The record comes back from cancel itself. A later event is ignored
            // once the job is settled.
            render_job(drive, live, flags, color, job);
            pump_job(drive, live, flags, color);
            Ok(())
        }
        Err(_) => {
            lock(live).settled = true;
            drive.exit_now(130);
            Err(CliError::Immediate(130))
        }
    }
}

async fn finish_text<D: Drive>(
    drive: &D,
    job: &JobRecord,
    flags: &RunFlags,
    color: bool,
    elapsed: i64,
) -> Result<(), CliError> {
    if flags.quiet {
        write_block(drive, &job.output);
        if job.status == JobStatus::Failed {
            drive.stderr(&format!(
                "{}\n",
                footer_line(JobStatus::Failed, elapsed, job.error.as_deref(), false)
            ));
        }
        return Ok(());
    }
    if !matches!(job.status, JobStatus::Done | JobStatus::Failed) {
        return Ok(());
    }
    write_block(drive, &job.output);
    drive.stdout(&format!(
        "{}\n",
        footer_line(job.status, elapsed, job.error.as_deref(), color)
    ));
    if crate::format::access_word(job) != "read" && job.change.is_some() {
        let id = job.change.clone().unwrap_or_default();
        let changes = list_merged(drive).await?;
        let found = changes.iter().find(|change| change.id == id);
        drive.stdout(&format!(
            "{}\n",
            change_footer(
                &id,
                found.map(|change| change.files.len()).unwrap_or(0),
                found.map(|change| change.insertions).unwrap_or(0),
                found.map(|change| change.deletions).unwrap_or(0),
            )
        ));
    }
    Ok(())
}

struct GoalLive {
    current: Option<String>,
    inbox: VecDeque<legate_lead::Goal>,
    jobs: VecDeque<JobRecord>,
    sigints: u32,
    seen_plan: i64,
    seen_proposal: String,
    seen_question: String,
    seen_checks: HashSet<i64>,
    printed: HashSet<String>,
    headers: HashSet<String>,
    closed: HashSet<String>,
    opened: std::collections::HashMap<String, i64>,
}

async fn follow_goal<D: Drive>(
    drive: &D,
    auto_yes: bool,
    input: GoalRequest,
) -> Result<i32, CliError> {
    let notify = Arc::new(Notify::new());
    let live = Arc::new(Mutex::new(GoalLive {
        current: None,
        inbox: VecDeque::new(),
        jobs: VecDeque::new(),
        sigints: 0,
        seen_plan: -1,
        seen_proposal: String::new(),
        seen_question: String::new(),
        seen_checks: HashSet::new(),
        printed: HashSet::new(),
        headers: HashSet::new(),
        closed: HashSet::new(),
        opened: std::collections::HashMap::new(),
    }));
    let (sig_tx, mut sig_rx) = mpsc::unbounded_channel();
    drive.on_sigint(Arc::new(move || {
        let _ = sig_tx.send(());
    }));
    let goals = live.clone();
    let goal_notify = notify.clone();
    let on_goal: Listener<legate_lead::Goal> = Arc::new(move |goal| {
        let mut state = lock(&goals);
        if state.current.as_ref().is_some_and(|id| id != &goal.id) {
            return;
        }
        state.inbox.push_back(goal);
        goal_notify.notify_one();
    });
    let _unsub_goal = drive.on_goal(on_goal);
    let jobs = live.clone();
    let job_notify = notify.clone();
    let _unsub_job = drive.on_job(Arc::new(move |job| {
        let mut state = lock(&jobs);
        state.jobs.push_back(job);
        job_notify.notify_one();
    }));

    let created = drive.create_goal(input).await?;
    lock(&live).current = Some(created.id.clone());
    let mut latest = Some(created);
    let result = loop {
        let Some(goal) = latest else {
            break Ok(1);
        };
        pump_goal_jobs(drive, &live);
        match react(drive, &live, &notify, auto_yes, &goal).await {
            Ok(Some(code)) => break Ok(code),
            Ok(None) => {}
            Err(err) => break Err(err),
        }
        latest = Some(take_goal(drive, &live, &notify, &mut sig_rx).await?);
    };
    drive.clear_sigint();
    result
}

fn pump_goal_jobs<D: Drive>(drive: &D, live: &Mutex<GoalLive>) {
    let pending: Vec<JobRecord> = {
        let mut state = lock(live);
        state.jobs.drain(..).collect()
    };
    for job in pending {
        render_goal_job(drive, live, job);
    }
}

fn render_goal_job<D: Drive>(drive: &D, live: &Mutex<GoalLive>, job: JobRecord) {
    let mut lines = Vec::new();
    {
        let mut state = lock(live);
        let Some(current) = state.current.clone() else {
            return;
        };
        if job.goal.as_deref() != Some(current.as_str()) {
            return;
        }
        state
            .opened
            .entry(job.id.clone())
            .or_insert_with(|| drive.now());
        let opened = state.opened.get(&job.id).copied().unwrap_or(0);
        let pending: Vec<_> = job
            .steps
            .iter()
            .flatten()
            .filter(|step| {
                !state.printed.contains(&step.id) && step_line(step, false, false).is_some()
            })
            .cloned()
            .collect();
        let terminal = matches!(job.status, JobStatus::Done | JobStatus::Failed);
        if !state.headers.contains(&job.id)
            && (job.model.is_some() || terminal || !pending.is_empty())
        {
            state.headers.insert(job.id.clone());
            lines.push(format!("{}\n", header_from_job(&job, drive.color())));
        }
        for step in &pending {
            state.printed.insert(step.id.clone());
        }
        for step in &pending {
            if let Some(line) = step_line(step, false, drive.color()) {
                lines.push(format!("{line}\n"));
            }
        }
        if terminal && !state.closed.contains(&job.id) {
            state.closed.insert(job.id.clone());
            let elapsed = drive.now() - opened;
            let output = job.output.clone();
            let footer = footer_line(job.status, elapsed, job.error.as_deref(), drive.color());
            drop(state);
            for line in lines {
                drive.stdout(&line);
            }
            write_block(drive, &output);
            drive.stdout(&format!("{footer}\n"));
            return;
        }
    }
    for line in lines {
        drive.stdout(&line);
    }
}

async fn react<D: Drive>(
    drive: &D,
    live: &Mutex<GoalLive>,
    notify: &Notify,
    auto_yes: bool,
    goal: &legate_lead::Goal,
) -> Result<Option<i32>, CliError> {
    print_check(drive, live, goal);
    {
        let mut state = lock(live);
        if goal.status != legate_lead::GoalStatus::Planning {
            state.seen_plan = -1;
        }
        if !(goal.status == legate_lead::GoalStatus::Waiting && goal.proposal.is_some()) {
            state.seen_proposal.clear();
        }
        if !(goal.status == legate_lead::GoalStatus::Waiting && goal.question.is_some()) {
            state.seen_question.clear();
        }
    }
    if matches!(
        goal.status,
        legate_lead::GoalStatus::Done
            | legate_lead::GoalStatus::Stopped
            | legate_lead::GoalStatus::Failed
    ) {
        if let Some(outcome) = goal.outcome.as_deref().filter(|text| !text.is_empty()) {
            drive.stdout(&format!("{outcome}\n"));
        }
        if lock(live).sigints > 0 {
            return Ok(Some(130));
        }
        return Ok(Some(if goal.status == legate_lead::GoalStatus::Done {
            0
        } else {
            1
        }));
    }
    if goal.status == legate_lead::GoalStatus::Planning {
        let n = goal.rounds.len() as i64 + 1;
        let fresh = lock(live).seen_plan != n;
        if fresh {
            lock(live).seen_plan = n;
            drive.stdout(&format!("Planning round {n}…\n"));
        }
        return Ok(None);
    }
    if goal.status == legate_lead::GoalStatus::Waiting
        && let Some(proposal) = &goal.proposal
    {
        let key = format!(
            "{}:{}:{}",
            goal.rounds.len(),
            proposal.summary,
            proposal
                .jobs
                .iter()
                .map(|job| format!(
                    "{}:{}:{}",
                    job.job_type.as_str(),
                    job.access.as_str(),
                    job.prompt
                ))
                .collect::<Vec<_>>()
                .join("\n")
        );
        if lock(live).seen_proposal == key {
            return Ok(None);
        }
        lock(live).seen_proposal = key;
        let round = goal.rounds.len() as i64 + 1;
        let jobs: Vec<ProposalJob<'_>> = proposal
            .jobs
            .iter()
            .map(|job| ProposalJob {
                job_type: job.job_type.as_str(),
                access: job.access.as_str(),
                prompt: &job.prompt,
            })
            .collect();
        drive.stdout(&format!(
            "{}\n",
            proposal_text(round, &proposal.summary, &jobs)
        ));
        if !auto_yes && !drive.stdin_is_tty() {
            drive.stderr(
                "Refusing to approve a round without --yes when input is not a terminal. Use --yes, or the app.\n",
            );
            return Ok(Some(2));
        }
        if auto_yes {
            remember(live, notify, drive.approve_goal(&goal.id)?);
            pump_goal_jobs(drive, live);
            return Ok(None);
        }
        let answer = drive
            .ask(&format!(
                "Approve round {round}? [y] yes  [n] stop  or type a note to plan again: "
            ))
            .await;
        let answer = answer.trim();
        let word = answer.to_lowercase();
        if word == "y" || word == "yes" {
            remember(live, notify, drive.approve_goal(&goal.id)?);
        } else if word == "n" || word == "no" || answer.is_empty() {
            remember(live, notify, drive.stop_goal(&goal.id)?);
        } else {
            remember(live, notify, drive.answer_goal(&goal.id, answer)?);
        }
        pump_goal_jobs(drive, live);
        return Ok(None);
    }
    if goal.status == legate_lead::GoalStatus::Waiting
        && let Some(question) = &goal.question
    {
        if lock(live).seen_question == *question {
            return Ok(None);
        }
        lock(live).seen_question = question.clone();
        drive.stdout(&format!("{question}\n"));
        let answer = drive.ask("Answer, or press enter to stop: ").await;
        let answer = answer.trim();
        if answer.is_empty() {
            remember(live, notify, drive.stop_goal(&goal.id)?);
        } else {
            remember(live, notify, drive.answer_goal(&goal.id, answer)?);
        }
        return Ok(None);
    }
    Ok(None)
}

fn remember(live: &Mutex<GoalLive>, notify: &Notify, goal: legate_lead::Goal) {
    let mut state = lock(live);
    let duplicate = state.inbox.back().is_some_and(|last| {
        last.id == goal.id
            && last.status == goal.status
            && last.updated_at == goal.updated_at
            && last.outcome == goal.outcome
            && last.question == goal.question
    });
    if !duplicate {
        state.inbox.push_back(goal);
        notify.notify_one();
    }
}

fn print_check<D: Drive>(drive: &D, live: &Mutex<GoalLive>, goal: &legate_lead::Goal) {
    let Some(round) = goal.rounds.last() else {
        return;
    };
    let Some(check) = &round.check else {
        return;
    };
    if !lock(live).seen_checks.insert(round.n) {
        return;
    }
    drive.stdout(&format!("{}\n", check_report(check.code, &check.output)));
}

async fn take_goal<D: Drive>(
    drive: &D,
    live: &Mutex<GoalLive>,
    notify: &Notify,
    sig_rx: &mut mpsc::UnboundedReceiver<()>,
) -> Result<legate_lead::Goal, CliError> {
    loop {
        if let Some(goal) = lock(live).inbox.pop_front() {
            return Ok(goal);
        }
        let notified = notify.notified();
        tokio::pin!(notified);
        tokio::select! {
            _ = &mut notified => {}
            Some(()) = sig_rx.recv() => on_goal_sigint(drive, live).await?,
        }
    }
}

async fn on_goal_sigint<D: Drive>(drive: &D, live: &Mutex<GoalLive>) -> Result<(), CliError> {
    let (sigints, current) = {
        let mut state = lock(live);
        state.sigints += 1;
        (state.sigints, state.current.clone())
    };
    if let (1, Some(id)) = (sigints, current.clone()) {
        let _ = drive.stop_goal(&id);
    }
    if sigints >= 2 || current.is_none() {
        drive.exit_now(130);
        return Err(CliError::Immediate(130));
    }
    Ok(())
}
