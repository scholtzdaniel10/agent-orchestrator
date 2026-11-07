//! Cloneable handle. Methods hop to the orchestrator thread and wait for the reply.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use legate_core::{
    AppInfo, ChangeSet, FileContent, FileList, JobAccess, JobRecord, JobType, LeadChat,
    LeadMessage, ModelOption, PlanStatus, Prefs, ProjectEntry, ProjectInfo, ProposedJob,
    ProviderId, SkillsState, TerminalInfo,
};
use legate_git::{CommitDetail, CommitPage, MergeOutcome, OpenedPr};
use legate_lead::{Goal, PlannedJob, Requirement, Routine, Trigger};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::error::AppError;
use crate::event::AppEvent;
use crate::options::{
    AppOptions, GoalInput, TerminalKind, TriggerInput, UsageOutcome, WorktreePick,
};
use crate::session::{Command, World, spawn_service};
use crate::usage::WindowSight;

struct Shared {
    tx: mpsc::UnboundedSender<Command>,
    events: broadcast::Sender<AppEvent>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

/// The Electron main process, without windows.
///
/// Cloning shares the process. The last drop stops the orchestrator thread.
#[derive(Clone)]
pub struct AppHandle {
    shared: Arc<Shared>,
}

impl Drop for Shared {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
        let handle = self
            .thread
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .take();
        if let Some(handle) = handle {
            let _ = handle.join();
        }
    }
}

impl AppHandle {
    /// Open the data directory, start the local-set thread, and wait until it is ready.
    pub async fn start(options: AppOptions) -> Result<Self, AppError> {
        let (tx, rx) = mpsc::unbounded_channel();
        let (events, _) = broadcast::channel(1024);
        let (ready_tx, ready_rx) = oneshot::channel();
        let thread_tx = tx.clone();
        let thread_events = events.clone();
        let thread = std::thread::Builder::new()
            .name("legate-app".to_string())
            .spawn(move || spawn_service(options, thread_events, thread_tx, rx, ready_tx))
            .map_err(|err| AppError::message(err.to_string()))?;
        match ready_rx.await {
            Ok(Ok(())) => Ok(Self {
                shared: Arc::new(Shared {
                    tx,
                    events,
                    thread: Mutex::new(Some(thread)),
                }),
            }),
            Ok(Err(err)) => {
                let _ = thread.join();
                Err(err)
            }
            Err(_) => {
                let _ = thread.join();
                Err(AppError::message("legate failed to start"))
            }
        }
    }

    /// Live pushes: jobs, plans, changes, goals, triggers, prefs, lead, terminals.
    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.shared.events.subscribe()
    }

    /// Stop the orchestrator thread and wait for it.
    pub async fn shutdown(&self) {
        let handle = self
            .shared
            .thread
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .take();
        let _ = self.shared.tx.send(Command::Shutdown);
        if let Some(handle) = handle {
            let _ = tokio::task::spawn_blocking(move || handle.join()).await;
        }
    }

    fn invoke<T, F>(&self, call: F) -> impl Future<Output = Result<T, AppError>> + Send
    where
        T: Send + 'static,
        F: for<'a> FnOnce(&'a World) -> crate::session::Task<T> + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        let sent = self.shared.tx.send(pack(call, tx));
        async move {
            sent.map_err(|_| AppError::message("legate is shutting down"))?;
            rx.await
                .map_err(|_| AppError::message("legate is shutting down"))?
        }
    }

    pub async fn submit_job(
        &self,
        job_type: JobType,
        prompt: impl Into<String>,
        provider: Option<ProviderId>,
        access: Option<JobAccess>,
        group: Option<String>,
    ) -> Result<JobRecord, AppError> {
        let prompt = prompt.into();
        self.invoke(move |world| world.submit_job(job_type, prompt, provider, access, group))
            .await
    }

    pub async fn list_jobs(&self) -> Result<Vec<JobRecord>, AppError> {
        self.invoke(|world| world.list_jobs()).await
    }

    pub async fn cancel_job(&self, id: impl Into<String>) -> Result<JobRecord, AppError> {
        let id = id.into();
        self.invoke(move |world| world.cancel_job(id)).await
    }

    pub async fn refresh_usage(&self) -> Result<UsageOutcome, AppError> {
        self.invoke(|world| world.refresh_usage()).await
    }

    pub async fn set_cursor_reading(
        &self,
        used_percent: f64,
        resets_at: i64,
    ) -> Result<UsageOutcome, AppError> {
        self.invoke(move |world| world.set_cursor_reading(used_percent, resets_at))
            .await
    }

    pub async fn clear_cursor_reading(&self) -> Result<(), AppError> {
        self.invoke(|world| world.clear_cursor_reading()).await
    }

    pub async fn take_over_job(
        &self,
        id: impl Into<String>,
        cols: i64,
        rows: i64,
    ) -> Result<TerminalInfo, AppError> {
        let id = id.into();
        self.invoke(move |world| world.take_over_job(id, cols, rows))
            .await
    }

    pub async fn list_plans(&self) -> Result<Vec<PlanStatus>, AppError> {
        self.invoke(|world| world.list_plans()).await
    }

    pub async fn recheck_plans(&self) -> Result<Vec<PlanStatus>, AppError> {
        self.invoke(|world| world.recheck_plans()).await
    }

    pub async fn list_models(&self, provider: ProviderId) -> Result<Vec<ModelOption>, AppError> {
        self.invoke(move |world| world.list_models(provider)).await
    }

    pub async fn set_model(
        &self,
        provider: ProviderId,
        model: Option<String>,
    ) -> Result<(), AppError> {
        self.invoke(move |world| world.set_model(provider, model))
            .await
    }

    pub async fn get_project(&self) -> Result<ProjectInfo, AppError> {
        self.invoke(|world| world.get_project()).await
    }

    pub async fn list_projects(&self) -> Result<Vec<ProjectEntry>, AppError> {
        self.invoke(|world| world.list_projects()).await
    }

    /// Non-UI half of `chooseProject`: `path` is the folder the person already picked.
    pub async fn choose_project(&self, path: impl Into<String>) -> Result<ProjectInfo, AppError> {
        let path = path.into();
        self.invoke(move |world| world.choose_project(path)).await
    }

    pub async fn switch_project(&self, path: impl Into<String>) -> Result<ProjectInfo, AppError> {
        let path = path.into();
        self.invoke(move |world| world.switch_project(path)).await
    }

    pub async fn remove_project(&self, path: impl Into<String>) -> Result<(), AppError> {
        let path = path.into();
        self.invoke(move |world| world.remove_project(path)).await
    }

    pub async fn github_available(&self) -> Result<bool, AppError> {
        self.invoke(|world| world.github_available()).await
    }

    pub async fn github_repos(&self) -> Result<Vec<legate_core::GithubRepo>, AppError> {
        self.invoke(|world| world.github_repos()).await
    }

    /// Non-UI half of `githubClone`: `parent_dir` is the folder the person already picked.
    pub async fn github_clone(
        &self,
        name: impl Into<String>,
        parent_dir: impl Into<String>,
    ) -> Result<ProjectInfo, AppError> {
        let name = name.into();
        let parent_dir = parent_dir.into();
        self.invoke(move |world| world.github_clone(name, parent_dir))
            .await
    }

    pub async fn github_open_pr(
        &self,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> Result<OpenedPr, AppError> {
        let title = title.into();
        let body = body.into();
        self.invoke(move |world| world.github_open_pr(title, body))
            .await
    }

    pub async fn setup_status(&self) -> Result<Vec<Requirement>, AppError> {
        self.invoke(|world| world.setup_status()).await
    }

    pub async fn setup_sign_in(&self, id: impl Into<String>) -> Result<TerminalInfo, AppError> {
        let id = id.into();
        self.invoke(move |world| world.setup_sign_in(id)).await
    }

    pub async fn skills_get(&self) -> Result<SkillsState, AppError> {
        self.invoke(|world| world.skills_get()).await
    }

    /// Non-UI half of `skillsAdd`: `folder` was already chosen.
    pub async fn skills_add(&self, folder: impl Into<String>) -> Result<SkillsState, AppError> {
        let folder = folder.into();
        self.invoke(move |world| world.skills_add(folder)).await
    }

    pub async fn skills_remove(&self, folder: impl Into<String>) -> Result<SkillsState, AppError> {
        let folder = folder.into();
        self.invoke(move |world| world.skills_remove(folder)).await
    }

    pub async fn skills_reload(&self) -> Result<SkillsState, AppError> {
        self.invoke(|world| world.skills_reload()).await
    }

    pub async fn list_changes(&self) -> Result<Vec<ChangeSet>, AppError> {
        self.invoke(|world| world.list_changes()).await
    }

    pub async fn change_diff(&self, id: impl Into<String>) -> Result<String, AppError> {
        let id = id.into();
        self.invoke(move |world| world.change_diff(id)).await
    }

    pub async fn merge_change(&self, id: impl Into<String>) -> Result<MergeOutcome, AppError> {
        let id = id.into();
        self.invoke(move |world| world.merge_change(id)).await
    }

    pub async fn discard_change(&self, id: impl Into<String>) -> Result<(), AppError> {
        let id = id.into();
        self.invoke(move |world| world.discard_change(id)).await
    }

    pub async fn create_worktree(&self, name: Option<String>) -> Result<ChangeSet, AppError> {
        self.invoke(move |world| world.create_worktree(name)).await
    }

    pub async fn rename_worktree(
        &self,
        id: impl Into<String>,
        name: impl Into<String>,
    ) -> Result<(), AppError> {
        let id = id.into();
        let name = name.into();
        self.invoke(move |world| world.rename_worktree(id, name))
            .await
    }

    pub async fn list_files(&self, where_id: Option<String>) -> Result<FileList, AppError> {
        self.invoke(move |world| world.list_files(where_id)).await
    }

    pub async fn read_file(
        &self,
        where_id: Option<String>,
        path: impl Into<String>,
    ) -> Result<FileContent, AppError> {
        let path = path.into();
        self.invoke(move |world| world.read_file(where_id, path))
            .await
    }

    pub async fn file_diff(
        &self,
        where_id: Option<String>,
        path: impl Into<String>,
    ) -> Result<String, AppError> {
        let path = path.into();
        self.invoke(move |world| world.file_diff(where_id, path))
            .await
    }

    pub async fn list_goals(&self) -> Result<Vec<Goal>, AppError> {
        self.invoke(|world| world.list_goals()).await
    }

    pub async fn create_goal(&self, input: GoalInput) -> Result<Goal, AppError> {
        self.invoke(move |world| world.create_goal(input)).await
    }

    pub async fn approve_goal(
        &self,
        id: impl Into<String>,
        jobs: Option<Vec<PlannedJob>>,
    ) -> Result<Goal, AppError> {
        let id = id.into();
        self.invoke(move |world| world.approve_goal(id, jobs)).await
    }

    pub async fn answer_goal(
        &self,
        id: impl Into<String>,
        text: impl Into<String>,
    ) -> Result<Goal, AppError> {
        let id = id.into();
        let text = text.into();
        self.invoke(move |world| world.answer_goal(id, text)).await
    }

    pub async fn stop_goal(&self, id: impl Into<String>) -> Result<Goal, AppError> {
        let id = id.into();
        self.invoke(move |world| world.stop_goal(id)).await
    }

    pub async fn remove_goal(&self, id: impl Into<String>) -> Result<(), AppError> {
        let id = id.into();
        self.invoke(move |world| world.remove_goal(id)).await
    }

    pub async fn list_triggers(&self) -> Result<Vec<Trigger>, AppError> {
        self.invoke(|world| world.list_triggers()).await
    }

    pub async fn add_trigger(&self, input: TriggerInput) -> Result<Trigger, AppError> {
        self.invoke(move |world| world.add_trigger(input)).await
    }

    pub async fn toggle_trigger(
        &self,
        id: impl Into<String>,
        enabled: bool,
    ) -> Result<Trigger, AppError> {
        let id = id.into();
        self.invoke(move |world| world.toggle_trigger(id, enabled))
            .await
    }

    pub async fn remove_trigger(&self, id: impl Into<String>) -> Result<(), AppError> {
        let id = id.into();
        self.invoke(move |world| world.remove_trigger(id)).await
    }

    pub async fn run_trigger(&self, id: impl Into<String>) -> Result<(), AppError> {
        let id = id.into();
        self.invoke(move |world| world.run_trigger(id)).await
    }

    pub async fn commit_log(
        &self,
        where_id: Option<String>,
        skip: i64,
    ) -> Result<CommitPage, AppError> {
        self.invoke(move |world| world.commit_log(where_id, skip))
            .await
    }

    pub async fn commit_detail(
        &self,
        where_id: Option<String>,
        hash: impl Into<String>,
    ) -> Result<CommitDetail, AppError> {
        let hash = hash.into();
        self.invoke(move |world| world.commit_detail(where_id, hash))
            .await
    }

    pub async fn commit_file_diff(
        &self,
        where_id: Option<String>,
        hash: impl Into<String>,
        path: impl Into<String>,
    ) -> Result<String, AppError> {
        let hash = hash.into();
        let path = path.into();
        self.invoke(move |world| world.commit_file_diff(where_id, hash, path))
            .await
    }

    pub async fn get_prefs(&self) -> Result<Prefs, AppError> {
        self.invoke(|world| world.get_prefs()).await
    }

    pub async fn set_prefs(&self, patch: Value) -> Result<Prefs, AppError> {
        self.invoke(move |world| world.set_prefs(patch)).await
    }

    /// Non-UI half of `chooseCloneDir`.
    pub async fn choose_clone_dir(&self, path: impl Into<String>) -> Result<Prefs, AppError> {
        let path = path.into();
        self.invoke(move |world| world.choose_clone_dir(path)).await
    }

    pub async fn get_app_info(&self) -> Result<AppInfo, AppError> {
        self.invoke(|world| world.get_app_info()).await
    }

    pub async fn get_project_setup(&self) -> Result<Vec<String>, AppError> {
        self.invoke(|world| world.get_project_setup()).await
    }

    pub async fn set_project_setup(&self, commands: Vec<String>) -> Result<(), AppError> {
        self.invoke(move |world| world.set_project_setup(commands))
            .await
    }

    pub async fn list_routines(&self) -> Result<Vec<Routine>, AppError> {
        self.invoke(|world| world.list_routines()).await
    }

    pub async fn save_routine(&self, routine: Value) -> Result<(), AppError> {
        self.invoke(move |world| world.save_routine(routine)).await
    }

    pub async fn remove_routine(&self, id: impl Into<String>) -> Result<(), AppError> {
        let id = id.into();
        self.invoke(move |world| world.remove_routine(id)).await
    }

    pub async fn run_routine(&self, id: impl Into<String>) -> Result<JobRecord, AppError> {
        let id = id.into();
        self.invoke(move |world| world.run_routine(id)).await
    }

    pub async fn get_lead_plan(&self) -> Result<Option<ProviderId>, AppError> {
        self.invoke(|world| world.get_lead_plan()).await
    }

    pub async fn set_lead_plan(&self, plan: Option<ProviderId>) -> Result<(), AppError> {
        self.invoke(move |world| world.set_lead_plan(plan)).await
    }

    pub async fn get_lead_model(&self, provider: ProviderId) -> Result<Option<String>, AppError> {
        self.invoke(move |world| world.get_lead_model(provider))
            .await
    }

    pub async fn set_lead_model(
        &self,
        provider: ProviderId,
        model: Option<String>,
    ) -> Result<(), AppError> {
        self.invoke(move |world| world.set_lead_model(provider, model))
            .await
    }

    pub async fn get_lead_access(&self) -> Result<JobAccess, AppError> {
        self.invoke(|world| world.get_lead_access()).await
    }

    pub async fn set_lead_access(&self, access: JobAccess) -> Result<(), AppError> {
        self.invoke(move |world| world.set_lead_access(access))
            .await
    }

    pub async fn send_lead(&self, text: impl Into<String>) -> Result<LeadMessage, AppError> {
        let text = text.into();
        self.invoke(move |world| world.send_lead(text)).await
    }

    pub async fn run_lead_proposal(
        &self,
        message_id: impl Into<String>,
        jobs: Option<Vec<ProposedJob>>,
    ) -> Result<Vec<JobRecord>, AppError> {
        let message_id = message_id.into();
        self.invoke(move |world| world.run_lead_proposal(message_id, jobs))
            .await
    }

    pub async fn dismiss_lead_proposal(
        &self,
        message_id: impl Into<String>,
    ) -> Result<(), AppError> {
        let message_id = message_id.into();
        self.invoke(move |world| world.dismiss_lead_proposal(message_id))
            .await
    }

    pub async fn list_lead_messages(&self) -> Result<Vec<LeadMessage>, AppError> {
        self.invoke(|world| world.list_lead_messages()).await
    }

    pub async fn list_lead_chats(&self) -> Result<Vec<LeadChat>, AppError> {
        self.invoke(|world| world.list_lead_chats()).await
    }

    pub async fn open_lead_chat(
        &self,
        id: impl Into<String>,
    ) -> Result<Vec<LeadMessage>, AppError> {
        let id = id.into();
        self.invoke(move |world| world.open_lead_chat(id)).await
    }

    pub async fn reset_lead(&self, direct: Option<ProviderId>) -> Result<(), AppError> {
        self.invoke(move |world| world.reset_lead(direct)).await
    }

    pub async fn get_lead_mode(&self) -> Result<Option<ProviderId>, AppError> {
        self.invoke(|world| world.get_lead_mode()).await
    }

    pub async fn rename_lead_chat(
        &self,
        id: impl Into<String>,
        title: impl Into<String>,
    ) -> Result<(), AppError> {
        let id = id.into();
        let title = title.into();
        self.invoke(move |world| world.rename_lead_chat(id, title))
            .await
    }

    pub async fn remove_lead_chat(&self, id: impl Into<String>) -> Result<(), AppError> {
        let id = id.into();
        self.invoke(move |world| world.remove_lead_chat(id)).await
    }

    pub async fn open_terminal(
        &self,
        kind: TerminalKind,
        cols: f64,
        rows: f64,
        worktree: Option<WorktreePick>,
        prompt: Option<String>,
    ) -> Result<TerminalInfo, AppError> {
        self.invoke(move |world| world.open_terminal(kind, cols, rows, worktree, prompt))
            .await
    }

    pub async fn write_terminal(
        &self,
        id: impl Into<String>,
        data: impl Into<String>,
    ) -> Result<(), AppError> {
        let id = id.into();
        let data = data.into();
        self.invoke(move |world| world.write_terminal(id, data))
            .await
    }

    pub async fn claim_terminal(&self, id: impl Into<String>) -> Result<(), AppError> {
        let id = id.into();
        self.invoke(move |world| world.claim_terminal(id)).await
    }

    pub async fn resize_terminal(
        &self,
        id: impl Into<String>,
        cols: f64,
        rows: f64,
    ) -> Result<(), AppError> {
        let id = id.into();
        self.invoke(move |world| world.resize_terminal(id, cols, rows))
            .await
    }

    pub async fn close_terminal(&self, id: impl Into<String>) -> Result<(), AppError> {
        let id = id.into();
        self.invoke(move |world| world.close_terminal(id)).await
    }

    pub async fn sleep_terminal(&self, id: impl Into<String>) -> Result<bool, AppError> {
        let id = id.into();
        self.invoke(move |world| world.sleep_terminal(id)).await
    }

    pub async fn wake_terminal(
        &self,
        id: impl Into<String>,
        cols: i64,
        rows: i64,
    ) -> Result<Option<TerminalInfo>, AppError> {
        let id = id.into();
        self.invoke(move |world| world.wake_terminal(id, cols, rows))
            .await
    }

    pub async fn list_terminals(&self) -> Result<Vec<TerminalInfo>, AppError> {
        self.invoke(|world| world.list_terminals()).await
    }

    pub async fn restore_terminals(
        &self,
        cols: i64,
        rows: i64,
    ) -> Result<Vec<TerminalInfo>, AppError> {
        self.invoke(move |world| world.restore_terminals(cols, rows))
            .await
    }

    pub async fn terminal_snapshot(&self, id: impl Into<String>) -> Result<String, AppError> {
        let id = id.into();
        self.invoke(move |world| world.terminal_snapshot(id)).await
    }

    /// A Legate window just gained focus. Starts a usage check when one is due.
    pub async fn note_window_focus(&self) -> Result<(), AppError> {
        self.set_window_sight(WindowSight {
            focused: true,
            visible: true,
        })
        .await
    }

    /// Whether any Legate window is focused or merely on screen. The Tauri shell reports this.
    pub async fn set_window_sight(&self, sight: WindowSight) -> Result<(), AppError> {
        self.invoke(move |world| world.set_sight(sight)).await
    }

    /// Data directory this handle opened. Useful before `get_app_info` when start failed.
    pub fn opened_data_dir(override_dir: Option<&std::path::Path>) -> PathBuf {
        crate::paths::app_data_dir(override_dir)
    }
}

use std::future::Future;

fn pack<T, F>(call: F, tx: oneshot::Sender<Result<T, AppError>>) -> Command
where
    T: Send + 'static,
    F: for<'a> FnOnce(&'a World) -> crate::session::Task<T> + Send + 'static,
{
    Command::Run(Box::new(move |world: &World| {
        let task = call(world);
        Box::pin(async move {
            let _ = tx.send(task.await);
        })
    }))
}
