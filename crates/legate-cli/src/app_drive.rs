//! `Drive` on top of `AppHandle`. Help and version never construct this.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use legate_core::{ChangeSet, JobRecord, PlanStatus, ProjectInfo, ProviderId};
use legate_git::{ListedName, MergeOutcome, OpenedPr};
use legate_lead::{Goal, Routine};
use legate_providers::{ClaudeAdapter, CursorAdapter, InteractiveCommand, InteractiveOptions};
use legate_store::Store;
use tokio::sync::{Notify, broadcast};

use crate::args::{self, clap_name};
use crate::drive::{
    CreatedTree, Drive, DriveError, GoalRequest, JobRequest, JobRow, Listener, LiveEvent,
    SigHandler, Unsub,
};
use crate::format::{PlanView, one_sentence};
use crate::run::run;
use legate_app::{AppEvent, AppHandle, AppOptions, GoalInput};

struct Fanout {
    jobs: Mutex<Vec<(u64, Listener<JobRecord>)>>,
    goals: Mutex<Vec<(u64, Listener<Goal>)>>,
    next: AtomicU64,
    live: broadcast::Sender<LiveEvent>,
    sigint: Mutex<Option<SigHandler>>,
    sig_on: Notify,
    sig_off: Notify,
}

pub struct AppDrive {
    app: AppHandle,
    fanout: Arc<Fanout>,
    store: Mutex<Store>,
    project: Mutex<String>,
    paths: Mutex<std::collections::HashMap<String, String>>,
    version: String,
    color: bool,
    tty: bool,
}

pub async fn launch(argv: &[String]) -> i32 {
    // Clap describes the same commands. Parsing stays in `args` so the error
    // sentences match the TypeScript tool.
    let _ = clap_name();
    let Ok(parsed) = args::parse_args(argv) else {
        return 2;
    };
    let tui = parsed.command.is_none() || parsed.command.as_deref() == Some("ui");
    match AppDrive::open(parsed.flags.project.clone(), !tui).await {
        Ok(drive) => run(argv, &drive).await,
        Err(err) => {
            eprintln!("{}\n", one_sentence(&err.message));
            1
        }
    }
}

impl AppDrive {
    async fn open(project: Option<String>, sigint: bool) -> Result<Self, DriveError> {
        let mut options = AppOptions::new();
        if let Some(project) = project {
            options.project = Some(PathBuf::from(project));
        }
        let app = AppHandle::start(options)
            .await
            .map_err(|err| DriveError::new(err.to_string()))?;
        let dir = AppHandle::opened_data_dir(None);
        let store = Store::open(dir.join("orchestrator.sqlite"))
            .map_err(|err| DriveError::new(err.to_string()))?;
        let project = match app.get_project().await {
            Ok(info) => info.path,
            Err(_) => std::env::current_dir()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
        };
        let (live, _) = broadcast::channel(256);
        let fanout = Arc::new(Fanout {
            jobs: Mutex::new(Vec::new()),
            goals: Mutex::new(Vec::new()),
            next: AtomicU64::new(1),
            live,
            sigint: Mutex::new(None),
            sig_on: Notify::new(),
            sig_off: Notify::new(),
        });
        spawn_events(app.subscribe(), Arc::clone(&fanout));
        if sigint {
            spawn_sigint(Arc::clone(&fanout));
        }
        Ok(Self {
            app,
            fanout,
            store: Mutex::new(store),
            project: Mutex::new(project),
            paths: Mutex::new(std::collections::HashMap::new()),
            version: env!("CARGO_PKG_VERSION").to_string(),
            color: color_enabled(),
            tty: std::io::IsTerminal::is_terminal(&std::io::stdin())
                && std::io::IsTerminal::is_terminal(&std::io::stdout()),
        })
    }
}

fn spawn_events(mut rx: broadcast::Receiver<AppEvent>, fanout: Arc<Fanout>) {
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(AppEvent::Job(job)) => {
                    let _ = fanout.live.send(LiveEvent::Job(Box::new(job.clone())));
                    for cb in lock(&fanout.jobs)
                        .iter()
                        .map(|(_, cb)| cb.clone())
                        .collect::<Vec<_>>()
                    {
                        cb(job.clone());
                    }
                }
                Ok(AppEvent::Plans(plans)) => {
                    let views = plans.iter().map(plan_view).collect();
                    let _ = fanout.live.send(LiveEvent::Plans(views));
                }
                Ok(AppEvent::Changes(_)) => {
                    let _ = fanout.live.send(LiveEvent::Changes);
                }
                Ok(AppEvent::Goal(goal)) => {
                    for cb in lock(&fanout.goals)
                        .iter()
                        .map(|(_, cb)| cb.clone())
                        .collect::<Vec<_>>()
                    {
                        cb(goal.clone());
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
                Ok(_) => {}
            }
        }
    });
}

fn spawn_sigint(fanout: Arc<Fanout>) {
    tokio::spawn(async move {
        loop {
            fanout.sig_on.notified().await;
            if lock(&fanout.sigint).is_none() {
                continue;
            }
            tokio::select! {
                result = tokio::signal::ctrl_c() => {
                    if result.is_err() {
                        break;
                    }
                    if let Some(handler) = lock(&fanout.sigint).clone() {
                        handler();
                    }
                }
                _ = fanout.sig_off.notified() => {}
            }
        }
    });
}

fn plan_view(plan: &PlanStatus) -> PlanView {
    PlanView {
        id: plan.id,
        available: plan.available,
        problem: plan.problem,
        headroom: (1.0 - plan.used).clamp(0.0, 1.0),
        resting_until: plan.resting_until,
        busy: plan.busy,
        running: plan.running,
        version: plan.version.clone(),
        version_status: plan.version_status,
        at_risk: plan.at_risk,
    }
}

fn color_enabled() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    std::io::IsTerminal::is_terminal(&std::io::stdout())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|dur| dur.as_millis() as i64)
        .unwrap_or(0)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

fn io_err(err: impl std::fmt::Display) -> DriveError {
    DriveError::new(err.to_string())
}

impl Drive for AppDrive {
    fn version(&self) -> &str {
        &self.version
    }

    fn project(&self) -> String {
        lock(&self.project).clone()
    }

    fn now(&self) -> i64 {
        now_ms()
    }

    fn color(&self) -> bool {
        self.color
    }

    fn tty(&self) -> bool {
        self.tty
    }

    fn term_size(&self) -> (u16, u16) {
        crossterm::terminal::size().unwrap_or((80, 24))
    }

    fn stdout(&self, text: &str) {
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }

    fn stderr(&self, text: &str) {
        use std::io::Write;
        let mut err = std::io::stderr().lock();
        let _ = err.write_all(text.as_bytes());
        let _ = err.flush();
    }

    fn stdin_is_tty(&self) -> bool {
        std::io::IsTerminal::is_terminal(&std::io::stdin())
    }

    async fn read_stdin(&self) -> String {
        tokio::task::spawn_blocking(|| {
            use std::io::Read;
            let mut buf = String::new();
            let _ = std::io::stdin().read_to_string(&mut buf);
            buf
        })
        .await
        .unwrap_or_default()
    }

    async fn ask(&self, prompt: &str) -> String {
        self.stdout(prompt);
        tokio::task::spawn_blocking(|| {
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
            line
        })
        .await
        .unwrap_or_default()
    }

    fn on_sigint(&self, handler: SigHandler) {
        *lock(&self.fanout.sigint) = Some(handler);
        self.fanout.sig_on.notify_one();
    }

    fn clear_sigint(&self) {
        *lock(&self.fanout.sigint) = None;
        self.fanout.sig_off.notify_one();
    }

    fn exit_now(&self, code: i32) {
        std::process::exit(code);
    }

    async fn init(&self) -> Result<(), DriveError> {
        // `AppHandle::start` already probed the plans. A second pass would
        // make `legate plans` wait on every CLI twice.
        Ok(())
    }

    async fn plans(&self) -> Result<Vec<PlanView>, DriveError> {
        let plans = self.app.list_plans().await.map_err(io_err)?;
        Ok(plans.iter().map(plan_view).collect())
    }

    async fn submit(&self, request: JobRequest) -> Result<JobRecord, DriveError> {
        self.app
            .submit_job(
                request.job_type,
                request.prompt,
                request.provider,
                Some(request.access),
                None,
            )
            .await
            .map_err(io_err)
    }

    async fn cancel(&self, id: &str) -> Result<JobRecord, DriveError> {
        self.app.cancel_job(id).await.map_err(io_err)
    }

    fn on_job(&self, cb: Listener<JobRecord>) -> Unsub {
        let id = self.fanout.next.fetch_add(1, Ordering::Relaxed);
        lock(&self.fanout.jobs).push((id, cb));
        let jobs = Arc::clone(&self.fanout);
        Unsub::new(move || {
            lock(&jobs.jobs).retain(|(slot, _)| *slot != id);
        })
    }

    fn jobs_newest(&self, limit: i64) -> Vec<JobRow> {
        let project = self.project();
        let rows = lock(&self.store)
            .jobs_newest(&project, limit)
            .unwrap_or_default();
        rows.into_iter()
            .map(|row| JobRow {
                created_at: if row.created_at == 0 {
                    None
                } else {
                    Some(row.created_at)
                },
                job: row.job,
            })
            .collect()
    }

    fn worktree_names(&self) -> Vec<ListedName> {
        let project = self.project();
        lock(&self.store)
            .worktree_names(&project)
            .unwrap_or_default()
            .into_iter()
            .map(|row| ListedName {
                id: row.id,
                name: row.name,
                created_at: row.created_at,
            })
            .collect()
    }

    fn set_worktree_name(&self, row: ListedName) {
        let project = self.project();
        let _ = lock(&self.store).set_worktree_name(&legate_store::WorktreeNameRow {
            id: row.id,
            project,
            name: row.name,
            created_at: row.created_at,
        });
    }

    fn delete_worktree_name(&self, id: &str) {
        let _ = lock(&self.store).delete_worktree_name(id);
    }

    async fn project_info(&self) -> Result<ProjectInfo, DriveError> {
        self.app.get_project().await.map_err(io_err)
    }

    async fn changes(&self) -> Result<Vec<ChangeSet>, DriveError> {
        let list = self.app.list_changes().await.map_err(io_err)?;
        let mut paths = lock(&self.paths);
        for change in &list {
            paths.insert(change.id.clone(), change.path.clone());
        }
        Ok(list)
    }

    async fn change_ids(&self) -> Result<Vec<String>, DriveError> {
        let list = self.changes().await?;
        Ok(list.into_iter().map(|change| change.id).collect())
    }

    fn folder(&self, id: &str) -> String {
        if let Some(path) = lock(&self.paths).get(id).cloned() {
            return path;
        }
        let app = self.app.clone();
        let id = id.to_string();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(async move { app.resolve_folder(Some(id)).await.unwrap_or_default() })
        })
    }

    async fn diff(&self, id: &str) -> Result<String, DriveError> {
        self.app.change_diff(id).await.map_err(io_err)
    }

    async fn merge(&self, id: &str) -> Result<MergeOutcome, DriveError> {
        self.app.merge_change(id).await.map_err(io_err)
    }

    async fn discard(&self, id: &str) -> Result<(), DriveError> {
        self.app.discard_change(id).await.map_err(io_err)
    }

    async fn create_tree(&self, _job_id: &str) -> Result<CreatedTree, DriveError> {
        let created = self.app.create_worktree(None).await.map_err(io_err)?;
        Ok(CreatedTree {
            id: created.id,
            path: created.path,
            branch: created.branch,
        })
    }

    async fn routines(&self) -> Result<Vec<Routine>, DriveError> {
        self.app.list_routines().await.map_err(io_err)
    }

    fn run_routine(&self, id: &str) -> Result<JobRecord, DriveError> {
        let app = self.app.clone();
        let id = id.to_string();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(async move { app.run_routine(id).await.map_err(io_err) })
        })
    }

    async fn open_pr(&self, title: &str, body: &str) -> Result<OpenedPr, DriveError> {
        self.app.github_open_pr(title, body).await.map_err(io_err)
    }

    async fn goals(&self) -> Result<Vec<Goal>, DriveError> {
        self.app.list_goals().await.map_err(io_err)
    }

    async fn create_goal(&self, input: GoalRequest) -> Result<Goal, DriveError> {
        self.app
            .create_goal(GoalInput {
                text: input.text,
                check: input.check,
                max_rounds: Some(input.max_rounds),
                quick_type: None,
                quick_access: None,
            })
            .await
            .map_err(io_err)
    }

    fn approve_goal(&self, id: &str) -> Result<Goal, DriveError> {
        let app = self.app.clone();
        let id = id.to_string();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(async move { app.approve_goal(id, None).await.map_err(io_err) })
        })
    }

    fn answer_goal(&self, id: &str, text: &str) -> Result<Goal, DriveError> {
        let app = self.app.clone();
        let id = id.to_string();
        let text = text.to_string();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(async move { app.answer_goal(id, text).await.map_err(io_err) })
        })
    }

    fn stop_goal(&self, id: &str) -> Result<Goal, DriveError> {
        let app = self.app.clone();
        let id = id.to_string();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(async move { app.stop_goal(id).await.map_err(io_err) })
        })
    }

    fn on_goal(&self, cb: Listener<Goal>) -> Unsub {
        let id = self.fanout.next.fetch_add(1, Ordering::Relaxed);
        lock(&self.fanout.goals).push((id, cb));
        let goals = Arc::clone(&self.fanout);
        Unsub::new(move || {
            lock(&goals.goals).retain(|(slot, _)| *slot != id);
        })
    }

    fn interactive(&self, provider: ProviderId) -> InteractiveCommand {
        let opts = InteractiveOptions::default();
        match provider {
            ProviderId::Claude => ClaudeAdapter::new(None).interactive(&opts),
            ProviderId::Codex => legate_providers::CodexAdapter::new(None).interactive(&opts),
            ProviderId::Cursor | ProviderId::Grok => CursorAdapter::new(None).interactive(&opts),
            ProviderId::Local => InteractiveCommand {
                command: "ollama".to_string(),
                args: Vec::new(),
                env: None,
            },
        }
    }

    async fn spawn_inherited(
        &self,
        spec: &InteractiveCommand,
        cwd: &str,
    ) -> Result<(), DriveError> {
        let mut cmd = std::process::Command::new(&spec.command);
        cmd.args(&spec.args)
            .current_dir(cwd)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        if let Some(env) = &spec.env {
            cmd.env_clear();
            for (key, value) in env {
                cmd.env(key, value);
            }
        }
        cmd.status().map(|_| ()).map_err(|_| {
            DriveError::new(format!(
                "Could not start {}. Install it, then run it once in a terminal to sign in.",
                spec.command
            ))
        })
    }

    fn events(&self) -> Option<broadcast::Receiver<LiveEvent>> {
        Some(self.fanout.live.subscribe())
    }
}
