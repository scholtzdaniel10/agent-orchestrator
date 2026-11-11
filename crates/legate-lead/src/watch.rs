//! Schedules and GitHub watches. Sleeps until the next fire time; it does not poll every second.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Notify;

use crate::error::LeadError;
use crate::goals::NewGoal;
use crate::ports::CommandRun;
use crate::text::split_lines;
use crate::triggers::{
    Firing, GithubFacts, Trigger, TriggerKind, fresh_firings, is_scheduled, next_due, parse_issues,
    parse_pulls, parse_runs,
};

const GITHUB_EVERY_MS: i64 = 5 * 60_000;
const LOG_TAIL: usize = 3500;

pub trait TriggerStore {
    fn load(&self) -> Result<Vec<String>, LeadError>;
    fn save(&self, trigger: &Trigger) -> Result<(), LeadError>;
}

impl TriggerStore for legate_store::Store {
    fn load(&self) -> Result<Vec<String>, LeadError> {
        Ok(legate_store::Store::triggers(self)?)
    }

    fn save(&self, trigger: &Trigger) -> Result<(), LeadError> {
        let data =
            serde_json::to_string(trigger).map_err(|err| LeadError::message(err.to_string()))?;
        Ok(legate_store::Store::save_trigger(
            self,
            &trigger.id,
            &trigger.project,
            &data,
        )?)
    }
}

/// Starts a goal for a trigger. `active` is true while one from this trigger is still open.
#[async_trait(?Send)]
pub trait GoalStarter {
    async fn start(&self, input: NewGoal) -> Result<(), LeadError>;
    fn active(&self, project: &str, source_id: &str) -> bool;
}

pub struct TriggerService {
    store: Rc<dyn TriggerStore>,
    goals: Rc<dyn GoalStarter>,
    commands: std::sync::Arc<dyn CommandRun>,
    now: Rc<dyn Fn() -> i64>,
    notify: Notify,
    stopped: AtomicBool,
    last_look: std::cell::RefCell<HashMap<String, i64>>,
    initial: Duration,
    retry_busy: Duration,
}

pub struct TriggerOpts {
    pub store: Rc<dyn TriggerStore>,
    pub goals: Rc<dyn GoalStarter>,
    pub commands: std::sync::Arc<dyn CommandRun>,
    pub now: Rc<dyn Fn() -> i64>,
    pub initial_delay: Option<Duration>,
    pub retry_busy: Option<Duration>,
}

impl TriggerService {
    pub fn new(opts: TriggerOpts) -> Rc<Self> {
        Rc::new(Self {
            store: opts.store,
            goals: opts.goals,
            commands: opts.commands,
            now: opts.now,
            notify: Notify::new(),
            stopped: AtomicBool::new(false),
            last_look: std::cell::RefCell::new(HashMap::new()),
            initial: opts
                .initial_delay
                .unwrap_or_else(|| Duration::from_secs(15)),
            retry_busy: opts.retry_busy.unwrap_or_else(|| Duration::from_secs(60)),
        })
    }

    /// Wake a sleep so a saved or deleted trigger is noticed without waiting out the old deadline.
    pub fn poke(&self) {
        self.notify.notify_waiters();
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.poke();
    }

    pub fn save(&self, trigger: &Trigger) -> Result<(), LeadError> {
        self.store.save(trigger)?;
        self.poke();
        Ok(())
    }

    pub fn start(self: &Rc<Self>) {
        let this = Rc::clone(self);
        tokio::task::spawn_local(async move {
            this.run().await;
        });
    }

    async fn run(&self) {
        if !self.initial.is_zero() {
            let due = (self.now)() + self.initial.as_millis() as i64;
            self.wait_until(Some(due)).await;
        }
        while !self.stopped.load(Ordering::Acquire) {
            if let Err(err) = self.tick().await {
                tracing::error!("{err}");
            }
            if self.stopped.load(Ordering::Acquire) {
                return;
            }
            self.wait_until(self.next_deadline()).await;
        }
    }

    fn next_deadline(&self) -> Option<i64> {
        let now = (self.now)();
        let Ok(triggers) = self.enabled() else {
            return Some(now + self.retry_busy.as_millis() as i64);
        };
        let mut deadline = None::<i64>;
        let mut retry = false;
        let mut projects = HashSet::new();
        for trigger in &triggers {
            if is_scheduled(trigger.kind) {
                match next_due(trigger, now) {
                    Some(due) if due <= now => retry = true,
                    Some(due) => deadline = Some(deadline.map(|at| at.min(due)).unwrap_or(due)),
                    None => {}
                }
            } else {
                projects.insert(trigger.project.clone());
            }
        }
        let looks = self.last_look.borrow();
        for project in projects {
            let due = looks
                .get(&project)
                .map(|last| last + GITHUB_EVERY_MS)
                .unwrap_or(now);
            if due <= now {
                retry = true;
            } else {
                deadline = Some(deadline.map(|at| at.min(due)).unwrap_or(due));
            }
        }
        if retry {
            let at = now + self.retry_busy.as_millis() as i64;
            deadline = Some(deadline.map(|due| due.min(at)).unwrap_or(at));
        }
        deadline
    }

    async fn tick(&self) -> Result<(), LeadError> {
        let now = (self.now)();
        for mut trigger in self.enabled()? {
            if is_scheduled(trigger.kind) && crate::triggers::is_due(&trigger, now) {
                self.fire(&mut trigger, None).await;
            }
        }
        let mut by_project: HashMap<String, Vec<Trigger>> = HashMap::new();
        for trigger in self.enabled()? {
            if is_scheduled(trigger.kind) {
                continue;
            }
            by_project
                .entry(trigger.project.clone())
                .or_default()
                .push(trigger);
        }
        for (project, list) in by_project {
            let last = self.last_look.borrow().get(&project).copied();
            if last.is_some_and(|last| now.saturating_sub(last) < GITHUB_EVERY_MS) {
                continue;
            }
            if !is_dir(&project) {
                continue;
            }
            self.last_look.borrow_mut().insert(project.clone(), now);
            let kinds: Vec<TriggerKind> = list.iter().map(|trigger| trigger.kind).collect();
            let facts = match github_facts(self.commands.as_ref(), &project, &kinds).await {
                Ok(facts) => facts,
                Err(err) => {
                    for mut trigger in list {
                        trigger.last_error = Some(err.clone());
                        let _ = self.store.save(&trigger);
                    }
                    continue;
                }
            };
            for mut trigger in list {
                let (fire, seen) = fresh_firings(&trigger, &facts);
                trigger.seen = seen;
                trigger.primed = true;
                trigger.last_error = None;
                let _ = self.store.save(&trigger);
                if let Some(first) = fire.into_iter().next() {
                    self.fire(&mut trigger, Some(&first)).await;
                }
            }
        }
        Ok(())
    }

    async fn fire(&self, trigger: &mut Trigger, firing: Option<&Firing>) {
        if self.goals.active(&trigger.project, &trigger.id) && firing.is_none() {
            return;
        }
        let mut text = firing
            .map(|firing| firing.text.clone())
            .unwrap_or_else(|| trigger.text.clone());
        if let Some(run_id) = firing.and_then(|firing| firing.run_id)
            && let Ok(log) = self
                .commands
                .run(
                    "gh",
                    &[
                        "run".to_string(),
                        "view".to_string(),
                        run_id.to_string(),
                        "--log-failed".to_string(),
                    ],
                    Some(&trigger.project),
                )
                .await
        {
            let tail = tail_trim(&log.stdout, LOG_TAIL);
            if !tail.is_empty() {
                text.push_str("\n\nThe end of the failing log:\n");
                text.push_str(&tail);
            }
        }
        trigger.last_fired = Some((self.now)());
        trigger.last_error = None;
        let _ = self.store.save(trigger);
        let input = NewGoal {
            project: trigger.project.clone(),
            text,
            check: trigger.check.clone(),
            max_rounds: Some(trigger.max_rounds as f64),
            quick: None,
            source: Some(crate::goals::GoalSource {
                id: trigger.id.clone(),
                label: label(trigger, firing),
            }),
        };
        if let Err(err) = self.goals.start(input).await {
            trigger.last_error = Some(err.to_string());
            let _ = self.store.save(trigger);
        }
    }

    fn enabled(&self) -> Result<Vec<Trigger>, LeadError> {
        let mut out = Vec::new();
        for raw in self.store.load()? {
            if let Some(trigger) = crate::triggers::parse_stored(&raw)
                && trigger.enabled
            {
                out.push(trigger);
            }
        }
        Ok(out)
    }

    async fn wait_until(&self, deadline: Option<i64>) {
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        let notified = self.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        let Some(deadline) = deadline else {
            notified.await;
            return;
        };
        let now = (self.now)();
        if now >= deadline {
            return;
        }
        let wait = Duration::from_millis((deadline - now) as u64);
        tokio::select! {
            _ = notified => {}
            _ = tokio::time::sleep(wait) => {}
        }
    }
}

fn label(trigger: &Trigger, firing: Option<&Firing>) -> String {
    match trigger.kind {
        TriggerKind::Daily => format!("Every day at {}", trigger.at.as_deref().unwrap_or("?")),
        TriggerKind::Hourly => format!("Every {} hours", trigger.hours.unwrap_or(24)),
        _ => match firing {
            Some(firing) if firing.key.starts_with("issue:") => "New issue".to_string(),
            Some(_) => "GitHub failure".to_string(),
            None => "GitHub".to_string(),
        },
    }
}

fn is_dir(path: &str) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| meta.is_dir())
}

fn tail_trim(text: &str, max: usize) -> String {
    let count = text.chars().count();
    let tail: String = if count > max {
        text.chars().skip(count - max).collect()
    } else {
        text.to_string()
    };
    tail.trim().to_string()
}

/// The git and `gh` commands a GitHub watch runs in the project.
pub async fn github_facts(
    commands: &dyn CommandRun,
    project: &str,
    kinds: &[TriggerKind],
) -> Result<GithubFacts, String> {
    let mut facts = GithubFacts::default();
    let head = commands
        .run(
            "git",
            &["rev-parse".into(), "--abbrev-ref".into(), "HEAD".into()],
            Some(project),
        )
        .await?;
    let branch = head.stdout.trim();
    facts.branch = if head.code == Some(0) && !branch.is_empty() && branch != "HEAD" {
        Some(branch.to_string())
    } else {
        None
    };
    let ask = |args: Vec<String>| {
        let project = project.to_string();
        async move {
            let result = commands.run("gh", &args, Some(&project)).await?;
            if result.code != Some(0) {
                let line = split_lines(result.stderr.trim())
                    .into_iter()
                    .next()
                    .unwrap_or("")
                    .to_string();
                return Err(if line.is_empty() {
                    "The GitHub CLI failed.".to_string()
                } else {
                    line
                });
            }
            Ok(result.stdout)
        }
    };
    if kinds.contains(&TriggerKind::CiFailed) {
        facts.pulls = parse_pulls(
            &ask(vec![
                "pr".into(),
                "list".into(),
                "--author".into(),
                "@me".into(),
                "--state".into(),
                "open".into(),
                "--limit".into(),
                "30".into(),
                "--json".into(),
                "number,title,headRefName,headRefOid,statusCheckRollup".into(),
            ])
            .await?,
        );
    }
    if kinds.contains(&TriggerKind::NewIssue) {
        facts.issues = parse_issues(
            &ask(vec![
                "issue".into(),
                "list".into(),
                "--state".into(),
                "open".into(),
                "--limit".into(),
                "30".into(),
                "--json".into(),
                "number,title,body".into(),
            ])
            .await?,
        );
    }
    if kinds.contains(&TriggerKind::MainBroken) {
        let name = ask(vec![
            "repo".into(),
            "view".into(),
            "--json".into(),
            "defaultBranchRef".into(),
            "-q".into(),
            ".defaultBranchRef.name".into(),
        ])
        .await?
        .trim()
        .to_string();
        facts.default_branch = if name.is_empty() {
            None
        } else {
            Some(name.clone())
        };
        if let Some(name) = facts.default_branch.clone() {
            facts.main_runs = parse_runs(
                &ask(vec![
                    "run".into(),
                    "list".into(),
                    "--branch".into(),
                    name,
                    "--limit".into(),
                    "5".into(),
                    "--json".into(),
                    "databaseId,name,status,conclusion".into(),
                ])
                .await?,
            );
        }
    }
    Ok(facts)
}

/// The next epoch-ms deadline, or `None` when nothing is scheduled and the loop should wait for [`TriggerService::poke`].
pub fn next_deadline_ms(
    now: i64,
    due: &[Option<i64>],
    github_due: &[Option<i64>],
    retry_ms: i64,
) -> Option<i64> {
    let mut deadline: Option<i64> = None;
    let mut retry = false;
    for item in due.iter().chain(github_due.iter()) {
        match item {
            Some(at) if *at <= now => retry = true,
            Some(at) => deadline = Some(deadline.map(|due| due.min(*at)).unwrap_or(*at)),
            None => {}
        }
    }
    if retry {
        let at = now + retry_ms;
        deadline = Some(deadline.map(|due| due.min(at)).unwrap_or(at));
    }
    deadline
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;

    use super::{TriggerOpts, TriggerService, TriggerStore, next_deadline_ms};
    use crate::error::LeadError;
    use crate::goals::NewGoal;
    use crate::ports::{CommandOutput, CommandRun};
    use crate::triggers::{NewTrigger, Trigger, TriggerKind, new_trigger};

    struct Mem {
        rows: RefCell<Vec<Trigger>>,
    }

    impl TriggerStore for Mem {
        fn load(&self) -> Result<Vec<String>, LeadError> {
            Ok(self
                .rows
                .borrow()
                .iter()
                .map(|trigger| serde_json::to_string(trigger).unwrap())
                .collect())
        }

        fn save(&self, trigger: &Trigger) -> Result<(), LeadError> {
            let mut rows = self.rows.borrow_mut();
            if let Some(found) = rows.iter_mut().find(|row| row.id == trigger.id) {
                *found = trigger.clone();
            } else {
                rows.push(trigger.clone());
            }
            Ok(())
        }
    }

    struct Starter {
        fires: RefCell<Vec<String>>,
    }

    #[async_trait(?Send)]
    impl super::GoalStarter for Starter {
        async fn start(&self, input: NewGoal) -> Result<(), LeadError> {
            self.fires.borrow_mut().push(input.text);
            Ok(())
        }

        fn active(&self, _project: &str, _source_id: &str) -> bool {
            false
        }
    }

    struct Quiet;

    #[async_trait::async_trait]
    impl CommandRun for Quiet {
        async fn run(
            &self,
            _command: &str,
            _args: &[String],
            _cwd: Option<&str>,
        ) -> Result<CommandOutput, String> {
            Ok(CommandOutput {
                code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    #[tokio::test(start_paused = true)]
    async fn an_hourly_trigger_sleeps_until_it_is_due() {
        let set = tokio::task::LocalSet::new();
        set.run_until(async {
            let started = tokio::time::Instant::now();
            let store = Rc::new(Mem {
                rows: RefCell::new(Vec::new()),
            });
            let trigger = new_trigger(NewTrigger {
                project: "/p".into(),
                kind: TriggerKind::Hourly,
                at: None,
                hours: Some(6.0),
                text: Some("check the build".into()),
                check: None,
                max_rounds: None,
                now: 0,
            })
            .unwrap();
            store.save(&trigger).unwrap();
            let starter = Rc::new(Starter {
                fires: RefCell::new(Vec::new()),
            });
            let service = TriggerService::new(TriggerOpts {
                store,
                goals: starter.clone(),
                commands: Arc::new(Quiet),
                now: Rc::new(move || started.elapsed().as_millis() as i64),
                initial_delay: None,
                retry_busy: None,
            });
            service.start();
            tokio::time::sleep(Duration::from_secs(1)).await;
            assert!(starter.fires.borrow().is_empty());
            tokio::time::sleep(Duration::from_secs(5 * 3600)).await;
            assert!(starter.fires.borrow().is_empty());
            tokio::time::sleep(Duration::from_secs(3600 + 5)).await;
            tokio::task::yield_now().await;
            assert_eq!(starter.fires.borrow().as_slice(), ["check the build"]);
            service.stop();
        })
        .await;
    }

    #[test]
    fn no_deadline_waits_for_a_poke_and_a_due_item_retries() {
        assert_eq!(next_deadline_ms(0, &[], &[], 60_000), None);
        assert_eq!(next_deadline_ms(10, &[Some(50)], &[], 60_000), Some(50));
        assert_eq!(next_deadline_ms(10, &[Some(10)], &[], 60_000), Some(60_010));
    }
}
