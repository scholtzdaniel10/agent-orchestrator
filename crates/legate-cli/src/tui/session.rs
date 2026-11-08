//! The terminal session. It follows `AppHandle` events and redraws at 10 Hz.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use legate_core::{ChangeSet, JobAccess, JobRecord, JobStatus, JobType, ProviderId};
use legate_git::ListedName;
use tokio::sync::{Notify, mpsc};

use super::render::draw;
use super::state::{
    Action, Effect, FileRow, JobItem, PlanItem, RoutineItem, TuiState, WorktreeItem, initial_state,
};
use crate::drive::{Drive, DriveError, JobRequest, LiveEvent};
use crate::format::{access_word, plan_short};

#[allow(dead_code)]
pub struct SessionHandle {
    shared: Arc<Shared>,
}

struct Shared {
    state: Mutex<TuiState>,
    actions: Mutex<VecDeque<Action>>,
    wake: Notify,
    alive: AtomicBool,
    mounted: AtomicBool,
    reloads: std::sync::atomic::AtomicUsize,
}

#[allow(dead_code)]
impl SessionHandle {
    pub fn state(&self) -> TuiState {
        lock(&self.shared.state).clone()
    }

    pub fn dispatch(&self, action: Action) {
        lock(&self.shared.actions).push_back(action);
        self.shared.wake.notify_one();
    }

    pub fn unmounted(&self) -> bool {
        !self.shared.mounted.load(Ordering::SeqCst)
    }
}

pub async fn open<D: Drive>(drive: &D) -> i32 {
    run_session(drive, true, |_| {}).await
}

pub async fn run_session<D: Drive>(
    drive: &D,
    terminal: bool,
    mount: impl FnOnce(SessionHandle),
) -> i32 {
    let (columns, rows) = drive.term_size();
    let project = drive.project();
    let name = std::path::Path::new(&project)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project")
        .to_string();
    let shared = Arc::new(Shared {
        state: Mutex::new(initial_state(
            &name,
            columns,
            rows,
            drive.color(),
            drive.now(),
        )),
        actions: Mutex::new(VecDeque::new()),
        wake: Notify::new(),
        alive: AtomicBool::new(true),
        mounted: AtomicBool::new(true),
        reloads: std::sync::atomic::AtomicUsize::new(0),
    });
    mount(SessionHandle {
        shared: shared.clone(),
    });
    if let Err(err) = drive.init().await {
        push(
            &shared,
            Action::Error {
                message: err.message,
            },
        );
    }
    if let Err(err) = reload(drive, &shared).await {
        push(
            &shared,
            Action::Error {
                message: err.message,
            },
        );
    }

    let mut events = drive.events();
    let stop = Arc::new(AtomicBool::new(false));
    let (key_tx, mut key_rx) = mpsc::unbounded_channel();
    if terminal {
        spawn_keys(key_tx, Arc::clone(&stop));
    }
    let mut raw = if terminal {
        RawTerm::enter().ok()
    } else {
        None
    };
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_draw = Instant::now() - Duration::from_secs(1);

    let code = loop {
        let effects = drain(&shared);
        if let Err(err) = run_effects(drive, &shared, &mut raw, effects).await {
            push(
                &shared,
                Action::Error {
                    message: err.message,
                },
            );
        }
        if !shared.alive.load(Ordering::SeqCst) {
            break 0;
        }
        if raw.is_some() && last_draw.elapsed() >= Duration::from_millis(100) {
            paint(&shared);
            last_draw = Instant::now();
        }
        tokio::select! {
            _ = shared.wake.notified() => {}
            _ = tick.tick() => {
                push(&shared, Action::Tick { now: drive.now() });
            }
            Some(action) = key_rx.recv() => push(&shared, action),
            event = next_event(&mut events) => {
                if let Some(event) = event {
                    apply_event(drive, &shared, event);
                }
            }
        }
    };
    stop.store(true, Ordering::SeqCst);
    drop(raw);
    shared.mounted.store(false, Ordering::SeqCst);
    code
}

fn spawn_keys(tx: mpsc::UnboundedSender<Action>, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            if !crossterm::event::poll(Duration::from_millis(50)).unwrap_or(false) {
                continue;
            }
            let Ok(event) = crossterm::event::read() else {
                break;
            };
            let Some(action) = key_action(event) else {
                continue;
            };
            if tx.send(action).is_err() {
                break;
            }
        }
    });
}

fn key_action(event: crossterm::event::Event) -> Option<Action> {
    use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
    match event {
        Event::Resize(columns, rows) => Some(Action::Resize { columns, rows }),
        Event::Key(key) if key.kind == KeyEventKind::Release => None,
        Event::Key(key) => {
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            let shift = key.modifiers.contains(KeyModifiers::SHIFT);
            let press = match key.code {
                KeyCode::Char(ch) => super::state::KeyPress {
                    input: ch.to_string(),
                    ctrl,
                    shift,
                    ..super::state::KeyPress::default()
                },
                KeyCode::Enter => super::state::KeyPress {
                    enter: true,
                    ctrl,
                    shift,
                    ..super::state::KeyPress::default()
                },
                KeyCode::Esc => super::state::KeyPress {
                    escape: true,
                    ctrl,
                    shift,
                    ..super::state::KeyPress::default()
                },
                KeyCode::Tab => super::state::KeyPress {
                    tab: true,
                    ctrl,
                    shift,
                    ..super::state::KeyPress::default()
                },
                KeyCode::Backspace => super::state::KeyPress {
                    backspace: true,
                    ctrl,
                    shift,
                    ..super::state::KeyPress::default()
                },
                KeyCode::Up => super::state::KeyPress {
                    up: true,
                    ctrl,
                    shift,
                    ..super::state::KeyPress::default()
                },
                KeyCode::Down => super::state::KeyPress {
                    down: true,
                    ctrl,
                    shift,
                    ..super::state::KeyPress::default()
                },
                KeyCode::PageUp => super::state::KeyPress {
                    page_up: true,
                    ctrl,
                    shift,
                    ..super::state::KeyPress::default()
                },
                KeyCode::PageDown => super::state::KeyPress {
                    page_down: true,
                    ctrl,
                    shift,
                    ..super::state::KeyPress::default()
                },
                _ => return None,
            };
            Some(Action::Key {
                key: press,
                draft: None,
            })
        }
        _ => None,
    }
}

fn apply_event<D: Drive>(drive: &D, shared: &Shared, event: LiveEvent) {
    match event {
        LiveEvent::Job(job) => {
            let prev = lock(&shared.state)
                .jobs
                .iter()
                .find(|item| item.id == job.id)
                .cloned();
            if job.status == JobStatus::Done
                && prev
                    .as_ref()
                    .is_none_or(|item| item.status != JobStatus::Done)
            {
                drive.note_job_finished(&job);
            }
            let created = prev
                .as_ref()
                .map(|item| item.created_at)
                .unwrap_or_else(|| drive.now());
            push(
                shared,
                Action::Job {
                    job: job_item(&job, created, false),
                },
            );
        }
        LiveEvent::Plans(plans) => {
            let now = drive.now();
            push(
                shared,
                Action::Plans {
                    plans: plans
                        .iter()
                        .map(|plan| PlanItem {
                            id: plan.id.as_str().to_string(),
                            text: plan_short(plan, now),
                        })
                        .collect(),
                },
            );
        }
        LiveEvent::Changes => {
            shared.reloads.fetch_add(1, Ordering::SeqCst);
            shared.wake.notify_one();
        }
    }
}

async fn reload<D: Drive>(drive: &D, shared: &Shared) -> Result<(), DriveError> {
    let now = drive.now();
    let plans = drive
        .plans()
        .await?
        .into_iter()
        .map(|plan| PlanItem {
            id: plan.id.as_str().to_string(),
            text: plan_short(&plan, now),
        })
        .collect();
    push(shared, Action::Plans { plans });
    let info = drive.project_info().await?;
    let listed = drive.changes().await?;
    let ids = drive.change_ids().await?;
    let names = drive.worktree_names();
    let changes =
        legate_git::merge_listed_changes(&listed, &ids, &names, |id| drive.folder(id), |_| None);
    let project_path = if info.path.is_empty() {
        drive.project()
    } else {
        info.path.clone()
    };
    let label = std::path::Path::new(&project_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project")
        .to_string();
    push(
        shared,
        Action::Worktrees {
            project: WorktreeItem {
                id: String::new(),
                project: true,
                label,
                branch: info.branch.clone(),
                added: 0,
                removed: 0,
                files: Vec::new(),
                path: project_path,
            },
            items: changes.iter().map(worktree_item).collect(),
            branch: info.branch,
        },
    );
    let jobs = drive
        .jobs_newest(100)
        .into_iter()
        .map(|row| job_item(&row.job, row.created_at.unwrap_or(now), false))
        .collect();
    push(shared, Action::Jobs { jobs });
    let routines = drive
        .routines()
        .await?
        .into_iter()
        .map(|routine| RoutineItem {
            id: routine.id,
            name: routine.name,
            job_type: routine.job_type.as_str().to_string(),
            worker: routine.worker.as_str().to_string(),
            access: routine.access.as_str().to_string(),
        })
        .collect();
    push(shared, Action::Routines { routines });
    push(shared, Action::Tick { now });
    Ok(())
}

fn drain(shared: &Shared) -> Vec<Effect> {
    let mut effects = Vec::new();
    while shared.reloads.swap(0, Ordering::SeqCst) > 0 {
        effects.push(Effect::LoadChanges);
    }
    loop {
        let next = lock(&shared.actions).pop_front();
        let Some(action) = next else {
            break;
        };
        let mut state = lock(&shared.state);
        let result = super::state::reduce(&state, action);
        *state = result.state;
        drop(state);
        effects.extend(result.effects);
    }
    effects
}

async fn run_effects<D: Drive>(
    drive: &D,
    shared: &Shared,
    raw: &mut Option<RawTerm>,
    effects: Vec<Effect>,
) -> Result<(), DriveError> {
    for effect in effects {
        match effect {
            Effect::Quit { stop } => {
                for id in stop {
                    let _ = drive.cancel(&id).await;
                }
                shared.alive.store(false, Ordering::SeqCst);
                return Ok(());
            }
            Effect::Submit {
                job_type,
                prompt,
                worker,
                access,
            } => {
                let Some(job_type) = JobType::parse(&job_type) else {
                    push_error(shared, "Unknown job type.");
                    continue;
                };
                let Some(access) = JobAccess::parse(&access) else {
                    push_error(shared, "Unknown access.");
                    continue;
                };
                let provider = if worker == "auto" {
                    None
                } else {
                    ProviderId::parse(&worker)
                };
                let job = drive
                    .submit(JobRequest {
                        job_type,
                        prompt,
                        provider,
                        access,
                    })
                    .await?;
                push(
                    shared,
                    Action::Submitted {
                        job: job_item(&job, drive.now(), true),
                    },
                );
            }
            Effect::Cancel { id } => {
                let job = drive.cancel(&id).await?;
                push(
                    shared,
                    Action::Job {
                        job: job_item(&job, drive.now(), false),
                    },
                );
            }
            Effect::RunRoutine { id } => {
                let job = drive.run_routine(&id)?;
                push(
                    shared,
                    Action::Submitted {
                        job: job_item(&job, drive.now(), true),
                    },
                );
            }
            Effect::LoadDiff { id } => {
                let text = drive.diff(&id).await?;
                push(shared, Action::Diff { id, text });
            }
            Effect::LoadChanges => reload(drive, shared).await?,
            Effect::Merge { id } => {
                let result = drive.merge(&id).await?;
                if result.ok {
                    drive.delete_worktree_name(&id);
                } else {
                    push_error(shared, &result.message);
                }
                reload(drive, shared).await?;
            }
            Effect::Discard { id } => {
                drive.discard(&id).await?;
                drive.delete_worktree_name(&id);
                reload(drive, shared).await?;
            }
            Effect::CreateWorktree { name } => {
                let about = drive.project_info().await?;
                if !about.is_repo {
                    push_error(
                        shared,
                        "This folder is not a git repository, so it cannot have worktrees.",
                    );
                    continue;
                }
                let created = drive.create_tree(&uuid::Uuid::new_v4().to_string()).await?;
                drive.set_worktree_name(ListedName {
                    id: created.id.clone(),
                    name,
                    created_at: drive.now(),
                });
                reload(drive, shared).await?;
                push(shared, Action::SelectWorktree { id: created.id });
            }
            Effect::OpenAgent { provider, cwd } => {
                let spec = drive.interactive(provider);
                if let Some(term) = raw.as_mut() {
                    term.suspend();
                }
                let spawned = drive.spawn_inherited(&spec, &cwd).await;
                if let Some(term) = raw.as_mut() {
                    term.resume();
                }
                spawned?;
                reload(drive, shared).await?;
            }
        }
    }
    Ok(())
}

fn push(shared: &Shared, action: Action) {
    lock(&shared.actions).push_back(action);
    shared.wake.notify_one();
}

fn push_error(shared: &Shared, message: &str) {
    push(
        shared,
        Action::Error {
            message: message.to_string(),
        },
    );
}

fn paint(shared: &Shared) {
    let state = lock(&shared.state).clone();
    let mut stdout = std::io::stdout();
    let backend = ratatui::backend::CrosstermBackend::new(&mut stdout);
    let mut terminal = match ratatui::Terminal::new(backend) {
        Ok(terminal) => terminal,
        Err(_) => return,
    };
    let _ = terminal.draw(|frame| draw(frame, &state));
}

fn job_item(job: &JobRecord, created_at: i64, started_here: bool) -> JobItem {
    let (files, added, removed) = edits(job);
    JobItem {
        id: job.id.clone(),
        status: job.status,
        provider: job.provider.map(|id| id.as_str().to_string()),
        job_type: job.job_type.as_str().to_string(),
        access: access_word(job).to_string(),
        model: job.model.clone().unwrap_or_default(),
        reason: job.reason.clone().unwrap_or_default(),
        prompt: job.prompt.clone(),
        output: job.output.clone(),
        error: job.error.clone().unwrap_or_default(),
        steps: job.steps.clone().unwrap_or_default(),
        created_at,
        started_here,
        change: job.change.clone(),
        files,
        added,
        removed,
    }
}

fn edits(job: &JobRecord) -> (i64, i64, i64) {
    let mut paths = std::collections::BTreeSet::new();
    let mut added = 0i64;
    let mut removed = 0i64;
    for step in job.steps.iter().flatten() {
        if let Some(edit) = &step.edit {
            paths.insert(edit.path.clone());
            added += edit.added;
            removed += edit.removed;
        }
    }
    (paths.len() as i64, added, removed)
}

fn worktree_item(change: &ChangeSet) -> WorktreeItem {
    let label = change
        .name
        .as_deref()
        .filter(|name| !name.is_empty())
        .unwrap_or(change.id.as_str())
        .to_string();
    WorktreeItem {
        id: change.id.clone(),
        project: false,
        label,
        branch: Some(change.branch.clone()),
        added: change.insertions,
        removed: change.deletions,
        files: change
            .files
            .iter()
            .map(|file| FileRow {
                path: file.path.clone(),
                added: file.insertions,
                removed: file.deletions,
            })
            .collect(),
        path: change.path.clone(),
    }
}

async fn next_event(
    events: &mut Option<tokio::sync::broadcast::Receiver<LiveEvent>>,
) -> Option<LiveEvent> {
    let Some(rx) = events else {
        std::future::pending::<()>().await;
        return None;
    };
    loop {
        match rx.recv().await {
            Ok(event) => return Some(event),
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

struct RawTerm;

impl RawTerm {
    fn enter() -> std::io::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::EnterAlternateScreen,
            crossterm::cursor::Hide
        )?;
        Ok(Self)
    }

    fn suspend(&mut self) {
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::cursor::Show
        );
        let _ = crossterm::terminal::disable_raw_mode();
    }

    fn resume(&mut self) {
        let _ = crossterm::terminal::enable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::EnterAlternateScreen,
            crossterm::cursor::Hide
        );
    }
}

impl Drop for RawTerm {
    fn drop(&mut self) {
        self.suspend();
    }
}
