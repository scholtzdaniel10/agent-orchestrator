//! Screen model from `src/cli/tui/view.ts`.

use legate_core::JobStatus;

use super::state::{Focus, JobItem, Pane, PromptMode, TuiState, WorktreeItem};
use crate::format::{format_duration, step_line};

const FOOTER_KEYS: [&str; 9] = [
    "n new job",
    "Tab switch pane",
    "Enter open",
    "d diff",
    "m merge",
    "x discard",
    "w worktree",
    "c/u agent here",
    "s stop",
];
const FOOTER_SHORT: &str = "? help  q quit";

pub fn is_narrow(columns: u16) -> bool {
    columns < 100
}

pub fn footer_text(columns: u16) -> String {
    if is_narrow(columns) {
        return FOOTER_SHORT.to_string();
    }
    let mut shown = Vec::new();
    let mut width = FOOTER_SHORT.chars().count();
    for key in FOOTER_KEYS {
        if width + key.chars().count() + 2 > columns as usize {
            break;
        }
        shown.push(key);
        width += key.chars().count() + 2;
    }
    shown.push(FOOTER_SHORT);
    shown.join("  ")
}

pub fn left_column(columns: u16) -> u16 {
    if columns < 40 {
        return (columns / 2).max(12);
    }
    if is_narrow(columns) {
        return 24;
    }
    36
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineTone {
    Plain,
    Dim,
    Add,
    Remove,
    Error,
    Accent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineModel {
    pub text: String,
    pub tone: LineTone,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowModel {
    pub text: String,
    pub selected: bool,
    pub focused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenModel {
    pub top: String,
    pub where_text: String,
    pub plans: Vec<String>,
    pub focus: Focus,
    pub worktrees_height: u16,
    pub jobs_height: u16,
    pub detail_height: u16,
    pub worktrees_title: String,
    pub jobs_title: String,
    pub worktrees: Vec<RowModel>,
    pub jobs: Vec<RowModel>,
    pub title: String,
    pub detail: Vec<LineModel>,
    pub editing: bool,
    pub placeholder: String,
    pub confirm: String,
    pub aside: String,
    pub error: Option<String>,
    pub footer: String,
    pub left: u16,
    pub color: bool,
}

pub fn body_rows(state: &TuiState) -> usize {
    let chrome = 3 + usize::from(state.error.is_some());
    (state.rows as usize).saturating_sub(chrome).max(8)
}

pub fn detail_page(state: &TuiState) -> usize {
    body_rows(state).saturating_sub(3).max(1)
}

pub fn detail_body(state: &TuiState) -> Vec<LineModel> {
    if state.pane == Pane::Help {
        return help_lines();
    }
    if state.pane == Pane::Routines {
        return routine_lines(state);
    }
    if state.pane == Pane::Diff {
        return diff_lines(&state.diff_text);
    }
    if state.pane == Pane::Worktree || (state.focus == Focus::Worktrees && state.pane != Pane::Job)
    {
        return file_lines(selected_worktree(state));
    }
    job_lines(state)
}

pub fn screen_model(state: &TuiState) -> ScreenModel {
    let left = left_column(state.columns);
    let inner = (left as usize).saturating_sub(2).max(8);
    let body = body_rows(state);
    let worktrees_height = (body / 2).max(4) as u16;
    let jobs_height = body.saturating_sub(worktrees_height as usize).max(4) as u16;
    let worktree_at = index_of(&state.worktrees, |item| item.id == state.worktree_id);
    let job_at = index_of(&state.jobs, |item| Some(&item.id) == state.job_id.as_ref());
    let title_width = (state.columns as usize)
        .saturating_sub(left as usize)
        .saturating_sub(2)
        .max(8);
    ScreenModel {
        top: top_line(state),
        where_text: where_text(state),
        plans: state.plans.iter().map(|plan| plan.text.clone()).collect(),
        focus: state.focus,
        worktrees_height,
        jobs_height,
        detail_height: body as u16,
        worktrees_title: count_title(
            "Worktrees",
            state.worktrees.iter().filter(|item| !item.project).count(),
        ),
        jobs_title: jobs_title(&state.jobs),
        worktrees: window_items(&state.worktrees, worktree_at, worktrees_height as usize - 3)
            .into_iter()
            .map(|(item, selected)| RowModel {
                text: fit(&format!("{}{}", mark(selected), worktree_text(item)), inner),
                selected,
                focused: state.focus == Focus::Worktrees && selected,
            })
            .collect(),
        jobs: window_items(&state.jobs, job_at, jobs_height as usize - 3)
            .into_iter()
            .map(|(item, selected)| RowModel {
                text: fit(&format!("{}{}", mark(selected), job_text(item)), inner),
                selected,
                focused: state.focus == Focus::Jobs && selected,
            })
            .collect(),
        title: fit(&right_title(state), title_width),
        detail: visible_detail(state),
        editing: matches!(state.mode, PromptMode::Job | PromptMode::Name),
        placeholder: if state.mode == PromptMode::Name {
            "worktree name, optional".to_string()
        } else {
            "type a job and press Enter".to_string()
        },
        confirm: confirm_text(state),
        aside: if matches!(state.mode, PromptMode::Idle | PromptMode::Job) {
            format!(
                "type: {} · worker: {} · access: {}",
                state.job_type, state.worker, state.access
            )
        } else {
            String::new()
        },
        error: state.error.clone(),
        footer: footer_text(state.columns),
        left,
        color: state.color,
    }
}

fn visible_detail(state: &TuiState) -> Vec<LineModel> {
    let lines = detail_body(state);
    let width = (state.columns as usize)
        .saturating_sub(left_column(state.columns) as usize)
        .saturating_sub(2)
        .max(8);
    let start = state.scroll;
    let end = start + detail_page(state);
    lines
        .into_iter()
        .skip(start)
        .take(end.saturating_sub(start))
        .map(|line| LineModel {
            text: fit(&line.text, width),
            tone: line.tone,
        })
        .collect()
}

fn where_text(state: &TuiState) -> String {
    let branch = state
        .branch
        .as_deref()
        .filter(|branch| !branch.is_empty())
        .unwrap_or("no branch");
    format!("{} · {branch}", state.project_name)
}

fn count_title(word: &str, count: usize) -> String {
    if count > 0 {
        format!("{word}  {count}")
    } else {
        word.to_string()
    }
}

fn jobs_title(jobs: &[JobItem]) -> String {
    let running = jobs
        .iter()
        .filter(|job| job.status == JobStatus::Running)
        .count();
    if running > 0 {
        format!("Jobs  {running} running")
    } else {
        count_title("Jobs", jobs.len())
    }
}

fn top_line(state: &TuiState) -> String {
    let left = format!("Legate · {}", where_text(state));
    let right = state
        .plans
        .iter()
        .map(|plan| plan.text.as_str())
        .collect::<Vec<_>>()
        .join("  ");
    if right.is_empty() {
        return fit(&left, state.columns as usize);
    }
    let room = (state.columns as usize)
        .saturating_sub(left.chars().count())
        .saturating_sub(2);
    if room < 8 {
        return fit(&left, state.columns as usize);
    }
    format!("{left}  {}", fit(&right, room))
}

fn right_title(state: &TuiState) -> String {
    if state.pane == Pane::Help {
        return "Keys".to_string();
    }
    if state.pane == Pane::Routines {
        return "Routines".to_string();
    }
    if state.pane == Pane::Diff {
        let row = selected_worktree(state);
        return format!("Diff {}", row.map(|row| row.label.as_str()).unwrap_or(""));
    }
    if state.pane == Pane::Worktree || (state.focus == Focus::Worktrees && state.pane != Pane::Job)
    {
        let Some(row) = selected_worktree(state) else {
            return "Worktree".to_string();
        };
        if row.project {
            return format!(
                "{} · {}",
                row.label,
                row.branch.as_deref().unwrap_or("no branch")
            );
        }
        return row.label.clone();
    }
    let Some(job) = selected_job(state) else {
        return "Job".to_string();
    };
    let mut parts = vec![format!("Job {}", chars_head(&job.id, 8))];
    if let Some(provider) = &job.provider {
        parts.push(provider.clone());
    }
    if !job.model.is_empty() {
        parts.push(job.model.clone());
    }
    parts.push(job.job_type.clone());
    parts.push(job.access.clone());
    if !job.reason.is_empty() {
        parts.push(job.reason.clone());
    }
    if job.status == JobStatus::Running {
        parts.push(format!(
            "running {}",
            format_duration(state.now - job.created_at)
        ));
    } else {
        parts.push(status_word(job.status).to_string());
    }
    parts.join(" · ")
}

fn job_lines(state: &TuiState) -> Vec<LineModel> {
    let Some(job) = selected_job(state) else {
        return vec![line("No jobs yet", LineTone::Plain)];
    };
    let mut lines = Vec::new();
    for step in &job.steps {
        let Some(text) = step_line(step, true, false) else {
            continue;
        };
        let tone = if matches!(
            step.tool,
            legate_core::StepTool::Say | legate_core::StepTool::Think
        ) {
            LineTone::Dim
        } else if step.status == legate_core::JobStepStatus::Failed {
            LineTone::Error
        } else {
            LineTone::Plain
        };
        lines.push(LineModel { text, tone });
    }
    if job.status == JobStatus::Running {
        lines.push(line("…", LineTone::Dim));
    }
    if job.status == JobStatus::Queued {
        lines.push(line("queued", LineTone::Dim));
    }
    if !job.output.trim().is_empty() {
        for row in job.output.replace("\r\n", "\n").split('\n') {
            lines.push(line(row, LineTone::Plain));
        }
    }
    if job.status == JobStatus::Failed {
        let why = if job.error.is_empty() {
            "failed"
        } else {
            job.error.as_str()
        };
        lines.push(line(&format!("failed: {why}"), LineTone::Error));
    }
    if job.files > 0 || job.added > 0 || job.removed > 0 {
        let noun = if job.files == 1 {
            "1 file".to_string()
        } else {
            format!("{} files", job.files)
        };
        lines.push(line(
            &format!("Edited {noun} +{} −{}", job.added, job.removed),
            LineTone::Plain,
        ));
    }
    lines
}

fn file_lines(row: Option<&WorktreeItem>) -> Vec<LineModel> {
    let Some(row) = row else {
        return vec![line("No folder selected", LineTone::Plain)];
    };
    if row.project {
        return vec![
            line("The project folder.", LineTone::Plain),
            line(
                "Pick a worktree to see its changes, w to make one, n to start a job.",
                LineTone::Dim,
            ),
        ];
    }
    if row.files.is_empty() {
        return vec![line("no changes", LineTone::Plain)];
    }
    row.files
        .iter()
        .map(|file| {
            if file.added.is_none() || file.removed.is_none() {
                return line(&file.path, LineTone::Plain);
            }
            line(
                &format!(
                    "{}  +{} −{}",
                    file.path,
                    file.added.unwrap_or(0),
                    file.removed.unwrap_or(0)
                ),
                LineTone::Plain,
            )
        })
        .collect()
}

fn diff_lines(text: &str) -> Vec<LineModel> {
    if text.trim().is_empty() {
        return vec![line("No diff", LineTone::Plain)];
    }
    text.replace("\r\n", "\n")
        .split('\n')
        .map(|row| LineModel {
            text: row.to_string(),
            tone: diff_tone(row),
        })
        .collect()
}

fn diff_tone(row: &str) -> LineTone {
    if row.starts_with("+++") || row.starts_with("---") {
        LineTone::Plain
    } else if row.starts_with('+') {
        LineTone::Add
    } else if row.starts_with('-') {
        LineTone::Remove
    } else if row.starts_with("@@") {
        LineTone::Accent
    } else {
        LineTone::Plain
    }
}

fn routine_lines(state: &TuiState) -> Vec<LineModel> {
    if state.routines.is_empty() {
        return vec![line("No routines.", LineTone::Plain)];
    }
    state
        .routines
        .iter()
        .enumerate()
        .map(|(index, routine)| {
            let mark = if index == state.routine_index {
                "▸ "
            } else {
                "  "
            };
            line(
                &format!(
                    "{mark}{}  {}  {}  {}",
                    routine.name, routine.job_type, routine.worker, routine.access
                ),
                LineTone::Plain,
            )
        })
        .collect()
}

fn help_lines() -> Vec<LineModel> {
    [
        "n or i        new job",
        "Tab           switch pane",
        "Up Down j k   move",
        "Enter         open",
        "d             diff",
        "Esc           back",
        "m             merge",
        "x             discard",
        "w             new worktree",
        "c             claude in this folder",
        "u             cursor agent in this folder",
        "s             stop the job",
        "r             routines",
        "g G           top and bottom",
        "PgUp PgDn     scroll",
        "Ctrl+T        job type",
        "Ctrl+W        worker",
        "Ctrl+A        access",
        "q or Ctrl+C   quit",
        "?             this help",
    ]
    .into_iter()
    .map(|text| line(text, LineTone::Plain))
    .collect()
}

fn confirm_text(state: &TuiState) -> String {
    if state.mode == PromptMode::Quit {
        return "Jobs are running. Quit and stop them? y/n".to_string();
    }
    if state.mode == PromptMode::Discard {
        let row = selected_worktree(state);
        return format!(
            "Discard {}? y/n",
            row.map(|row| row.label.as_str()).unwrap_or("worktree")
        );
    }
    String::new()
}

fn worktree_text(item: &WorktreeItem) -> String {
    if item.project {
        return format!(
            "{}  {}",
            item.label,
            item.branch.as_deref().unwrap_or("no branch")
        );
    }
    if item.files.is_empty() && item.added == 0 && item.removed == 0 {
        return format!("{}  no changes", item.label);
    }
    format!("{}  +{} −{}", item.label, item.added, item.removed)
}

fn job_text(job: &JobItem) -> String {
    let status = pad_end(status_word(job.status), 7);
    let worker = pad_end(job.provider.as_deref().unwrap_or("-"), 6);
    let prompt = job.prompt.replace(['\r', '\n'], " ");
    format!("{status} {worker} {prompt}")
}

fn selected_worktree(state: &TuiState) -> Option<&WorktreeItem> {
    state
        .worktrees
        .iter()
        .find(|item| item.id == state.worktree_id)
        .or(state.worktrees.first())
}

fn selected_job(state: &TuiState) -> Option<&JobItem> {
    state
        .jobs
        .iter()
        .find(|item| Some(&item.id) == state.job_id.as_ref())
}

fn window_items<T>(items: &[T], selected: usize, slots: usize) -> Vec<(&T, bool)> {
    if items.is_empty() || slots == 0 {
        return Vec::new();
    }
    let at = if selected >= items.len() { 0 } else { selected };
    let mut start = 0usize;
    if items.len() > slots {
        start = at.saturating_sub(slots / 2);
        if start + slots > items.len() {
            start = items.len() - slots;
        }
    }
    let end = items.len().min(start + slots);
    items[start..end]
        .iter()
        .enumerate()
        .map(|(offset, item)| (item, start + offset == at))
        .collect()
}

fn index_of<T>(items: &[T], matches: impl FnMut(&T) -> bool) -> usize {
    items.iter().position(matches).unwrap_or(0)
}

fn mark(selected: bool) -> &'static str {
    if selected { "▸ " } else { "  " }
}

fn fit(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    if width == 1 {
        return "…".to_string();
    }
    let head: String = text.chars().take(width - 1).collect();
    format!("{head}…")
}

fn pad_end(text: &str, width: usize) -> String {
    let extra = width.saturating_sub(text.chars().count());
    format!("{text}{}", " ".repeat(extra))
}

fn chars_head(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

fn status_word(status: JobStatus) -> &'static str {
    match status {
        JobStatus::Queued => "queued",
        JobStatus::Running => "running",
        JobStatus::Done => "done",
        JobStatus::Failed => "failed",
    }
}

fn line(text: &str, tone: LineTone) -> LineModel {
    LineModel {
        text: text.to_string(),
        tone,
    }
}
