use std::collections::BTreeMap;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use async_trait::async_trait;
use futures_core::Stream;
use legate_core::{AgentEvent, ProviderId, RunOptions};
use tokio::sync::{mpsc::UnboundedReceiver, watch};

/// Executable to spawn. Extra args are placed before the per-call args.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bin {
    pub command: String,
    pub args: Vec<String>,
}

impl Bin {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
        }
    }

    pub fn with_args(
        command: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            command: command.into(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }
}

/// How `spawn` finished. `unreadable` is set when the CLI exited 0 without a `result` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessExit {
    pub code: Option<i32>,
    pub stderr: String,
    pub unreadable: bool,
}

/// The TypeScript `isLimitError` overload: a parsed event, or stderr text.
pub enum LimitError<'a> {
    Event(&'a AgentEvent),
    Text(&'a str),
}

/// Options for an interactive CLI session (no `-p`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InteractiveOptions {
    pub model: Option<String>,
    pub session_id: Option<String>,
    pub resume: Option<String>,
    pub prompt: Option<String>,
}

/// Command line for an interactive session. `env` is set only when the CLI needs a filtered environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractiveCommand {
    pub command: String,
    pub args: Vec<String>,
    pub env: Option<BTreeMap<String, String>>,
}

/// Parsed events in order. The stream ends when the process closes and stdout has been read.
pub struct EventStream {
    pub(crate) rx: UnboundedReceiver<AgentEvent>,
}

impl EventStream {
    pub async fn recv(&mut self) -> Option<AgentEvent> {
        self.rx.recv().await
    }
}

impl Stream for EventStream {
    type Item = AgentEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().rx.poll_recv(cx)
    }
}

/// A running CLI. `exit` resolves even when spawn failed. `kill` ends the whole process tree.
#[must_use]
pub struct RunHandle {
    pub events: EventStream,
    exit_rx: watch::Receiver<Option<ProcessExit>>,
    kill: Arc<dyn Fn() + Send + Sync>,
}

impl RunHandle {
    pub(crate) fn new(
        events: EventStream,
        exit_rx: watch::Receiver<Option<ProcessExit>>,
        kill: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self {
            events,
            exit_rx,
            kill,
        }
    }

    pub(crate) fn failed(message: String) -> Self {
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let (_exit_tx, exit_rx) = watch::channel(Some(ProcessExit {
            code: None,
            stderr: message,
            unreadable: false,
        }));
        Self::new(EventStream { rx }, exit_rx, Arc::new(|| {}))
    }

    pub async fn exit(&self) -> ProcessExit {
        let mut rx = self.exit_rx.clone();
        loop {
            if let Some(exit) = rx.borrow().clone() {
                return exit;
            }
            if rx.changed().await.is_err() {
                return ProcessExit {
                    code: None,
                    stderr: "provider run ended without a status".to_string(),
                    unreadable: false,
                };
            }
        }
    }

    /// Kills the CLI and every process it started.
    pub fn kill(&self) {
        (self.kill)();
    }
}

/// One official CLI Legate can run headless.
#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn id(&self) -> ProviderId;

    async fn is_installed(&self) -> bool;

    /// Token from `--version`, or `None` when the CLI did not print one.
    async fn version(&self) -> Option<String>;

    async fn is_signed_in(&self) -> bool;

    /// Spawns the CLI. `job_id` is the Legate job; the prompt is written to stdin.
    fn run(&self, job_id: &str, prompt: &str, cwd: &Path, opts: &RunOptions) -> RunHandle;

    /// First event on a stream-json line, if it has one. Lines can carry more than one;
    /// the adapters' `parse_events` returns the full list the run stream emits.
    fn parse_event(&self, line: &str) -> Option<AgentEvent>;

    /// True for a `limit` event, or for stderr text that says the plan is spent.
    fn is_limit_error(&self, error: LimitError<'_>) -> bool;
}
