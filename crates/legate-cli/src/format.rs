//! Plain-text lines from `src/cli/format.ts`, without colour.

use legate_core::{
    JobAccess, JobRecord, JobStatus, JobStep, JobStepStatus, PlanProblem, PlanStatus, ProviderId,
    StepTool, VersionStatus,
};

pub fn format_duration(ms: i64) -> String {
    let seconds = (ms / 1000).max(0);
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

pub fn allowance_percent(headroom: f64) -> i64 {
    if !headroom.is_finite() || headroom < 0.0 {
        return 0;
    }
    let raw = headroom.mul_add(100.0, 0.0).round() as i64;
    raw.clamp(0, 100)
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

fn status_word(status: JobStatus) -> &'static str {
    match status {
        JobStatus::Queued => "queued",
        JobStatus::Running => "running",
        JobStatus::Done => "done",
        JobStatus::Failed => "failed",
    }
}

pub fn header_line(job: &JobRecord) -> String {
    let mut parts = Vec::new();
    if let Some(provider) = job.provider {
        parts.push(provider.as_str().to_string());
    }
    if let Some(model) = job.model.as_deref().filter(|model| !model.is_empty()) {
        parts.push(model.to_string());
    }
    parts.push(job.job_type.as_str().to_string());
    parts.push(access_word(job).to_string());
    if let Some(reason) = job.reason.as_deref().filter(|reason| !reason.is_empty()) {
        parts.push(reason.to_string());
    }
    parts.join(" · ")
}

pub fn footer_line(status: JobStatus, elapsed_ms: i64, error: Option<&str>) -> String {
    if status == JobStatus::Done {
        return format!("done in {}", format_duration(elapsed_ms));
    }
    let why = error.filter(|text| !text.is_empty()).unwrap_or("failed");
    format!("failed: {why}")
}

pub fn step_line(step: &JobStep, verbose: bool) -> Option<String> {
    if step.status == JobStepStatus::Running {
        return None;
    }
    if !verbose && matches!(step.tool, StepTool::Say | StepTool::Think) {
        return None;
    }
    let word = tool_word(step.tool);
    let gap = " ".repeat(7usize.saturating_sub(word.len()).max(1));
    let mut line = format!("{word}{gap}{}", step.title);
    if let Some(edit) = &step.edit {
        line.push_str(&format!(" +{} −{}", edit.added, edit.removed));
    }
    if step.status == JobStepStatus::Failed {
        line.push_str(" (failed)");
    }
    Some(line)
}

pub fn change_footer(id: &str, files: usize, added: i64, removed: i64) -> String {
    let noun = if files == 1 {
        "1 file".to_string()
    } else {
        format!("{files} files")
    };
    format!("change {id} · {noun} +{added} −{removed} · legate diff {id}")
}

fn plan_word(plan: &PlanStatus, now: i64) -> String {
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

fn version_phrase(plan: &PlanStatus, tested: &str) -> String {
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

pub fn plan_line(plan: &PlanStatus, tested: &str, now: i64) -> String {
    let word = plan_word(plan, now);
    let mut parts = vec![plan.id.as_str().to_string()];
    if plan.id == ProviderId::Grok {
        parts.push("uses the Cursor plan".to_string());
    }
    parts.push(word.clone());
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
        let left = 1.0 - plan.used;
        parts.push(format!("{}% left", allowance_percent(left)));
        if plan.at_risk {
            parts.push("Unused allowance expires soon".to_string());
        }
    }
    parts.join(" · ")
}

/// Age is `-` because a [`JobRecord`] has no created-at column.
pub fn job_cells(job: &JobRecord) -> Vec<String> {
    let prompt = job.prompt.split(['\r', '\n']).collect::<Vec<_>>().join(" ");
    let prompt: String = prompt.chars().take(60).collect();
    vec![
        job.id.chars().take(8).collect(),
        status_word(job.status).to_string(),
        job.provider
            .map(|id| id.as_str().to_string())
            .unwrap_or_else(|| "-".to_string()),
        job.job_type.as_str().to_string(),
        access_word(job).to_string(),
        "-".to_string(),
        prompt,
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
            widths[index] = widths[index].max(cell.chars().count());
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
                    let extra = widths[index].saturating_sub(cell.chars().count());
                    format!("{cell}{}", " ".repeat(extra))
                })
                .collect::<Vec<_>>()
                .join("  ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::{format_duration, plan_line, table};
    use legate_core::{PlanProblem, PlanStatus, ProviderId, VersionStatus};

    fn plan(id: ProviderId) -> PlanStatus {
        PlanStatus {
            id,
            available: true,
            problem: None,
            used: 0.25,
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
            tested_version: String::new(),
        }
    }

    #[test]
    fn duration_and_plan_line_match_the_typescript_words() {
        assert_eq!(format_duration(4_000), "4s");
        assert_eq!(format_duration(125_000), "2m 5s");
        assert_eq!(format_duration(3_600_000), "1h");
        let mut local = plan(ProviderId::Local);
        local.used = 0.0;
        assert_eq!(
            plan_line(&local, "", 0),
            "local · ready · CLI version unknown · 100% left"
        );
        let mut claude = plan(ProviderId::Claude);
        claude.available = false;
        claude.problem = Some(PlanProblem::NotInstalled);
        assert_eq!(
            plan_line(&claude, "2.1.292", 0),
            "claude · not installed · Claude Code is not installed. Install it, then run `claude` once in a terminal to sign in."
        );
    }

    #[test]
    fn table_pads_every_column_but_the_last() {
        assert_eq!(table(&[vec!["ab".into(), "c".into()]]), "ab  c");
    }
}
