//! Reducer, screen, and session tests from `src/cli/tui`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use legate_core::{JobStatus, JobStep, JobStepStatus, StepTool};

use super::render::frame_text;
use super::session::run_session;
use super::state::{
    Action, Effect, FileRow, Focus, JobItem, KeyPress, Pane, PromptMode, TuiState, WorktreeItem,
    edit_action, initial_state, key_press, reduce,
};
use super::view::{detail_body, detail_page, footer_text, is_narrow};
use crate::drive::{DriveError, JobRequest};
use crate::harness::{Fake, job as record};

fn press(input: &str) -> KeyPress {
    key_press(input)
}

fn key(input: &str) -> Action {
    Action::Key {
        key: press(input),
        draft: None,
    }
}

fn flag(mut key: KeyPress) -> Action {
    key.input.clear();
    Action::Key { key, draft: None }
}

fn job_item(id: &str) -> JobItem {
    JobItem {
        id: id.to_string(),
        status: JobStatus::Done,
        provider: Some("cursor".to_string()),
        job_type: "planning".to_string(),
        access: "read".to_string(),
        model: String::new(),
        reason: String::new(),
        prompt: "do the thing".to_string(),
        output: String::new(),
        error: String::new(),
        steps: Vec::new(),
        created_at: 1,
        started_here: false,
        change: None,
        files: 0,
        added: 0,
        removed: 0,
    }
}

fn tree(id: &str, label: &str) -> WorktreeItem {
    WorktreeItem {
        id: id.to_string(),
        project: false,
        label: label.to_string(),
        branch: Some(format!("orch/{id}")),
        added: 0,
        removed: 0,
        files: Vec::new(),
        path: format!("/wt/{id}"),
    }
}

fn base() -> TuiState {
    let mut state = initial_state("agent-orchestrator", 120, 30, true, 10_000);
    state.project_path = "/proj".to_string();
    state.branch = Some("v1".to_string());
    let mut project = tree("", "agent-orchestrator");
    project.project = true;
    project.branch = Some("v1".to_string());
    project.path = "/proj".to_string();
    let mut login = tree("login123", "login fix");
    login.added = 10;
    login.removed = 3;
    login.files = vec![FileRow {
        path: "a.ts".to_string(),
        added: Some(10),
        removed: Some(3),
    }];
    state.worktrees = vec![project, login, tree("3687475d", "3687475d")];
    state.worktree_id = "login123".to_string();
    let mut running = job_item("job-run");
    running.status = JobStatus::Running;
    running.prompt = "Add a button".to_string();
    running.created_at = 9_000;
    running.started_here = true;
    let mut done = job_item("job-done");
    done.provider = Some("claude".to_string());
    done.prompt = "Review the diff".to_string();
    done.created_at = 8_000;
    let mut failed = job_item("job-bad");
    failed.status = JobStatus::Failed;
    failed.prompt = "Fix the crash".to_string();
    failed.created_at = 7_000;
    failed.error = "tests failed".to_string();
    state.jobs = vec![running, done, failed];
    state.job_id = Some("job-run".to_string());
    state.focus = Focus::Jobs;
    state.pane = Pane::Job;
    state
}

fn screen_job(id: &str) -> JobItem {
    let mut job = job_item(id);
    job.job_type = "boilerplate".to_string();
    job.access = "full".to_string();
    job.model = "Grok 4.6".to_string();
    job
}

fn screen_state() -> TuiState {
    let mut state = initial_state("agent-orchestrator", 120, 30, true, 20_000);
    state.project_path = "/proj".to_string();
    state.branch = Some("v1".to_string());
    let mut project = tree("", "agent-orchestrator");
    project.project = true;
    project.branch = Some("v1".to_string());
    project.path = "/proj".to_string();
    let mut login = tree("login123", "login fix");
    login.branch = None;
    login.added = 10;
    login.removed = 3;
    login.files = vec![FileRow {
        path: "src/core/github.ts".to_string(),
        added: Some(10),
        removed: Some(3),
    }];
    let mut other = tree("3687475d", "3687475d");
    other.branch = None;
    state.worktrees = vec![project, login, other];
    state.worktree_id = "login123".to_string();
    let mut running = screen_job("aaaa1111");
    running.status = JobStatus::Running;
    running.prompt = "Add a button".to_string();
    running.created_at = 12_000;
    let mut done = screen_job("bbbb2222");
    done.provider = Some("claude".to_string());
    done.prompt = "Review the diff".to_string();
    done.created_at = 8_000;
    let mut failed = screen_job("cccc3333");
    failed.status = JobStatus::Failed;
    failed.prompt = "Fix the crash".to_string();
    failed.created_at = 4_000;
    state.jobs = vec![running, done, failed];
    state.job_id = Some("aaaa1111".to_string());
    state.focus = Focus::Jobs;
    state.pane = Pane::Job;
    state
}

fn read_step(index: usize) -> JobStep {
    JobStep {
        id: format!("s{index}"),
        tool: StepTool::Read,
        title: format!("src/file-{index}.ts"),
        detail: None,
        status: JobStepStatus::Done,
        started_at: 1,
        ended_at: None,
        edit: None,
    }
}

#[test]
fn j_and_k_move_the_focused_list_and_stop_at_the_ends() {
    let start = TuiState {
        focus: Focus::Worktrees,
        pane: Pane::Worktree,
        worktree_id: "login123".to_string(),
        ..base()
    };
    let down = reduce(&start, key("j")).state;
    assert_eq!(down.worktree_id, "3687475d");
    let stuck = reduce(
        &down,
        flag(KeyPress {
            down: true,
            ..KeyPress::default()
        }),
    )
    .state;
    assert_eq!(stuck.worktree_id, "3687475d");
    let up = reduce(&stuck, key("k")).state;
    assert_eq!(up.worktree_id, "login123");
    let top = reduce(
        &TuiState {
            worktree_id: String::new(),
            ..up
        },
        flag(KeyPress {
            up: true,
            ..KeyPress::default()
        }),
    )
    .state;
    assert_eq!(top.worktree_id, "");
}

#[test]
fn tab_cycles_focus_between_worktrees_jobs_and_the_detail() {
    let mut state = TuiState {
        focus: Focus::Worktrees,
        pane: Pane::Worktree,
        ..base()
    };
    state = reduce(
        &state,
        flag(KeyPress {
            tab: true,
            ..KeyPress::default()
        }),
    )
    .state;
    assert_eq!(state.focus, Focus::Jobs);
    assert_eq!(state.pane, Pane::Job);
    state = reduce(
        &state,
        flag(KeyPress {
            tab: true,
            ..KeyPress::default()
        }),
    )
    .state;
    assert_eq!(state.focus, Focus::Detail);
    assert_eq!(state.pane, Pane::Job);
    state = reduce(
        &state,
        flag(KeyPress {
            tab: true,
            ..KeyPress::default()
        }),
    )
    .state;
    assert_eq!(state.focus, Focus::Worktrees);
    assert_eq!(state.pane, Pane::Worktree);
}

#[test]
fn the_prompt_edits_cycles_and_ignores_an_empty_submit() {
    let mut state = reduce(&base(), key("n")).state;
    assert_eq!(state.mode, PromptMode::Job);
    state = reduce(&state, key("hi")).state;
    state = reduce(
        &state,
        flag(KeyPress {
            backspace: true,
            ..KeyPress::default()
        }),
    )
    .state;
    assert_eq!(state.draft, "h");
    state = reduce(&state, ctrl("t")).state;
    assert_eq!(state.job_type, "debugging");
    state = reduce(&state, ctrl("w")).state;
    assert_eq!(state.worker, "claude");
    state = reduce(&state, ctrl("w")).state;
    assert_eq!(state.worker, "cursor");
    state = reduce(&state, ctrl("w")).state;
    assert_eq!(state.worker, "grok");
    state = reduce(&state, ctrl("w")).state;
    assert_eq!(state.worker, "auto");
    state = reduce(&state, ctrl("a")).state;
    assert_eq!(state.access, "edit");
    state = reduce(&state, ctrl("a")).state;
    assert_eq!(state.access, "full");
    state = reduce(&state, ctrl("a")).state;
    assert_eq!(state.access, "read");
    let empty = reduce(
        &TuiState {
            draft: "   ".to_string(),
            ..state.clone()
        },
        flag(KeyPress {
            enter: true,
            ..KeyPress::default()
        }),
    );
    assert!(empty.effects.is_empty());
    assert_eq!(empty.state.mode, PromptMode::Job);
    let left = reduce(
        &state,
        flag(KeyPress {
            escape: true,
            ..KeyPress::default()
        }),
    )
    .state;
    assert_eq!(left.mode, PromptMode::Idle);
    assert_eq!(left.draft, "h");
}

fn ctrl(input: &str) -> Action {
    Action::Key {
        key: KeyPress {
            input: input.to_string(),
            ctrl: true,
            ..KeyPress::default()
        },
        draft: None,
    }
}

#[test]
fn enter_submits_the_draft_and_the_new_job_becomes_the_selection() {
    let mut state = reduce(&base(), key("i")).state;
    state.draft = "ship the ui".to_string();
    state.job_type = "refactor".to_string();
    state.worker = "cursor".to_string();
    state.access = "edit".to_string();
    let submitted = reduce(
        &state,
        flag(KeyPress {
            enter: true,
            ..KeyPress::default()
        }),
    );
    assert_eq!(
        submitted.effects,
        vec![Effect::Submit {
            job_type: "refactor".to_string(),
            prompt: "ship the ui".to_string(),
            worker: "cursor".to_string(),
            access: "edit".to_string(),
        }]
    );
    assert_eq!(submitted.state.mode, PromptMode::Idle);
    assert_eq!(submitted.state.draft, "");
    let mut arrived_job = job_item("job-new");
    arrived_job.status = JobStatus::Queued;
    arrived_job.prompt = "ship the ui".to_string();
    arrived_job.created_at = 11_000;
    arrived_job.started_here = true;
    let arrived = reduce(&submitted.state, Action::Submitted { job: arrived_job }).state;
    assert_eq!(arrived.job_id.as_deref(), Some("job-new"));
    assert_eq!(arrived.pane, Pane::Job);
    assert_eq!(arrived.focus, Focus::Detail);
    assert_eq!(arrived.jobs[0].id, "job-new");
}

#[test]
fn follow_stays_at_the_bottom_until_the_reader_scrolls_up() {
    let steps: Vec<JobStep> = (0..8).map(read_step).collect();
    let mut running = job_item("job-run");
    running.status = JobStatus::Running;
    running.steps = steps.clone();
    running.created_at = 9_000;
    running.started_here = true;
    let mut state = reduce(
        &TuiState {
            rows: 10,
            jobs: vec![running.clone()],
            job_id: Some("job-run".to_string()),
            follow: true,
            scroll: 0,
            ..base()
        },
        Action::Tick { now: 10_000 },
    )
    .state;
    let max = detail_body(&state)
        .len()
        .saturating_sub(detail_page(&state));
    assert!(max > 0);
    assert!(state.follow);
    assert_eq!(state.scroll, max);

    state = reduce(
        &state,
        flag(KeyPress {
            page_up: true,
            ..KeyPress::default()
        }),
    )
    .state;
    assert!(!state.follow);
    let held = state.scroll;
    assert!(held < max);

    let mut more = running;
    let mut extra = read_step(0);
    extra.id = "s-extra".to_string();
    extra.tool = StepTool::Shell;
    extra.title = "pnpm test".to_string();
    more.steps.push(extra);
    state = reduce(&state, Action::Job { job: more }).state;
    assert!(!state.follow);
    assert_eq!(state.scroll, held);

    state = reduce(&state, key("G")).state;
    let end = detail_body(&state)
        .len()
        .saturating_sub(detail_page(&state));
    assert!(state.follow);
    assert_eq!(state.scroll, end);
}

#[test]
fn discard_and_quit_wait_for_y_and_n_leaves_them_alone() {
    let discard = reduce(&base(), key("x"));
    assert_eq!(discard.state.mode, PromptMode::Discard);
    assert!(discard.effects.is_empty());
    let kept = reduce(&discard.state, key("n"));
    assert_eq!(kept.state.mode, PromptMode::Idle);
    assert!(kept.effects.is_empty());
    let armed = reduce(&base(), key("x")).state;
    let gone = reduce(&armed, key("y"));
    assert_eq!(
        gone.effects,
        vec![Effect::Discard {
            id: "login123".to_string()
        }]
    );

    let quit = reduce(&base(), key("q"));
    assert_eq!(quit.state.mode, PromptMode::Quit);
    assert!(quit.effects.is_empty());
    let stopped = reduce(&quit.state, key("y"));
    assert_eq!(
        stopped.effects,
        vec![Effect::Quit {
            stop: vec!["job-run".to_string()]
        }]
    );

    let mut idle = base();
    for job in &mut idle.jobs {
        job.started_here = false;
    }
    let free = reduce(&idle, key("q"));
    assert_eq!(free.effects, vec![Effect::Quit { stop: Vec::new() }]);
}

#[test]
fn narrow_widths_use_the_short_footer() {
    assert!(is_narrow(80));
    assert!(is_narrow(99));
    assert!(!is_narrow(100));
    assert_eq!(footer_text(80), "? help  q quit");
    assert!(footer_text(120).contains("n new job"));
    let narrow = reduce(
        &base(),
        Action::Resize {
            columns: 80,
            rows: 24,
        },
    )
    .state;
    assert_eq!(footer_text(narrow.columns), "? help  q quit");
    assert_eq!(narrow.rows, 24);
}

#[test]
fn an_error_line_clears_on_the_next_key_which_still_counts() {
    let state = TuiState {
        error: Some("does not apply cleanly".to_string()),
        focus: Focus::Jobs,
        ..base()
    };
    let next = reduce(&state, key("j")).state;
    assert!(next.error.is_none());
    assert_eq!(next.job_id.as_deref(), Some("job-done"));
}

#[test]
fn merge_diff_and_stop_describe_what_went_wrong() {
    let project = TuiState {
        worktree_id: String::new(),
        ..base()
    };
    assert_eq!(
        reduce(&project, key("m")).state.error.as_deref(),
        Some("Select a worktree to merge.")
    );
    assert_eq!(
        reduce(&project, key("d")).state.error.as_deref(),
        Some("Select a worktree to see its diff.")
    );
    let stopped = reduce(
        &TuiState {
            job_id: Some("job-done".to_string()),
            ..base()
        },
        key("s"),
    );
    assert!(stopped.effects.is_empty());
    assert_eq!(
        stopped.state.error.as_deref(),
        Some("That job is not running.")
    );
    let diff = reduce(&base(), key("d"));
    assert_eq!(diff.state.pane, Pane::Diff);
    assert_eq!(
        diff.effects,
        vec![Effect::LoadDiff {
            id: "login123".to_string()
        }]
    );
}

#[test]
fn the_main_screen_at_120_by_30_lists_worktrees_and_jobs() {
    let text = frame_text(&screen_state());
    assert!(text.contains("WORKTREES  2"));
    assert!(text.contains("JOBS  1 running"));
    assert!(text.contains("login fix"));
    assert!(text.contains("3687475d"));
    assert!(text.contains("no changes"));
    assert!(text.contains("Add a button"));
    assert!(text.contains("Review the diff"));
    assert!(text.contains("Fix the crash"));
    assert!(text.contains("▌"));
    assert!(text.contains("n new job"));
    assert!(text.contains("type: planning · worker: auto · access: read"));
    assert!(text.contains("Legate · agent-orchestrator · v1"));
}

#[test]
fn the_same_screen_at_80_by_24_keeps_the_short_footer() {
    let mut state = screen_state();
    state.columns = 80;
    state.rows = 24;
    let text = frame_text(&state);
    assert!(text.contains("login fix"));
    assert!(text.contains("running"));
    assert!(text.contains("failed"));
    assert!(text.contains("? help"));
    assert!(text.contains("q quit"));
    assert!(!text.contains("n new job"));
}

#[test]
fn a_job_shows_its_steps_and_the_edit_summary() {
    let mut job = screen_job("aaaa1111");
    job.prompt = "Add a button".to_string();
    job.output = "Done.".to_string();
    job.files = 2;
    job.added = 10;
    job.removed = 3;
    job.steps = vec![
        JobStep {
            id: "s1".to_string(),
            tool: StepTool::Read,
            title: "src/core/github.ts".to_string(),
            detail: None,
            status: JobStepStatus::Done,
            started_at: 1,
            ended_at: None,
            edit: None,
        },
        JobStep {
            id: "s2".to_string(),
            tool: StepTool::Edit,
            title: "src/core/github.ts".to_string(),
            detail: None,
            status: JobStepStatus::Done,
            started_at: 1,
            ended_at: None,
            edit: Some(legate_core::FileEdit {
                path: "src/core/github.ts".to_string(),
                added: 4,
                removed: 1,
            }),
        },
        JobStep {
            id: "s3".to_string(),
            tool: StepTool::Shell,
            title: "pnpm test".to_string(),
            detail: None,
            status: JobStepStatus::Failed,
            started_at: 1,
            ended_at: None,
            edit: None,
        },
        JobStep {
            id: "s4".to_string(),
            tool: StepTool::Say,
            title: "looking".to_string(),
            detail: None,
            status: JobStepStatus::Done,
            started_at: 1,
            ended_at: None,
            edit: None,
        },
    ];
    let mut state = screen_state();
    state.jobs = vec![job];
    state.job_id = Some("aaaa1111".to_string());
    let text = frame_text(&state);
    assert!(text.contains("READ"));
    assert!(text.contains("src/core/github.ts"));
    assert!(text.contains("EDIT"));
    assert!(text.contains("+4"));
    assert!(text.contains("RUN"));
    assert!(text.contains("pnpm test"));
    assert!(text.contains("(failed)"));
    assert!(text.contains("SAY"));
    assert!(text.contains("Edited 2 files +10 −3"));
    assert!(text.contains("Done."));
}

#[test]
fn the_diff_view_shows_added_and_removed_lines() {
    let mut state = screen_state();
    state.pane = Pane::Diff;
    state.diff_id = "login123".to_string();
    state.diff_text =
        "+++ b/src/app.ts\n--- a/src/app.ts\n@@ -1 +1 @@\n-gone\n+hello\n".to_string();
    state.follow = false;
    state.scroll = 0;
    state.focus = Focus::Detail;
    let text = frame_text(&state);
    assert!(text.contains("Diff login fix"));
    assert!(text.contains("+hello"));
    assert!(text.contains("-gone"));
}

#[test]
fn ctrl_t_cycles_the_job_type_and_does_not_insert_the_letter() {
    let mut state = screen_state();
    state.columns = 100;
    state.rows = 24;
    state = reduce(&state, key("n")).state;
    state = reduce(&state, key("ab")).state;
    let typed = format!("{}t", state.draft);
    let action = edit_action(
        &typed,
        KeyPress {
            input: "t".to_string(),
            ctrl: true,
            ..KeyPress::default()
        },
    );
    state = reduce(&state, action).state;
    let text = frame_text(&state);
    assert!(text.contains("type: debugging"));
    assert!(text.contains("ab"));
    assert!(!text.contains("abt"));
}

#[test]
fn the_help_screen_lists_the_keys() {
    let mut state = screen_state();
    state.pane = Pane::Help;
    state.focus = Focus::Detail;
    state.follow = false;
    state.scroll = 0;
    state.rows = 40;
    let text = frame_text(&state);
    assert!(text.contains("Keys"));
    assert!(text.contains("Tab           switch pane"));
    assert!(text.contains("Ctrl+T        job type"));
    assert!(text.contains("q or Ctrl+C   quit"));
}

#[test]
fn without_colour_the_screen_keeps_plain_headings() {
    let mut state = screen_state();
    state.color = false;
    let text = frame_text(&state);
    assert!(text.contains("Worktrees  2"));
    assert!(text.contains("Jobs  1 running"));
    assert!(text.contains("▸"));
    assert!(text.contains("Legate · agent-orchestrator · v1"));
}

#[test]
fn the_boxes_fill_the_terminal_height() {
    let mut state = screen_state();
    state.columns = 120;
    state.rows = 30;
    let text = frame_text(&state);
    assert_eq!(text.split('\n').count(), 30);
}

#[test]
fn the_project_row_explains_itself() {
    let mut state = screen_state();
    state.worktree_id.clear();
    state.focus = Focus::Worktrees;
    state.pane = Pane::Worktree;
    let text = frame_text(&state);
    assert!(text.contains("The project folder."));
    assert_eq!(text.split("agent-orchestrator · v1").count(), 3);
}

#[tokio::test]
async fn a_submit_reaches_the_drive_and_quit_unmounts() {
    let fake = Fake::new();
    fake.set_project("/work/agent-orchestrator");
    fake.set_now(|| 5_000);
    fake.set_color(false);
    fake.set_size(80, 24);
    fake.set_info(legate_core::ProjectInfo {
        path: "/work/agent-orchestrator".to_string(),
        is_repo: true,
        branch: Some("v1".to_string()),
    });
    let submitted = Arc::new(Mutex::new(Vec::new()));
    let slot = Arc::clone(&submitted);
    fake.set_submit(move |_, request: JobRequest| {
        let provider = request
            .provider
            .map(|id| id.as_str().to_string())
            .unwrap_or_else(|| "auto".to_string());
        lock_push(
            &slot,
            format!(
                "{}:{provider}:{}:{}",
                request.job_type.as_str(),
                request.access.as_str(),
                request.prompt
            ),
        );
        let mut job = record("made-1", JobStatus::Done);
        job.job_type = request.job_type;
        job.prompt = request.prompt;
        job.provider = request.provider;
        job.access = Some(request.access);
        Ok::<_, DriveError>(job)
    });
    let handle = Arc::new(Mutex::new(None));
    let mounted = Arc::clone(&handle);
    let drive = fake.clone();
    let session = tokio::spawn(async move {
        run_session(&drive, false, |session| {
            *mounted.lock().unwrap_or_else(|err| err.into_inner()) = Some(session);
        })
        .await
    });
    let ready = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let branch = handle
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .as_ref()
                .map(|session| session.state().branch.clone());
            if branch
                .as_ref()
                .is_some_and(|branch| branch.as_deref() == Some("v1"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(ready.is_ok(), "timed out waiting for the terminal UI");
    let session_handle = handle
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take()
        .expect("mounted");
    session_handle.dispatch(key("n"));
    session_handle.dispatch(Action::Draft {
        text: "ship it".to_string(),
    });
    session_handle.dispatch(flag(KeyPress {
        enter: true,
        ..KeyPress::default()
    }));
    let saw_job = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if session_handle.state().job_id.as_deref() == Some("made-1") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(saw_job.is_ok(), "timed out waiting for the submitted job");
    assert_eq!(
        submitted
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone(),
        vec!["planning:auto:read:ship it".to_string()]
    );
    session_handle.dispatch(key("q"));
    let code = tokio::time::timeout(Duration::from_secs(2), session)
        .await
        .expect("quit")
        .expect("session");
    assert_eq!(code, 0);
    assert!(session_handle.unmounted());
}

fn lock_push(slot: &Mutex<Vec<String>>, line: String) {
    slot.lock()
        .unwrap_or_else(|err| err.into_inner())
        .push(line);
}
