//! The lead chat: one session that proposes jobs, and direct chats that do not.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::time::Instant;

use legate_core::{
    ApprovalStatus, JobAccess, JobRecord, JobStatus, LeadApproval, LeadChat, LeadMessage,
    LeadMessageKind, LeadMode, LeadProposal, LeadRole, LeadStatus, ProposalStatus, ProposedJob,
    ProviderId, RouterRules, RunOptions, needs_usage_credits, plan_of,
};
use legate_router::{headroom, rest_after_limit};
use legate_store::{ChatRow, NewWindow, RunOutcome, RunRow, Store};
use tokio::sync::watch;

use crate::error::LeadError;
use crate::ports::{BridgeSource, EmptyJobs, JobRequest, JobSink, LeadAdapter};
use crate::prompts::LEAD_INSTRUCTIONS;
use crate::routines::chat_plan;
use crate::text::{fresh_id, tail_chars};

pub const NO_PLAN: &str = "No plan is available for the lead right now.";
const REPORT_OUTPUT_MAX: usize = 4000;
const MAX_AUTO_ROUNDS: u64 = 5;
const AUTO_ROUNDS: &str = "autoRounds";

pub struct LeadOpts {
    pub adapters: Vec<std::sync::Arc<dyn LeadAdapter>>,
    pub store: Rc<Store>,
    pub rules: RouterRules,
    pub bridge: std::sync::Arc<dyn BridgeSource>,
    pub dir: String,
    pub project: Rc<dyn Fn() -> String>,
    pub prefer: Option<Rc<dyn Fn() -> Option<ProviderId>>>,
    pub now: Option<Rc<dyn Fn() -> i64>>,
    pub model_for: Option<Rc<dyn Fn(ProviderId) -> Option<String>>>,
}

struct Pending {
    chat_id: String,
    lead_message_id: String,
    order: i64,
}

struct ChatContext {
    chat_id: Option<String>,
    history: Vec<LeadMessage>,
    session_id: Option<String>,
    session_provider: Option<ProviderId>,
    chat_title: String,
    chat_created_at: i64,
    direct: Option<ProviderId>,
}

type LeadListener = Rc<dyn Fn(LeadMessage, Option<String>)>;

struct State {
    history: Vec<LeadMessage>,
    listeners: Vec<(u64, LeadListener)>,
    next_listener: u64,
    session_id: Option<String>,
    session_provider: Option<ProviderId>,
    busy: bool,
    draining: bool,
    turn_id: Option<String>,
    stop_turn: Option<watch::Sender<bool>>,
    chat_id: Option<String>,
    view_chat_id: Option<String>,
    chat_title: String,
    direct: Option<ProviderId>,
    chat_created_at: i64,
    pending: Vec<Pending>,
    submitting: Option<String>,
    follow_scheduled: HashSet<String>,
    skip_follow: HashSet<String>,
    follow_order: i64,
    turn_chat: HashMap<String, String>,
    /// A turn whose plan hit its limit before answering; it goes to the next plan.
    retry_turn: Option<String>,
    /// Questions the Lead's CLI is waiting on, by approval id.
    approvals: HashMap<String, tokio::sync::oneshot::Sender<bool>>,
}

#[derive(Default)]
struct Availability {
    run_mode: Cell<LeadMode>,
    off_plans: std::cell::RefCell<Vec<ProviderId>>,
    available: std::cell::RefCell<Vec<ProviderId>>,
    checking: tokio::sync::Mutex<()>,
}

pub struct Lead {
    adapters: Vec<std::sync::Arc<dyn LeadAdapter>>,
    store: Rc<Store>,
    rules: RouterRules,
    bridge: std::sync::Arc<dyn BridgeSource>,
    dir: String,
    project: Rc<dyn Fn() -> String>,
    prefer: Option<Rc<dyn Fn() -> Option<ProviderId>>>,
    now: Rc<dyn Fn() -> i64>,
    model_for: Option<Rc<dyn Fn(ProviderId) -> Option<String>>>,
    availability: Rc<Availability>,
    jobs: std::cell::RefCell<Rc<dyn JobSink>>,
    unsub: std::cell::RefCell<Option<Box<dyn FnOnce()>>>,
    state: std::cell::RefCell<State>,
    this: Weak<Lead>,
}

struct Plan {
    provider: ProviderId,
    resume: Option<String>,
    prompt: String,
}

impl Lead {
    pub fn new(opts: LeadOpts) -> Rc<Self> {
        Self::with_availability(opts, Rc::new(Availability::default()))
    }

    fn with_availability(opts: LeadOpts, availability: Rc<Availability>) -> Rc<Self> {
        Rc::new_cyclic(|this| Self {
            adapters: opts.adapters,
            store: opts.store,
            rules: opts.rules,
            bridge: opts.bridge,
            dir: opts.dir,
            project: opts.project,
            prefer: opts.prefer,
            now: opts
                .now
                .unwrap_or_else(|| Rc::new(crate::goals::system_now)),
            model_for: opts.model_for,
            availability,
            jobs: std::cell::RefCell::new(Rc::new(EmptyJobs)),
            unsub: std::cell::RefCell::new(None),
            state: std::cell::RefCell::new(State {
                history: Vec::new(),
                listeners: Vec::new(),
                next_listener: 1,
                session_id: None,
                session_provider: None,
                busy: false,
                draining: false,
                turn_id: None,
                stop_turn: None,
                chat_id: None,
                view_chat_id: None,
                chat_title: String::new(),
                direct: None,
                chat_created_at: 0,
                pending: Vec::new(),
                submitting: None,
                follow_scheduled: HashSet::new(),
                skip_follow: HashSet::new(),
                follow_order: 0,
                turn_chat: HashMap::new(),
                retry_turn: None,
                approvals: HashMap::new(),
            }),
            this: this.clone(),
        })
    }

    /// Independent session sharing provider configuration, never a mutable project pointer.
    pub fn for_project(
        &self,
        project: String,
        bridge: std::sync::Arc<dyn BridgeSource>,
    ) -> Rc<Self> {
        Self::with_availability(
            LeadOpts {
                adapters: self.adapters.clone(),
                store: self.store.clone(),
                rules: self.rules.clone(),
                bridge,
                dir: self.dir.clone(),
                project: Rc::new(move || project.clone()),
                prefer: self.prefer.clone(),
                now: Some(self.now.clone()),
                model_for: self.model_for.clone(),
            },
            self.availability.clone(),
        )
    }

    pub async fn init(&self) -> Result<(), LeadError> {
        self.recheck().await
    }

    pub fn set_off_plans(&self, plans: Vec<ProviderId>) {
        *self.availability.off_plans.borrow_mut() = plans;
    }

    pub fn set_mode(&self, mode: LeadMode) {
        self.availability.run_mode.set(mode);
        if mode != LeadMode::Legit {
            return;
        }
        let Some(id) = self.view_chat() else {
            return;
        };
        let result = self.with_chat(&id, || {
            let waiting: Vec<_> = self
                .state
                .borrow()
                .history
                .iter()
                .filter(|message| {
                    message
                        .proposal
                        .as_ref()
                        .is_some_and(|p| p.status == ProposalStatus::Waiting)
                })
                .map(|message| message.id.clone())
                .collect();
            for turn in waiting {
                if let Err(err) = self.run_proposal(&turn, None) {
                    tracing::error!("{err}");
                }
            }
            Ok(())
        });
        if let Err(err) = result {
            tracing::error!("{err}");
        }
    }

    fn is_off(&self, provider: ProviderId) -> bool {
        self.availability
            .off_plans
            .borrow()
            .contains(&plan_of(provider))
    }

    pub async fn recheck(&self) -> Result<(), LeadError> {
        let _checking = self.availability.checking.lock().await;
        let mut available = Vec::new();
        for adapter in &self.adapters {
            let id = adapter.id();
            if !self.is_off(id)
                && adapter.is_installed().await
                && !self.is_off(id)
                && adapter.is_signed_in().await
            {
                available.push(id);
            }
        }
        *self.availability.available.borrow_mut() = available;
        Ok(())
    }

    pub async fn send(&self, text: &str) -> Result<LeadMessage, LeadError> {
        if text.trim().is_empty() {
            return Err(LeadError::message("empty message"));
        }
        if self.state.borrow().busy {
            return Err(LeadError::message("lead is busy"));
        }
        let user = LeadMessage {
            id: fresh_id(),
            role: LeadRole::User,
            text: text.to_string(),
            provider: None,
            model: None,
            status: LeadStatus::Done,
            kind: None,
            proposal: None,
            extra: Default::default(),
        };
        let lead = LeadMessage {
            id: fresh_id(),
            role: LeadRole::Lead,
            text: String::new(),
            provider: None,
            model: None,
            status: LeadStatus::Streaming,
            kind: None,
            proposal: None,
            extra: Default::default(),
        };
        let lead_id = lead.id.clone();
        {
            let mut state = self.state.borrow_mut();
            state.busy = true;
            drop(state);
            self.ensure_chat(text);
            let mut state = self.state.borrow_mut();
            state.history.push(user.clone());
            state.history.push(lead);
            state.turn_id = Some(lead_id.clone());
            if let Some(chat_id) = state.chat_id.clone() {
                state.turn_chat.insert(lead_id.clone(), chat_id);
            }
        }
        let _guard = BusyClear {
            lead: self.this.clone(),
        };
        self.emit_id(&user.id);
        self.turn_with_failover(&lead_id, text).await;
        Ok(self.message(&lead_id).unwrap_or(user))
    }

    pub fn view_chat(&self) -> Option<String> {
        self.state.borrow().view_chat_id.clone()
    }

    pub fn attach_jobs(self: &Rc<Self>, jobs: Rc<dyn JobSink>) {
        if let Some(unsub) = self.unsub.borrow_mut().take() {
            unsub();
        }
        let this = Rc::downgrade(self);
        let unsub = jobs.on_update(Rc::new(move |job| {
            if let Some(this) = this.upgrade() {
                this.on_job_update(&job);
            }
        }));
        *self.unsub.borrow_mut() = Some(unsub);
        *self.jobs.borrow_mut() = jobs;
    }

    pub fn note_restored_jobs(&self, jobs: &[JobRecord]) {
        let mut state = self.state.borrow_mut();
        for job in jobs {
            if job.error.as_deref() == Some("interrupted when the app closed")
                && let Some(lead_message) = &job.lead_message
            {
                state.skip_follow.insert(lead_message.clone());
            }
        }
    }

    pub async fn when_quiet(&self) {
        loop {
            let (busy, draining, pending) = {
                let state = self.state.borrow();
                (state.busy, state.draining, !state.pending.is_empty())
            };
            if !busy && !draining && !pending {
                return;
            }
            if !busy && !draining && pending {
                self.spawn_drain();
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    pub fn is_busy(&self) -> bool {
        self.state.borrow().busy
    }

    pub fn attach_proposal(
        &self,
        turn: &str,
        proposal: LeadProposal,
    ) -> Result<ProposalStatus, LeadError> {
        let mode = self.availability.run_mode.get();
        let rounds = {
            let mut state = self.state.borrow_mut();
            if state.skip_follow.contains(turn) {
                return Err(LeadError::message("That turn was stopped."));
            }
            let message = state
                .history
                .iter_mut()
                .find(|item| item.id == turn)
                .ok_or_else(|| LeadError::message("that lead turn is not open"))?;
            if message
                .proposal
                .as_ref()
                .is_some_and(|p| p.status == ProposalStatus::Running)
            {
                return Err(LeadError::message(
                    "This turn's plan is already running. Wait for its report before proposing more.",
                ));
            }
            message.proposal = Some(proposal);
            message
                .extra
                .get(AUTO_ROUNDS)
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
        };
        if mode == LeadMode::Legit || (mode == LeadMode::Auto && rounds < MAX_AUTO_ROUNDS) {
            let round = (mode == LeadMode::Auto).then(|| rounds + 1);
            self.start_proposal(turn, None, round)
                .inspect_err(|_| self.emit_id(turn))?;
            Ok(ProposalStatus::Running)
        } else {
            self.emit_id(turn);
            Ok(ProposalStatus::Waiting)
        }
    }

    pub fn run_proposal(
        &self,
        message_id: &str,
        jobs: Option<Vec<ProposedJob>>,
    ) -> Result<Vec<JobRecord>, LeadError> {
        self.start_proposal(message_id, jobs, None)
    }

    fn start_proposal(
        &self,
        message_id: &str,
        jobs: Option<Vec<ProposedJob>>,
        auto_round: Option<u64>,
    ) -> Result<Vec<JobRecord>, LeadError> {
        let (index, proposal) = {
            let state = self.state.borrow();
            let index = state.history.iter().position(|item| item.id == message_id);
            let proposal = index.and_then(|index| state.history[index].proposal.clone());
            (index, proposal)
        };
        let Some(index) = index else {
            return Err(LeadError::message("That plan is not open."));
        };
        let Some(proposal) = proposal else {
            return Err(LeadError::message("That plan is not open."));
        };
        if proposal.status != ProposalStatus::Waiting {
            return Err(LeadError::message(
                "That plan already ran or was dismissed.",
            ));
        }
        let mut chosen: Vec<ProposedJob> = jobs
            .unwrap_or(proposal.jobs)
            .into_iter()
            .filter(|job| !job.prompt.trim().is_empty())
            .collect();
        if chosen.is_empty() {
            return Err(LeadError::message("The plan has no jobs left to run."));
        }
        if self.availability.run_mode.get() == LeadMode::Legit {
            for job in &mut chosen {
                if job.access == JobAccess::Edit {
                    job.access = JobAccess::Full;
                }
            }
        }
        let chat_id = self.state.borrow().chat_id.clone();
        if let Some(chat_id) = chat_id {
            self.state
                .borrow_mut()
                .turn_chat
                .insert(message_id.to_string(), chat_id);
        }
        let sink = Rc::clone(&self.jobs.borrow());
        let mut started = Vec::new();
        let mut failure = None;
        self.state.borrow_mut().submitting = Some(message_id.to_owned());
        for job in &chosen {
            match sink.submit(JobRequest {
                job_type: job.job_type,
                prompt: job.prompt.clone(),
                provider: job.worker,
                access: Some(job.access),
                title: Some(job.title.clone()),
                lead_message: Some(message_id.to_string()),
                ..JobRequest::default()
            }) {
                Ok(record) => started.push(record),
                Err(err) => {
                    failure = Some(err);
                    break;
                }
            }
        }
        self.state.borrow_mut().submitting = None;
        if started.is_empty()
            && let Some(err) = failure.take()
        {
            return Err(err);
        }
        chosen.truncate(started.len());
        let ids: Vec<String> = started.iter().map(|job| job.id.clone()).collect();
        {
            let mut state = self.state.borrow_mut();
            if let Some(message) = state.history.get_mut(index)
                && let Some(stored) = message.proposal.as_mut()
            {
                stored.jobs = chosen;
                stored.status = ProposalStatus::Running;
                stored.job_ids = Some(ids);
                if let Some(round) = auto_round {
                    message.extra.insert(AUTO_ROUNDS.into(), round.into());
                }
            }
        }
        self.persist_message(message_id);
        self.emit_id(message_id);
        if let Some(job) = started.last() {
            self.on_job_update(job);
        }
        if let Some(err) = failure {
            return Err(err);
        }
        Ok(started)
    }

    pub fn dismiss_proposal(&self, message_id: &str) -> Result<(), LeadError> {
        let index = self
            .state
            .borrow()
            .history
            .iter()
            .position(|item| item.id == message_id);
        let Some(index) = index else {
            return Err(LeadError::message("That plan is not open."));
        };
        let waiting = self.state.borrow().history[index]
            .proposal
            .as_ref()
            .map(|proposal| proposal.status == ProposalStatus::Waiting);
        match waiting {
            None => return Err(LeadError::message("That plan is not open.")),
            Some(false) => return Ok(()),
            Some(true) => {}
        }
        if let Some(proposal) = self.state.borrow_mut().history[index].proposal.as_mut() {
            proposal.status = ProposalStatus::Dismissed;
        }
        self.persist_message(message_id);
        self.emit_id(message_id);
        Ok(())
    }

    /// The Lead's CLI asks before it uses a tool (Manual). The question shows on the turn's
    /// message with Allow and Deny, and the answer arrives on the returned channel. A channel
    /// dropped unanswered (the turn ended) counts as Deny.
    pub fn ask_approval(
        &self,
        turn: &str,
        tool: &str,
        detail: &str,
    ) -> Result<tokio::sync::oneshot::Receiver<bool>, LeadError> {
        let id = fresh_id();
        let (answer, waiting) = tokio::sync::oneshot::channel();
        {
            let mut state = self.state.borrow_mut();
            let Some(message) = state.history.iter_mut().find(|item| item.id == turn) else {
                return Err(LeadError::message("That turn is not open."));
            };
            let mut approvals = message.approvals();
            approvals.push(LeadApproval {
                id: id.clone(),
                tool: tool.to_string(),
                detail: detail.to_string(),
                status: ApprovalStatus::Waiting,
            });
            message.set_approvals(&approvals);
            state.approvals.insert(id, answer);
        }
        self.emit_id(turn);
        Ok(waiting)
    }

    pub fn answer_approval(
        &self,
        message_id: &str,
        approval_id: &str,
        allow: bool,
    ) -> Result<(), LeadError> {
        let Some(answer) = self.state.borrow_mut().approvals.remove(approval_id) else {
            return Err(LeadError::message("That question is no longer waiting."));
        };
        if let Some(message) = self
            .state
            .borrow_mut()
            .history
            .iter_mut()
            .find(|item| item.id == message_id)
        {
            let mut approvals = message.approvals();
            for approval in approvals.iter_mut().filter(|item| item.id == approval_id) {
                approval.status = if allow {
                    ApprovalStatus::Allowed
                } else {
                    ApprovalStatus::Denied
                };
            }
            message.set_approvals(&approvals);
        }
        let _ = answer.send(allow);
        self.emit_id(message_id);
        Ok(())
    }

    pub fn current_turn(&self) -> Option<String> {
        self.state.borrow().turn_id.clone()
    }

    /// Stop this chat's CLI and jobs. Return the last user message for the composer.
    pub async fn stop(&self, chat_id: Option<&str>) -> Result<Option<String>, LeadError> {
        let Some(chat_id) = chat_id.map(str::to_owned).or_else(|| self.view_chat()) else {
            return Ok(None);
        };
        let stop_turn = {
            let state = self.state.borrow();
            state
                .stop_turn
                .clone()
                .filter(|_| state.chat_id.as_deref() == Some(chat_id.as_str()))
        };
        let result = self.with_chat(&chat_id, || {
            let history = self.state.borrow().history.clone();
            let ids: HashSet<_> = history
                .iter()
                .filter(|m| m.role == LeadRole::Lead)
                .map(|m| m.id.clone())
                .collect();
            {
                let mut state = self.state.borrow_mut();
                // Suppress reports before cancel: a sink can publish its final update inline.
                state.skip_follow.extend(ids.iter().cloned());
                state.pending.retain(|item| item.chat_id != chat_id);
            }
            if let Some(signal) = &stop_turn {
                signal.send_replace(true);
            }
            let sink = Rc::clone(&self.jobs.borrow());
            let mut first_error = None;
            for job in sink.list() {
                if matches!(job.status, JobStatus::Queued | JobStatus::Running)
                    && job.lead_message.as_ref().is_some_and(|id| ids.contains(id))
                    && let Err(err) = sink.cancel(&job.id)
                {
                    first_error.get_or_insert(err);
                }
            }
            let mut stopped = Vec::new();
            for message in &mut self.state.borrow_mut().history {
                if let Some(proposal) = &mut message.proposal
                    && matches!(
                        proposal.status,
                        ProposalStatus::Waiting | ProposalStatus::Running
                    )
                {
                    proposal.status = ProposalStatus::Stopped;
                    stopped.push(message.id.clone());
                }
            }
            for id in stopped {
                self.persist_message(&id);
                self.emit_id(&id);
            }
            if let Some(err) = first_error {
                return Err(err);
            }
            Ok(history
                .into_iter()
                .rev()
                .find(|m| m.role == LeadRole::User)
                .map(|m| m.text))
        });
        if let Some(signal) = stop_turn {
            signal.closed().await;
        }
        result
    }

    pub fn messages(&self) -> Vec<LeadMessage> {
        let state = self.state.borrow();
        if state.view_chat_id == state.chat_id {
            return state.history.clone();
        }
        state
            .view_chat_id
            .as_ref()
            .and_then(|id| self.store.messages(id).ok())
            .unwrap_or_default()
    }

    pub fn chats(&self) -> Result<Vec<LeadChat>, LeadError> {
        let active = self.view_chat();
        let mut out = Vec::new();
        for row in self.store.chats(&(self.project)())? {
            let pending = self.chat_has_pending(&row.id)?;
            let id = row.id;
            let session_of = |plan: &str| {
                row.session_id.is_some()
                    && row.direct.as_deref() == Some(plan)
                    && row.session_provider.as_deref() == Some(plan)
            };
            let claude_session = session_of("claude");
            let cursor_session = session_of("cursor");
            out.push(LeadChat {
                active: Some(id.as_str()) == active.as_deref(),
                id,
                title: row.title,
                updated_at: row.updated_at,
                pending: pending.then_some(true),
                direct: chat_plan(row.direct.as_deref()),
                source: row.source,
                claude_session: claude_session.then_some(true),
                cursor_session: cursor_session.then_some(true),
            });
        }
        Ok(out)
    }

    /// The chat a turn is running in, if any.
    pub fn busy_chat(&self) -> Option<String> {
        let state = self.state.borrow();
        if state.busy {
            state.chat_id.clone()
        } else {
            None
        }
    }

    /// Reloads a chat whose stored messages changed underneath the lead (the chat sync mirrored
    /// new turns from another app). Does nothing while a turn runs.
    pub fn refresh_chat(&self, id: &str) {
        let (busy, open, viewed) = {
            let state = self.state.borrow();
            (
                state.busy,
                state.chat_id.as_deref() == Some(id),
                state.view_chat_id.as_deref() == Some(id),
            )
        };
        if busy || !open {
            return;
        }
        if let Err(err) = self.load_chat(id, viewed) {
            tracing::error!("{err}");
        }
    }

    pub fn open(&self, id: &str) -> Result<(), LeadError> {
        if self.state.borrow().busy {
            return Err(LeadError::message("lead is busy"));
        }
        self.load_chat(id, true)
    }

    pub fn open_latest(&self) -> Result<(), LeadError> {
        if self.state.borrow().busy {
            return Err(LeadError::message("lead is busy"));
        }
        let newest = self.store.chats(&(self.project)())?.into_iter().next();
        let Some(newest) = newest else {
            self.clear_open_chat();
            return Ok(());
        };
        self.open(&newest.id)
    }

    pub fn on_update(&self, callback: LeadListener) -> Box<dyn FnOnce()> {
        let mut state = self.state.borrow_mut();
        let id = state.next_listener;
        state.next_listener += 1;
        state.listeners.push((id, callback));
        let this = self.this.clone();
        Box::new(move || {
            if let Some(this) = this.upgrade() {
                this.state
                    .borrow_mut()
                    .listeners
                    .retain(|(item, _)| *item != id);
            }
        })
    }

    pub fn provider(&self) -> Option<ProviderId> {
        self.state.borrow().session_provider
    }

    pub fn reset(&self, direct: Option<ProviderId>) -> Result<(), LeadError> {
        if self.state.borrow().busy {
            return Err(LeadError::message("lead is busy"));
        }
        self.clear_open_chat();
        self.state.borrow_mut().direct = direct;
        Ok(())
    }

    pub fn mode(&self) -> Option<ProviderId> {
        let state = self.state.borrow();
        if state.view_chat_id == state.chat_id {
            return state.direct;
        }
        state
            .view_chat_id
            .as_ref()
            .and_then(|id| self.store.chat(id).ok().flatten())
            .and_then(|chat| chat_plan(chat.direct.as_deref()))
    }

    pub fn rename(&self, id: &str, title: &str) -> Result<(), LeadError> {
        let trimmed = title.trim();
        if !(1..=80).contains(&trimmed.chars().count()) {
            return Err(LeadError::message("invalid title"));
        }
        let row = self.store.chat(id)?;
        if row
            .as_ref()
            .is_none_or(|row| row.project != (self.project)())
        {
            return Err(LeadError::message("unknown chat"));
        }
        self.store.rename_chat(id, trimmed)?;
        if self.state.borrow().chat_id.as_deref() == Some(id) {
            self.state.borrow_mut().chat_title = trimmed.to_string();
        }
        Ok(())
    }

    pub fn remove(&self, id: &str) -> Result<(), LeadError> {
        let row = self.store.chat(id)?;
        if row
            .as_ref()
            .is_none_or(|row| row.project != (self.project)())
        {
            return Err(LeadError::message("unknown chat"));
        }
        let state = self.state.borrow();
        if state.busy && state.chat_id.as_deref() == Some(id) {
            return Err(LeadError::message("lead is busy"));
        }
        if state.pending.iter().any(|item| item.chat_id == id) {
            return Err(LeadError::message("lead is busy"));
        }
        drop(state);
        self.store.delete_chat(id)?;
        if self.state.borrow().chat_id.as_deref() == Some(id) {
            self.clear_open_chat();
        }
        Ok(())
    }

    pub fn catch_up(&self, job: &JobRecord) {
        self.on_job_update(job);
    }

    fn clear_open_chat(&self) {
        let mut state = self.state.borrow_mut();
        state.direct = None;
        state.history.clear();
        state.session_id = None;
        state.session_provider = None;
        state.chat_id = None;
        state.view_chat_id = None;
        state.chat_title.clear();
        state.chat_created_at = 0;
    }

    fn ensure_chat(&self, text: &str) {
        let chat_id = self.state.borrow().chat_id.clone();
        if let Some(chat_id) = chat_id {
            let row = self.store.chat(&chat_id).ok().flatten();
            if row.is_some_and(|row| row.project == (self.project)()) {
                return;
            }
            let mut state = self.state.borrow_mut();
            state.history.clear();
            state.session_id = None;
            state.session_provider = None;
            state.chat_id = None;
            state.view_chat_id = None;
            state.chat_title.clear();
            state.chat_created_at = 0;
        }
        let id = fresh_id();
        let title = chat_title(text);
        let now = (self.now)();
        {
            let mut state = self.state.borrow_mut();
            state.chat_id = Some(id);
            state.view_chat_id = state.chat_id.clone();
            state.chat_title = title;
            state.chat_created_at = now;
        }
        self.persist_chat();
    }

    fn preferred(&self) -> Option<ProviderId> {
        self.prefer
            .as_ref()
            .and_then(|prefer| prefer())
            .filter(|id| !self.is_off(*id))
    }

    fn follow_preference(&self) {
        let prefer = {
            let state = self.state.borrow();
            if state.direct.is_some()
                || state.session_provider.is_none()
                || state.session_id.is_none()
            {
                return;
            }
            let Some(prefer) = self.preferred() else {
                return;
            };
            if Some(prefer) == state.session_provider
                || !self.availability.available.borrow().contains(&prefer)
            {
                return;
            }
            prefer
        };
        let room = headroom(prefer, self.store.as_ref(), &self.rules, (self.now)()).unwrap_or(0.0);
        if room <= 0.0 {
            return;
        }
        let mut state = self.state.borrow_mut();
        state.session_id = None;
        state.session_provider = None;
        drop(state);
        self.persist_chat();
    }

    fn plan_for(&self, text: &str, user_request: bool) -> Result<Option<Plan>, LeadError> {
        let state = self.state.borrow();
        if let Some(direct) = state.direct {
            if self.is_off(direct) {
                return Ok(None);
            }
            let resume = if state.session_provider == Some(direct) {
                state.session_id.clone()
            } else {
                None
            };
            if resume.is_none() {
                if !self.availability.available.borrow().contains(&direct) {
                    return Ok(None);
                }
                let room = headroom(direct, self.store.as_ref(), &self.rules, (self.now)())?;
                if room <= 0.0 {
                    return Ok(None);
                }
            }
            return Ok(Some(Plan {
                provider: direct,
                resume,
                prompt: text.to_string(),
            }));
        }
        let mode = self.availability.run_mode.get();
        let project = if user_request || mode == LeadMode::Legit {
            project_line(&(self.project)())
        } else {
            String::new()
        };
        let mode_line = match mode {
            LeadMode::Manual => "",
            LeadMode::Auto => concat!(
                "Auto is on: your plan runs as soon as you propose it; nobody presses Run. ",
                "It also overrides the rule above against doing the work yourself: you have your CLI's own tools ",
                "and the person's own setup (commands, edits, their MCP servers and skills) as far as your CLI's auto mode allows. ",
                "Do quick things yourself in the open project, and hand bigger or parallel work out as jobs."
            ),
            LeadMode::Legit => concat!(
                "LEG IT! is on, and it overrides every rule above about what you may do. ",
                "You have every tool with no prompts, and the open project is yours to change and run commands in directly. ",
                "Do the work yourself, hand jobs out, or both: whatever finishes fastest. ",
                "Plans you propose run at once with full access; nobody presses Run or approves anything. ",
                "Follow the legate-leg-it skill. In short: decide the check that proves the work done, loop until it passes, ",
                "never stop to ask, and reply only \"Working.\" until then; report once at the end. ",
                "Even so: stay inside the project folder, never push or rewrite git history, never touch credentials or sign in, ",
                "and stop and say so if the work needs any of that."
            ),
        };
        let line = [project.as_str(), mode_line]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if let (Some(provider), Some(session)) = (state.session_provider, state.session_id.clone())
            && !self.is_off(provider)
        {
            return Ok(Some(Plan {
                provider,
                resume: Some(session),
                prompt: if line.is_empty() {
                    text.to_string()
                } else {
                    format!("{line}\n\n{text}")
                },
            }));
        }
        drop(state);
        let Some(provider) = self.choose_provider()? else {
            return Ok(None);
        };
        let prompt = if line.is_empty() {
            format!("{LEAD_INSTRUCTIONS}\n\nUser request:\n{text}")
        } else {
            format!("{LEAD_INSTRUCTIONS}\n\n{line}\n\nUser request:\n{text}")
        };
        Ok(Some(Plan {
            provider,
            resume: None,
            prompt,
        }))
    }

    fn choose_provider(&self) -> Result<Option<ProviderId>, LeadError> {
        let available = self.availability.available.borrow();
        let mut rooms = Vec::new();
        let mut any = false;
        for id in available.iter().filter(|id| !self.is_off(**id)) {
            let room = headroom(*id, self.store.as_ref(), &self.rules, (self.now)())?;
            if room > 0.0 {
                any = true;
            }
            rooms.push((*id, room));
        }
        if !any {
            return Ok(None);
        }
        if let Some(prefer) = self.preferred()
            && rooms
                .iter()
                .find(|(id, _)| *id == prefer)
                .is_some_and(|(_, room)| *room > 0.0)
        {
            return Ok(Some(prefer));
        }
        let mut best: Option<(ProviderId, f64)> = None;
        for (id, room) in rooms {
            let replace = match best {
                None => false,
                Some((_, best_room)) => {
                    room > best_room
                        || ((room - best_room).abs() < f64::EPSILON
                            && room > 0.0
                            && id == ProviderId::Cursor)
                }
            };
            if best.is_none() && room > 0.0 || replace {
                best = Some((id, room));
            }
        }
        Ok(best.map(|(id, _)| id))
    }

    async fn turn_with_failover(&self, lead_id: &str, text: &str) {
        self.turn(lead_id, text).await;
        // Each limit rests its plan, so this ends; the cap only guards a store that won't save.
        for _ in 0..legate_core::PROVIDERS.len() {
            if !self.retry_on_next_plan(lead_id) {
                break;
            }
            self.turn(lead_id, text).await;
        }
    }

    /// Clear a turn that hit a limit before answering, when another plan can take it.
    fn retry_on_next_plan(&self, lead_id: &str) -> bool {
        if self.state.borrow_mut().retry_turn.take().as_deref() != Some(lead_id) {
            return false;
        }
        if !matches!(self.choose_provider(), Ok(Some(_))) {
            return false;
        }
        if let Some(message) = self
            .state
            .borrow_mut()
            .history
            .iter_mut()
            .find(|item| item.id == lead_id)
        {
            message.status = LeadStatus::Streaming;
            message.text.clear();
            message.provider = None;
            message.model = None;
        }
        self.emit_id(lead_id);
        true
    }

    async fn turn(&self, lead_id: &str, text: &str) {
        let user_request =
            self.message(lead_id).and_then(|message| message.kind) != Some(LeadMessageKind::Report);
        self.follow_preference();
        let plan = match self.plan_for(text, user_request) {
            Ok(plan) => plan,
            Err(err) => {
                self.fail_turn(lead_id, &err.to_string());
                return;
            }
        };
        let Some(plan) = plan else {
            let direct = self.state.borrow().direct;
            let text = match direct {
                None => NO_PLAN.to_string(),
                Some(provider) => format!("{} is not available right now.", provider.as_str()),
            };
            self.fail_turn(lead_id, &text);
            return;
        };
        {
            let mut state = self.state.borrow_mut();
            if let Some(message) = state.history.iter_mut().find(|item| item.id == lead_id) {
                message.provider = Some(plan.provider);
            }
        }
        self.emit_id(lead_id);
        let started_at = (self.now)();
        let started = Instant::now();
        let Some(adapter) = self
            .adapters
            .iter()
            .find(|adapter| adapter.id() == plan.provider)
            .cloned()
        else {
            self.fail_turn(
                lead_id,
                &format!("no adapter for {}", plan.provider.as_str()),
            );
            self.record(
                lead_id,
                plan.provider,
                started_at,
                started,
                None,
                None,
                None,
                plan.resume.clone(),
                RunOutcome::Error,
            );
            return;
        };
        let direct = self.state.borrow().direct.is_some();
        let mut opts = RunOptions {
            resume: plan.resume.clone(),
            model: self
                .model_for
                .as_ref()
                .and_then(|model_for| model_for(plan.provider)),
            ..RunOptions::default()
        };
        let mode = self.availability.run_mode.get();
        opts.auto = (mode == LeadMode::Auto).then_some(true);
        if direct {
            // A direct chat (one of Legate's, or a session mirrored from the Claude app or a CLI)
            // works as far as the Lead mode lets it. A headless turn can't ask for approval, so
            // Manual stays read-only.
            opts.access = match mode {
                LeadMode::Manual => None,
                LeadMode::Auto => Some(JobAccess::Edit),
                LeadMode::Legit => Some(JobAccess::Full),
            };
            // Legate's job tools come along, with the person's own setup. Cursor reads MCP
            // config only from the folder it runs in, the person's project here, so it goes
            // without rather than writing Legate's files there.
            if matches!(plan.provider, ProviderId::Claude | ProviderId::Codex) {
                opts.bridge = Some(self.bridge.info());
            }
        } else {
            if mode == LeadMode::Legit {
                opts.access = Some(JobAccess::Full);
            }
            opts.bridge = Some(self.bridge.info());
            let project = (self.project)();
            opts.read_dirs = Some(if project.is_empty() {
                Vec::new()
            } else {
                vec![project]
            });
        }
        // A direct chat runs where its session lives: the project, or, for a chat mirrored from a
        // session that ran in one of the project's worktrees, that folder. Claude Code finds a
        // session to resume by the folder it runs in.
        let cwd = if direct {
            let chat_id = self.state.borrow().chat_id.clone();
            chat_id
                .and_then(|id| self.store.chat(&id).ok().flatten())
                .and_then(|row| row.cwd)
                .filter(|dir| std::path::Path::new(dir).is_dir())
                .unwrap_or_else(|| (self.project)())
        } else {
            self.dir.clone()
        };
        let mut session =
            match adapter.run(lead_id, &plan.prompt, std::path::Path::new(&cwd), &opts) {
                Ok(session) => session,
                Err(err) => {
                    self.fail_turn(lead_id, &err);
                    self.record(
                        lead_id,
                        plan.provider,
                        started_at,
                        started,
                        None,
                        None,
                        None,
                        plan.resume.clone(),
                        RunOutcome::Error,
                    );
                    return;
                }
            };
        let mut result: Option<legate_core::AgentEvent> = None;
        let mut limit_resets: Option<Option<i64>> = None;
        let mut session_id = plan.resume.clone();
        let mut cost = None;
        let mut tokens = None;
        let mut utilization = None;
        let (signal, mut stop) = watch::channel(false);
        self.state.borrow_mut().stop_turn = Some(signal);
        loop {
            let event = tokio::select! {
                biased;
                _ = stop.changed() => break,
                next = session.recv() => next,
            };
            let Some(event) = event else { break };
            match event {
                legate_core::AgentEvent::Init {
                    session_id: id,
                    model,
                } => {
                    session_id = Some(id.clone());
                    {
                        let mut state = self.state.borrow_mut();
                        state.session_id = Some(id);
                        state.session_provider = Some(plan.provider);
                    }
                    self.persist_chat();
                    if let Some(model) = model {
                        if let Some(message) = self
                            .state
                            .borrow_mut()
                            .history
                            .iter_mut()
                            .find(|item| item.id == lead_id)
                        {
                            message.model = Some(model);
                        }
                        self.emit_id(lead_id);
                    }
                }
                legate_core::AgentEvent::Text { text } => self.append(lead_id, &text),
                legate_core::AgentEvent::Usage {
                    utilization: used,
                    windows,
                    ..
                } => {
                    utilization = Some(used);
                    if let Some(windows) = windows {
                        let rows: Vec<NewWindow> = windows
                            .into_iter()
                            .map(|window| NewWindow {
                                name: window.name,
                                utilization: window.utilization,
                                resets_at: window.resets_at.map(|seconds| seconds * 1000),
                            })
                            .collect();
                        if let Err(err) = self.store.save_windows(
                            plan_of(plan.provider).as_str(),
                            &rows,
                            (self.now)(),
                        ) {
                            tracing::error!("{err}");
                        }
                    }
                }
                legate_core::AgentEvent::Result {
                    ok,
                    text,
                    session_id: id,
                    cost_usd,
                    tokens: used,
                    ..
                } => {
                    if let Some(id) = id.clone() {
                        session_id = Some(id.clone());
                        let mut state = self.state.borrow_mut();
                        state.session_id = Some(id);
                        state.session_provider = Some(plan.provider);
                        drop(state);
                        self.persist_chat();
                    }
                    if cost_usd.is_some() {
                        cost = cost_usd;
                    }
                    if used.is_some() {
                        tokens = used;
                    }
                    let empty = self
                        .message(lead_id)
                        .is_some_and(|message| message.text.is_empty());
                    if empty && !text.is_empty() {
                        self.append(lead_id, &text);
                    }
                    if !ok {
                        self.set_status(lead_id, LeadStatus::Error);
                        self.emit_id(lead_id);
                    }
                    result = Some(legate_core::AgentEvent::Result {
                        ok,
                        text,
                        session_id: id,
                        duration_ms: None,
                        cost_usd,
                        tokens: used,
                    });
                }
                legate_core::AgentEvent::Limit { resets_at, .. } => {
                    limit_resets = Some(resets_at);
                    break;
                }
                legate_core::AgentEvent::Step { .. } => {}
            }
        }
        if limit_resets.is_some() || *stop.borrow() {
            session.kill();
        }
        let exit = tokio::select! {
            biased;
            _ = stop.changed(), if !*stop.borrow() => {
                session.kill();
                session.exit_status().await
            }
            exit = session.exit_status() => exit,
        };
        self.state.borrow_mut().stop_turn = None;
        if *stop.borrow() {
            self.set_status(lead_id, LeadStatus::Done);
            self.keep_session(plan.provider, session_id.clone());
            self.append(lead_id, "[Stopped.]");
            self.record(
                lead_id,
                plan.provider,
                started_at,
                started,
                cost,
                tokens,
                utilization,
                session_id,
                RunOutcome::Ok,
            );
            return;
        }
        let stderr_limit = limit_resets.is_none()
            && result.is_none()
            && exit.code != Some(0)
            && adapter.is_limit_error(&exit.stderr);
        if limit_resets.is_some() || stderr_limit {
            if stderr_limit {
                session.kill();
            }
            let model = self.message(lead_id).and_then(|message| message.model);
            self.apply_limit(
                lead_id,
                plan.provider,
                model.as_deref().or(opts.model.as_deref()),
                limit_resets.flatten(),
            );
            self.record(
                lead_id,
                plan.provider,
                started_at,
                started,
                cost,
                tokens,
                utilization,
                session_id,
                RunOutcome::Limit,
            );
            return;
        }
        if let Some(legate_core::AgentEvent::Result {
            ok: false, text, ..
        }) = &result
        {
            self.set_status(lead_id, LeadStatus::Error);
            if self
                .message(lead_id)
                .is_some_and(|message| message.text.is_empty())
            {
                self.set_text(lead_id, text);
            }
            self.keep_session(plan.provider, session_id.clone());
            self.emit_id(lead_id);
            self.record(
                lead_id,
                plan.provider,
                started_at,
                started,
                cost,
                tokens,
                utilization,
                session_id,
                RunOutcome::Error,
            );
            return;
        }
        if result.is_none() && exit.code != Some(0) {
            self.set_status(lead_id, LeadStatus::Error);
            if self
                .message(lead_id)
                .is_some_and(|message| message.text.is_empty())
            {
                self.set_text(lead_id, &stderr_tail(&exit.stderr));
            }
            self.keep_session(plan.provider, session_id.clone());
            self.emit_id(lead_id);
            self.record(
                lead_id,
                plan.provider,
                started_at,
                started,
                cost,
                tokens,
                utilization,
                session_id,
                RunOutcome::Error,
            );
            return;
        }
        self.set_status(lead_id, LeadStatus::Done);
        self.keep_session(plan.provider, session_id.clone());
        self.emit_id(lead_id);
        self.record(
            lead_id,
            plan.provider,
            started_at,
            started,
            cost,
            tokens,
            utilization,
            session_id,
            RunOutcome::Ok,
        );
    }

    fn fail_turn(&self, lead_id: &str, text: &str) {
        if let Some(message) = self
            .state
            .borrow_mut()
            .history
            .iter_mut()
            .find(|item| item.id == lead_id)
        {
            message.status = LeadStatus::Error;
            if message.text.is_empty() {
                message.text = text.to_string();
            }
        }
        self.emit_id(lead_id);
    }

    fn apply_limit(
        &self,
        lead_id: &str,
        provider: ProviderId,
        model: Option<&str>,
        resets_at: Option<i64>,
    ) {
        let credits = needs_usage_credits(provider, model);
        let notice = if credits {
            "[Fable is out of usage credits. Pick another Claude model to continue.]".to_string()
        } else {
            let now = (self.now)();
            let default = now + (self.rules.default_rest_hours * 3_600_000.0) as i64;
            let until = match resets_at {
                Some(seconds) if seconds != 0 => seconds.saturating_mul(1000),
                _ => rest_after_limit(
                    self.store.as_ref(),
                    plan_of(provider),
                    self.rules.default_rest_hours,
                    now,
                )
                .unwrap_or(default),
            };
            if let Err(err) = self.store.set_resting(plan_of(provider).as_str(), until) {
                tracing::error!("{err}");
            }
            format!(
                "[The lead's plan ({}) hit its usage limit. Send your message again to continue on another plan.]",
                provider.as_str()
            )
        };
        let mut answered = false;
        if let Some(message) = self
            .state
            .borrow_mut()
            .history
            .iter_mut()
            .find(|item| item.id == lead_id)
        {
            answered = !message.text.is_empty();
            message.status = LeadStatus::Error;
            message.text = if credits || message.text.is_empty() {
                notice
            } else {
                format!("{}\n{notice}", message.text)
            };
        }
        {
            let mut state = self.state.borrow_mut();
            if !credits && !answered && state.direct.is_none() {
                state.retry_turn = Some(lead_id.to_string());
            }
            state.session_id = None;
            state.session_provider = None;
        }
        self.persist_chat();
        self.emit_id(lead_id);
    }

    fn keep_session(&self, provider: ProviderId, session_id: Option<String>) {
        let mut state = self.state.borrow_mut();
        if let Some(session_id) = session_id {
            state.session_id = Some(session_id);
            state.session_provider = Some(provider);
        } else {
            state.session_id = None;
            state.session_provider = None;
        }
        drop(state);
        self.persist_chat();
    }

    fn append(&self, lead_id: &str, text: &str) {
        if let Some(message) = self
            .state
            .borrow_mut()
            .history
            .iter_mut()
            .find(|item| item.id == lead_id)
        {
            if message.text.is_empty() {
                message.text = text.to_string();
            } else {
                message.text = format!("{}\n{text}", message.text);
            }
        }
        self.emit_id(lead_id);
    }

    fn set_status(&self, lead_id: &str, status: LeadStatus) {
        if let Some(message) = self
            .state
            .borrow_mut()
            .history
            .iter_mut()
            .find(|item| item.id == lead_id)
        {
            message.status = status;
        }
    }

    fn set_text(&self, lead_id: &str, text: &str) {
        if let Some(message) = self
            .state
            .borrow_mut()
            .history
            .iter_mut()
            .find(|item| item.id == lead_id)
        {
            message.text = text.to_string();
        }
    }

    fn message(&self, id: &str) -> Option<LeadMessage> {
        self.state
            .borrow()
            .history
            .iter()
            .find(|item| item.id == id)
            .cloned()
    }

    fn emit_id(&self, id: &str) {
        let Some(message) = self.message(id) else {
            return;
        };
        if message.status != LeadStatus::Streaming {
            self.persist_message(id);
        }
        let chat_id = self.state.borrow().chat_id.clone();
        let listeners: Vec<_> = self
            .state
            .borrow()
            .listeners
            .iter()
            .map(|(_, callback)| Rc::clone(callback))
            .collect();
        let snap = self.message(id).unwrap_or(message);
        for callback in listeners {
            callback(snap.clone(), chat_id.clone());
        }
    }

    fn on_job_update(&self, job: &JobRecord) {
        let Some(lead_message_id) = job.lead_message.clone() else {
            return;
        };
        {
            let state = self.state.borrow();
            if state.submitting.as_deref() == Some(lead_message_id.as_str())
                || state.skip_follow.contains(&lead_message_id)
                || state.follow_scheduled.contains(&lead_message_id)
            {
                return;
            }
        }
        let group = self.jobs_for(&lead_message_id);
        if group.is_empty()
            || !group
                .iter()
                .all(|item| matches!(item.status, JobStatus::Done | JobStatus::Failed))
        {
            return;
        }
        let chat_id = self
            .state
            .borrow()
            .turn_chat
            .get(&lead_message_id)
            .cloned()
            .or_else(|| self.store.chat_for_message(&lead_message_id).ok().flatten());
        let Some(chat_id) = chat_id else {
            return;
        };
        {
            let mut state = self.state.borrow_mut();
            state.follow_scheduled.insert(lead_message_id.clone());
            let order = state.follow_order;
            state.follow_order += 1;
            state.pending.push(Pending {
                chat_id,
                lead_message_id,
                order,
            });
            state.pending.sort_by_key(|item| item.order);
        }
        self.spawn_drain();
    }

    fn jobs_for(&self, lead_message_id: &str) -> Vec<JobRecord> {
        self.jobs
            .borrow()
            .list()
            .into_iter()
            .filter(|job| job.lead_message.as_deref() == Some(lead_message_id))
            .collect()
    }

    fn spawn_drain(&self) {
        let Some(this) = self.this.upgrade() else {
            return;
        };
        tokio::task::spawn_local(async move {
            this.drain().await;
        });
    }

    async fn drain(&self) {
        {
            let state = self.state.borrow();
            if state.draining || state.busy {
                return;
            }
        }
        self.state.borrow_mut().draining = true;
        loop {
            let next = {
                let mut state = self.state.borrow_mut();
                if state.busy || state.pending.is_empty() {
                    break;
                }
                state.pending.remove(0)
            };
            self.run_report(&next.chat_id, &next.lead_message_id).await;
        }
        self.state.borrow_mut().draining = false;
        let again = {
            let state = self.state.borrow();
            !state.pending.is_empty() && !state.busy
        };
        if again {
            self.spawn_drain();
        }
    }

    async fn run_report(&self, chat_id: &str, source_id: &str) {
        let jobs = self.jobs_for(source_id);
        if jobs.is_empty() {
            return;
        }
        let saved = self.save_context();
        self.state.borrow_mut().busy = true;
        let mut lead = LeadMessage {
            id: fresh_id(),
            role: LeadRole::Lead,
            text: String::new(),
            provider: None,
            model: None,
            status: LeadStatus::Streaming,
            kind: Some(LeadMessageKind::Report),
            proposal: None,
            extra: Default::default(),
        };
        let lead_id = lead.id.clone();
        let prompt = build_report(&jobs, self.availability.run_mode.get() == LeadMode::Legit);
        if self.load_chat(chat_id, false).is_err()
            || self
                .message(source_id)
                .and_then(|m| m.proposal)
                .is_some_and(|p| p.status == ProposalStatus::Stopped)
        {
            self.state.borrow_mut().busy = false;
            self.restore_context(saved);
            return;
        }
        {
            let mut state = self.state.borrow_mut();
            // A follow-up continues its source request's budget, even when another
            // user message or another chat intervened.
            if let Some(rounds) = state
                .history
                .iter()
                .find(|m| m.id == source_id)
                .and_then(|m| m.extra.get(AUTO_ROUNDS))
            {
                lead.extra.insert(AUTO_ROUNDS.into(), rounds.clone());
            }
            state.history.push(lead);
            state.turn_id = Some(lead_id.clone());
            state.turn_chat.insert(lead_id.clone(), chat_id.to_string());
        }
        self.emit_id(&lead_id);
        self.turn_with_failover(&lead_id, &prompt).await;
        self.state.borrow_mut().busy = false;
        self.state.borrow_mut().turn_id = None;
        if saved
            .chat_id
            .as_ref()
            .is_none_or(|id| self.load_chat(id, false).is_err())
        {
            self.restore_context(saved);
        }
        self.spawn_drain();
    }

    fn load_chat(&self, id: &str, view: bool) -> Result<(), LeadError> {
        let row = self.store.chat(id)?;
        let Some(row) = row else {
            return Err(LeadError::message("unknown chat"));
        };
        if row.project != (self.project)() {
            return Err(LeadError::message("unknown chat"));
        }
        let history = self.store.messages(id)?;
        let mut state = self.state.borrow_mut();
        state.chat_id = Some(row.id.clone());
        if view {
            state.view_chat_id = Some(row.id);
        }
        state.chat_title = row.title;
        state.chat_created_at = row.created_at;
        state.history = history.clone();
        state.session_id = row.session_id;
        state.session_provider = chat_plan(row.session_provider.as_deref());
        state.direct = chat_plan(row.direct.as_deref());
        for message in history {
            if message.role == LeadRole::Lead {
                state.turn_chat.insert(message.id, id.to_string());
            }
        }
        Ok(())
    }

    fn save_context(&self) -> ChatContext {
        let state = self.state.borrow();
        ChatContext {
            chat_id: state.chat_id.clone(),
            history: state.history.clone(),
            session_id: state.session_id.clone(),
            session_provider: state.session_provider,
            chat_title: state.chat_title.clone(),
            chat_created_at: state.chat_created_at,
            direct: state.direct,
        }
    }

    /// Perform a synchronous operation on another chat without moving the visible selection.
    fn with_chat<T>(
        &self,
        id: &str,
        f: impl FnOnce() -> Result<T, LeadError>,
    ) -> Result<T, LeadError> {
        if self.state.borrow().chat_id.as_deref() == Some(id) {
            return f();
        }
        let saved = self.save_context();
        self.load_chat(id, false)?;
        let result = f();
        self.restore_context(saved);
        result
    }

    fn restore_context(&self, ctx: ChatContext) {
        let mut state = self.state.borrow_mut();
        state.chat_id = ctx.chat_id;
        state.history = ctx.history;
        state.session_id = ctx.session_id;
        state.session_provider = ctx.session_provider;
        state.chat_title = ctx.chat_title;
        state.chat_created_at = ctx.chat_created_at;
        state.direct = ctx.direct;
    }

    fn chat_has_pending(&self, chat_id: &str) -> Result<bool, LeadError> {
        let state = self.state.borrow();
        if state.busy && state.chat_id.as_deref() == Some(chat_id) {
            return Ok(true);
        }
        if state.pending.iter().any(|item| item.chat_id == chat_id) {
            return Ok(true);
        }
        drop(state);
        let ids: HashSet<String> = self
            .store
            .messages(chat_id)?
            .into_iter()
            .filter(|message| message.role == LeadRole::Lead)
            .map(|message| message.id)
            .collect();
        Ok(self.jobs.borrow().list().into_iter().any(|job| {
            job.lead_message.as_ref().is_some_and(|id| ids.contains(id))
                && !matches!(job.status, JobStatus::Done | JobStatus::Failed)
        }))
    }

    fn persist_message(&self, id: &str) {
        let Some(chat_id) = self.state.borrow().chat_id.clone() else {
            return;
        };
        let Some(message) = self.message(id) else {
            return;
        };
        let seq = self
            .state
            .borrow()
            .history
            .iter()
            .position(|item| item.id == id)
            .unwrap_or(0) as i64;
        if let Err(err) = self.store.save_message(&chat_id, seq, &message) {
            tracing::error!("{err}");
            return;
        }
        self.persist_chat();
    }

    fn persist_chat(&self) {
        let state = self.state.borrow();
        let Some(id) = state.chat_id.clone() else {
            return;
        };
        // What the chat sync owns (folder, source, file signature) is kept as it was.
        let kept = self.store.chat(&id).ok().flatten();
        let row = ChatRow {
            cwd: kept.as_ref().and_then(|row| row.cwd.clone()),
            source: kept.as_ref().and_then(|row| row.source.clone()),
            source_sig: kept.and_then(|row| row.source_sig),
            id,
            project: (self.project)(),
            title: state.chat_title.clone(),
            created_at: state.chat_created_at,
            updated_at: (self.now)(),
            session_id: state.session_id.clone(),
            session_provider: state.session_provider.map(|id| id.as_str().to_string()),
            direct: state.direct.map(|id| id.as_str().to_string()),
        };
        drop(state);
        if let Err(err) = self.store.save_chat(&row) {
            tracing::error!("{err}");
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn record(
        &self,
        lead_id: &str,
        provider: ProviderId,
        started_at: i64,
        started: Instant,
        cost: Option<f64>,
        tokens: Option<i64>,
        utilization: Option<f64>,
        session_id: Option<String>,
        outcome: RunOutcome,
    ) {
        let row = RunRow {
            id: None,
            job_id: lead_id.to_string(),
            provider: provider.as_str().to_string(),
            job_type: "lead".to_string(),
            started_at,
            duration_ms: Some(started.elapsed().as_millis() as i64),
            cost_usd: cost,
            tokens,
            utilization,
            outcome,
            session_id,
        };
        if let Err(err) = self.store.insert_run(&row) {
            tracing::error!("{err}");
        }
    }
}

impl Drop for Lead {
    fn drop(&mut self) {
        if let Some(unsub) = self.unsub.borrow_mut().take() {
            unsub();
        }
    }
}

struct BusyClear {
    lead: Weak<Lead>,
}

impl Drop for BusyClear {
    fn drop(&mut self) {
        let Some(lead) = self.lead.upgrade() else {
            return;
        };
        {
            let mut state = lead.state.borrow_mut();
            state.busy = false;
            state.turn_id = None;
        }
        lead.spawn_drain();
    }
}

pub fn project_line(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    let trimmed = path.trim_end_matches(['/', '\\']);
    let name = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
    format!("Open project: {name} ({path})")
}

fn chat_title(text: &str) -> String {
    let line = text.split(['\n', '\r']).next().unwrap_or("");
    let trimmed = line.trim();
    trimmed.chars().take(60).collect()
}

fn build_report(jobs: &[JobRecord], legit: bool) -> String {
    let mut lines = vec![
        if legit {
            "Worker jobs from an earlier turn have finished. LEG IT! is still on: this is not the moment to report unless the work is done."
        } else {
            "Worker jobs from an earlier turn have finished. Report to the user in a few lines."
        }.to_string(),
        String::new(),
    ];
    for job in jobs {
        let worker = job
            .provider
            .map(|id| id.as_str().to_string())
            .unwrap_or_else(|| "none".to_string());
        lines.push(format!(
            "Job {} · type {} · worker {worker} · status {}",
            job.id,
            job.job_type.as_str(),
            status_str(job.status)
        ));
        if let Some(error) = &job.error {
            lines.push(format!("Error: {error}"));
        }
        if job.edit == Some(true) && job.change.is_some() {
            lines.push("This job left a change waiting for review.".to_string());
        }
        if !job.output.is_empty() {
            if job.output.chars().count() > REPORT_OUTPUT_MAX {
                lines.push(format!("Output (cut to {REPORT_OUTPUT_MAX} characters):"));
                lines.push(tail_chars(&job.output, REPORT_OUTPUT_MAX));
            } else {
                lines.push("Output:".to_string());
                lines.push(job.output.clone());
            }
        }
        lines.push(String::new());
    }
    lines.push(if legit {
        "If the check passed and nothing is left, give your one final report. Otherwise fix it yourself or propose the next plan now, and reply with exactly \"Working.\""
    } else {
        "Summarize for the user; keep it short."
    }.to_string());
    lines.join("\n")
}

fn status_str(status: JobStatus) -> &'static str {
    match status {
        JobStatus::Queued => "queued",
        JobStatus::Running => "running",
        JobStatus::Done => "done",
        JobStatus::Failed => "failed",
    }
}

#[cfg(test)]
#[path = "lead_tests.rs"]
mod lead_tests;

fn stderr_tail(stderr: &str) -> String {
    let trimmed = stderr.trim();
    if trimmed.chars().count() <= 500 {
        trimmed.to_string()
    } else {
        tail_chars(trimmed, 500)
    }
}
