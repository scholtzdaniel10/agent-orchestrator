//! One method per `OrchestratorApi` call. Each future is `'static` so it can be
//! spawned on the local set from a `Send` command.

use std::path::Path;
use std::rc::Rc;

use legate_core::{
    FileContent, FileList, JobAccess, JobRecord, JobStatus, JobType, LeadChat, LeadMessage,
    ModelOption, PlanStatus, Prefs, ProjectEntry, ProjectInfo, ProjectWorktree, ProviderId,
    SkillsState, TerminalInfo,
};
use legate_git::{
    CommitDetail, CommitPage, GitError, GuardJob, ListedName, MergeOutcome, OpenedPr, PAGE,
    Worktrees, assert_worktree_idle, parse_setup_commands, parse_worktree_name, require_hash,
};
use legate_lead::{
    Firing, Goal, GoalSource, LeadError, NewGoal, NewTrigger, PlannedJob, QuickJob, Trigger,
    fresh_firings, github_facts, is_scheduled, is_trigger_kind, new_trigger,
};
use legate_pty::{OpenOptions, ShellOptions};
use legate_router::{SubmitAccess, SubmitExtra, system_now_ms};
use legate_store::WorktreeNameRow;
use serde_json::Value;

use crate::error::AppError;
use crate::event::AppEvent;
use crate::options::{GoalInput, TerminalKind, TriggerInput, UsageOutcome, WorktreePick};
use crate::paths::is_folder;
use crate::session::{
    Task, World, fail, mirror_project, publish_plans, ready, skills_state, sync_models,
};
use crate::usage::UsageTrigger;

const WINDOW_ID: u64 = 1;
const GONE: &str = "That worktree folder is gone. It may have been merged or discarded.";
const NOT_A_REPO: &str = "This folder is not a git repository, so it cannot have worktrees.";

fn lead_err(err: LeadError) -> AppError {
    AppError::message(err.to_string())
}

fn git_err(err: GitError) -> AppError {
    AppError::message(err.to_string())
}

fn job_status(status: JobStatus) -> &'static str {
    match status {
        JobStatus::Queued => "queued",
        JobStatus::Running => "running",
        JobStatus::Done => "done",
        JobStatus::Failed => "failed",
    }
}

fn guard_of(job: &JobRecord) -> GuardJob {
    GuardJob {
        status: job_status(job.status).to_string(),
        change: job.change.clone(),
        owner_id: job.owner_id.clone(),
        heartbeat_at: job.heartbeat_at,
    }
}

impl World {
    fn refuse_switch(&self) -> Result<(), AppError> {
        if self.lead.is_busy() {
            return Err(AppError::message(
                "Wait for the lead to finish its reply before changing project.",
            ));
        }
        if self.orch.busy() {
            return Err(AppError::message("Refused while work is running"));
        }
        Ok(())
    }

    fn refuse_pr(&self) -> Result<(), AppError> {
        let busy = self
            .orch
            .list()
            .iter()
            .any(|job| matches!(job.status, JobStatus::Queued | JobStatus::Running));
        if busy {
            return Err(AppError::message(
                "Wait for running jobs to finish, or stop them, before opening a pull request.",
            ));
        }
        Ok(())
    }

    pub fn submit_job(
        &self,
        job_type: JobType,
        prompt: String,
        provider: Option<ProviderId>,
        access: Option<JobAccess>,
        group: Option<String>,
    ) -> Task<JobRecord> {
        if prompt.trim().is_empty() {
            return fail("empty prompt");
        }
        let orch = Rc::clone(&self.orch);
        Box::pin(async move {
            let group = group.filter(|value| !value.is_empty() && value.len() <= 64);
            Ok(orch.submit(
                job_type,
                &prompt,
                provider,
                access.map(SubmitAccess::Level),
                SubmitExtra {
                    group,
                    ..SubmitExtra::default()
                },
            ))
        })
    }

    pub fn list_jobs(&self) -> Task<Vec<JobRecord>> {
        let jobs = self.orch.list();
        ready(Ok(jobs))
    }

    pub fn cancel_job(&self, id: String) -> Task<JobRecord> {
        if id.is_empty() || id.len() > 64 {
            return fail("invalid job id");
        }
        let orch = Rc::clone(&self.orch);
        Box::pin(async move {
            orch.cancel(&id)
                .map_err(|err| AppError::message(err.to_string()))
        })
    }

    pub fn refresh_usage(&self) -> Task<UsageOutcome> {
        self.begin_usage(UsageTrigger::Manual)
    }

    pub fn set_cursor_reading(&self, used_percent: f64, resets_at: i64) -> Task<UsageOutcome> {
        let now = system_now_ms();
        let reading =
            match legate_router::dashboard_reading(Some(used_percent), Some(resets_at), now) {
                Ok(reading) => reading,
                Err(message) => return ready(Ok(UsageOutcome::failure(message))),
            };
        let store = Rc::clone(&self.store);
        let events = self.events.clone();
        let orch = Rc::clone(&self.orch);
        let settings = Rc::clone(&self.settings);
        Box::pin(async move {
            store
                .save_windows(
                    "cursor",
                    &[legate_store::NewWindow {
                        name: reading.name.to_string(),
                        utilization: reading.utilization,
                        resets_at: Some(reading.resets_at),
                    }],
                    now,
                )
                .map_err(|err| AppError::message(err.to_string()))?;
            let plans = crate::session::plan_list(&orch, &settings.borrow(), &store);
            let _ = events.send(AppEvent::Plans(plans));
            Ok(UsageOutcome::success())
        })
    }

    pub fn clear_cursor_reading(&self) -> Task<()> {
        let store = Rc::clone(&self.store);
        let orch = Rc::clone(&self.orch);
        let settings = Rc::clone(&self.settings);
        let events = self.events.clone();
        Box::pin(async move {
            store
                .delete_window("cursor", legate_router::DASHBOARD_WINDOW)
                .map_err(|err| AppError::message(err.to_string()))?;
            let plans = crate::session::plan_list(&orch, &settings.borrow(), &store);
            let _ = events.send(AppEvent::Plans(plans));
            Ok(())
        })
    }

    pub fn take_over_job(&self, id: String, cols: i64, rows: i64) -> Task<TerminalInfo> {
        if id.is_empty() || id.len() > 64 {
            return fail("invalid job id");
        }
        let orch = Rc::clone(&self.orch);
        let terms = self.terms.clone();
        let trees = Worktrees::new(&self.worktrees_dir);
        let project = self.project.borrow().clone();
        let push = self.change_push.clone();
        Box::pin(async move {
            let job = orch
                .get(&id)
                .ok_or_else(|| AppError::message("That job is gone."))?;
            let (provider, session) = match (job.provider, job.session_id.clone()) {
                (Some(provider), Some(session)) => (provider, session),
                _ => {
                    return Err(AppError::message(
                        "This job has not started a session yet. Try again in a moment.",
                    ));
                }
            };
            let mut opts = OpenOptions {
                resume: Some(session),
                cwd: Some(orch.project_of(&job.id).unwrap_or(project)),
                ..OpenOptions::default()
            };
            if let Some(change) = job.change.clone() {
                let folder = trees.folder(&change).map_err(git_err)?;
                if !is_folder(&folder) {
                    return Err(AppError::message(GONE));
                }
                opts.cwd = Some(folder.to_string_lossy().into_owned());
                opts.change = Some(change);
            }
            if matches!(job.status, JobStatus::Running | JobStatus::Queued) {
                let _ = orch.cancel(&job.id);
            }
            let info = terms
                .open(provider, cols as f64, rows as f64, opts)
                .await
                .map_err(|err| AppError::message(err.to_string()))?;
            if job.change.is_some() {
                push.spawn();
            }
            Ok(info)
        })
    }

    pub fn list_plans(&self) -> Task<Vec<PlanStatus>> {
        let plans = crate::session::plan_list(&self.orch, &self.settings.borrow(), &self.store);
        ready(Ok(plans))
    }

    pub fn recheck_plans(&self) -> Task<Vec<PlanStatus>> {
        let orch = Rc::clone(&self.orch);
        let lead = Rc::clone(&self.lead);
        let settings = Rc::clone(&self.settings);
        let store = Rc::clone(&self.store);
        let events = self.events.clone();
        Box::pin(async move {
            orch.recheck().await;
            lead.recheck().await.map_err(lead_err)?;
            let plans = crate::session::plan_list(&orch, &settings.borrow(), &store);
            let _ = events.send(AppEvent::Plans(plans.clone()));
            Ok(plans)
        })
    }

    pub fn list_models(&self, provider: ProviderId) -> Task<Vec<ModelOption>> {
        let claude = Rc::new(self.claude.clone());
        let cursor = self.cursor.clone();
        let grok = self.grok.clone();
        let local = self.local.clone();
        Box::pin(async move {
            let models = match provider {
                ProviderId::Claude => claude.list_models().await,
                ProviderId::Grok => grok.list_models().await,
                ProviderId::Local => local.list_models().await,
                ProviderId::Cursor => cursor.list_models().await,
            };
            Ok(models)
        })
    }

    pub fn set_model(&self, provider: ProviderId, model: Option<String>) -> Task<()> {
        if let Some(model) = &model
            && !legate_core::is_valid_model(model)
        {
            return fail("invalid model name");
        }
        let settings = Rc::clone(&self.settings);
        let models = std::sync::Arc::clone(&self.models);
        let orch = Rc::clone(&self.orch);
        let store = Rc::clone(&self.store);
        let events = self.events.clone();
        Box::pin(async move {
            settings
                .borrow_mut()
                .set_model(provider, model.as_deref())
                .map_err(|err| AppError::message(err.to_string()))?;
            sync_models(&settings.borrow(), &models);
            let plans = crate::session::plan_list(&orch, &settings.borrow(), &store);
            let _ = events.send(AppEvent::Plans(plans));
            Ok(())
        })
    }

    pub fn get_project(&self) -> Task<ProjectInfo> {
        let trees = Worktrees::new(&self.worktrees_dir);
        let project = self.project.borrow().clone();
        Box::pin(async move { Ok(trees.info(Path::new(&project)).await) })
    }

    pub fn list_projects(&self) -> Task<Vec<ProjectEntry>> {
        let active = self.project.borrow().clone();
        let mut paths = self
            .settings
            .borrow()
            .projects()
            .into_iter()
            .filter(|path| is_folder(Path::new(path)))
            .collect::<Vec<_>>();
        if paths.is_empty() {
            paths.push(active.clone());
        }
        let orch = Rc::clone(&self.orch);
        let store = Rc::clone(&self.store);
        let tracker = Rc::clone(&self.setup_tracker);
        let root = self.worktrees_dir.clone();
        Box::pin(async move {
            let mut entries = Vec::new();
            for path in paths {
                entries.push(describe(&orch, &store, &tracker, &root, &path, path == active).await);
            }
            Ok(entries)
        })
    }

    pub fn choose_project(&self, path: String) -> Task<ProjectInfo> {
        if let Err(err) = self.refuse_switch() {
            return ready(Err(err));
        }
        if !is_folder(Path::new(&path)) {
            return fail("unknown project");
        }
        self.finish_activate(path)
    }

    pub fn switch_project(&self, path: String) -> Task<ProjectInfo> {
        if !self
            .settings
            .borrow()
            .projects()
            .iter()
            .any(|item| item == &path)
            || !is_folder(Path::new(&path))
        {
            return fail("unknown project");
        }
        if let Err(err) = self.refuse_switch() {
            return ready(Err(err));
        }
        self.finish_activate(path)
    }

    pub fn finish_activate(&self, path: String) -> Task<ProjectInfo> {
        if let Err(err) = self.settings.borrow_mut().set_project(&path) {
            return ready(Err(AppError::message(err.to_string())));
        }
        mirror_project(self, &path);
        self.reload();
        self.change_push.spawn();
        for job in self.orch.list() {
            let _ = self.events.send(AppEvent::Job(job));
        }
        for message in self.lead.messages() {
            let _ = self.events.send(AppEvent::Lead {
                message,
                chat_id: self.lead.view_chat(),
            });
        }
        publish_plans(self);
        let trees = Worktrees::new(&self.worktrees_dir);
        Box::pin(async move { Ok(trees.info(Path::new(&path)).await) })
    }

    pub fn remove_project(&self, path: String) -> Task<()> {
        if path.is_empty() {
            return fail("unknown project");
        }
        match self.settings.borrow_mut().remove_project(&path) {
            Ok(()) => ready(Ok(())),
            Err(err) => ready(Err(AppError::message(err.to_string()))),
        }
    }

    pub fn github_available(&self) -> Task<bool> {
        let client = clone_github();
        Box::pin(async move { Ok(client.available().await) })
    }

    pub fn github_repos(&self) -> Task<Vec<legate_core::GithubRepo>> {
        let client = clone_github();
        Box::pin(async move { Ok(client.repos().await) })
    }

    pub fn github_clone(&self, name: String, parent: String) -> Task<ProjectInfo> {
        if !valid_repo(&name) {
            return fail("invalid repository name");
        }
        if let Err(err) = self.refuse_switch() {
            return ready(Err(err));
        }
        if !is_folder(Path::new(&parent)) {
            return fail("unknown project");
        }
        let client = clone_github();
        let parent_buf = parent.clone();
        let settings = Rc::clone(&self.settings);
        let project_cell = Rc::clone(&self.project);
        let mirror = std::sync::Arc::clone(&self.project_mirror);
        let lead = Rc::clone(&self.lead);
        let orch = Rc::clone(&self.orch);
        let goals = Rc::clone(&self.goals);
        let missed = Rc::clone(&self.missed);
        let events = self.events.clone();
        let push = self.change_push.clone();
        let store = Rc::clone(&self.store);
        let trees = Worktrees::new(&self.worktrees_dir);
        Box::pin(async move {
            let cloned = client
                .clone(&name, Path::new(&parent_buf))
                .await
                .map_err(git_err)?;
            let path = cloned.to_string_lossy().into_owned();
            settings
                .borrow_mut()
                .set_project(&path)
                .map_err(|err| AppError::message(err.to_string()))?;
            *project_cell.borrow_mut() = path.clone();
            *mirror.write().unwrap_or_else(|err| err.into_inner()) = path.clone();
            crate::session::reload_project(&lead, &orch, &goals, &path, &missed);
            push.spawn();
            for job in orch.list() {
                let _ = events.send(AppEvent::Job(job));
            }
            for message in lead.messages() {
                let _ = events.send(AppEvent::Lead {
                    message,
                    chat_id: lead.view_chat(),
                });
            }
            let plans = crate::session::plan_list(&orch, &settings.borrow(), &store);
            let _ = events.send(AppEvent::Plans(plans));
            Ok(trees.info(Path::new(&path)).await)
        })
    }

    pub fn github_open_pr(&self, title: String, body: String) -> Task<OpenedPr> {
        if title.trim().is_empty()
            || title.len() > 120
            || title.contains('\n')
            || title.contains('\r')
        {
            return fail("invalid title");
        }
        if body.len() > 4000 {
            return fail("invalid description");
        }
        if let Err(err) = self.refuse_pr() {
            return ready(Err(err));
        }
        let client = clone_github();
        let project = self.project.borrow().clone();
        let push = self.change_push.clone();
        Box::pin(async move {
            let opened = client
                .open_pr(Path::new(&project), &title, &body)
                .await
                .map_err(git_err)?;
            push.spawn();
            Ok(opened)
        })
    }

    pub fn setup_status(&self) -> Task<Vec<legate_lead::Requirement>> {
        let setup = SetupHold {
            claude: self.claude.clone(),
            cursor: self.cursor.clone(),
        };
        Box::pin(async move { Ok(setup.run().await) })
    }

    pub fn setup_sign_in(&self, id: String) -> Task<TerminalInfo> {
        let label = match id.as_str() {
            "claude" => "Claude Code CLI",
            "cursor" => "Cursor CLI",
            "gh" => "GitHub CLI",
            _ => return fail("unknown setup tool"),
        };
        let line = sign_in_line(&id, &self.claude, &self.cursor);
        let terms = self.terms.clone();
        let sign_in = Rc::clone(&self.sign_in);
        let home = home_dir();
        Box::pin(async move {
            let info = terms
                .open_shell(ShellOptions {
                    cwd: home,
                    change: None,
                    title: format!("sign in · {label}"),
                    commands: vec![line],
                    cols: 100.0,
                    rows: 28.0,
                })
                .map_err(|err| AppError::message(err.to_string()))?;
            sign_in.borrow_mut().insert(info.id.clone());
            Ok(info)
        })
    }

    pub fn skills_get(&self) -> Task<SkillsState> {
        ready(Ok(skills_state(self)))
    }

    pub fn skills_add(&self, folder: String) -> Task<SkillsState> {
        if !is_folder(Path::new(&folder)) {
            return fail("Folder not found.");
        }
        let mut folders = self.settings.borrow().skill_folders();
        folders.push(folder);
        self.write_skills(folders)
    }

    pub fn skills_remove(&self, folder: String) -> Task<SkillsState> {
        let folders = self
            .settings
            .borrow()
            .skill_folders()
            .into_iter()
            .filter(|entry| entry != &folder)
            .collect::<Vec<_>>();
        self.write_skills(folders)
    }

    pub fn skills_reload(&self) -> Task<SkillsState> {
        let folders = self.settings.borrow().skill_folders();
        self.write_skills(folders)
    }

    fn write_skills(&self, folders: Vec<String>) -> Task<SkillsState> {
        let refs = folders.iter().map(String::as_str).collect::<Vec<_>>();
        if let Err(err) = self.settings.borrow_mut().set_skill_folders(&refs) {
            return ready(Err(AppError::message(err.to_string())));
        }
        self.reapply_skills();
        ready(Ok(skills_state(self)))
    }

    pub fn list_changes(&self) -> Task<Vec<legate_core::ChangeSet>> {
        let push = self.change_push.clone();
        Box::pin(async move { Ok(push.collect_pub().await) })
    }

    pub fn change_diff(&self, id: String) -> Task<String> {
        if !is_change_id(&id) {
            return fail("invalid change id");
        }
        let trees = Worktrees::new(&self.worktrees_dir);
        let project = self.project.borrow().clone();
        Box::pin(async move { trees.diff(Path::new(&project), &id).await.map_err(git_err) })
    }

    pub fn merge_change(&self, id: String) -> Task<MergeOutcome> {
        if !is_change_id(&id) {
            return fail("invalid change id");
        }
        if let Err(err) = self.idle_worktree(&id) {
            return ready(Err(err));
        }
        let terms = self.terms.clone();
        let trees = Worktrees::new(&self.worktrees_dir);
        let store = Rc::clone(&self.store);
        let tracker = Rc::clone(&self.setup_tracker);
        let project = self.project.borrow().clone();
        let push = self.change_push.clone();
        Box::pin(async move {
            terms.close_change(&id).await;
            let result = trees
                .merge(Path::new(&project), &id)
                .await
                .map_err(git_err)?;
            if result.ok {
                let _ = store.delete_worktree_name(&id);
                tracker.borrow_mut().clear(&id);
            }
            push.spawn();
            Ok(result)
        })
    }

    pub fn discard_change(&self, id: String) -> Task<()> {
        if !is_change_id(&id) {
            return fail("invalid change id");
        }
        if let Err(err) = self.idle_worktree(&id) {
            return ready(Err(err));
        }
        let terms = self.terms.clone();
        let trees = Worktrees::new(&self.worktrees_dir);
        let store = Rc::clone(&self.store);
        let tracker = Rc::clone(&self.setup_tracker);
        let project = self.project.borrow().clone();
        let push = self.change_push.clone();
        Box::pin(async move {
            terms.close_change(&id).await;
            trees
                .discard(Path::new(&project), &id)
                .await
                .map_err(git_err)?;
            let _ = store.delete_worktree_name(&id);
            tracker.borrow_mut().clear(&id);
            push.spawn();
            Ok(())
        })
    }

    fn idle_worktree(&self, id: &str) -> Result<(), AppError> {
        let mut jobs = self.orch.list().iter().map(guard_of).collect::<Vec<_>>();
        if let Ok(saved) = self.store.jobs(&self.project.borrow(), 200) {
            jobs.extend(saved.iter().map(guard_of));
        }
        assert_worktree_idle(&jobs, id, system_now_ms()).map_err(git_err)
    }

    pub fn create_worktree(&self, name: Option<String>) -> Task<legate_core::ChangeSet> {
        let label = match parse_worktree_name(name.as_deref(), false) {
            Ok(label) => label.unwrap_or_default(),
            Err(err) => return ready(Err(git_err(err))),
        };
        let trees = Worktrees::new(&self.worktrees_dir);
        let project = self.project.borrow().clone();
        let store = Rc::clone(&self.store);
        let terms = self.terms.clone();
        let tracker = Rc::clone(&self.setup_tracker);
        let push = self.change_push.clone();
        Box::pin(async move {
            let about = trees.info(Path::new(&project)).await;
            if !about.is_repo {
                return Err(AppError::message(NOT_A_REPO));
            }
            let id = uuid::Uuid::new_v4().to_string();
            let created = trees
                .create(Path::new(&project), &id, None)
                .await
                .map_err(git_err)?;
            let _ = store.set_worktree_name(&WorktreeNameRow {
                id: created.id.clone(),
                project: project.clone(),
                name: label.clone(),
                created_at: system_now_ms(),
            });
            let commands = store.get_project_setup(&project).unwrap_or_default();
            if !commands.is_empty() {
                let title = if label.is_empty() {
                    format!("setup · {}", created.id)
                } else {
                    format!("setup · {label}")
                };
                match terms.open_shell(ShellOptions {
                    cwd: created.path.to_string_lossy().into_owned(),
                    change: Some(created.id.clone()),
                    title,
                    commands,
                    cols: 100.0,
                    rows: 30.0,
                }) {
                    Ok(_) => tracker.borrow_mut().begin(&created.id, true),
                    Err(err) => {
                        tracing::error!("{err}");
                        tracker.borrow_mut().begin(&created.id, false);
                    }
                }
            }
            let list = push.collect_pub().await;
            push.spawn();
            list.into_iter()
                .find(|change| change.id == created.id)
                .ok_or_else(|| {
                    AppError::message(
                        "The worktree was created, but it did not appear in the list. Try again.",
                    )
                })
        })
    }

    pub fn rename_worktree(&self, id: String, name: String) -> Task<()> {
        if !is_change_id(&id) {
            return fail("invalid change id");
        }
        let label = match parse_worktree_name(Some(&name), true) {
            Ok(Some(label)) => label,
            Ok(None) | Err(_) => {
                return fail("Name must be 1 to 40 characters on one line.");
            }
        };
        let trees = Worktrees::new(&self.worktrees_dir);
        let folder = match trees.folder(&id) {
            Ok(folder) if is_folder(&folder) => folder,
            _ => return fail(GONE),
        };
        let _ = folder;
        let existing = self
            .store
            .worktree_names(&self.project.borrow())
            .unwrap_or_default()
            .into_iter()
            .find(|row| row.id == id);
        let project = self.project.borrow().clone();
        let store = Rc::clone(&self.store);
        let push = self.change_push.clone();
        Box::pin(async move {
            store
                .set_worktree_name(&WorktreeNameRow {
                    id,
                    project,
                    name: label,
                    created_at: existing
                        .map(|row| row.created_at)
                        .unwrap_or_else(system_now_ms),
                })
                .map_err(|err| AppError::message(err.to_string()))?;
            push.spawn();
            Ok(())
        })
    }

    pub fn list_files(&self, where_id: Option<String>) -> Task<FileList> {
        let root = match self.files_root(where_id) {
            Ok(root) => root,
            Err(err) => return ready(Err(err)),
        };
        let files = clone_files();
        Box::pin(async move { Ok(files.list(Path::new(&root)).await) })
    }

    pub fn read_file(&self, where_id: Option<String>, path: String) -> Task<FileContent> {
        if !valid_file_path(&path) {
            return fail("invalid path");
        }
        let root = match self.files_root(where_id) {
            Ok(root) => root,
            Err(err) => return ready(Err(err)),
        };
        let files = clone_files();
        Box::pin(async move { files.read(Path::new(&root), &path).await.map_err(git_err) })
    }

    pub fn file_diff(&self, where_id: Option<String>, path: String) -> Task<String> {
        if !valid_file_path(&path) {
            return fail("invalid path");
        }
        let root = match self.files_root(where_id) {
            Ok(root) => root,
            Err(err) => return ready(Err(err)),
        };
        let files = clone_files();
        Box::pin(async move { files.diff(Path::new(&root), &path).await.map_err(git_err) })
    }

    fn files_root(&self, where_id: Option<String>) -> Result<String, AppError> {
        let Some(id) = where_id else {
            return Ok(self.project.borrow().clone());
        };
        if !is_change_id(&id) {
            return Err(AppError::message("invalid change id"));
        }
        let folder = Worktrees::new(&self.worktrees_dir)
            .folder(&id)
            .map_err(git_err)?;
        if !is_folder(&folder) {
            return Err(AppError::message(GONE));
        }
        Ok(folder.to_string_lossy().into_owned())
    }

    pub fn list_goals(&self) -> Task<Vec<Goal>> {
        let project = self.project.borrow().clone();
        match self.goals.list(&project) {
            Ok(goals) => ready(Ok(goals)),
            Err(err) => ready(Err(lead_err(err))),
        }
    }

    pub fn create_goal(&self, input: GoalInput) -> Task<Goal> {
        if input.text.is_empty() || input.text.len() > 20_000 {
            return fail("Say what the goal is.");
        }
        let check = input.check.filter(|value| value.len() <= 2000);
        let quick = match (input.quick_type, input.quick_access) {
            (Some(job_type), Some(access)) => Some(QuickJob { job_type, access }),
            _ => None,
        };
        let goals = Rc::clone(&self.goals);
        let project = self.project.borrow().clone();
        Box::pin(async move {
            goals
                .create(NewGoal {
                    project,
                    text: input.text,
                    check,
                    max_rounds: input.max_rounds,
                    quick,
                    source: None,
                })
                .await
                .map_err(lead_err)
        })
    }

    pub fn approve_goal(&self, id: String, jobs: Option<Vec<PlannedJob>>) -> Task<Goal> {
        if id.is_empty() {
            return fail("unknown goal");
        }
        let goals = Rc::clone(&self.goals);
        Box::pin(async move { goals.approve(&id, jobs).map_err(lead_err) })
    }

    pub fn answer_goal(&self, id: String, text: String) -> Task<Goal> {
        if id.is_empty() || text.len() > 20_000 {
            return fail("invalid answer");
        }
        let goals = Rc::clone(&self.goals);
        Box::pin(async move { goals.answer(&id, &text).map_err(lead_err) })
    }

    pub fn stop_goal(&self, id: String) -> Task<Goal> {
        if id.is_empty() {
            return fail("unknown goal");
        }
        let goals = Rc::clone(&self.goals);
        Box::pin(async move { goals.stop(&id).map_err(lead_err) })
    }

    pub fn remove_goal(&self, id: String) -> Task<()> {
        if id.is_empty() {
            return fail("unknown goal");
        }
        let goals = Rc::clone(&self.goals);
        let push = self.change_push.clone();
        Box::pin(async move {
            goals.remove(&id).await.map_err(lead_err)?;
            push.spawn();
            Ok(())
        })
    }

    pub fn list_triggers(&self) -> Task<Vec<Trigger>> {
        ready(Ok(self.triggers_here()))
    }

    pub fn add_trigger(&self, input: TriggerInput) -> Task<Trigger> {
        if !is_trigger_kind(&input.kind) {
            return fail("unknown trigger");
        }
        let kind = match legate_lead::TriggerKind::parse(&input.kind) {
            Some(kind) => kind,
            None => return fail("unknown trigger"),
        };
        let trigger = match new_trigger(NewTrigger {
            project: self.project.borrow().clone(),
            kind,
            at: input.at,
            hours: input.hours,
            text: input.text.map(|text| text.chars().take(20_000).collect()),
            check: input.check.map(|text| text.chars().take(2000).collect()),
            max_rounds: input.max_rounds,
            now: system_now_ms(),
        }) {
            Ok(trigger) => trigger,
            Err(err) => return ready(Err(lead_err(err))),
        };
        if let Err(err) = self.triggers.save(&trigger) {
            return ready(Err(lead_err(err)));
        }
        if !is_scheduled(trigger.kind) {
            let triggers = Rc::clone(&self.triggers);
            let goals = Rc::clone(&self.goals);
            let store = Rc::clone(&self.store);
            let project = trigger.project.clone();
            tokio::task::spawn_local(async move {
                force_github(&store, &triggers, &goals, &project).await;
            });
        }
        ready(Ok(trigger))
    }

    pub fn toggle_trigger(&self, id: String, enabled: bool) -> Task<Trigger> {
        let mut trigger = match self.find_trigger(&id) {
            Some(trigger) => trigger,
            None => return fail("unknown trigger"),
        };
        trigger.enabled = enabled;
        if let Err(err) = self.triggers.save(&trigger) {
            return ready(Err(lead_err(err)));
        }
        ready(Ok(trigger))
    }

    pub fn remove_trigger(&self, id: String) -> Task<()> {
        if self.find_trigger(&id).is_none() {
            return fail("unknown trigger");
        }
        if let Err(err) = self.store.delete_trigger(&id) {
            return ready(Err(AppError::message(err.to_string())));
        }
        self.triggers.poke();
        self.publish_triggers();
        ready(Ok(()))
    }

    pub fn run_trigger(&self, id: String) -> Task<()> {
        let Some(trigger) = self.find_trigger(&id) else {
            return fail("unknown trigger");
        };
        let goals = Rc::clone(&self.goals);
        let triggers = Rc::clone(&self.triggers);
        let store = Rc::clone(&self.store);
        Box::pin(async move {
            if is_scheduled(trigger.kind) {
                fire_one(&triggers, &goals, trigger, None).await;
            } else {
                let project = trigger.project.clone();
                force_github(&store, &triggers, &goals, &project).await;
            }
            Ok(())
        })
    }

    fn triggers_here(&self) -> Vec<Trigger> {
        let project = self.project.borrow().clone();
        let Ok(raw) = self.store.triggers() else {
            return Vec::new();
        };
        raw.into_iter()
            .filter_map(|item| crate::session::stored_trigger(&item))
            .filter(|trigger| trigger.project == project)
            .collect()
    }

    fn find_trigger(&self, id: &str) -> Option<Trigger> {
        self.triggers_here()
            .into_iter()
            .find(|trigger| trigger.id == id)
    }

    fn publish_triggers(&self) {
        let _ = self.events.send(AppEvent::Triggers(self.triggers_here()));
    }

    pub fn commit_log(&self, where_id: Option<String>, skip: i64) -> Task<CommitPage> {
        let root = match self.files_root(where_id) {
            Ok(root) => root,
            Err(err) => return ready(Err(err)),
        };
        let skip = if skip >= 0 { skip } else { 0 };
        let history = clone_history();
        Box::pin(async move {
            history
                .log(Path::new(&root), skip, PAGE)
                .await
                .map_err(git_err)
        })
    }

    pub fn commit_detail(&self, where_id: Option<String>, hash: String) -> Task<CommitDetail> {
        let hash = match require_hash(&hash) {
            Ok(hash) => hash,
            Err(err) => return ready(Err(git_err(err))),
        };
        let root = match self.files_root(where_id) {
            Ok(root) => root,
            Err(err) => return ready(Err(err)),
        };
        let history = clone_history();
        Box::pin(async move {
            history
                .detail(Path::new(&root), &hash)
                .await
                .map_err(git_err)
        })
    }

    pub fn commit_file_diff(
        &self,
        where_id: Option<String>,
        hash: String,
        path: String,
    ) -> Task<String> {
        if !valid_file_path(&path) {
            return fail("invalid path");
        }
        let hash = match require_hash(&hash) {
            Ok(hash) => hash,
            Err(err) => return ready(Err(git_err(err))),
        };
        let root = match self.files_root(where_id) {
            Ok(root) => root,
            Err(err) => return ready(Err(err)),
        };
        let history = clone_history();
        Box::pin(async move {
            history
                .file_diff(Path::new(&root), &hash, &path)
                .await
                .map_err(git_err)
        })
    }

    pub fn get_prefs(&self) -> Task<Prefs> {
        ready(Ok(self.settings.borrow().prefs()))
    }

    pub fn set_prefs(&self, patch: Value) -> Task<Prefs> {
        let prefs = match self.settings.borrow_mut().set_prefs(&patch) {
            Ok(prefs) => prefs,
            Err(err) => return ready(Err(AppError::message(err.to_string()))),
        };
        self.local
            .set_config(legate_providers::LocalConfig::from_prefs(&prefs));
        sync_models(&self.settings.borrow(), &self.models);
        let _ = self.events.send(AppEvent::Prefs(prefs.clone()));
        ready(Ok(prefs))
    }

    pub fn choose_clone_dir(&self, path: String) -> Task<Prefs> {
        if !is_folder(Path::new(&path)) {
            return fail("unknown project");
        }
        let patch = serde_json::json!({ "cloneDir": path });
        self.set_prefs(patch)
    }

    pub fn get_app_info(&self) -> Task<legate_core::AppInfo> {
        ready(Ok(self.app_info()))
    }

    pub fn get_project_setup(&self) -> Task<Vec<String>> {
        match self.store.get_project_setup(&self.project.borrow()) {
            Ok(commands) => ready(Ok(commands)),
            Err(err) => ready(Err(AppError::message(err.to_string()))),
        }
    }

    pub fn set_project_setup(&self, commands: Vec<String>) -> Task<()> {
        let value = Value::Array(commands.into_iter().map(Value::String).collect());
        let commands = match parse_setup_commands(&value) {
            Ok(commands) => commands,
            Err(err) => return ready(Err(git_err(err))),
        };
        match self
            .store
            .set_project_setup(&self.project.borrow(), &commands)
        {
            Ok(()) => ready(Ok(())),
            Err(err) => ready(Err(AppError::message(err.to_string()))),
        }
    }

    pub fn list_routines(&self) -> Task<Vec<legate_lead::Routine>> {
        match self.routines.list() {
            Ok(routines) => ready(Ok(routines)),
            Err(err) => ready(Err(lead_err(err))),
        }
    }

    pub fn save_routine(&self, value: Value) -> Task<()> {
        match self.routines.save(&value) {
            Ok(_) => ready(Ok(())),
            Err(err) => ready(Err(lead_err(err))),
        }
    }

    pub fn remove_routine(&self, id: String) -> Task<()> {
        if id.is_empty() {
            return fail("invalid routine id");
        }
        match self.routines.remove(&id) {
            Ok(()) => ready(Ok(())),
            Err(err) => ready(Err(lead_err(err))),
        }
    }

    pub fn run_routine(&self, id: String) -> Task<JobRecord> {
        if id.is_empty() || id.len() > 64 {
            return fail("invalid routine id");
        }
        match self.routines.run(&id) {
            Ok(job) => ready(Ok(job)),
            Err(err) => ready(Err(lead_err(err))),
        }
    }

    pub fn get_lead_plan(&self) -> Task<Option<ProviderId>> {
        ready(Ok(self.settings.borrow().lead_plan()))
    }

    pub fn set_lead_plan(&self, plan: Option<ProviderId>) -> Task<()> {
        match self.settings.borrow_mut().set_lead_plan(plan) {
            Ok(()) => ready(Ok(())),
            Err(err) => ready(Err(AppError::message(err.to_string()))),
        }
    }

    pub fn get_lead_model(&self, provider: ProviderId) -> Task<Option<String>> {
        ready(Ok(self.settings.borrow().lead_model(provider)))
    }

    pub fn set_lead_model(&self, provider: ProviderId, model: Option<String>) -> Task<()> {
        if let Some(model) = &model
            && !legate_core::is_valid_model(model)
        {
            return fail("invalid model name");
        }
        match self
            .settings
            .borrow_mut()
            .set_lead_model(provider, model.as_deref())
        {
            Ok(()) => ready(Ok(())),
            Err(err) => ready(Err(AppError::message(err.to_string()))),
        }
    }

    pub fn get_lead_access(&self) -> Task<JobAccess> {
        ready(Ok(self.settings.borrow().lead_access()))
    }

    pub fn set_lead_access(&self, access: JobAccess) -> Task<()> {
        match self.settings.borrow_mut().set_lead_access(access) {
            Ok(()) => ready(Ok(())),
            Err(err) => ready(Err(AppError::message(err.to_string()))),
        }
    }

    pub fn send_lead(&self, text: String) -> Task<LeadMessage> {
        if text.is_empty() {
            return fail("empty message");
        }
        let lead = Rc::clone(&self.lead);
        Box::pin(async move { lead.send(&text).await.map_err(lead_err) })
    }

    pub fn run_lead_proposal(
        &self,
        message_id: String,
        jobs: Option<Vec<legate_core::ProposedJob>>,
    ) -> Task<Vec<JobRecord>> {
        if message_id.is_empty() || message_id.len() > 64 {
            return fail("invalid message id");
        }
        match self.lead.run_proposal(&message_id, jobs) {
            Ok(jobs) => ready(Ok(jobs)),
            Err(err) => ready(Err(lead_err(err))),
        }
    }

    pub fn dismiss_lead_proposal(&self, message_id: String) -> Task<()> {
        if message_id.is_empty() || message_id.len() > 64 {
            return fail("invalid message id");
        }
        match self.lead.dismiss_proposal(&message_id) {
            Ok(()) => ready(Ok(())),
            Err(err) => ready(Err(lead_err(err))),
        }
    }

    pub fn list_lead_messages(&self) -> Task<Vec<LeadMessage>> {
        ready(Ok(self.lead.messages()))
    }

    pub fn list_lead_chats(&self) -> Task<Vec<LeadChat>> {
        match self.lead.chats() {
            Ok(chats) => ready(Ok(chats)),
            Err(err) => ready(Err(lead_err(err))),
        }
    }

    pub fn open_lead_chat(&self, id: String) -> Task<Vec<LeadMessage>> {
        if id.is_empty() || id.len() > 64 {
            return fail("invalid chat id");
        }
        if let Err(err) = self.lead.open(&id) {
            return ready(Err(lead_err(err)));
        }
        ready(Ok(self.lead.messages()))
    }

    pub fn reset_lead(&self, direct: Option<ProviderId>) -> Task<()> {
        match self.lead.reset(direct) {
            Ok(()) => ready(Ok(())),
            Err(err) => ready(Err(lead_err(err))),
        }
    }

    pub fn get_lead_mode(&self) -> Task<Option<ProviderId>> {
        ready(Ok(self.lead.mode()))
    }

    pub fn rename_lead_chat(&self, id: String, title: String) -> Task<()> {
        if id.is_empty() || id.len() > 64 {
            return fail("invalid chat id");
        }
        if title.len() > 80 {
            return fail("invalid title");
        }
        match self.lead.rename(&id, &title) {
            Ok(()) => ready(Ok(())),
            Err(err) => ready(Err(lead_err(err))),
        }
    }

    pub fn remove_lead_chat(&self, id: String) -> Task<()> {
        if id.is_empty() || id.len() > 64 {
            return fail("invalid chat id");
        }
        match self.lead.remove(&id) {
            Ok(()) => ready(Ok(())),
            Err(err) => ready(Err(lead_err(err))),
        }
    }

    pub fn open_terminal(
        &self,
        kind: TerminalKind,
        cols: f64,
        rows: f64,
        worktree: Option<WorktreePick>,
        prompt: Option<String>,
    ) -> Task<TerminalInfo> {
        let task = match prompt {
            None => None,
            Some(value) => {
                if value.contains('\0') {
                    return fail("invalid prompt");
                }
                let trimmed = value.trim();
                if trimmed.is_empty() || trimmed.len() > 8000 {
                    return fail("invalid prompt");
                }
                Some(trimmed.to_string())
            }
        };
        let terms = self.terms.clone();
        let trees = Worktrees::new(&self.worktrees_dir);
        let project = self.project.borrow().clone();
        let store = Rc::clone(&self.store);
        let tracker = Rc::clone(&self.setup_tracker);
        let push = self.change_push.clone();
        Box::pin(async move {
            let mut cwd = None;
            let mut change = None;
            if let Some(pick) = worktree {
                match pick {
                    WorktreePick::New => {
                        let about = trees.info(Path::new(&project)).await;
                        if !about.is_repo {
                            return Err(AppError::message(NOT_A_REPO));
                        }
                        let created = trees
                            .create(Path::new(&project), &uuid::Uuid::new_v4().to_string(), None)
                            .await
                            .map_err(git_err)?;
                        let _ = store.set_worktree_name(&WorktreeNameRow {
                            id: created.id.clone(),
                            project: project.clone(),
                            name: String::new(),
                            created_at: system_now_ms(),
                        });
                        let commands = store.get_project_setup(&project).unwrap_or_default();
                        if !commands.is_empty() {
                            match terms.open_shell(ShellOptions {
                                cwd: created.path.to_string_lossy().into_owned(),
                                change: Some(created.id.clone()),
                                title: format!("setup · {}", created.id),
                                commands,
                                cols,
                                rows,
                            }) {
                                Ok(_) => tracker.borrow_mut().begin(&created.id, true),
                                Err(err) => {
                                    tracing::error!("{err}");
                                    tracker.borrow_mut().begin(&created.id, false);
                                }
                            }
                        }
                        cwd = Some(created.path.to_string_lossy().into_owned());
                        change = Some(created.id);
                    }
                    WorktreePick::Existing(id) => {
                        if !is_change_id(&id) {
                            return Err(AppError::message("invalid change id"));
                        }
                        let folder = trees.folder(&id).map_err(git_err)?;
                        if !is_folder(&folder) {
                            return Err(AppError::message(GONE));
                        }
                        cwd = Some(folder.to_string_lossy().into_owned());
                        change = Some(id);
                    }
                }
            }
            let info = match kind {
                TerminalKind::Shell => {
                    let command = task.unwrap_or_else(shell_marker);
                    terms
                        .open_shell(ShellOptions {
                            cwd: cwd.unwrap_or(project),
                            change,
                            title: "shell".to_string(),
                            commands: vec![command],
                            cols,
                            rows,
                        })
                        .map_err(|err| AppError::message(err.to_string()))?
                }
                TerminalKind::Plan(provider) => {
                    let mut opts = OpenOptions {
                        cwd,
                        change,
                        prompt: task,
                        ..OpenOptions::default()
                    };
                    if opts.cwd.is_none() && opts.change.is_none() && opts.prompt.is_none() {
                        opts = OpenOptions::default();
                    }
                    terms
                        .open(provider, cols, rows, opts)
                        .await
                        .map_err(|err| AppError::message(err.to_string()))?
                }
            };
            if info.change.is_some() {
                push.spawn();
            }
            Ok(info)
        })
    }

    pub fn write_terminal(&self, id: String, data: String) -> Task<()> {
        self.terms.claim_terminal(&id, WINDOW_ID);
        self.terms.write(&id, &data);
        ready(Ok(()))
    }

    pub fn claim_terminal(&self, id: String) -> Task<()> {
        if id.is_empty() || id.len() > 64 {
            return ready(Ok(()));
        }
        self.terms.claim_terminal(&id, WINDOW_ID);
        ready(Ok(()))
    }

    pub fn resize_terminal(&self, id: String, cols: f64, rows: f64) -> Task<()> {
        if !cols.is_finite() || !rows.is_finite() {
            return ready(Ok(()));
        }
        let _ = self.terms.resize_claimed(&id, WINDOW_ID, cols, rows);
        ready(Ok(()))
    }

    pub fn close_terminal(&self, id: String) -> Task<()> {
        self.terms.close(&id);
        ready(Ok(()))
    }

    pub fn sleep_terminal(&self, id: String) -> Task<bool> {
        if id.is_empty() || id.len() > 64 {
            return ready(Ok(false));
        }
        ready(Ok(self.terms.sleep(&id)))
    }

    pub fn wake_terminal(&self, id: String, cols: i64, rows: i64) -> Task<Option<TerminalInfo>> {
        if id.is_empty() || id.len() > 64 {
            return fail("invalid terminal id");
        }
        match self.terms.wake(&id, cols as f64, rows as f64) {
            Ok(info) => ready(Ok(info)),
            Err(err) => ready(Err(AppError::message(err.to_string()))),
        }
    }

    pub fn list_terminals(&self) -> Task<Vec<TerminalInfo>> {
        ready(Ok(self.terms.list()))
    }

    pub fn restore_terminals(&self, cols: i64, rows: i64) -> Task<Vec<TerminalInfo>> {
        if !(2..=1000).contains(&cols) {
            return fail("invalid cols");
        }
        if !(2..=1000).contains(&rows) {
            return fail("invalid rows");
        }
        let project = self.project.borrow().clone();
        ready(Ok(self.terms.restore(&project, cols as f64, rows as f64)))
    }

    pub fn terminal_snapshot(&self, id: String) -> Task<String> {
        ready(Ok(self.terms.snapshot(&id)))
    }

    pub fn set_sight(&self, sight: crate::usage::WindowSight) -> Task<()> {
        self.sight.set(sight);
        if sight.focused {
            let usage = self.begin_usage(UsageTrigger::Focus);
            return Box::pin(async move {
                usage.await?;
                Ok(())
            });
        }
        ready(Ok(()))
    }
}

fn is_change_id(id: &str) -> bool {
    id.len() == 8
        && id
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn valid_file_path(path: &str) -> bool {
    !path.is_empty() && path.len() <= 1024 && !path.contains('\0')
}

fn valid_repo(name: &str) -> bool {
    let Some((owner, repo)) = name.split_once('/') else {
        return false;
    };
    let part = |value: &str| {
        !value.is_empty()
            && value.bytes().all(
                |byte| matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-'),
            )
    };
    part(owner) && part(repo) && !name.starts_with('-') && !name.contains("/-")
}

fn shell_marker() -> String {
    if cfg!(windows) {
        "Write-Output legate-shell-ok".to_string()
    } else {
        "echo legate-shell-ok".to_string()
    }
}

fn home_dir() -> String {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string())
}

fn sign_in_line(
    id: &str,
    claude: &legate_providers::ClaudeAdapter,
    cursor: &legate_providers::CursorAdapter,
) -> String {
    if id == "gh" {
        return shell_line("gh", &["auth".to_string(), "login".to_string()]);
    }
    if id == "claude" {
        let spec = claude.interactive(&legate_providers::InteractiveOptions::default());
        let mut args = spec.args;
        args.push("auth".to_string());
        args.push("login".to_string());
        return shell_line(&spec.command, &args);
    }
    let spec = cursor.interactive(&legate_providers::InteractiveOptions::default());
    let mut args = spec.args;
    args.push("login".to_string());
    shell_line(&spec.command, &args)
}

fn shell_line(command: &str, args: &[String]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(quote_arg(command));
    for arg in args {
        parts.push(quote_arg(arg));
    }
    if cfg!(windows) {
        format!("& {}", parts.join(" "))
    } else {
        parts.join(" ")
    }
}

fn quote_arg(value: &str) -> String {
    if value
        .bytes()
        .all(|byte| matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'/' | b'\\' | b':' | b'-'))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "''"))
}

async fn describe(
    orch: &legate_router::Orchestrator,
    store: &legate_store::Store,
    tracker: &std::cell::RefCell<legate_git::SetupTracker>,
    root: &Path,
    path: &str,
    active: bool,
) -> ProjectEntry {
    let chats = store.chat_count(path).unwrap_or(0);
    let running = i64::try_from(orch.active_in(path)).unwrap_or(i64::MAX);
    let names = store
        .worktree_names(path)
        .unwrap_or_default()
        .into_iter()
        .map(|row| ListedName {
            id: row.id,
            name: row.name,
            created_at: row.created_at,
        })
        .collect::<Vec<_>>();
    let working = orch.active_changes_in(path);
    let working_refs = working.iter().map(String::as_str).collect::<Vec<_>>();
    let counts = Worktrees::new(root)
        .project_git(Path::new(path), active, &names, &working_refs, |id| {
            tracker.borrow().get(id)
        })
        .await;
    ProjectEntry {
        path: path.to_string(),
        is_repo: counts.info.is_repo,
        branch: counts.info.branch,
        active,
        changes: counts.changes,
        insertions: counts.insertions,
        deletions: counts.deletions,
        dirty: counts.dirty,
        ahead: counts.ahead,
        behind: counts.behind,
        chats,
        running,
        worktrees: counts.worktrees.map(|items| {
            items
                .into_iter()
                .map(|item| ProjectWorktree {
                    id: item.id,
                    name: item.name,
                    branch: item.branch,
                    files: item.files,
                    insertions: item.insertions,
                    deletions: item.deletions,
                    working: item.working,
                })
                .collect()
        }),
    }
}

fn clone_github() -> legate_git::Github<legate_git::ProcessRun> {
    legate_git::Github::new()
}

fn clone_files() -> legate_git::Files<legate_git::ProcessRun, legate_git::OsFs> {
    legate_git::Files::new()
}

fn clone_history() -> legate_git::History<legate_git::ProcessRun> {
    legate_git::History::new()
}

struct SetupHold {
    claude: std::sync::Arc<legate_providers::ClaudeAdapter>,
    cursor: std::sync::Arc<legate_providers::CursorAdapter>,
}

impl SetupHold {
    async fn run(self) -> Vec<legate_lead::Requirement> {
        let mut env = legate_git::EnvMap::new();
        if let Ok(path) = std::env::var("PATH") {
            env.set_path(path);
        }
        legate_git::refresh_path(
            &legate_git::ProcessRun::command(),
            &mut env,
            legate_pty::host_platform(),
        )
        .await;
        if !env.path().is_empty() {
            // The setup probe's PATH is the one the person's shell would see.
            // `set_var` is process-global; only `setup_status` calls this.
            unsafe { std::env::set_var("PATH", env.path()) };
        }
        let setup = legate_lead::Setup::new(
            std::sync::Arc::new(legate_lead::SystemCommand),
            std::sync::Arc::new(legate_lead::ProviderSetup(self.claude)),
            std::sync::Arc::new(legate_lead::ProviderSetup(self.cursor)),
            legate_lead::Platform::current(),
        );
        setup.status().await
    }
}

async fn fire_one(
    triggers: &legate_lead::TriggerService,
    goals: &Rc<legate_lead::Goals>,
    mut trigger: Trigger,
    firing: Option<Firing>,
) {
    let text = firing
        .as_ref()
        .map(|item| item.text.clone())
        .unwrap_or_else(|| trigger.text.clone());
    trigger.last_fired = Some(system_now_ms());
    trigger.last_error = None;
    let _ = triggers.save(&trigger);
    let source = GoalSource {
        id: trigger.id.clone(),
        label: if is_scheduled(trigger.kind) {
            match trigger.kind {
                legate_lead::TriggerKind::Daily => {
                    format!("Every day at {}", trigger.at.as_deref().unwrap_or("?"))
                }
                _ => format!("Every {} hours", trigger.hours.unwrap_or(24)),
            }
        } else {
            "GitHub".to_string()
        },
    };
    if let Err(err) = goals
        .create(NewGoal {
            project: trigger.project.clone(),
            text,
            check: trigger.check.clone(),
            max_rounds: Some(trigger.max_rounds as f64),
            quick: None,
            source: Some(source),
        })
        .await
    {
        trigger.last_error = Some(err.to_string());
        let _ = triggers.save(&trigger);
    }
}

async fn force_github(
    store: &Rc<legate_store::Store>,
    triggers: &Rc<legate_lead::TriggerService>,
    goals: &Rc<legate_lead::Goals>,
    project: &str,
) {
    let Ok(raw) = store.triggers() else {
        return;
    };
    let list = raw
        .into_iter()
        .filter_map(|item| crate::session::stored_trigger(&item))
        .filter(|trigger| {
            trigger.project == project && trigger.enabled && !is_scheduled(trigger.kind)
        })
        .collect::<Vec<_>>();
    if list.is_empty() {
        return;
    }
    let kinds = list.iter().map(|trigger| trigger.kind).collect::<Vec<_>>();
    let facts = match github_facts(&legate_lead::SystemCommand, project, &kinds).await {
        Ok(facts) => facts,
        Err(err) => {
            for mut trigger in list {
                trigger.last_error = Some(err.clone());
                let _ = triggers.save(&trigger);
            }
            return;
        }
    };
    for mut trigger in list {
        let (fire, seen) = fresh_firings(&trigger, &facts);
        trigger.seen = seen;
        trigger.primed = true;
        trigger.last_error = None;
        let _ = triggers.save(&trigger);
        if let Some(first) = fire.into_iter().next() {
            fire_one(triggers, goals, trigger, Some(first)).await;
        }
    }
}
