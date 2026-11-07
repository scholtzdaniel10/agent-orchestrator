//! Thin `legate` commands: `jobs`, `plans`, and `run`.
//!
//! The full terminal UI stays in the TypeScript CLI for now. Wording of the lines
//! this binary does print follows `src/cli/format.ts`.

mod format;

use std::path::PathBuf;
use std::time::Instant;

use legate_app::{AppEvent, AppHandle, AppOptions, JobAccess, JobType};
use legate_core::{JobRecord, JobStatus, ProviderId};
use legate_router::tested_version;

/// Binary name, matching the `legate` console command.
pub const NAME: &str = "legate";

const USAGE: &str = "\
legate [--project <path>] <command>
legate jobs
legate plans
legate run <type> <prompt> [--plan claude|cursor|grok|local] [--access read|edit|full]
legate --help
legate --version";

/// Run one invocation. `0` is success, `1` is a failed job, `2` is bad arguments.
pub async fn execute(args: Vec<String>) -> i32 {
    match prepare(&args) {
        Prepared::Exit { code, text, stderr } => {
            if stderr {
                eprint!("{text}");
            } else {
                print!("{text}");
            }
            code
        }
        Prepared::Jobs { project } => jobs(project).await,
        Prepared::Plans { project } => plans(project).await,
        Prepared::Run {
            project,
            job_type,
            prompt,
            plan,
            access,
        } => run_job(project, job_type, prompt, plan, access).await,
    }
}

enum Prepared {
    Exit {
        code: i32,
        text: String,
        stderr: bool,
    },
    Jobs {
        project: Option<PathBuf>,
    },
    Plans {
        project: Option<PathBuf>,
    },
    Run {
        project: Option<PathBuf>,
        job_type: JobType,
        prompt: String,
        plan: Option<ProviderId>,
        access: JobAccess,
    },
}

fn prepare(args: &[String]) -> Prepared {
    let mut project = None;
    let mut plan = None;
    let mut access = None;
    let mut help = false;
    let mut version = false;
    let mut command: Option<String> = None;
    let mut positionals = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--help" || arg == "-h" {
            help = true;
        } else if arg == "--version" || arg == "-V" {
            version = true;
        } else if let Some(rest) = arg.strip_prefix("--") {
            let (name, inline) = match rest.split_once('=') {
                Some((name, value)) => (name, Some(value.to_string())),
                None => (rest, None),
            };
            let value = match inline {
                Some(value) => value,
                None => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return exit(2, format!("Missing value for --{name}.\n{USAGE}\n"), true);
                    };
                    value.clone()
                }
            };
            match name {
                "project" => project = Some(value),
                "plan" => plan = Some(value),
                "access" => access = Some(value),
                _ => {
                    return exit(2, format!("Unknown option \"--{name}\".\n{USAGE}\n"), true);
                }
            }
        } else if command.is_none() {
            command = Some(arg.clone());
        } else {
            positionals.push(arg.clone());
        }
        index += 1;
    }
    let usage = usage_for(command.as_deref());
    if help {
        return exit(0, format!("{usage}\n"), false);
    }
    if version {
        return exit(0, format!("{NAME} {}\n", env!("CARGO_PKG_VERSION")), false);
    }
    let Some(command) = command else {
        return exit(2, format!("Missing command.\n{USAGE}\n"), true);
    };
    let project = project.map(PathBuf::from);
    match command.as_str() {
        "jobs" => {
            if let Some(extra) = positionals.first() {
                return exit(
                    2,
                    format!("Unexpected argument \"{extra}\".\n{usage}\n"),
                    true,
                );
            }
            Prepared::Jobs { project }
        }
        "plans" => {
            if let Some(extra) = positionals.first() {
                return exit(
                    2,
                    format!("Unexpected argument \"{extra}\".\n{usage}\n"),
                    true,
                );
            }
            Prepared::Plans { project }
        }
        "run" => prepare_run(project, &positionals, plan.as_deref(), access.as_deref()),
        other => exit(2, format!("Unknown command \"{other}\".\n{USAGE}\n"), true),
    }
}

fn prepare_run(
    project: Option<PathBuf>,
    positionals: &[String],
    plan: Option<&str>,
    access: Option<&str>,
) -> Prepared {
    let usage = usage_for(Some("run"));
    let Some(job_type) = positionals.first() else {
        return exit(2, format!("Missing job type.\n{usage}\n"), true);
    };
    let Some(job_type) = JobType::parse(job_type) else {
        return exit(
            2,
            format!("Unknown job type \"{job_type}\".\n{usage}\n"),
            true,
        );
    };
    let Some(prompt) = positionals.get(1) else {
        return exit(2, format!("Missing prompt.\n{usage}\n"), true);
    };
    if let Some(extra) = positionals.get(2) {
        return exit(
            2,
            format!("Unexpected argument \"{extra}\".\n{usage}\n"),
            true,
        );
    }
    if prompt.trim().is_empty() {
        return exit(2, format!("Empty prompt.\n{usage}\n"), true);
    }
    let plan = match plan {
        None => None,
        Some(value) => match ProviderId::parse(value) {
            Some(id) => Some(id),
            None => {
                return exit(
                    2,
                    format!("Plan must be claude, cursor, grok, or local.\n{usage}\n"),
                    true,
                );
            }
        },
    };
    let access = match access {
        None => JobAccess::Read,
        Some(value) => match JobAccess::parse(value) {
            Some(access) => access,
            None => {
                return exit(
                    2,
                    format!("Access must be read, edit, or full.\n{usage}\n"),
                    true,
                );
            }
        },
    };
    Prepared::Run {
        project,
        job_type,
        prompt: prompt.clone(),
        plan,
        access,
    }
}

fn usage_for(command: Option<&str>) -> &'static str {
    match command {
        Some("jobs") => "legate jobs",
        Some("plans") => "legate plans",
        Some("run") => {
            "legate run <type> <prompt> [--plan claude|cursor|grok|local] [--access read|edit|full]"
        }
        _ => USAGE,
    }
}

fn exit(code: i32, text: String, stderr: bool) -> Prepared {
    Prepared::Exit { code, text, stderr }
}

async fn open(project: Option<PathBuf>) -> Result<AppHandle, String> {
    AppHandle::start(AppOptions {
        project,
        ..AppOptions::default()
    })
    .await
    .map_err(|err| err.to_string())
}

async fn jobs(project: Option<PathBuf>) -> i32 {
    let app = match open(project).await {
        Ok(app) => app,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let listed = match app.list_jobs().await {
        Ok(jobs) => jobs,
        Err(err) => {
            eprintln!("{err}");
            app.shutdown().await;
            return 1;
        }
    };
    app.shutdown().await;
    if listed.is_empty() {
        eprintln!("No jobs yet. Start one with: legate run <type> \"<prompt>\"");
        return 0;
    }
    let rows = listed.iter().map(format::job_cells).collect::<Vec<_>>();
    println!("{}", format::table(&rows));
    0
}

async fn plans(project: Option<PathBuf>) -> i32 {
    let app = match open(project).await {
        Ok(app) => app,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let listed = match app.list_plans().await {
        Ok(plans) => plans,
        Err(err) => {
            eprintln!("{err}");
            app.shutdown().await;
            return 1;
        }
    };
    let project = app.get_project().await.ok();
    app.shutdown().await;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0);
    for plan in &listed {
        let tested = tested_version(plan.id).unwrap_or("");
        println!("{}", format::plan_line(plan, tested, now));
    }
    if let Some(project) = project {
        println!("{}", project.path);
    }
    0
}

async fn run_job(
    project: Option<PathBuf>,
    job_type: JobType,
    prompt: String,
    plan: Option<ProviderId>,
    access: JobAccess,
) -> i32 {
    let app = match open(project).await {
        Ok(app) => app,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let started = Instant::now();
    let mut rx = app.subscribe();
    let submitted = match app
        .submit_job(job_type, prompt, plan, Some(access), None)
        .await
    {
        Ok(job) => job,
        Err(err) => {
            eprintln!("{err}");
            app.shutdown().await;
            return 1;
        }
    };
    let final_job = match follow(&mut rx, &submitted).await {
        Ok(job) => job,
        Err(err) => {
            eprintln!("{err}");
            app.shutdown().await;
            return 1;
        }
    };
    let elapsed = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
    finish(&app, &final_job, elapsed).await;
    let code = if final_job.status == JobStatus::Done {
        0
    } else {
        1
    };
    app.shutdown().await;
    code
}

async fn follow(
    rx: &mut tokio::sync::broadcast::Receiver<AppEvent>,
    submitted: &JobRecord,
) -> Result<JobRecord, String> {
    let mut latest = submitted.clone();
    let mut header = false;
    let mut printed = std::collections::HashSet::new();
    render(&latest, &mut header, &mut printed);
    if is_terminal(latest.status) {
        return Ok(latest);
    }
    loop {
        let event = rx
            .recv()
            .await
            .map_err(|_| "legate closed the job stream")?;
        let AppEvent::Job(job) = event else {
            continue;
        };
        if job.id != submitted.id {
            continue;
        }
        latest = job;
        render(&latest, &mut header, &mut printed);
        if is_terminal(latest.status) {
            return Ok(latest);
        }
    }
}

fn render(job: &JobRecord, header: &mut bool, printed: &mut std::collections::HashSet<String>) {
    let pending = job
        .steps
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .filter(|step| !printed.contains(&step.id) && format::step_line(step, false).is_some());
    let pending = pending.cloned().collect::<Vec<_>>();
    let terminal = is_terminal(job.status);
    if !*header && (job.model.is_some() || terminal || !pending.is_empty()) {
        *header = true;
        println!("{}", format::header_line(job));
    }
    for step in pending {
        printed.insert(step.id.clone());
        if let Some(line) = format::step_line(&step, false) {
            println!("{line}");
        }
    }
}

async fn finish(app: &AppHandle, job: &JobRecord, elapsed: i64) {
    if !job.output.is_empty() {
        if job.output.ends_with('\n') {
            print!("{}", job.output);
        } else {
            println!("{}", job.output);
        }
    }
    if !is_terminal(job.status) {
        return;
    }
    println!(
        "{}",
        format::footer_line(job.status, elapsed, job.error.as_deref())
    );
    if format::access_word(job) != "read"
        && let Some(id) = job.change.as_deref()
        && let Ok(changes) = app.list_changes().await
    {
        let found = changes.iter().find(|change| change.id == id);
        println!(
            "{}",
            format::change_footer(
                id,
                found.map(|change| change.files.len()).unwrap_or(0),
                found.map(|change| change.insertions).unwrap_or(0),
                found.map(|change| change.deletions).unwrap_or(0),
            )
        );
    }
}

fn is_terminal(status: JobStatus) -> bool {
    matches!(status, JobStatus::Done | JobStatus::Failed)
}

#[cfg(test)]
mod tests {
    use super::{Prepared, prepare};

    fn owned(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_string()).collect()
    }

    #[test]
    fn help_and_version_do_not_need_the_app() {
        let help = prepare(&owned(&["--help"]));
        match help {
            Prepared::Exit { code, text, stderr } => {
                assert_eq!(code, 0);
                assert!(!stderr);
                assert!(text.contains("legate jobs"));
                assert!(text.contains("legate run"));
            }
            _ => panic!("help should exit"),
        }
        let version = prepare(&owned(&["--version"]));
        match version {
            Prepared::Exit { code, text, .. } => {
                assert_eq!(code, 0);
                assert!(text.starts_with("legate "));
            }
            _ => panic!("version should exit"),
        }
    }

    #[test]
    fn run_rejects_a_bad_type_and_accepts_a_plan() {
        match prepare(&owned(&["run", "nope", "do it"])) {
            Prepared::Exit { code, text, stderr } => {
                assert_eq!(code, 2);
                assert!(stderr);
                assert!(text.contains("Unknown job type \"nope\""));
            }
            _ => panic!("bad type should exit"),
        }
        match prepare(&owned(&[
            "--project",
            "C:/work",
            "run",
            "debugging",
            "fix the test",
            "--plan",
            "local",
            "--access",
            "edit",
        ])) {
            Prepared::Run {
                job_type,
                prompt,
                plan,
                access,
                project,
            } => {
                assert_eq!(job_type, legate_core::JobType::Debugging);
                assert_eq!(prompt, "fix the test");
                assert_eq!(plan, Some(legate_core::ProviderId::Local));
                assert_eq!(access, legate_core::JobAccess::Edit);
                assert_eq!(project.unwrap().to_string_lossy(), "C:/work");
            }
            _ => panic!("expected a run"),
        }
    }

    #[test]
    fn unknown_command_exits_with_the_usage() {
        match prepare(&owned(&["ui"])) {
            Prepared::Exit { code, text, .. } => {
                assert_eq!(code, 2);
                assert!(text.contains("Unknown command \"ui\""));
                assert!(text.contains("legate plans"));
            }
            _ => panic!("ui is not in this first binary"),
        }
    }
}
