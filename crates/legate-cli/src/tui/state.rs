//! Key reducer from `src/cli/tui/state.ts`.

use legate_core::{JobStatus, JobStep, ProviderId};

use super::view::{detail_body, detail_page};

pub const JOB_TYPES: [&str; 5] = ["planning", "debugging", "review", "refactor", "boilerplate"];
pub const WORKERS: [&str; 4] = ["auto", "claude", "cursor", "grok"];
pub const ACCESSES: [&str; 3] = ["read", "edit", "full"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Worktrees,
    Jobs,
    Detail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Job,
    Worktree,
    Diff,
    Routines,
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptMode {
    Idle,
    Job,
    Name,
    Discard,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    pub path: String,
    pub added: Option<i64>,
    pub removed: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeItem {
    pub id: String,
    pub project: bool,
    pub label: String,
    pub branch: Option<String>,
    pub added: i64,
    pub removed: i64,
    pub files: Vec<FileRow>,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JobItem {
    pub id: String,
    pub status: JobStatus,
    pub provider: Option<String>,
    pub job_type: String,
    pub access: String,
    pub model: String,
    pub reason: String,
    pub prompt: String,
    pub output: String,
    pub error: String,
    pub steps: Vec<JobStep>,
    pub created_at: i64,
    pub started_here: bool,
    pub change: Option<String>,
    pub files: i64,
    pub added: i64,
    pub removed: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanItem {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineItem {
    pub id: String,
    pub name: String,
    pub job_type: String,
    pub worker: String,
    pub access: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TuiState {
    pub project_name: String,
    pub project_path: String,
    pub branch: Option<String>,
    pub plans: Vec<PlanItem>,
    pub worktrees: Vec<WorktreeItem>,
    pub jobs: Vec<JobItem>,
    pub routines: Vec<RoutineItem>,
    pub focus: Focus,
    pub pane: Pane,
    pub mode: PromptMode,
    pub worktree_id: String,
    pub job_id: Option<String>,
    pub routine_index: usize,
    pub draft: String,
    pub job_type: String,
    pub worker: String,
    pub access: String,
    pub diff_id: String,
    pub diff_text: String,
    pub scroll: usize,
    pub follow: bool,
    pub error: Option<String>,
    pub columns: u16,
    pub rows: u16,
    pub now: i64,
    pub color: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyPress {
    pub input: String,
    pub ctrl: bool,
    pub shift: bool,
    pub enter: bool,
    pub escape: bool,
    pub tab: bool,
    pub backspace: bool,
    pub up: bool,
    pub down: bool,
    pub page_up: bool,
    pub page_down: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Submit {
        job_type: String,
        prompt: String,
        worker: String,
        access: String,
    },
    Cancel {
        id: String,
    },
    Merge {
        id: String,
    },
    Discard {
        id: String,
    },
    CreateWorktree {
        name: String,
    },
    OpenAgent {
        provider: ProviderId,
        cwd: String,
    },
    LoadDiff {
        id: String,
    },
    LoadChanges,
    RunRoutine {
        id: String,
    },
    Quit {
        stop: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Key {
        key: KeyPress,
        draft: Option<String>,
    },
    #[allow(dead_code)]
    Draft {
        text: String,
    },
    Resize {
        columns: u16,
        rows: u16,
    },
    Tick {
        now: i64,
    },
    Plans {
        plans: Vec<PlanItem>,
    },
    Error {
        message: String,
    },
    Worktrees {
        project: WorktreeItem,
        items: Vec<WorktreeItem>,
        branch: Option<String>,
    },
    Jobs {
        jobs: Vec<JobItem>,
    },
    Job {
        job: JobItem,
    },
    Submitted {
        job: JobItem,
    },
    Routines {
        routines: Vec<RoutineItem>,
    },
    Diff {
        id: String,
        text: String,
    },
    SelectWorktree {
        id: String,
    },
}

pub struct ReduceResult {
    pub state: TuiState,
    pub effects: Vec<Effect>,
}

const PROMPT_MAX: usize = 8_000;

pub fn initial_state(
    project_name: &str,
    columns: u16,
    rows: u16,
    color: bool,
    now: i64,
) -> TuiState {
    TuiState {
        project_name: project_name.to_string(),
        project_path: String::new(),
        branch: None,
        plans: Vec::new(),
        worktrees: vec![project_row(project_name, "", None)],
        jobs: Vec::new(),
        routines: Vec::new(),
        focus: Focus::Worktrees,
        pane: Pane::Worktree,
        mode: PromptMode::Idle,
        worktree_id: String::new(),
        job_id: None,
        routine_index: 0,
        draft: String::new(),
        job_type: "planning".to_string(),
        worker: "auto".to_string(),
        access: "read".to_string(),
        diff_id: String::new(),
        diff_text: String::new(),
        scroll: 0,
        follow: true,
        error: None,
        columns,
        rows,
        now,
        color,
    }
}

#[allow(dead_code)]
pub fn key_press(input: &str) -> KeyPress {
    KeyPress {
        input: input.to_string(),
        ..KeyPress::default()
    }
}

/// Ink's text field inserts a ctrl letter, then the app strips it.
#[allow(dead_code)]
pub fn edit_action(draft: &str, key: KeyPress) -> Action {
    if key.escape || (key.ctrl && key.input == "c") {
        return Action::Key { key, draft: None };
    }
    if key.ctrl && matches!(key.input.as_str(), "t" | "w" | "a") {
        let next = if draft.ends_with(&key.input) {
            draft[..draft.len() - key.input.len()].to_string()
        } else {
            draft.to_string()
        };
        return Action::Key {
            key,
            draft: Some(next),
        };
    }
    Action::Key { key, draft: None }
}

pub fn reduce(state: &TuiState, action: Action) -> ReduceResult {
    match action {
        Action::Key { key, draft } => handle_key(&clear_error(state, draft.as_deref()), &key),
        Action::Draft { text } => done(
            TuiState {
                draft: text.chars().take(PROMPT_MAX).collect(),
                ..state.clone()
            },
            vec![],
        ),
        Action::Resize { columns, rows } => {
            if state.columns == columns && state.rows == rows {
                return done(state.clone(), vec![]);
            }
            done(
                TuiState {
                    columns,
                    rows,
                    ..state.clone()
                },
                vec![],
            )
        }
        Action::Tick { now } => done(
            TuiState {
                now,
                ..state.clone()
            },
            vec![],
        ),
        Action::Plans { plans } => done(
            TuiState {
                plans,
                ..state.clone()
            },
            vec![],
        ),
        Action::Error { message } => done(
            TuiState {
                error: Some(message),
                ..state.clone()
            },
            vec![],
        ),
        Action::Worktrees {
            project,
            items,
            branch,
        } => apply_worktrees(state, project, items, branch),
        Action::Jobs { jobs } => apply_jobs(state, jobs),
        Action::Job { job } => apply_job(state, job, false),
        Action::Submitted { job } => apply_job(state, job, true),
        Action::Routines { routines } => {
            let routine_index = state.routine_index.min(routines.len().saturating_sub(1));
            done(
                TuiState {
                    routines,
                    routine_index,
                    ..state.clone()
                },
                vec![],
            )
        }
        Action::Diff { id, text } => {
            if state.pane != Pane::Diff || state.diff_id != id {
                return done(state.clone(), vec![]);
            }
            done(
                TuiState {
                    diff_text: text,
                    ..state.clone()
                },
                vec![],
            )
        }
        Action::SelectWorktree { id } => done(
            TuiState {
                worktree_id: id,
                pane: Pane::Worktree,
                focus: Focus::Worktrees,
                ..state.clone()
            },
            vec![],
        ),
    }
}

pub fn selected_worktree(state: &TuiState) -> Option<&WorktreeItem> {
    state
        .worktrees
        .iter()
        .find(|item| item.id == state.worktree_id)
        .or(state.worktrees.first())
}

pub fn selected_job(state: &TuiState) -> Option<&JobItem> {
    state
        .jobs
        .iter()
        .find(|item| Some(&item.id) == state.job_id.as_ref())
}

fn project_row(label: &str, path: &str, branch: Option<String>) -> WorktreeItem {
    WorktreeItem {
        id: String::new(),
        project: true,
        label: label.to_string(),
        branch,
        added: 0,
        removed: 0,
        files: Vec::new(),
        path: path.to_string(),
    }
}

fn clear_error(state: &TuiState, draft: Option<&str>) -> TuiState {
    let mut next = state.clone();
    if next.error.is_some() {
        next.error = None;
    }
    if let Some(draft) = draft {
        next.draft = draft.to_string();
    }
    next
}

fn done(state: TuiState, effects: Vec<Effect>) -> ReduceResult {
    ReduceResult {
        state: clamp_scroll(state),
        effects,
    }
}

fn clamp_scroll(mut state: TuiState) -> TuiState {
    let max = detail_body(&state)
        .len()
        .saturating_sub(detail_page(&state));
    let scroll = if state.follow {
        max
    } else {
        state.scroll.min(max)
    };
    if scroll != state.scroll {
        state.scroll = scroll;
    }
    state
}

fn handle_key(state: &TuiState, key: &KeyPress) -> ReduceResult {
    if matches!(state.mode, PromptMode::Discard | PromptMode::Quit) {
        return confirm_key(state, key);
    }
    if matches!(state.mode, PromptMode::Job | PromptMode::Name) {
        return prompt_key(state, key);
    }
    browse_key(state, key)
}

fn confirm_key(state: &TuiState, key: &KeyPress) -> ReduceResult {
    if key.input == "y" {
        if state.mode == PromptMode::Discard {
            let row = selected_worktree(state);
            if row.is_none_or(|row| row.project) {
                return done(idle(state), vec![]);
            }
            let id = row.map(|row| row.id.clone()).unwrap_or_default();
            return done(idle(state), vec![Effect::Discard { id }]);
        }
        return done(
            state.clone(),
            vec![Effect::Quit {
                stop: running_here(state),
            }],
        );
    }
    if key.input == "n" || key.escape {
        return done(idle(state), vec![]);
    }
    if key.ctrl && key.input == "c" {
        return done(
            state.clone(),
            vec![Effect::Quit {
                stop: running_here(state),
            }],
        );
    }
    done(state.clone(), vec![])
}

fn idle(state: &TuiState) -> TuiState {
    TuiState {
        mode: PromptMode::Idle,
        ..state.clone()
    }
}

fn prompt_key(state: &TuiState, key: &KeyPress) -> ReduceResult {
    if key.escape {
        return done(idle(state), vec![]);
    }
    if key.ctrl && key.input == "c" {
        return request_quit(state);
    }
    if state.mode == PromptMode::Job && key.ctrl && key.input == "t" {
        return done(
            TuiState {
                job_type: cycle(&JOB_TYPES, &state.job_type),
                ..state.clone()
            },
            vec![],
        );
    }
    if state.mode == PromptMode::Job && key.ctrl && key.input == "w" {
        return done(
            TuiState {
                worker: cycle(&WORKERS, &state.worker),
                ..state.clone()
            },
            vec![],
        );
    }
    if state.mode == PromptMode::Job && key.ctrl && key.input == "a" {
        return done(
            TuiState {
                access: cycle(&ACCESSES, &state.access),
                ..state.clone()
            },
            vec![],
        );
    }
    if key.enter {
        return if state.mode == PromptMode::Job {
            submit_draft(state)
        } else {
            name_draft(state)
        };
    }
    if key.backspace {
        let mut draft = state.draft.clone();
        draft.pop();
        return done(
            TuiState {
                draft,
                ..state.clone()
            },
            vec![],
        );
    }
    if key.ctrl || key.tab || key.up || key.down || key.page_up || key.page_down {
        return done(state.clone(), vec![]);
    }
    if key.input.is_empty() {
        return done(state.clone(), vec![]);
    }
    let draft: String = (state.draft.clone() + &key.input)
        .chars()
        .take(PROMPT_MAX)
        .collect();
    done(
        TuiState {
            draft,
            ..state.clone()
        },
        vec![],
    )
}

fn browse_key(state: &TuiState, key: &KeyPress) -> ReduceResult {
    if key.ctrl && key.input == "c" {
        return request_quit(state);
    }
    if key.input == "q" {
        return request_quit(state);
    }
    if key.escape && state.pane == Pane::Diff {
        return done(
            TuiState {
                pane: Pane::Worktree,
                follow: false,
                scroll: 0,
                ..state.clone()
            },
            vec![],
        );
    }
    if key.escape && matches!(state.pane, Pane::Help | Pane::Routines) {
        return done(close_overlay(state), vec![]);
    }
    if state.pane == Pane::Help && key.input == "?" {
        return done(close_overlay(state), vec![]);
    }
    if key.tab {
        return tab_focus(state);
    }
    if key.up || key.input == "k" {
        return move_sel(state, -1);
    }
    if key.down || key.input == "j" {
        return move_sel(state, 1);
    }
    if key.page_up {
        return scroll_by(state, -(detail_page(state) as i64));
    }
    if key.page_down {
        return scroll_by(state, detail_page(state) as i64);
    }
    if key.input == "g" && !key.shift {
        return jump(state, false);
    }
    if key.input == "G" || (key.input == "g" && key.shift) {
        return jump(state, true);
    }
    if key.input == "n" || key.input == "i" {
        return done(
            TuiState {
                mode: PromptMode::Job,
                ..state.clone()
            },
            vec![],
        );
    }
    if key.input == "?" {
        return done(
            TuiState {
                pane: Pane::Help,
                focus: Focus::Detail,
                follow: false,
                scroll: 0,
                ..state.clone()
            },
            vec![],
        );
    }
    if key.input == "r" {
        return done(
            TuiState {
                pane: Pane::Routines,
                focus: Focus::Detail,
                follow: false,
                scroll: 0,
                ..state.clone()
            },
            vec![],
        );
    }
    if key.enter {
        return open_selection(state);
    }
    if key.input == "d" {
        return open_diff(state);
    }
    if key.input == "m" {
        return merge_selected(state);
    }
    if key.input == "x" {
        return discard_selected(state);
    }
    if key.input == "w" {
        return done(
            TuiState {
                mode: PromptMode::Name,
                draft: String::new(),
                ..state.clone()
            },
            vec![],
        );
    }
    if key.input == "s" {
        return stop_selected(state);
    }
    if key.input == "c" || key.input == "u" {
        let provider = if key.input == "c" {
            ProviderId::Claude
        } else {
            ProviderId::Cursor
        };
        return open_agent(state, provider);
    }
    done(state.clone(), vec![])
}

fn close_overlay(state: &TuiState) -> TuiState {
    TuiState {
        pane: if state.focus == Focus::Worktrees {
            Pane::Worktree
        } else {
            Pane::Job
        },
        ..state.clone()
    }
}

fn tab_focus(state: &TuiState) -> ReduceResult {
    let order = [Focus::Worktrees, Focus::Jobs, Focus::Detail];
    let index = order
        .iter()
        .position(|item| *item == state.focus)
        .unwrap_or(0);
    let focus = order[(index + 1) % order.len()];
    let mut pane = state.pane;
    if matches!(pane, Pane::Job | Pane::Worktree) {
        if focus == Focus::Worktrees {
            pane = Pane::Worktree;
        } else if focus == Focus::Jobs {
            pane = Pane::Job;
        }
    }
    done(
        TuiState {
            focus,
            pane,
            ..state.clone()
        },
        vec![],
    )
}

fn move_sel(state: &TuiState, delta: i64) -> ReduceResult {
    if state.pane == Pane::Routines {
        let max = state.routines.len().saturating_sub(1);
        return done(
            TuiState {
                routine_index: clamp_index(state.routine_index as i64 + delta, max),
                ..state.clone()
            },
            vec![],
        );
    }
    if matches!(state.pane, Pane::Diff | Pane::Help) || state.focus == Focus::Detail {
        return scroll_by(state, delta);
    }
    if state.focus == Focus::Worktrees {
        let max = state.worktrees.len().saturating_sub(1);
        let index = clamp_index(worktree_index(state) as i64 + delta, max);
        let id = state
            .worktrees
            .get(index)
            .map(|row| row.id.clone())
            .unwrap_or_default();
        return done(
            TuiState {
                worktree_id: id,
                ..state.clone()
            },
            vec![],
        );
    }
    let max = state.jobs.len().saturating_sub(1);
    let index = clamp_index(job_index(state) as i64 + delta, max);
    let Some(row) = state.jobs.get(index) else {
        return done(state.clone(), vec![]);
    };
    done(
        TuiState {
            job_id: Some(row.id.clone()),
            pane: Pane::Job,
            follow: true,
            ..state.clone()
        },
        vec![],
    )
}

fn scroll_by(state: &TuiState, delta: i64) -> ReduceResult {
    if state.pane == Pane::Routines {
        return move_sel(state, delta);
    }
    let max = detail_body(state).len().saturating_sub(detail_page(state));
    let scroll = clamp_index(state.scroll as i64 + delta, max);
    let follow = delta > 0 && scroll >= max;
    done(
        TuiState {
            scroll,
            follow: if delta < 0 { false } else { follow },
            ..state.clone()
        },
        vec![],
    )
}

fn jump(state: &TuiState, end: bool) -> ReduceResult {
    if state.pane == Pane::Routines {
        let routine_index = if end {
            state.routines.len().saturating_sub(1)
        } else {
            0
        };
        return done(
            TuiState {
                routine_index,
                ..state.clone()
            },
            vec![],
        );
    }
    if end {
        return done(
            TuiState {
                follow: true,
                ..state.clone()
            },
            vec![],
        );
    }
    done(
        TuiState {
            scroll: 0,
            follow: false,
            ..state.clone()
        },
        vec![],
    )
}

fn open_selection(state: &TuiState) -> ReduceResult {
    if state.pane == Pane::Routines {
        let Some(routine) = state.routines.get(state.routine_index) else {
            return done(state.clone(), vec![]);
        };
        return done(
            TuiState {
                pane: Pane::Job,
                focus: Focus::Detail,
                follow: true,
                ..state.clone()
            },
            vec![Effect::RunRoutine {
                id: routine.id.clone(),
            }],
        );
    }
    if state.pane == Pane::Help {
        return done(close_overlay(state), vec![]);
    }
    if state.focus == Focus::Worktrees || state.pane == Pane::Worktree {
        return open_diff(state);
    }
    done(
        TuiState {
            focus: Focus::Detail,
            pane: Pane::Job,
            ..state.clone()
        },
        vec![],
    )
}

fn open_diff(state: &TuiState) -> ReduceResult {
    let row = selected_worktree(state);
    if row.is_none_or(|row| row.project) {
        return done(
            TuiState {
                error: Some("Select a worktree to see its diff.".to_string()),
                ..state.clone()
            },
            vec![],
        );
    }
    let id = row.map(|row| row.id.clone()).unwrap_or_default();
    done(
        TuiState {
            pane: Pane::Diff,
            diff_id: id.clone(),
            diff_text: String::new(),
            follow: false,
            scroll: 0,
            focus: Focus::Detail,
            ..state.clone()
        },
        vec![Effect::LoadDiff { id }],
    )
}

fn merge_selected(state: &TuiState) -> ReduceResult {
    let row = selected_worktree(state);
    if row.is_none_or(|row| row.project) {
        return done(
            TuiState {
                error: Some("Select a worktree to merge.".to_string()),
                ..state.clone()
            },
            vec![],
        );
    }
    let id = row.map(|row| row.id.clone()).unwrap_or_default();
    done(state.clone(), vec![Effect::Merge { id }])
}

fn discard_selected(state: &TuiState) -> ReduceResult {
    let row = selected_worktree(state);
    if row.is_none_or(|row| row.project) {
        return done(
            TuiState {
                error: Some("Select a worktree to discard.".to_string()),
                ..state.clone()
            },
            vec![],
        );
    }
    done(
        TuiState {
            mode: PromptMode::Discard,
            ..state.clone()
        },
        vec![],
    )
}

fn stop_selected(state: &TuiState) -> ReduceResult {
    let job = selected_job(state);
    if job.is_none_or(|job| !matches!(job.status, JobStatus::Running | JobStatus::Queued)) {
        return done(
            TuiState {
                error: Some("That job is not running.".to_string()),
                ..state.clone()
            },
            vec![],
        );
    }
    let id = job.map(|job| job.id.clone()).unwrap_or_default();
    done(state.clone(), vec![Effect::Cancel { id }])
}

fn open_agent(state: &TuiState, provider: ProviderId) -> ReduceResult {
    let row = selected_worktree(state);
    let cwd = row
        .map(|row| row.path.clone())
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| state.project_path.clone());
    if cwd.is_empty() {
        return done(
            TuiState {
                error: Some("Select a folder first.".to_string()),
                ..state.clone()
            },
            vec![],
        );
    }
    done(state.clone(), vec![Effect::OpenAgent { provider, cwd }])
}

fn submit_draft(state: &TuiState) -> ReduceResult {
    let prompt = state.draft.trim().to_string();
    if prompt.is_empty() {
        return done(state.clone(), vec![]);
    }
    let effect = Effect::Submit {
        job_type: state.job_type.clone(),
        prompt,
        worker: state.worker.clone(),
        access: state.access.clone(),
    };
    done(
        TuiState {
            mode: PromptMode::Idle,
            draft: String::new(),
            pane: Pane::Job,
            focus: Focus::Detail,
            follow: true,
            ..state.clone()
        },
        vec![effect],
    )
}

fn name_draft(state: &TuiState) -> ReduceResult {
    if state.draft.chars().any(|ch| ch == '\r' || ch == '\n')
        || state.draft.trim().chars().count() > 40
    {
        return done(
            TuiState {
                error: Some("Name must be 1 to 40 characters on one line.".to_string()),
                ..state.clone()
            },
            vec![],
        );
    }
    let name = state.draft.trim().to_string();
    done(
        TuiState {
            mode: PromptMode::Idle,
            draft: String::new(),
            ..state.clone()
        },
        vec![Effect::CreateWorktree { name }],
    )
}

fn request_quit(state: &TuiState) -> ReduceResult {
    let stop = running_here(state);
    if !stop.is_empty() {
        return done(
            TuiState {
                mode: PromptMode::Quit,
                ..state.clone()
            },
            vec![],
        );
    }
    done(state.clone(), vec![Effect::Quit { stop }])
}

fn running_here(state: &TuiState) -> Vec<String> {
    state
        .jobs
        .iter()
        .filter(|job| {
            job.started_here && matches!(job.status, JobStatus::Running | JobStatus::Queued)
        })
        .map(|job| job.id.clone())
        .collect()
}

fn apply_worktrees(
    state: &TuiState,
    project: WorktreeItem,
    items: Vec<WorktreeItem>,
    branch: Option<String>,
) -> ReduceResult {
    let mut worktrees = vec![project.clone()];
    worktrees.extend(items.iter().cloned());
    let worktree_id = if worktrees.iter().any(|item| item.id == state.worktree_id) {
        state.worktree_id.clone()
    } else {
        worktrees
            .first()
            .map(|item| item.id.clone())
            .unwrap_or_default()
    };
    let jobs = state
        .jobs
        .iter()
        .map(|job| apply_tree_counts(job, &items))
        .collect();
    done(
        TuiState {
            worktrees,
            worktree_id,
            branch,
            project_path: project.path,
            project_name: project.label,
            jobs,
            ..state.clone()
        },
        vec![],
    )
}

fn apply_jobs(state: &TuiState, incoming: Vec<JobItem>) -> ReduceResult {
    let mut jobs: Vec<JobItem> = incoming
        .into_iter()
        .map(
            |job| match state.jobs.iter().find(|item| item.id == job.id) {
                None => job,
                Some(prev) => JobItem {
                    started_here: prev.started_here || job.started_here,
                    created_at: prev.created_at,
                    ..job
                },
            },
        )
        .collect();
    jobs.sort_by(by_newest);
    let job_id = match &state.job_id {
        Some(id) if jobs.iter().any(|job| &job.id == id) => Some(id.clone()),
        _ => jobs.first().map(|job| job.id.clone()),
    };
    done(
        TuiState {
            jobs,
            job_id,
            ..state.clone()
        },
        vec![],
    )
}

fn apply_job(state: &TuiState, job: JobItem, select: bool) -> ReduceResult {
    let id = job.id.clone();
    let prev = state.jobs.iter().find(|item| item.id == id).cloned();
    let ended = prev.as_ref().is_some_and(|prev| {
        prev.status != job.status && matches!(job.status, JobStatus::Done | JobStatus::Failed)
    });
    let merged = merge_job(prev.as_ref(), job);
    let mut jobs: Vec<JobItem> = if prev.is_none() {
        let mut jobs = vec![merged];
        jobs.extend(state.jobs.iter().cloned());
        jobs
    } else {
        state
            .jobs
            .iter()
            .map(|item| {
                if item.id == id {
                    merged.clone()
                } else {
                    item.clone()
                }
            })
            .collect()
    };
    jobs.sort_by(by_newest);
    let mut effects = Vec::new();
    if ended {
        effects.push(Effect::LoadChanges);
    }
    if !select {
        return done(
            TuiState {
                jobs,
                ..state.clone()
            },
            effects,
        );
    }
    done(
        TuiState {
            jobs,
            job_id: Some(id),
            pane: Pane::Job,
            focus: Focus::Detail,
            follow: true,
            mode: PromptMode::Idle,
            ..state.clone()
        },
        effects,
    )
}

fn merge_job(prev: Option<&JobItem>, job: JobItem) -> JobItem {
    let Some(prev) = prev else {
        return job;
    };
    let keep = job.files == 0 && job.added == 0 && job.removed == 0;
    JobItem {
        created_at: prev.created_at,
        started_here: prev.started_here || job.started_here,
        files: if keep { prev.files } else { job.files },
        added: if keep { prev.added } else { job.added },
        removed: if keep { prev.removed } else { job.removed },
        ..job
    }
}

fn apply_tree_counts(job: &JobItem, items: &[WorktreeItem]) -> JobItem {
    if job.change.is_none() {
        return job.clone();
    }
    if job.status == JobStatus::Running && (job.added > 0 || job.removed > 0) {
        return job.clone();
    }
    let Some(id) = job.change.as_deref() else {
        return job.clone();
    };
    let Some(matched) = items.iter().find(|item| item.id == id) else {
        return job.clone();
    };
    JobItem {
        files: matched.files.len() as i64,
        added: matched.added,
        removed: matched.removed,
        ..job.clone()
    }
}

fn by_newest(a: &JobItem, b: &JobItem) -> std::cmp::Ordering {
    b.created_at
        .cmp(&a.created_at)
        .then_with(|| a.id.cmp(&b.id))
}

fn worktree_index(state: &TuiState) -> usize {
    state
        .worktrees
        .iter()
        .position(|item| item.id == state.worktree_id)
        .unwrap_or(0)
}

fn job_index(state: &TuiState) -> usize {
    state
        .jobs
        .iter()
        .position(|item| Some(&item.id) == state.job_id.as_ref())
        .unwrap_or(0)
}

fn cycle(list: &[&str], current: &str) -> String {
    let index = list.iter().position(|item| *item == current).unwrap_or(0);
    list[(index + 1) % list.len()].to_string()
}

fn clamp_index(value: i64, max: usize) -> usize {
    if value < 0 {
        0
    } else if value as usize > max {
        max
    } else {
        value as usize
    }
}
