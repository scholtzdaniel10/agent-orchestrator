//! One OS thread owns the `LocalSet`: the orchestrator, the lead, goals, and triggers.
//!
//! `AppHandle` sends a [`Command`] here. Each command is `spawn_local`'d so a lead
//! turn can wait on the bridge while the bridge's reply is handled on this same thread.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::{Arc, RwLock};

use legate_bridge::{ToolConfig, orchestrator_tools, start_bridge};
use legate_core::{
    AgentEvent, AppInfo, ChangeSet, JobAccess, JobRecord, JobStatus, PROVIDERS, PlanStatus,
    PlanWindow, ProviderId, RunOptions, SkillFolderReport, SkillsState, TerminalInfo,
};
use legate_git::{ListedName, SetupTracker, Worktrees, merge_listed_changes};
use legate_lead::{
    AgentPlugins, BridgeSource, Goal, GoalChecks, GoalDeps, GoalPlanner, GoalStarter, GoalStore,
    Goals, Lead, LeadOpts, NewGoal, PlanWorker, RoutineStore, Routines, SystemCommand, Trigger,
    TriggerKind, TriggerOpts, TriggerService, TriggerStore, adapt_provider, build_user_plugin,
    bundled_plugins, bundled_skills, run_goal_check, with_plugins,
};
use legate_providers::{
    AgentPlugins as ProviderPlugins, ClaudeAdapter, CursorAdapter, InteractiveCommand,
    InteractiveOptions, LocalAdapter, ProviderAdapter,
};
use legate_pty::{Launch, LaunchOpts, PtyHost, PtyHostOptions, chat_fn};
use legate_router::{
    CwdSource, Orchestrator, OrchestratorOpts, Subscription, SystemClock, WatchGuard,
    system_now_ms, tested_version,
};
use legate_store::{NewWindow, Store};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::debounce::{ChangeRefresh, LocalTimers};
use crate::error::AppError;
use crate::event::AppEvent;
use crate::options::AppOptions;
use crate::paths::{
    app_data_dir, bring_old_data, database_path, default_worktrees_dir, is_folder, lead_dir,
    settings_path,
};
use crate::plugins::install_bundled;
use crate::usage::{UsageTrigger, WindowSight, usage_check_due};
use crate::wire::{AdapterRunner, AppBackend, GitGoals, GitSpaces, OrchJobs, goal_submit};

pub(crate) type LocalTask = Pin<Box<dyn Future<Output = ()> + 'static>>;
pub(crate) type Call = Box<dyn FnOnce(&World) -> LocalTask + Send>;

pub(crate) enum Command {
    Run(Call),
    Shutdown,
}

pub(crate) type Task<T> = Pin<Box<dyn Future<Output = Result<T, AppError>> + 'static>>;

pub(crate) fn ready<T: Send + 'static>(value: Result<T, AppError>) -> Task<T> {
    Box::pin(async move { value })
}

pub(crate) fn fail<T: Send + 'static>(message: impl Into<String>) -> Task<T> {
    ready(Err(AppError::message(message)))
}

pub(crate) struct Flight {
    waiters: Vec<oneshot::Sender<crate::options::UsageOutcome>>,
}

#[derive(Clone)]
pub(crate) struct ChangePush {
    events: broadcast::Sender<AppEvent>,
    store: Rc<Store>,
    root: PathBuf,
    project: Rc<RefCell<String>>,
    tracker: Rc<RefCell<SetupTracker>>,
}

impl ChangePush {
    pub fn spawn(&self) {
        let this = self.clone();
        tokio::task::spawn_local(async move {
            let list = this.collect().await;
            let _ = this.events.send(AppEvent::Changes(list));
        });
    }

    pub(crate) async fn collect_pub(&self) -> Vec<ChangeSet> {
        self.collect().await
    }

    async fn collect(&self) -> Vec<ChangeSet> {
        let project = self.project.borrow().clone();
        let trees = Worktrees::new(&self.root);
        let listed = trees.list(Path::new(&project)).await;
        let ids = trees.ids(Path::new(&project)).await;
        let names = self
            .store
            .worktree_names(&project)
            .unwrap_or_default()
            .into_iter()
            .map(|row| ListedName {
                id: row.id,
                name: row.name,
                created_at: row.created_at,
            })
            .collect::<Vec<_>>();
        let tracker = Rc::clone(&self.tracker);
        let trees_for_folder = Worktrees::new(&self.root);
        merge_listed_changes(
            &listed,
            &ids,
            &names,
            move |id| {
                trees_for_folder
                    .folder(id)
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_default()
            },
            move |id| tracker.borrow().get(id),
        )
    }
}

struct LivePlanner {
    orch: Rc<Orchestrator>,
    settings: Rc<RefCell<legate_core::Settings>>,
    adapter: Arc<dyn Fn(ProviderId) -> Arc<dyn ProviderAdapter> + Send + Sync>,
    models: Arc<RwLock<HashMap<ProviderId, Option<String>>>>,
}

/// `parse_stored` is public inside legate-lead but not re-exported.
pub(crate) fn stored_trigger(raw: &str) -> Option<Trigger> {
    let trigger: Trigger = serde_json::from_str(raw).ok()?;
    TriggerKind::parse(trigger.kind.as_str())?;
    Some(trigger)
}

#[async_trait::async_trait(?Send)]
impl GoalPlanner for LivePlanner {
    async fn plan(&self, _goal: &Goal, folder: &str, prompt: &str) -> Result<String, String> {
        let workers: Vec<PlanWorker> = self
            .orch
            .workers()
            .into_iter()
            .map(|worker| PlanWorker {
                id: worker.id,
                available: worker.available,
            })
            .collect();
        let preferred = self.settings.borrow().lead_plan();
        let provider = workers
            .iter()
            .filter(|worker| worker.available)
            .find(|worker| Some(worker.id) == preferred)
            .or_else(|| workers.iter().find(|worker| worker.available))
            .map(|worker| worker.id)
            .ok_or_else(|| "No plan is signed in and ready.".to_string())?;
        let model = self
            .models
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .get(&provider)
            .cloned()
            .flatten();
        let adapter = (self.adapter)(provider);
        let mut handle = adapter.run(
            &uuid::Uuid::new_v4().to_string(),
            prompt,
            Path::new(folder),
            &RunOptions {
                access: Some(JobAccess::Read),
                model,
                ..RunOptions::default()
            },
        );
        let mut result = String::new();
        let mut said = String::new();
        while let Some(event) = handle.events.recv().await {
            match event {
                AgentEvent::Result { text, .. } => result = text,
                AgentEvent::Text { text } => said.push_str(&text),
                _ => {}
            }
        }
        let exit = handle.exit().await;
        let text = if result.trim().is_empty() {
            said
        } else {
            result
        };
        if text.trim().is_empty() {
            let line = exit
                .stderr
                .lines()
                .next_back()
                .filter(|line| !line.trim().is_empty())
                .unwrap_or("no answer");
            return Err(line.to_string());
        }
        Ok(text)
    }
}

struct LiveChecks;

#[async_trait::async_trait(?Send)]
impl GoalChecks for LiveChecks {
    async fn run(&self, command: &str, folder: &str) -> Result<(Option<i32>, String), String> {
        let output = run_goal_check(command, folder).await;
        Ok((output.code, output.output))
    }
}

struct LiveStarter {
    goals: Rc<Goals>,
}

#[async_trait::async_trait(?Send)]
impl GoalStarter for LiveStarter {
    async fn start(&self, input: NewGoal) -> Result<(), legate_lead::LeadError> {
        self.goals.create(input).await?;
        Ok(())
    }

    fn active(&self, project: &str, source_id: &str) -> bool {
        self.goals
            .list(project)
            .map(|goals| {
                goals.iter().any(|goal| {
                    goal.source
                        .as_ref()
                        .is_some_and(|source| source.id == source_id)
                        && !matches!(
                            goal.status,
                            legate_lead::GoalStatus::Done
                                | legate_lead::GoalStatus::Stopped
                                | legate_lead::GoalStatus::Failed
                        )
                })
            })
            .unwrap_or(false)
    }
}

struct EchoTriggers {
    store: Rc<Store>,
    events: broadcast::Sender<AppEvent>,
    project: Rc<RefCell<String>>,
}

impl EchoTriggers {
    fn publish(&self) {
        let project = self.project.borrow().clone();
        let Ok(raw) = self.store.triggers() else {
            return;
        };
        let mut list = Vec::new();
        for item in raw {
            if let Some(trigger) = stored_trigger(&item)
                && trigger.project == project
            {
                list.push(trigger);
            }
        }
        let _ = self.events.send(AppEvent::Triggers(list));
    }
}

impl TriggerStore for EchoTriggers {
    fn load(&self) -> Result<Vec<String>, legate_lead::LeadError> {
        Ok(self.store.triggers()?)
    }

    fn save(&self, trigger: &Trigger) -> Result<(), legate_lead::LeadError> {
        let data = serde_json::to_string(trigger)
            .map_err(|err| legate_lead::LeadError::message(err.to_string()))?;
        self.store
            .save_trigger(&trigger.id, &trigger.project, &data)?;
        self.publish();
        Ok(())
    }
}

pub(crate) struct World {
    pub settings: Rc<RefCell<legate_core::Settings>>,
    pub project: Rc<RefCell<String>>,
    pub project_mirror: Arc<RwLock<String>>,
    pub models: Arc<RwLock<HashMap<ProviderId, Option<String>>>>,
    pub store: Rc<Store>,
    pub orch: Rc<Orchestrator>,
    pub lead: Rc<Lead>,
    pub goals: Rc<Goals>,
    pub triggers: Rc<TriggerService>,
    pub routines: Rc<Routines>,
    pub setup_tracker: Rc<RefCell<SetupTracker>>,
    pub terms: PtyHost,
    pub claude: Arc<ClaudeAdapter>,
    pub cursor: Arc<CursorAdapter>,
    pub grok: Arc<CursorAdapter>,
    pub local: Arc<LocalAdapter>,
    pub events: broadcast::Sender<AppEvent>,
    pub changes: Rc<ChangeRefresh>,
    pub change_push: ChangePush,
    pub rules_max: i64,
    pub data_dir: PathBuf,
    pub worktrees_dir: PathBuf,
    pub sight: Rc<Cell<WindowSight>>,
    pub usage_last: Rc<Cell<Option<i64>>>,
    pub usage_flight: Rc<RefCell<Option<Flight>>>,
    pub sign_in: Rc<RefCell<std::collections::HashSet<String>>>,
    pub skill_reports: Rc<RefCell<Vec<SkillFolderReport>>>,
    pub bundled: AgentPlugins,
    pub bridge: legate_bridge::Bridge,
    pub missed: Rc<RefCell<HashMap<String, JobRecord>>>,
    /// Keeps the store subscription alive for the life of the process.
    pub _watch: WatchGuard,
    pub _job_sub: Subscription,
    pub _lead_unsub: Box<dyn FnOnce()>,
    pub _data_sub: legate_pty::Subscription,
    pub _update_sub: legate_pty::Subscription,
    pub _parked: broadcast::Receiver<AppEvent>,
}

pub(crate) fn spawn_service(
    options: AppOptions,
    events: broadcast::Sender<AppEvent>,
    tx: mpsc::UnboundedSender<Command>,
    rx: mpsc::UnboundedReceiver<Command>,
    ready: oneshot::Sender<Result<(), AppError>>,
) {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            let _ = ready.send(Err(AppError::message(err.to_string())));
            return;
        }
    };
    runtime.block_on(async move {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async move {
                match boot(options, events, tx).await {
                    Ok(world) => {
                        let _ = ready.send(Ok(()));
                        run_loop(world, rx).await;
                    }
                    Err(err) => {
                        let _ = ready.send(Err(err));
                    }
                }
            })
            .await;
    });
}

async fn run_loop(world: World, mut rx: mpsc::UnboundedReceiver<Command>) {
    let world = Rc::new(world);
    loop {
        let Some(command) = rx.recv().await else {
            break;
        };
        match command {
            Command::Shutdown => break,
            Command::Run(call) => {
                let world = Rc::clone(&world);
                tokio::task::spawn_local(async move {
                    let task = call(&world);
                    task.await;
                });
            }
        }
    }
    world.triggers.stop();
    world.changes.clear();
    world.terms.close_all();
    let _ = world.bridge.close().await;
}

async fn boot(
    options: AppOptions,
    events: broadcast::Sender<AppEvent>,
    tx: mpsc::UnboundedSender<Command>,
) -> Result<World, AppError> {
    let data_dir = app_data_dir(options.data_dir.as_deref());
    bring_old_data(&data_dir, options.data_dir.is_none())?;
    std::fs::create_dir_all(&data_dir).map_err(|err| AppError::message(err.to_string()))?;
    std::fs::create_dir_all(lead_dir(&data_dir))
        .map_err(|err| AppError::message(err.to_string()))?;
    let worktrees_dir = default_worktrees_dir(options.worktrees_dir.as_deref());
    std::fs::create_dir_all(&worktrees_dir).map_err(|err| AppError::message(err.to_string()))?;

    let settings = Rc::new(RefCell::new(legate_core::Settings::open(settings_path(
        &data_dir,
    ))));
    if let Some(project) = &options.project
        && is_folder(project)
    {
        settings
            .borrow_mut()
            .set_project(&project.to_string_lossy())
            .map_err(|err| AppError::message(err.to_string()))?;
    }
    let project_path = resolve_project(&settings.borrow());
    let project = Rc::new(RefCell::new(project_path.clone()));
    let project_mirror = Arc::new(RwLock::new(project_path));
    let models = Arc::new(RwLock::new(HashMap::new()));
    sync_models(&settings.borrow(), &models);

    install_bundled(&data_dir);
    let bundled = bundled_plugins(&data_dir);
    let skill_reports = Rc::new(RefCell::new(Vec::new()));
    let claude = options
        .claude
        .unwrap_or_else(|| Arc::new(ClaudeAdapter::new(None)));
    let cursor = options
        .cursor
        .unwrap_or_else(|| Arc::new(CursorAdapter::new(None)));
    let grok = options.grok.unwrap_or_else(|| {
        let models = Arc::clone(&models);
        Arc::new(CursorAdapter::grok(
            None,
            Some(Arc::new(move || {
                models
                    .read()
                    .unwrap_or_else(|err| err.into_inner())
                    .get(&ProviderId::Grok)
                    .cloned()
                    .flatten()
            })),
        ))
    });
    let local = options
        .local
        .unwrap_or_else(|| Arc::new(LocalAdapter::from_prefs(&settings.borrow().prefs())));
    apply_skills(
        &settings.borrow(),
        &data_dir,
        &bundled,
        &skill_reports,
        &claude,
        &cursor,
        &grok,
    );

    let store = Rc::new(
        Store::open(database_path(&data_dir)).map_err(|err| AppError::message(err.to_string()))?,
    );
    let pty_store = Arc::new(std::sync::Mutex::new(
        Store::open(database_path(&data_dir)).map_err(|err| AppError::message(err.to_string()))?,
    ));

    let rules_path = std::env::var("ORCH_RULES")
        .ok()
        .filter(|value| !value.is_empty());
    let mut rules = legate_core::load_rules(rules_path.as_deref().map(Path::new))
        .map_err(|err| AppError::message(err.to_string()))?;
    let rules_max = rules.max_parallel;
    if let Some(max) = settings.borrow().prefs().max_parallel {
        rules.max_parallel = max;
    }
    let lead_rules = rules.clone();

    let claude_run: Arc<dyn ProviderAdapter> = claude.clone();
    let cursor_run: Arc<dyn ProviderAdapter> = cursor.clone();
    let grok_run: Arc<dyn ProviderAdapter> = grok.clone();
    let local_run: Arc<dyn ProviderAdapter> = local.clone();
    let runners: Vec<Rc<dyn legate_router::Runner>> = vec![
        Rc::new(AdapterRunner::new(claude_run)),
        Rc::new(AdapterRunner::new(cursor_run)),
        Rc::new(AdapterRunner::new(grok_run)),
        Rc::new(AdapterRunner::new(local_run)),
    ];
    let cwd_project = Rc::clone(&project);
    let model_settings = Rc::clone(&settings);
    let setup_tracker = Rc::new(RefCell::new(SetupTracker::new()));
    let change_push = ChangePush {
        events: events.clone(),
        store: Rc::clone(&store),
        root: worktrees_dir.clone(),
        project: Rc::clone(&project),
        tracker: Rc::clone(&setup_tracker),
    };
    let on_changes = change_push.clone();
    let orch = Orchestrator::new(OrchestratorOpts {
        runners,
        store: Rc::clone(&store),
        rules,
        cwd: CwdSource::Dynamic(Rc::new(move || cwd_project.borrow().clone())),
        clock: Some(Rc::new(SystemClock::new())),
        model_for: Some(Rc::new(move |id| model_settings.borrow().model(id))),
        workspaces: Some(Rc::new(GitSpaces::new(worktrees_dir.clone()))),
        on_changes: Some(Rc::new(move || on_changes.spawn())),
        owner_id: None,
    })
    .map_err(|err| AppError::message(err.to_string()))?;
    orch.init().await;

    let jobs: Rc<dyn legate_lead::JobSink> = Rc::new(OrchJobs::new(Rc::clone(&orch)));
    let routine_store: Rc<dyn RoutineStore> = Rc::clone(&store) as Rc<dyn RoutineStore>;
    let routine_project = Rc::clone(&project);
    let routines = Rc::new(Routines::new(
        routine_store,
        Rc::clone(&jobs),
        Rc::new(move || routine_project.borrow().clone()),
        None,
    ));

    let backend = Arc::new(AppBackend::new(tx.clone()));
    let bridge = start_bridge(orchestrator_tools(backend, ToolConfig { propose: true }))
        .await
        .map_err(|err| AppError::message(err.to_string()))?;
    let bridge_info: Arc<dyn BridgeSource> = Arc::new(bridge.info().clone());
    let env_prefer = std::env::var("ORCH_LEAD")
        .ok()
        .as_deref()
        .and_then(ProviderId::parse);
    let prefer_settings = Rc::clone(&settings);
    let lead_model_settings = Rc::clone(&settings);
    let lead_project = Rc::clone(&project);
    let lead = Lead::new(LeadOpts {
        adapters: vec![
            adapt_provider(claude.clone()),
            adapt_provider(cursor.clone()),
            adapt_provider(grok.clone()),
            adapt_provider(local.clone()),
        ],
        store: Rc::clone(&store),
        rules: lead_rules,
        bridge: bridge_info,
        dir: lead_dir(&data_dir).to_string_lossy().into_owned(),
        project: Rc::new(move || lead_project.borrow().clone()),
        prefer: Some(Rc::new(move || {
            prefer_settings.borrow().lead_plan().or(env_prefer)
        })),
        now: None,
        model_for: Some(Rc::new(move |id| {
            let settings = lead_model_settings.borrow();
            settings.lead_model(id).or_else(|| settings.model(id))
        })),
    });
    lead.init()
        .await
        .map_err(|err| AppError::message(err.to_string()))?;
    lead.attach_jobs(Rc::clone(&jobs));

    let claude_plan: Arc<dyn ProviderAdapter> = claude.clone();
    let cursor_plan: Arc<dyn ProviderAdapter> = cursor.clone();
    let grok_plan: Arc<dyn ProviderAdapter> = grok.clone();
    let local_plan: Arc<dyn ProviderAdapter> = local.clone();
    let adapter = Arc::new(move |id: ProviderId| -> Arc<dyn ProviderAdapter> {
        match id {
            ProviderId::Claude => Arc::clone(&claude_plan),
            ProviderId::Grok => Arc::clone(&grok_plan),
            ProviderId::Local => Arc::clone(&local_plan),
            ProviderId::Cursor => Arc::clone(&cursor_plan),
        }
    });
    let goal_events = events.clone();
    let goal_project = Rc::clone(&project);
    let goal_push = change_push.clone();
    let goal_store: Rc<dyn GoalStore> = Rc::clone(&store) as Rc<dyn GoalStore>;
    let name_store = Rc::clone(&store);
    let ceiling_settings = Rc::clone(&settings);
    let goals = Goals::new(GoalDeps {
        store: goal_store,
        worktrees: Rc::new(GitGoals::new(worktrees_dir.clone())),
        submit: goal_submit(&orch),
        job: {
            let orch = Rc::clone(&orch);
            Rc::new(move |id| orch.get(id))
        },
        cancel: {
            let orch = Rc::clone(&orch);
            Rc::new(move |id| {
                let _ = orch.cancel(id);
            })
        },
        planner: Rc::new(LivePlanner {
            orch: Rc::clone(&orch),
            settings: Rc::clone(&settings),
            adapter,
            models: Arc::clone(&models),
        }),
        checks: Rc::new(LiveChecks),
        ceiling: Rc::new(move || ceiling_settings.borrow().lead_access()),
        name_worktree: Some(Rc::new(move |id, project, name| {
            let _ = name_store.set_worktree_name(&legate_store::WorktreeNameRow {
                id: id.to_string(),
                project: project.to_string(),
                name: name.to_string(),
                created_at: system_now_ms(),
            });
        })),
        on_update: Some(Rc::new(move |goal| {
            if goal.project == *goal_project.borrow() {
                let _ = goal_events.send(AppEvent::Goal(goal.clone()));
            }
            goal_push.spawn();
        })),
        now: None,
    });

    let triggers = TriggerService::new(TriggerOpts {
        store: Rc::new(EchoTriggers {
            store: Rc::clone(&store),
            events: events.clone(),
            project: Rc::clone(&project),
        }),
        goals: Rc::new(LiveStarter {
            goals: Rc::clone(&goals),
        }),
        commands: Arc::new(SystemCommand),
        now: Rc::new(system_now_ms),
        initial_delay: None,
        retry_busy: None,
    });
    triggers.start();

    let missed = Rc::new(RefCell::new(HashMap::new()));
    let last_status = Rc::new(RefCell::new(HashMap::new()));

    let job_events = events.clone();
    let job_orch = Rc::clone(&orch);
    let job_project = Rc::clone(&project);
    let job_goals = Rc::clone(&goals);
    let job_routines = Rc::clone(&routines);
    let job_push = change_push.clone();
    let job_settings = Rc::clone(&settings);
    let job_store = Rc::clone(&store);
    let job_last = Rc::clone(&last_status);
    let job_missed = Rc::clone(&missed);
    let job_sub = orch.on_update(Rc::new(move |job| {
        let _ = job_goals.on_job(Some(&job), None);
        let here = job_orch
            .project_of(&job.id)
            .map(|owner| owner == *job_project.borrow())
            .unwrap_or(true);
        if !here
            && job.lead_message.is_some()
            && matches!(job.status, JobStatus::Done | JobStatus::Failed)
            && let Some(message) = job.lead_message.clone()
        {
            job_missed.borrow_mut().insert(message, job.clone());
        }
        if job_last.borrow().get(&job.id) != Some(&job.status) {
            job_last.borrow_mut().insert(job.id.clone(), job.status);
            job_push.spawn();
        }
        if here {
            let _ = job_events.send(AppEvent::Job(job.clone()));
        }
        if job.status == JobStatus::Done && here {
            job_routines.on_job_finished(&job);
        }
        let plans = plan_list(&job_orch, &job_settings.borrow(), &job_store);
        let _ = job_events.send(AppEvent::Plans(plans));
        let again = job_events.clone();
        let orch = Rc::clone(&job_orch);
        let settings = Rc::clone(&job_settings);
        let store = Rc::clone(&job_store);
        tokio::task::spawn_local(async move {
            tokio::task::yield_now().await;
            let plans = plan_list(&orch, &settings.borrow(), &store);
            let _ = again.send(AppEvent::Plans(plans));
        });
    }));

    let lead_events = events.clone();
    let lead_orch = Rc::clone(&orch);
    let lead_settings = Rc::clone(&settings);
    let lead_store = Rc::clone(&store);
    let lead_unsub = lead.on_update(Rc::new(move |message, chat_id| {
        let streaming = message.status == legate_core::LeadStatus::Streaming;
        let _ = lead_events.send(AppEvent::Lead { message, chat_id });
        if !streaming {
            let plans = plan_list(&lead_orch, &lead_settings.borrow(), &lead_store);
            let _ = lead_events.send(AppEvent::Plans(plans));
        }
    }));
    reload_project(&lead, &orch, &goals, &project.borrow(), &missed);

    let claude_launch = Arc::clone(&claude);
    let cursor_launch = Arc::clone(&cursor);
    let grok_launch = Arc::clone(&grok);
    let launch = Arc::new(move |provider, opts: &LaunchOpts| {
        let interactive = InteractiveOptions {
            model: opts.model.clone(),
            session_id: opts.session_id.clone(),
            resume: opts.resume.clone(),
            prompt: opts.prompt.clone(),
        };
        let spec = match provider {
            ProviderId::Claude => claude_launch.interactive(&interactive),
            ProviderId::Grok => grok_launch.interactive(&interactive),
            ProviderId::Local => InteractiveCommand {
                command: "legate-local-has-no-cli".to_string(),
                args: Vec::new(),
                env: None,
            },
            ProviderId::Cursor => cursor_launch.interactive(&interactive),
        };
        Launch {
            command: spec.command,
            args: spec.args,
            env: spec.env,
        }
    });
    let cwd_mirror = Arc::clone(&project_mirror);
    let model_mirror = Arc::clone(&models);
    let chat_cursor = Arc::clone(&cursor);
    let terms = PtyHost::new(PtyHostOptions {
        launch,
        cwd: Arc::new(move || {
            cwd_mirror
                .read()
                .unwrap_or_else(|err| err.into_inner())
                .clone()
        }),
        model_for: Some(Arc::new(move |id| {
            model_mirror
                .read()
                .unwrap_or_else(|err| err.into_inner())
                .get(&id)
                .cloned()
                .flatten()
        })),
        store: Some(pty_store),
        spawn: None,
        kill_tree: None,
        now: Some(Arc::new(system_now_ms)),
        scheduler: None,
        create_chat: Some(chat_fn(move || {
            let cursor = Arc::clone(&chat_cursor);
            async move {
                let spec = cursor.interactive(&InteractiveOptions::default());
                let mut command = tokio::process::Command::new(&spec.command);
                command.args(&spec.args).arg("create-chat");
                if let Some(env) = &spec.env {
                    for (key, value) in env {
                        command.env(key, value);
                    }
                }
                let output =
                    tokio::time::timeout(std::time::Duration::from_secs(10), command.output())
                        .await
                        .map_err(|_| legate_pty::PtyError::TimedOut)?
                        .map_err(|err| legate_pty::PtyError::Message(err.to_string()))?;
                Ok(String::from_utf8_lossy(&output.stdout).into_owned())
            }
        })),
        max_terminals: None,
        exists: None,
        platform: Some(legate_pty::host_platform()),
    });

    let sign_in = Rc::new(RefCell::new(std::collections::HashSet::new()));
    let data_tx = tx.clone();
    let data_events = events.clone();
    let data_sub = terms.on_data(move |id, data| {
        let _ = data_events.send(AppEvent::TerminalData {
            id: id.to_string(),
            data: data.to_string(),
        });
        let id = id.to_string();
        let _ = data_tx.send(Command::Run(Box::new(move |world| {
            if let Some(change) = world
                .terms
                .list()
                .into_iter()
                .find(|info| info.id == id)
                .and_then(|info| info.change)
            {
                world.changes.note(&change);
            }
            Box::pin(async {})
        })));
    });
    let update_tx = tx.clone();
    let update_events = events.clone();
    let update_sub = terms.on_update(move |info, removed| {
        let _ = update_events.send(AppEvent::TerminalUpdate {
            info: info.clone(),
            removed,
        });
        let info = info.clone();
        let _ = update_tx.send(Command::Run(Box::new(move |world| {
            let watch = TerminalWatch::from_world(world);
            Box::pin(async move {
                watch.note(&info, removed).await;
            })
        })));
    });

    let timers: Rc<RefCell<dyn crate::debounce::Timers>> =
        Rc::new(RefCell::new(LocalTimers::new()));
    let refresh_push = change_push.clone();
    let refresh_terms = terms.clone();
    let changes = ChangeRefresh::start(
        timers,
        move || refresh_push.spawn(),
        move || {
            refresh_terms.list().iter().any(|info| {
                info.change.is_some() && info.status == legate_core::TerminalStatus::Running
            })
        },
    );

    let sight = Rc::new(Cell::new(WindowSight::none()));
    let usage_last = Rc::new(Cell::new(None));
    let usage_flight = Rc::new(RefCell::new(None));
    let watch = orch.watch_store();
    let parked = events.subscribe();

    let usage_orch = Rc::clone(&orch);
    let usage_settings = Rc::clone(&settings);
    let usage_store = Rc::clone(&store);
    let usage_claude = Arc::clone(&claude);
    let usage_events = events.clone();
    let usage_sight = Rc::clone(&sight);
    let usage_stamp = Rc::clone(&usage_last);
    let usage_flight_timer = Rc::clone(&usage_flight);
    let usage_data = data_dir.clone();
    tokio::task::spawn_local(async move {
        tokio::time::sleep(std::time::Duration::from_secs(15)).await;
        let ctx = UsageBits {
            orch: usage_orch,
            settings: usage_settings,
            store: usage_store,
            claude: usage_claude,
            events: usage_events,
            sight: usage_sight,
            usage_last: usage_stamp,
            flight: usage_flight_timer,
            data_dir: usage_data,
        };
        ctx.start(UsageTrigger::Focus);
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(
                crate::usage::USAGE_TICK_MS as u64,
            ))
            .await;
            ctx.start(UsageTrigger::Tick);
        }
    });

    Ok(World {
        settings,
        project,
        project_mirror,
        models,
        store,
        orch,
        lead,
        goals,
        triggers,
        routines,
        setup_tracker,
        terms,
        claude,
        cursor,
        grok,
        local,
        events,
        changes,
        change_push,
        rules_max,
        data_dir,
        worktrees_dir,
        sight,
        usage_last,
        usage_flight,
        sign_in,
        skill_reports,
        bundled,
        bridge,
        missed,
        _watch: watch,
        _job_sub: job_sub,
        _lead_unsub: lead_unsub,
        _data_sub: data_sub,
        _update_sub: update_sub,
        _parked: parked,
    })
}

fn resolve_project(settings: &legate_core::Settings) -> String {
    if let Some(path) = settings.project()
        && is_folder(Path::new(&path))
    {
        return path;
    }
    if let Ok(path) = std::env::var("ORCH_CWD")
        && !path.is_empty()
        && is_folder(Path::new(&path))
    {
        return path;
    }
    std::env::current_dir()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| ".".to_string())
}

pub(crate) fn sync_models(
    settings: &legate_core::Settings,
    models: &RwLock<HashMap<ProviderId, Option<String>>>,
) {
    let mut guard = models.write().unwrap_or_else(|err| err.into_inner());
    for id in PROVIDERS {
        guard.insert(id, settings.model(id));
    }
}

fn apply_skills(
    settings: &legate_core::Settings,
    data_dir: &Path,
    bundled: &AgentPlugins,
    reports: &RefCell<Vec<SkillFolderReport>>,
    claude: &ClaudeAdapter,
    cursor: &CursorAdapter,
    grok: &CursorAdapter,
) {
    let folders = settings
        .skill_folders()
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let plugins = match build_user_plugin(&folders, data_dir) {
        Ok(built) => {
            *reports.borrow_mut() = built.reports;
            match built.plugin {
                Some(plugin) => with_plugins(bundled, &[plugin]),
                None => bundled.clone(),
            }
        }
        Err(err) => {
            tracing::error!("{err}");
            *reports.borrow_mut() = Vec::new();
            bundled.clone()
        }
    };
    let provider = to_provider_plugins(&plugins);
    claude.set_plugins(provider.clone());
    cursor.set_plugins(provider.clone());
    grok.set_plugins(provider);
}

fn to_provider_plugins(plugins: &AgentPlugins) -> ProviderPlugins {
    ProviderPlugins {
        lead: plugins.lead.clone(),
        worker: plugins.worker.clone(),
    }
}

impl World {
    pub(crate) fn reapply_skills(&self) {
        apply_skills(
            &self.settings.borrow(),
            &self.data_dir,
            &self.bundled,
            &self.skill_reports,
            &self.claude,
            &self.cursor,
            &self.grok,
        );
    }
}

pub(crate) fn skills_state(world: &World) -> SkillsState {
    SkillsState {
        built_in: bundled_skills(&world.bundled),
        folders: world.skill_reports.borrow().clone(),
    }
}

pub(crate) fn reload_project(
    lead: &Lead,
    orch: &Orchestrator,
    goals: &Goals,
    project: &str,
    missed: &RefCell<HashMap<String, JobRecord>>,
) {
    let _ = lead.open_latest();
    orch.restore(project);
    lead.note_restored_jobs(&orch.list());
    let here = orch
        .list()
        .into_iter()
        .map(|job| job.id)
        .collect::<std::collections::HashSet<_>>();
    let pending = missed
        .borrow()
        .iter()
        .filter(|(_, job)| here.contains(&job.id))
        .map(|(message, job)| (message.clone(), job.clone()))
        .collect::<Vec<_>>();
    for (message, job) in pending {
        missed.borrow_mut().remove(&message);
        lead.catch_up(&job);
    }
    let _ = goals.resume(project);
}

pub(crate) fn plan_list(
    orch: &Orchestrator,
    settings: &legate_core::Settings,
    store: &Store,
) -> Vec<PlanStatus> {
    orch.workers()
        .into_iter()
        .map(|worker| PlanStatus {
            id: worker.id,
            available: worker.available,
            problem: worker.problem,
            used: (1.0 - worker.headroom).clamp(0.0, 1.0),
            resting_until: worker.resting_until,
            resets_at: worker.resets_at,
            at_risk: worker.at_risk,
            busy: worker.busy,
            running: i64::try_from(worker.running).unwrap_or(i64::MAX),
            queued: i64::try_from(worker.queued).unwrap_or(i64::MAX),
            model: settings.model(worker.id),
            windows: worker
                .windows
                .into_iter()
                .map(|window| PlanWindow {
                    name: window.name,
                    used: window.used,
                    resets_at: window.resets_at,
                })
                .collect(),
            usage_at: usage_at(store, worker.id),
            version: worker.version,
            version_status: worker.version_status,
            tested_version: tested_version(worker.id).unwrap_or("").to_string(),
        })
        .collect()
}

fn usage_at(store: &Store, id: ProviderId) -> Option<i64> {
    let rows = store.windows(legate_core::plan_of(id).as_str()).ok()?;
    rows.iter().map(|window| window.observed_at).max()
}

pub(crate) fn publish_plans(world: &World) {
    let plans = plan_list(&world.orch, &world.settings.borrow(), &world.store);
    let _ = world.events.send(AppEvent::Plans(plans));
}

struct UsageBits {
    orch: Rc<Orchestrator>,
    settings: Rc<RefCell<legate_core::Settings>>,
    store: Rc<Store>,
    claude: Arc<ClaudeAdapter>,
    events: broadcast::Sender<AppEvent>,
    sight: Rc<Cell<WindowSight>>,
    usage_last: Rc<Cell<Option<i64>>>,
    flight: Rc<RefCell<Option<Flight>>>,
    data_dir: PathBuf,
}

impl UsageBits {
    fn start(&self, trigger: UsageTrigger) {
        if self.flight.borrow().is_some() {
            return;
        }
        *self.flight.borrow_mut() = Some(Flight {
            waiters: Vec::new(),
        });
        let bits = self.clone_bits();
        tokio::task::spawn_local(async move {
            let outcome = bits.probe(trigger).await;
            let waiters = bits
                .flight
                .borrow_mut()
                .take()
                .map(|flight| flight.waiters)
                .unwrap_or_default();
            for waiter in waiters {
                let _ = waiter.send(outcome.clone());
            }
        });
    }

    fn clone_bits(&self) -> Self {
        Self {
            orch: Rc::clone(&self.orch),
            settings: Rc::clone(&self.settings),
            store: Rc::clone(&self.store),
            claude: Arc::clone(&self.claude),
            events: self.events.clone(),
            sight: Rc::clone(&self.sight),
            usage_last: Rc::clone(&self.usage_last),
            flight: Rc::clone(&self.flight),
            data_dir: self.data_dir.clone(),
        }
    }

    async fn probe(&self, trigger: UsageTrigger) -> crate::options::UsageOutcome {
        let automatic = !matches!(trigger, UsageTrigger::Manual);
        if automatic && !self.settings.borrow().prefs().usage_check {
            return crate::options::UsageOutcome::success();
        }
        let reported = usage_at(&self.store, ProviderId::Claude);
        let last = match (self.usage_last.get(), reported) {
            (None, other) => other,
            (Some(stamp), None) => Some(stamp),
            (Some(stamp), Some(reported)) => Some(stamp.max(reported)),
        };
        let now = system_now_ms();
        if !usage_check_due(trigger, now, last, self.sight.get()) {
            return crate::options::UsageOutcome::success();
        }
        let claude_open = self
            .orch
            .workers()
            .iter()
            .any(|worker| worker.id == ProviderId::Claude && worker.available);
        if !claude_open {
            return crate::options::UsageOutcome::failure("Claude is not signed in.");
        }
        self.usage_last.set(Some(now));
        let dir = self.data_dir.join("usage-probe");
        if let Err(err) = std::fs::create_dir_all(&dir) {
            return crate::options::UsageOutcome::failure(err.to_string());
        }
        let mut handle = self.claude.usage_probe(&dir);
        let mut saw = false;
        while let Some(event) = handle.events.recv().await {
            if let AgentEvent::Usage {
                windows: Some(windows),
                ..
            } = event
            {
                saw = true;
                let rows = windows
                    .iter()
                    .map(|window| NewWindow {
                        name: window.name.clone(),
                        utilization: window.utilization,
                        resets_at: window.resets_at.map(|seconds| seconds.saturating_mul(1000)),
                    })
                    .collect::<Vec<_>>();
                let _ = self.store.save_windows("claude", &rows, system_now_ms());
            }
        }
        let _ = handle.exit().await;
        let plans = plan_list(&self.orch, &self.settings.borrow(), &self.store);
        let _ = self.events.send(AppEvent::Plans(plans));
        if saw {
            crate::options::UsageOutcome::success()
        } else {
            crate::options::UsageOutcome::failure("Claude did not report its usage.")
        }
    }
}

impl World {
    pub fn begin_usage(&self, trigger: UsageTrigger) -> Task<crate::options::UsageOutcome> {
        if self.usage_flight.borrow().is_some() {
            let (tx, rx) = oneshot::channel();
            if let Some(flight) = self.usage_flight.borrow_mut().as_mut() {
                flight.waiters.push(tx);
            }
            return Box::pin(async move {
                Ok(rx
                    .await
                    .unwrap_or_else(|_| crate::options::UsageOutcome::failure("usage check ended")))
            });
        }
        let (tx, rx) = oneshot::channel();
        *self.usage_flight.borrow_mut() = Some(Flight { waiters: vec![tx] });
        let bits = UsageBits {
            orch: Rc::clone(&self.orch),
            settings: Rc::clone(&self.settings),
            store: Rc::clone(&self.store),
            claude: Arc::clone(&self.claude),
            events: self.events.clone(),
            sight: Rc::clone(&self.sight),
            usage_last: Rc::clone(&self.usage_last),
            flight: Rc::clone(&self.usage_flight),
            data_dir: self.data_dir.clone(),
        };
        tokio::task::spawn_local(async move {
            let outcome = bits.probe(trigger).await;
            let waiters = bits
                .flight
                .borrow_mut()
                .take()
                .map(|flight| flight.waiters)
                .unwrap_or_default();
            for waiter in waiters {
                let _ = waiter.send(outcome.clone());
            }
        });
        Box::pin(async move {
            Ok(rx
                .await
                .unwrap_or_else(|_| crate::options::UsageOutcome::failure("usage check ended")))
        })
    }
}

struct TerminalWatch {
    tracker: Rc<RefCell<SetupTracker>>,
    push: ChangePush,
    sign_in: Rc<RefCell<std::collections::HashSet<String>>>,
    orch: Rc<Orchestrator>,
    lead: Rc<Lead>,
    events: broadcast::Sender<AppEvent>,
    store: Rc<Store>,
    settings: Rc<RefCell<legate_core::Settings>>,
}

impl TerminalWatch {
    fn from_world(world: &World) -> Self {
        Self {
            tracker: Rc::clone(&world.setup_tracker),
            push: world.change_push.clone(),
            sign_in: Rc::clone(&world.sign_in),
            orch: Rc::clone(&world.orch),
            lead: Rc::clone(&world.lead),
            events: world.events.clone(),
            store: Rc::clone(&world.store),
            settings: Rc::clone(&world.settings),
        }
    }

    async fn note(&self, info: &TerminalInfo, removed: bool) {
        let provider = match info.provider {
            legate_core::TerminalProvider::Shell => "shell",
            legate_core::TerminalProvider::Claude => "claude",
            legate_core::TerminalProvider::Cursor => "cursor",
            legate_core::TerminalProvider::Grok => "grok",
            legate_core::TerminalProvider::Local => "local",
        };
        let before = info
            .change
            .as_deref()
            .and_then(|id| self.tracker.borrow().get(id));
        self.tracker.borrow_mut().on_terminal(
            info.change.as_deref(),
            provider,
            removed,
            match info.status {
                legate_core::TerminalStatus::Running => "running",
                legate_core::TerminalStatus::Exited => "exited",
                legate_core::TerminalStatus::Sleeping => "sleeping",
            },
            info.exit_code,
        );
        let after = info
            .change
            .as_deref()
            .and_then(|id| self.tracker.borrow().get(id));
        if before != after {
            self.push.spawn();
        }
        let signing_in = self.sign_in.borrow().contains(&info.id);
        if signing_in && (removed || info.status == legate_core::TerminalStatus::Exited) {
            self.sign_in.borrow_mut().remove(&info.id);
            self.orch.recheck().await;
            let _ = self.lead.recheck().await;
            let plans = plan_list(&self.orch, &self.settings.borrow(), &self.store);
            let _ = self.events.send(AppEvent::Plans(plans));
        }
    }
}

impl World {
    pub fn app_info(&self) -> AppInfo {
        AppInfo {
            version: env!("CARGO_PKG_VERSION").to_string(),
            data_dir: self.data_dir.to_string_lossy().into_owned(),
            worktrees_dir: self.worktrees_dir.to_string_lossy().into_owned(),
            rules_max_parallel: self.rules_max,
        }
    }

    pub fn reload(&self) {
        reload_project(
            &self.lead,
            &self.orch,
            &self.goals,
            &self.project.borrow(),
            &self.missed,
        );
    }
}

pub(crate) fn mirror_project(world: &World, path: &str) {
    *world.project.borrow_mut() = path.to_string();
    *world
        .project_mirror
        .write()
        .unwrap_or_else(|err| err.into_inner()) = path.to_string();
}
