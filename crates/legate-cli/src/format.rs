//! Plain-text lines from `src/cli/format.ts`.

use legate_core::{
    JobAccess, JobRecord, JobStatus, JobStep, JobStepStatus, PlanProblem, ProviderId, StepTool,
    VersionStatus,
};

/// A plan the way `planLine` sees it: `headroom` is allowance left, 0..1.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanView {
    pub id: ProviderId,
    pub available: bool,
    pub problem: Option<PlanProblem>,
    pub headroom: f64,
    pub resting_until: Option<i64>,
    pub busy: bool,
    pub running: i64,
    pub version: Option<String>,
    pub version_status: VersionStatus,
    pub at_risk: bool,
}

pub const ACCENT: &str = "38;2;224;52;60";

const PLAN_CODE: &[(&str, &str)] = &[
    ("claude", "38;2;255;160;90"),
    ("cursor", "38;2;80;230;170"),
    ("grok", "38;2;80;230;170"),
];

pub fn plan_name(id: &str, color: bool) -> String {
    let code = PLAN_CODE
        .iter()
        .find(|(name, _)| *name == id)
        .map(|(_, code)| *code)
        .unwrap_or("36");
    paint(id, code, color)
}

pub fn paint(text: &str, code: &str, color: bool) -> String {
    if !color || text.is_empty() {
        return text.to_string();
    }
    format!("\u{1b}[{code}m{text}\u{1b}[0m")
}

pub fn format_duration(ms: i64) -> String {
    let seconds = (ms as f64 / 1000.0).round().max(0.0) as i64;
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    let rem = seconds % 60;
    if minutes < 60 {
        return if rem == 0 {
            format!("{minutes}m")
        } else {
            format!("{minutes}m {rem}s")
        };
    }
    let hours = minutes / 60;
    let min_rem = minutes % 60;
    if min_rem == 0 {
        format!("{hours}h")
    } else {
        format!("{hours}h {min_rem}m")
    }
}

/// Same buckets the app uses for a chat's age.
pub fn format_age(ms: i64) -> String {
    let minutes = (ms.max(0)) / 60_000;
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h");
    }
    format!("{}d", hours / 24)
}

pub fn allowance_percent(headroom: f64) -> i64 {
    if !headroom.is_finite() {
        return 0;
    }
    let raw = (headroom * 100.0).round() as i64;
    if raw < 0 {
        return 0;
    }
    if raw > 100 {
        return 100;
    }
    raw
}

fn tool_word(tool: StepTool) -> &'static str {
    match tool {
        StepTool::Say => "SAY",
        StepTool::Think => "THINK",
        StepTool::Read => "READ",
        StepTool::Edit => "EDIT",
        StepTool::Shell => "RUN",
        StepTool::Search => "SEARCH",
        StepTool::Web => "WEB",
        StepTool::Mcp => "MCP",
        StepTool::Other => "TOOL",
    }
}

pub fn access_word(job: &JobRecord) -> &'static str {
    match job.access {
        Some(access) => access.as_str(),
        None if job.edit == Some(true) => JobAccess::Edit.as_str(),
        None => JobAccess::Read.as_str(),
    }
}

pub fn status_word(status: JobStatus) -> &'static str {
    match status {
        JobStatus::Queued => "queued",
        JobStatus::Running => "running",
        JobStatus::Done => "done",
        JobStatus::Failed => "failed",
    }
}

pub fn step_line(step: &JobStep, verbose: bool, color: bool) -> Option<String> {
    if step.status == JobStepStatus::Running {
        return None;
    }
    if !verbose && matches!(step.tool, StepTool::Say | StepTool::Think) {
        return None;
    }
    let word = tool_word(step.tool);
    let gap = " ".repeat(7usize.saturating_sub(word.len()).max(1));
    let mut line = format!("{}{gap}{}", paint(word, "36", color), step.title);
    if let Some(edit) = &step.edit {
        let plus = paint(&format!("+{}", edit.added), "32", color);
        let minus = paint(&format!("−{}", edit.removed), "31", color);
        line.push_str(&format!(" {plus} {minus}"));
    }
    if step.status == JobStepStatus::Failed {
        line.push(' ');
        line.push_str(&paint("(failed)", "31", color));
    }
    Some(line)
}

pub struct HeaderView<'a> {
    pub provider: Option<&'a str>,
    pub model: Option<&'a str>,
    pub job_type: &'a str,
    pub access: &'a str,
    pub reason: Option<&'a str>,
}

pub fn header_line(job: &HeaderView<'_>, color: bool) -> String {
    let mut parts = Vec::new();
    if let Some(provider) = job.provider {
        parts.push(plan_name(provider, color));
    }
    if let Some(model) = job.model.filter(|model| !model.is_empty()) {
        parts.push(model.to_string());
    }
    parts.push(job.job_type.to_string());
    parts.push(job.access.to_string());
    if let Some(reason) = job.reason.filter(|reason| !reason.is_empty()) {
        parts.push(reason.to_string());
    }
    parts.join(" · ")
}

pub fn header_from_job(job: &JobRecord, color: bool) -> String {
    header_line(
        &HeaderView {
            provider: job.provider.map(|id| id.as_str()),
            model: job.model.as_deref(),
            job_type: job.job_type.as_str(),
            access: access_word(job),
            reason: job.reason.as_deref(),
        },
        color,
    )
}

pub fn footer_line(status: JobStatus, elapsed_ms: i64, error: Option<&str>, color: bool) -> String {
    if status == JobStatus::Done {
        return paint(
            &format!("done in {}", format_duration(elapsed_ms)),
            "32",
            color,
        );
    }
    let why = error.filter(|text| !text.is_empty()).unwrap_or("failed");
    paint(&format!("failed: {why}"), "31", color)
}

pub fn change_footer(id: &str, files: usize, added: i64, removed: i64) -> String {
    let noun = if files == 1 {
        "1 file".to_string()
    } else {
        format!("{files} files")
    };
    format!("change {id} · {noun} +{added} −{removed} · legate diff {id}")
}

pub fn plan_word(plan: &PlanView, now: i64) -> String {
    if plan.problem == Some(PlanProblem::NotInstalled) {
        return "not installed".to_string();
    }
    if plan.problem == Some(PlanProblem::SignedOut) || !plan.available {
        return "signed out".to_string();
    }
    if plan.resting_until.is_some_and(|until| until > now) {
        return "resting".to_string();
    }
    if plan.busy || plan.running > 0 {
        if plan.running > 1 {
            return format!("{} running", plan.running);
        }
        return "busy".to_string();
    }
    "ready".to_string()
}

pub fn version_phrase(plan: &PlanView, tested: &str) -> String {
    // Local is not a CLI. `version` is the Ollama or llama-server build.
    // The router marks that status unknown because there is no tested CLI build,
    // which used to hide the server version behind "CLI version unknown".
    if plan.id == ProviderId::Local {
        return match plan.version.as_deref().filter(|text| !text.is_empty()) {
            Some(version) => format!("CLI {version}"),
            None => "CLI version unknown".to_string(),
        };
    }
    match (&plan.version, plan.version_status) {
        (None, _) | (_, VersionStatus::Unknown) => "CLI version unknown".to_string(),
        (Some(version), VersionStatus::Tested) => format!("CLI {version}"),
        (Some(version), VersionStatus::Newer) => {
            format!("CLI {version} · newer than tested ({tested})")
        }
        (Some(version), VersionStatus::Older) => {
            format!("CLI {version} · older than tested ({tested}) — update it")
        }
    }
}

fn problem_sentence(id: ProviderId, problem: PlanProblem) -> Option<&'static str> {
    match (id, problem) {
        (ProviderId::Codex, PlanProblem::NotInstalled) => {
            Some("Codex CLI is not installed. Install it, then run `codex login`.")
        }
        (ProviderId::Codex, PlanProblem::SignedOut) => {
            Some("Codex needs a ChatGPT login. Run `codex login` and sign in with ChatGPT.")
        }
        (ProviderId::Claude, PlanProblem::NotInstalled) => Some(
            "Claude Code is not installed. Install it, then run `claude` once in a terminal to sign in.",
        ),
        (ProviderId::Claude, PlanProblem::SignedOut) => {
            Some("Claude Code is signed out. Run `claude` in a terminal and sign in.")
        }
        (ProviderId::Cursor | ProviderId::Grok, PlanProblem::NotInstalled) => Some(
            "The Cursor CLI is not installed. Install it, then run `agent login` in a terminal.",
        ),
        (ProviderId::Cursor | ProviderId::Grok, PlanProblem::SignedOut) => {
            Some("The Cursor CLI is signed out. Run `agent login` in a terminal.")
        }
        (ProviderId::Local, _) => None,
    }
}

pub fn plan_line(plan: &PlanView, tested: &str, now: i64, color: bool) -> String {
    let word = plan_word(plan, now);
    let tone = if word == "ready" {
        "32"
    } else if word == "not installed" || word == "signed out" {
        ACCENT
    } else {
        "33"
    };
    let mut parts = vec![plan_name(plan.id.as_str(), color)];
    if plan.id == ProviderId::Grok {
        parts.push("uses the Cursor plan".to_string());
    }
    parts.push(paint(&word, tone, color));
    if plan.problem != Some(PlanProblem::NotInstalled) {
        parts.push(version_phrase(plan, tested));
    }
    if word == "not installed" || word == "signed out" {
        if let Some(sentence) =
            problem_sentence(plan.id, plan.problem.unwrap_or(PlanProblem::SignedOut))
        {
            parts.push(sentence.to_string());
        }
    } else {
        if plan.id == ProviderId::Codex {
            parts.push("usage not reported".into());
        } else {
            parts.push(format!("{}% left", allowance_percent(plan.headroom)));
        }
        if plan.at_risk {
            parts.push("Unused allowance expires soon".to_string());
        }
    }
    parts.join(" · ")
}

/// The plan in a few words, for a one-line header: no version, no advice.
pub fn plan_short(plan: &PlanView, now: i64) -> String {
    let word = plan_word(plan, now);
    if word != "ready" {
        return format!("{} {word}", plan.id.as_str());
    }
    if plan.id == ProviderId::Codex {
        return "codex ready - usage not reported".into();
    }
    format!(
        "{} ready {}% left",
        plan.id.as_str(),
        allowance_percent(plan.headroom)
    )
}

pub struct JobCells<'a> {
    pub id: &'a str,
    pub status: &'a str,
    pub provider: Option<&'a str>,
    pub job_type: &'a str,
    pub access: &'a str,
    /// `None` prints `-` when the job has no timestamp at all.
    pub age_ms: Option<i64>,
    pub prompt: &'a str,
}

pub fn job_cells(input: JobCells<'_>) -> Vec<String> {
    let prompt = input.prompt.replace(['\r', '\n'], " ");
    let prompt: String = prompt.chars().take(60).collect();
    let age = match input.age_ms {
        Some(ms) => format_age(ms),
        None => "-".to_string(),
    };
    vec![
        input.id.chars().take(8).collect(),
        input.status.to_string(),
        input.provider.unwrap_or("-").to_string(),
        input.job_type.to_string(),
        input.access.to_string(),
        age,
        prompt,
    ]
}

/// Milliseconds of age for a list cell. `None` is the `-` cell.
/// A stored `created_at` of 0 is missing; the earliest step start still counts.
pub fn age_ms(now: i64, created_at: Option<i64>, job: &JobRecord) -> Option<i64> {
    let stamp = created_at
        .filter(|at| *at != 0)
        .or_else(|| earliest_step(job));
    stamp.map(|at| now.saturating_sub(at))
}

fn earliest_step(job: &JobRecord) -> Option<i64> {
    job.steps.as_ref().and_then(|steps| {
        steps
            .iter()
            .map(|step| step.started_at)
            .filter(|at| *at > 0)
            .min()
    })
}

pub fn change_cells(
    id: &str,
    name: Option<&str>,
    files: usize,
    insertions: i64,
    deletions: i64,
) -> Vec<String> {
    let noun = if files == 1 {
        "1 file".to_string()
    } else {
        format!("{files} files")
    };
    let name = name.filter(|text| !text.is_empty()).unwrap_or("-");
    vec![
        id.to_string(),
        name.to_string(),
        noun,
        format!("+{insertions} −{deletions}"),
    ]
}

pub fn table(rows: &[Vec<String>]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut widths = vec![0usize; cols];
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(visible_width(cell));
        }
    }
    rows.iter()
        .map(|row| {
            row.iter()
                .enumerate()
                .map(|(index, cell)| {
                    if index + 1 == row.len() {
                        return cell.clone();
                    }
                    let extra = widths[index].saturating_sub(visible_width(cell));
                    format!("{cell}{}", " ".repeat(extra))
                })
                .collect::<Vec<_>>()
                .join("  ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn one_sentence(message: &str) -> String {
    message
        .split(['\r', '\n'])
        .find(|item| !item.trim().is_empty())
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .unwrap_or("Something went wrong.")
        .to_string()
}

pub fn goal_cells(status: &str, used: usize, max: i64, text: &str) -> Vec<String> {
    let line = text.trim().split(['\r', '\n']).next().unwrap_or("");
    vec![
        status.to_string(),
        format!("{used}/{max}"),
        line.to_string(),
    ]
}

pub struct ProposalJob<'a> {
    pub job_type: &'a str,
    pub access: &'a str,
    pub prompt: &'a str,
}

pub fn proposal_text(round: i64, summary: &str, jobs: &[ProposalJob<'_>]) -> String {
    let mut lines = vec![format!("Round {round}: {summary}")];
    for job in jobs {
        lines.push(format!("{} · {}", job.job_type, job.access));
        let preview = prompt_preview(job.prompt);
        if !preview.is_empty() {
            lines.push(preview);
        }
    }
    lines.join("\n")
}

pub fn check_report(code: Option<i32>, output: &str) -> String {
    let head = if code == Some(0) {
        "Check passed".to_string()
    } else {
        format!("Check failed (exit {})", code_text(code))
    };
    let mut lines: Vec<&str> = output.split(['\r', '\n']).collect();
    if lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    let start = lines.len().saturating_sub(15);
    let mut out = vec![head];
    out.extend(lines[start..].iter().map(|line| (*line).to_string()));
    out.join("\n")
}

fn code_text(code: Option<i32>) -> String {
    match code {
        Some(code) => code.to_string(),
        None => "null".to_string(),
    }
}

fn prompt_preview(prompt: &str) -> String {
    let trimmed = prompt.trim_end();
    let lines: Vec<&str> = trimmed.split(['\r', '\n']).collect();
    let mut shown: Vec<String> = lines
        .iter()
        .take(3)
        .map(|line| {
            let trimmed = line.trim_end();
            if trimmed.chars().count() > 200 {
                let head: String = trimmed.chars().take(199).collect();
                format!("{head}…")
            } else {
                trimmed.to_string()
            }
        })
        .collect();
    if lines.len() > 3 {
        shown.push("…".to_string());
    }
    shown.join("\n")
}

fn visible_width(text: &str) -> usize {
    let mut width = 0usize;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if next == 'm' {
                    break;
                }
            }
            continue;
        }
        width += 1;
    }
    width
}

pub fn json_job(job: &JobRecord) -> String {
    let value = serde_json::json!({
        "id": job.id,
        "status": status_word(job.status),
        "provider": job.provider.map(|id| id.as_str()),
        "model": job.model,
        "access": access_word(job),
        "output": job.output,
        "error": job.error,
        "change": job.change,
        "steps": job.steps.clone().unwrap_or_default(),
    });
    format!("{value}\n")
}

#[cfg(test)]
mod tests {
    use super::{
        HeaderView, JobCells, PlanView, ProposalJob, change_cells, change_footer, check_report,
        footer_line, format_age, format_duration, goal_cells, header_line, job_cells, plan_line,
        proposal_text, step_line, table,
    };
    use legate_core::{
        FileEdit, JobStep, JobStepStatus, PlanProblem, ProviderId, StepTool, VersionStatus,
    };

    fn step(id: &str, tool: StepTool, title: &str, status: JobStepStatus) -> JobStep {
        JobStep {
            id: id.to_string(),
            tool,
            title: title.to_string(),
            detail: None,
            status,
            started_at: 1,
            ended_at: None,
            edit: None,
        }
    }

    fn plan(id: ProviderId) -> PlanView {
        PlanView {
            id,
            available: true,
            problem: None,
            headroom: 0.8,
            resting_until: None,
            busy: false,
            running: 0,
            version: Some("2.1.292".to_string()),
            version_status: VersionStatus::Tested,
            at_risk: false,
        }
    }

    #[test]
    fn step_lines_name_the_tool_the_size_and_a_failure() {
        let read = step("s1", StepTool::Read, "Read src/app.ts", JobStepStatus::Done);
        assert_eq!(
            step_line(&read, false, false).as_deref(),
            Some("READ   Read src/app.ts")
        );
        let mut edit = step(
            "s2",
            StepTool::Edit,
            "Edited src/app.ts",
            JobStepStatus::Done,
        );
        edit.edit = Some(FileEdit {
            path: "src/app.ts".to_string(),
            added: 4,
            removed: 1,
        });
        assert_eq!(
            step_line(&edit, false, false).as_deref(),
            Some("EDIT   Edited src/app.ts +4 −1")
        );
        let ran = step(
            "s3",
            StepTool::Shell,
            "Ran pnpm test",
            JobStepStatus::Failed,
        );
        assert_eq!(
            step_line(&ran, false, false).as_deref(),
            Some("RUN    Ran pnpm test (failed)")
        );
        let say = step("s4", StepTool::Say, "hello", JobStepStatus::Done);
        assert_eq!(step_line(&say, false, false), None);
        assert_eq!(
            step_line(&say, true, false).as_deref(),
            Some("SAY    hello")
        );
        let running = step(
            "s1",
            StepTool::Read,
            "Read src/app.ts",
            JobStepStatus::Running,
        );
        assert_eq!(step_line(&running, true, false), None);
    }

    #[test]
    fn colour_wraps_the_tool_word_and_leaves_the_title_readable() {
        let read = step("s1", StepTool::Read, "Read src/app.ts", JobStepStatus::Done);
        let line = step_line(&read, false, true).unwrap();
        assert!(line.contains("\u{1b}[36mREAD\u{1b}[0m"));
        assert!(line.contains("Read src/app.ts"));
        assert!(!step_line(&read, false, false).unwrap().contains('\u{1b}'));
    }

    #[test]
    fn header_footer_durations_and_ages() {
        assert_eq!(
            header_line(
                &HeaderView {
                    provider: Some("claude"),
                    model: Some("sonnet"),
                    job_type: "planning",
                    access: "read",
                    reason: Some("first choice"),
                },
                false
            ),
            "claude · sonnet · planning · read · first choice"
        );
        assert_eq!(
            header_line(
                &HeaderView {
                    provider: Some("claude"),
                    model: None,
                    job_type: "planning",
                    access: "read",
                    reason: None,
                },
                false
            ),
            "claude · planning · read"
        );
        assert!(
            header_line(
                &HeaderView {
                    provider: Some("claude"),
                    model: None,
                    job_type: "planning",
                    access: "read",
                    reason: None,
                },
                true
            )
            .contains("\u{1b}[38;2;255;160;90mclaude\u{1b}[0m")
        );
        assert_eq!(
            footer_line(legate_core::JobStatus::Done, 42_000, None, false),
            "done in 42s"
        );
        assert_eq!(
            footer_line(legate_core::JobStatus::Failed, 1500, Some("boom"), false),
            "failed: boom"
        );
        assert!(footer_line(legate_core::JobStatus::Done, 42_000, None, true).contains("\u{1b}["));
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(61_000), "1m 1s");
        assert_eq!(format_duration(120_000), "2m");
        assert_eq!(format_duration(3_600_000), "1h");
        assert_eq!(format_age(30_000), "0m");
        assert_eq!(format_age(5 * 60_000), "5m");
        assert_eq!(format_age(2 * 3_600_000), "2h");
        assert_eq!(format_age(3 * 86_400_000), "3d");
    }

    #[test]
    fn tables_pad_every_column_but_the_last() {
        assert_eq!(
            table(&[
                vec!["id".into(), "done".into(), "prompt".into()],
                vec!["abcd".into(), "running".into(), "hi".into()]
            ]),
            ["id    done     prompt", "abcd  running  hi"].join("\n")
        );
        assert_eq!(
            job_cells(JobCells {
                id: "abcdef123456",
                status: "done",
                provider: Some("claude"),
                job_type: "planning",
                access: "read",
                age_ms: Some(60_000),
                prompt: "one\ntwo",
            }),
            vec![
                "abcdef12", "done", "claude", "planning", "read", "1m", "one two"
            ]
        );
        assert_eq!(
            change_cells("abcd1234", Some("login"), 2, 10, 3),
            vec!["abcd1234", "login", "2 files", "+10 −3"]
        );
    }

    #[test]
    fn plan_lines_use_the_app_wording() {
        assert_eq!(
            plan_line(&plan(ProviderId::Claude), "2.1.292", 0, false),
            "claude · ready · CLI 2.1.292 · 80% left"
        );
        let mut cursor = plan(ProviderId::Cursor);
        cursor.available = false;
        cursor.problem = Some(PlanProblem::NotInstalled);
        cursor.version = None;
        cursor.version_status = VersionStatus::Unknown;
        assert_eq!(
            plan_line(&cursor, "2026.10.01", 0, false),
            "cursor · not installed · The Cursor CLI is not installed. Install it, then run `agent login` in a terminal."
        );
        let mut newer = plan(ProviderId::Claude);
        newer.version = Some("9.0.0".to_string());
        newer.version_status = VersionStatus::Newer;
        assert_eq!(
            plan_line(&newer, "2.1.292", 0, false),
            "claude · ready · CLI 9.0.0 · newer than tested (2.1.292) · 80% left"
        );
        let mut older = plan(ProviderId::Claude);
        older.version = Some("1.0.0".to_string());
        older.version_status = VersionStatus::Older;
        older.headroom = 0.5;
        assert_eq!(
            plan_line(&older, "2.1.292", 0, false),
            "claude · ready · CLI 1.0.0 · older than tested (2.1.292) — update it · 50% left"
        );
        assert!(plan_line(&plan(ProviderId::Claude), "2.1.292", 0, true).contains("\u{1b}["));
        let mut grok = plan(ProviderId::Grok);
        grok.version = Some("2026.10.01-e373342".to_string());
        assert_eq!(
            plan_line(&grok, "2026.10.01-e373342", 0, false),
            "grok · uses the Cursor plan · ready · CLI 2026.10.01-e373342 · 80% left"
        );
        let mut local = plan(ProviderId::Local);
        local.version = Some("0.6.5".to_string());
        local.version_status = VersionStatus::Unknown;
        local.headroom = 1.0;
        assert_eq!(
            plan_line(&local, "", 0, false),
            "local · ready · CLI 0.6.5 · 100% left"
        );
    }

    #[test]
    fn a_goal_list_row_a_proposal_and_a_check() {
        assert_eq!(
            goal_cells("done", 2, 5, "Newer goal\nmore detail"),
            vec!["done", "2/5", "Newer goal"]
        );
        let text = proposal_text(
            1,
            "Add the form",
            &[
                ProposalJob {
                    job_type: "refactor",
                    access: "edit",
                    prompt: "Write the form\nUse the field.\nKeep the types.\nHidden line.",
                },
                ProposalJob {
                    job_type: "review",
                    access: "full",
                    prompt: "Check it",
                },
            ],
        );
        assert_eq!(
            text,
            [
                "Round 1: Add the form",
                "refactor · edit",
                "Write the form",
                "Use the field.",
                "Keep the types.",
                "…",
                "review · full",
                "Check it",
            ]
            .join("\n")
        );
        assert_eq!(
            check_report(Some(0), "all green\n"),
            "Check passed\nall green"
        );
        let lines: Vec<String> = (0..20).map(|index| format!("line {index}")).collect();
        let tail: Vec<String> = lines[lines.len() - 15..].to_vec();
        let mut expected = vec!["Check failed (exit 3)".to_string()];
        expected.extend(tail);
        assert_eq!(
            check_report(Some(3), &lines.join("\n")),
            expected.join("\n")
        );
    }

    #[test]
    fn a_change_footer_names_the_diff_command() {
        assert_eq!(
            change_footer("abcd1234", 2, 4, 1),
            "change abcd1234 · 2 files +4 −1 · legate diff abcd1234"
        );
        assert_eq!(
            change_footer("abcd1234", 1, 1, 0),
            "change abcd1234 · 1 file +1 −0 · legate diff abcd1234"
        );
    }
    #[test]
    fn codex_does_not_claim_an_unreported_quota() {
        let p = plan(ProviderId::Codex);
        assert!(!plan_line(&p, "", 0, false).contains("% left"));
        assert!(!super::plan_short(&p, 0).contains("% left"));
    }
}
