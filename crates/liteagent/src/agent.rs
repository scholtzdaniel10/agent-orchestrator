//! Cascade and the append-only agent loop.
//!
//! Each step goes to the small model first. It can call a tool, finish, or pick
//! `ask_bigger_model`. Invalid JSON, an unknown tool, or a repeated call also
//! hands the step to the big model. With no small model, the big model does
//! every step.
//!
//! The conversation only grows at the end, so the rendered prompt keeps a stable
//! prefix and llama-server's `cache_prompt` can reuse it. A repeated call is not
//! executed again: the model is reminded of the result, and the grammar then
//! allows only `final_answer`.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Instant;

use futures_core::Stream;
use serde_json::Value;

use crate::ServerKind;
use crate::client::{Backend, CallStats, Message};
use crate::error::Result;
use crate::prefix_store::{PrefixStats, PrefixStore};
use crate::toolpath::{
    ESCALATE, FINAL, Step, Tool, ToolSpec, finish_specs, parse_step, specs_for, step_schema,
    tools_prompt,
};

/// Why the small model did not keep the step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EscalationReason {
    /// The small model did not return a JSON step.
    Invalid,
    /// The small model called `ask_bigger_model`.
    Asked {
        /// Reason text from the model, when it sent one.
        reason: String,
    },
    /// The small model named a tool that is not registered.
    UnknownTool {
        /// Tool name from the model.
        name: String,
    },
    /// The small model repeated a call it had already run.
    Repeated {
        /// Tool name that was repeated.
        name: String,
    },
}

/// Progress events from [`Agent::run_stream`].
#[derive(Debug, Clone)]
pub enum StepEvent {
    /// The model chose a tool that will be executed.
    ToolCall {
        /// `big` or `small`.
        model: String,
        /// The model's one-line thought.
        thought: String,
        /// Tool name.
        name: String,
        /// Arguments object.
        args: Value,
    },
    /// The tool returned.
    ToolResult {
        /// Tool name.
        name: String,
        /// Text fed back to the model.
        result: String,
    },
    /// The small model handed this step to the big model.
    Escalation {
        /// Why the step was handed up.
        reason: EscalationReason,
    },
    /// Loop guard: this call was not run again.
    Repeat {
        /// Tool name.
        name: String,
        /// Result from the earlier call.
        prior_result: String,
    },
    /// The loop finished. `answer` is `None` when the step limit was hit.
    Final {
        /// Model that produced the answer, if there was one.
        model: String,
        /// Final answer text.
        answer: Option<String>,
        /// Timings and counters for the whole task.
        stats: RunStats,
    },
}

/// Totals for one [`Agent::run`].
#[derive(Debug, Clone)]
pub struct RunStats {
    /// One entry per model step, not including the prefix-build call.
    pub calls: Vec<CallStats>,
    /// Times the small model handed a step to the big model.
    pub escalations: u32,
    /// Steps that were not valid JSON tool calls.
    pub invalid: u32,
    /// Repeated tool calls caught by the loop guard.
    pub repeats: u32,
    /// Wall time of the task, seconds.
    pub wall_s: f64,
    /// Prompt tokens processed (not reused).
    pub prompt_tokens: u64,
    /// Prompt tokens reused from a cache.
    pub cached_tokens: u64,
    /// Sum of time-to-first-token across steps, milliseconds.
    pub wait_before_step_ms: f64,
    /// Mean time-to-first-token, milliseconds. Zero when there were no calls.
    pub avg_wait_before_step_ms: f64,
    /// Generated tokens.
    pub gen_tokens: u64,
    /// Generation time, milliseconds.
    pub gen_ms: f64,
    /// Generation speed across the task.
    pub tokens_per_sec: f64,
}

impl RunStats {
    /// `cached / (processed + cached)`. Zero when both are zero.
    pub fn reuse_ratio(&self) -> f64 {
        let total = self.prompt_tokens + self.cached_tokens;
        if total == 0 {
            0.0
        } else {
            self.cached_tokens as f64 / total as f64
        }
    }

    fn from_parts(
        calls: Vec<CallStats>,
        escalations: u32,
        invalid: u32,
        repeats: u32,
        wall_s: f64,
    ) -> Self {
        let prompt_tokens = calls.iter().map(|c| c.prompt_tokens).sum();
        let cached_tokens = calls.iter().map(|c| c.cached_tokens).sum();
        let wait_before_step_ms = calls.iter().map(|c| c.prompt_ms).sum();
        let gen_tokens = calls.iter().map(|c| c.gen_tokens).sum();
        let gen_ms = calls.iter().map(|c| c.gen_ms).sum();
        let avg_wait_before_step_ms = if calls.is_empty() {
            0.0
        } else {
            wait_before_step_ms / calls.len() as f64
        };
        let tokens_per_sec = if gen_ms > 0.0 {
            gen_tokens as f64 / (gen_ms / 1000.0)
        } else {
            0.0
        };
        Self {
            calls,
            escalations,
            invalid,
            repeats,
            wall_s,
            prompt_tokens,
            cached_tokens,
            wait_before_step_ms,
            avg_wait_before_step_ms,
            gen_tokens,
            gen_ms,
            tokens_per_sec,
        }
    }
}

/// Result of [`Agent::run`].
#[derive(Debug, Clone)]
pub struct AgentOutcome {
    /// `final_answer` text, or `None` if the step budget ran out.
    pub answer: Option<String>,
    /// Accepted steps, including the final one.
    pub steps: Vec<Step>,
    /// Timings and counters.
    pub stats: RunStats,
}

struct Brain {
    server: Backend,
    system: String,
    schema: Value,
    specs: Vec<ToolSpec>,
    store: PrefixStore,
}

impl Brain {
    fn new(
        server: Backend,
        base_system: &str,
        tools: &[Tool],
        escalate: bool,
        save_dir: Option<std::path::PathBuf>,
    ) -> Self {
        let kind = server.kind();
        Self {
            store: PrefixStore::new(kind, save_dir),
            system: full_system(base_system, tools, escalate),
            schema: step_schema(tools, escalate),
            specs: specs_for(tools, escalate),
            server,
        }
    }

    fn retarget(&mut self, base_system: &str, tools: &[Tool], escalate: bool) {
        self.system = full_system(base_system, tools, escalate);
        self.schema = step_schema(tools, escalate);
        self.specs = specs_for(tools, escalate);
    }

    async fn shared_prefix(&mut self) -> Result<String> {
        let system = self.system.clone();
        let a = self.server.render(&pair(&system, "A")).await?;
        let b = self.server.render(&pair(&system, "B")).await?;
        Ok(common_prefix(&a, &b))
    }

    async fn step(
        &mut self,
        convo: &[Message],
        n_predict: u32,
        schema: Option<&Value>,
        specs: Option<&[ToolSpec]>,
    ) -> Result<(Option<Step>, String, CallStats)> {
        let schema_owned = schema.cloned().unwrap_or_else(|| self.schema.clone());
        let specs_owned = specs
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| self.specs.clone());
        if self.server.kind() == ServerKind::LlamaServer && self.server.cache() {
            let prefix = self.shared_prefix().await?;
            self.store.ensure(&mut self.server, &prefix).await?;
        }
        let messages = self.with_system(convo);
        let (text, stats) = match self.server.kind() {
            ServerKind::LlamaServer => {
                let prompt = self.server.render(&messages).await?;
                self.server
                    .complete_prompt(&prompt, Some(&schema_owned), n_predict)
                    .await?
            }
            ServerKind::Ollama | ServerKind::OpenAi => {
                self.server
                    .complete_chat(&messages, &schema_owned, &specs_owned, n_predict)
                    .await?
            }
        };
        Ok((parse_step(&text), text, stats))
    }

    fn with_system(&self, convo: &[Message]) -> Vec<Message> {
        let mut messages = Vec::with_capacity(convo.len() + 1);
        messages.push(Message {
            role: "system".to_string(),
            content: self.system.clone(),
        });
        messages.extend(convo.iter().cloned());
        messages
    }
}

fn full_system(base: &str, tools: &[Tool], escalate: bool) -> String {
    format!("{base}\n\n{}", tools_prompt(tools, escalate))
}

fn pair(system: &str, user: &str) -> Vec<Message> {
    vec![
        Message {
            role: "system".to_string(),
            content: system.to_string(),
        },
        Message {
            role: "user".to_string(),
            content: user.to_string(),
        },
    ]
}

fn common_prefix(a: &str, b: &str) -> String {
    let n = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
    a.chars().take(n).collect()
}

/// The agent loop. One big backend, an optional small backend, and the tools.
pub struct Agent {
    base_system: String,
    tools: Vec<Tool>,
    big: Brain,
    small: Option<Brain>,
    max_steps: usize,
    n_predict: u32,
    verbose: bool,
    finish_schema: Value,
    finish_specs: Vec<ToolSpec>,
}

impl Agent {
    /// `system` is the base prompt. The tool list is appended, with
    /// `ask_bigger_model` added only for the small model.
    pub fn new(
        big: Backend,
        small: Option<Backend>,
        tools: Vec<Tool>,
        system: impl Into<String>,
    ) -> Self {
        let base_system = system.into();
        let save_dir = big
            .slot_dir()
            .or_else(|| small.as_ref().and_then(Backend::slot_dir));
        let big_brain = Brain::new(big, &base_system, &tools, false, save_dir.clone());
        let small_brain =
            small.map(|server| Brain::new(server, &base_system, &tools, true, save_dir));
        Self {
            base_system,
            tools,
            big: big_brain,
            small: small_brain,
            max_steps: 8,
            n_predict: 384,
            verbose: false,
            finish_schema: step_schema(&[], false),
            finish_specs: finish_specs(),
        }
    }

    /// Step budget. The Python default is 8.
    pub fn with_max_steps(mut self, max_steps: usize) -> Self {
        self.max_steps = max_steps;
        self
    }

    /// Generation cap per step. The Python default is 384.
    pub fn with_n_predict(mut self, n_predict: u32) -> Self {
        self.n_predict = n_predict;
        self
    }

    /// Print each accepted step to stdout, same shape as the Python runtime.
    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    /// Add or replace a host tool and rebuild the grammars.
    ///
    /// `final_answer` and `ask_bigger_model` stay reserved and are not tools.
    pub fn register_tool(&mut self, tool: Tool) {
        if let Some(existing) = self.tools.iter_mut().find(|t| t.name == tool.name) {
            *existing = tool;
        } else {
            self.tools.push(tool);
        }
        self.big.retarget(&self.base_system, &self.tools, false);
        if let Some(small) = &mut self.small {
            small.retarget(&self.base_system, &self.tools, true);
        }
    }

    /// Prefix-store counters for the big model.
    pub fn prefix_stats(&self) -> PrefixStats {
        self.big.store.stats
    }

    /// Prefix-store counters for the small model, when there is one.
    pub fn small_prefix_stats(&self) -> Option<PrefixStats> {
        self.small.as_ref().map(|brain| brain.store.stats)
    }

    /// Run `task` to completion.
    pub async fn run(&mut self, task: &str) -> Result<AgentOutcome> {
        self.run_events(task, |_| {}).await
    }

    /// Same loop as [`Agent::run`], yielding each step as it happens.
    ///
    /// The last item is [`StepEvent::Final`]. Poll the stream (or call
    /// [`RunStream::next_event`]) until it returns `None`.
    pub fn run_stream(&mut self, task: impl Into<String>) -> RunStream<'_> {
        let task = task.into();
        let queue = Arc::new(Mutex::new(VecDeque::new()));
        let emit_to = Arc::clone(&queue);
        let fut = async move {
            if let Err(err) = self
                .run_events(&task, |event| enqueue(&emit_to, Ok(event)))
                .await
            {
                enqueue(&emit_to, Err(err));
            }
        };
        RunStream {
            fut: Box::pin(fut),
            queue,
            done: false,
        }
    }

    async fn run_events(
        &mut self,
        task: &str,
        mut emit: impl FnMut(StepEvent),
    ) -> Result<AgentOutcome> {
        let started = Instant::now();
        let mut steps = Vec::new();
        let mut calls = Vec::new();
        let mut escalations = 0u32;
        let mut invalid = 0u32;
        let mut repeats = 0u32;
        let mut answer: Option<String> = None;
        let mut convo = vec![Message {
            role: "user".to_string(),
            content: task.to_string(),
        }];
        let mut results: Vec<(String, String)> = Vec::new();
        let mut force_finish = false;

        for _ in 0..self.max_steps {
            let schema = if force_finish {
                Some(self.finish_schema.clone())
            } else {
                None
            };
            let specs = if force_finish {
                Some(self.finish_specs.clone())
            } else {
                None
            };

            let mut step: Option<Step> = None;
            if let Some((parsed, stats)) = self
                .ask_small(convo.as_slice(), schema.as_ref(), specs.as_deref())
                .await?
            {
                calls.push(stats);
                if parsed.is_none() {
                    invalid += 1;
                }
                if let Some(reason) = small_failure(&parsed, &self.tools, &results, &mut repeats) {
                    escalations += 1;
                    emit(StepEvent::Escalation { reason });
                } else {
                    step = parsed;
                }
            }

            if step.is_none() {
                let (parsed, raw, stats) = self
                    .big
                    .step(
                        convo.as_slice(),
                        self.n_predict,
                        schema.as_ref(),
                        specs.as_deref(),
                    )
                    .await?;
                calls.push(stats);
                if parsed.is_none() {
                    invalid += 1;
                    convo.push(Message {
                        role: "assistant".to_string(),
                        content: raw,
                    });
                    convo.push(Message {
                        role: "user".to_string(),
                        content: "That was not valid. Reply with one JSON step.".to_string(),
                    });
                    continue;
                }
                step = parsed;
            }

            let Some(step) = step else {
                continue;
            };
            steps.push(step.clone());
            if self.verbose {
                let model = calls.last().map(|c| c.model.as_str()).unwrap_or("");
                let args = step.args_json();
                let shown: String = args.chars().take(120).collect();
                println!("  [{model}] {} {shown}", step.tool);
            }
            convo.push(Message {
                role: "assistant".to_string(),
                content: step.to_json(),
            });
            if step.tool == FINAL {
                answer = Some(answer_of(&step));
                break;
            }
            let signature = step.signature();
            if let Some(prior) = lookup(&results, &signature) {
                repeats += 1;
                force_finish = true;
                emit(StepEvent::Repeat {
                    name: step.tool.clone(),
                    prior_result: prior.to_string(),
                });
                convo.push(Message {
                    role: "user".to_string(),
                    content: format!(
                        "You already ran this and got: {prior}\nDo not repeat it. Give the final answer now using final_answer."
                    ),
                });
                continue;
            }
            let model = calls.last().map(|c| c.model.clone()).unwrap_or_default();
            emit(StepEvent::ToolCall {
                model,
                thought: step.thought.clone(),
                name: step.tool.clone(),
                args: step.args.clone(),
            });
            let result = self.run_tool(&step);
            results.push((signature, result.clone()));
            emit(StepEvent::ToolResult {
                name: step.tool.clone(),
                result: result.clone(),
            });
            convo.push(Message {
                role: "user".to_string(),
                content: format!(
                    "Tool result:\n{result}\nIf this answers the task, call final_answer next."
                ),
            });
        }

        let stats = RunStats::from_parts(
            calls,
            escalations,
            invalid,
            repeats,
            started.elapsed().as_secs_f64(),
        );
        let model = stats
            .calls
            .last()
            .map(|c| c.model.clone())
            .unwrap_or_default();
        emit(StepEvent::Final {
            model,
            answer: answer.clone(),
            stats: stats.clone(),
        });
        Ok(AgentOutcome {
            answer,
            steps,
            stats,
        })
    }

    fn run_tool(&self, step: &Step) -> String {
        match self.tools.iter().find(|tool| tool.name == step.tool) {
            Some(tool) => tool.call(&step.args),
            None => format!("error: unknown tool {}", step.tool),
        }
    }

    async fn ask_small(
        &mut self,
        convo: &[Message],
        schema: Option<&Value>,
        specs: Option<&[ToolSpec]>,
    ) -> Result<Option<(Option<Step>, CallStats)>> {
        let Some(small) = self.small.as_mut() else {
            return Ok(None);
        };
        let (parsed, _raw, stats) = small.step(convo, self.n_predict, schema, specs).await?;
        Ok(Some((parsed, stats)))
    }
}

fn remembered(results: &[(String, String)], signature: &str) -> bool {
    results.iter().any(|(sig, _)| sig == signature)
}

fn lookup<'a>(results: &'a [(String, String)], signature: &str) -> Option<&'a str> {
    results
        .iter()
        .find(|(sig, _)| sig == signature)
        .map(|(_, result)| result.as_str())
}

/// `None` means the small model produced a step the loop should keep.
fn small_failure(
    parsed: &Option<Step>,
    tools: &[Tool],
    results: &[(String, String)],
    repeats: &mut u32,
) -> Option<EscalationReason> {
    let Some(step) = parsed else {
        return Some(EscalationReason::Invalid);
    };
    if step.tool == ESCALATE {
        let reason = step
            .args
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        return Some(EscalationReason::Asked { reason });
    }
    let known = step.tool == FINAL || tools.iter().any(|tool| tool.name == step.tool);
    if !known {
        return Some(EscalationReason::UnknownTool {
            name: step.tool.clone(),
        });
    }
    if step.tool != FINAL && remembered(results, &step.signature()) {
        *repeats += 1;
        return Some(EscalationReason::Repeated {
            name: step.tool.clone(),
        });
    }
    None
}

fn answer_of(step: &Step) -> String {
    match step.args.get("answer") {
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

/// Stream of [`StepEvent`] from [`Agent::run_stream`].
#[must_use = "streams do nothing unless polled"]
pub struct RunStream<'a> {
    fut: Pin<Box<dyn Future<Output = ()> + Send + 'a>>,
    queue: Arc<Mutex<VecDeque<Result<StepEvent>>>>,
    done: bool,
}

impl RunStream<'_> {
    /// Next event, without requiring [`Stream`] in scope.
    pub async fn next_event(&mut self) -> Option<Result<StepEvent>> {
        std::future::poll_fn(|cx| Pin::new(&mut *self).poll_next(cx)).await
    }

    /// Drive the loop and collect every event.
    pub async fn into_events(mut self) -> Vec<Result<StepEvent>> {
        let mut out = Vec::new();
        while let Some(item) = self.next_event().await {
            out.push(item);
        }
        out
    }
}

impl Stream for RunStream<'_> {
    type Item = Result<StepEvent>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if let Some(item) = dequeue(&this.queue) {
                if !this.done {
                    cx.waker().wake_by_ref();
                }
                return Poll::Ready(Some(item));
            }
            if this.done {
                return Poll::Ready(None);
            }
            match this.fut.as_mut().poll(cx) {
                Poll::Ready(()) => this.done = true,
                Poll::Pending => {
                    if let Some(item) = dequeue(&this.queue) {
                        cx.waker().wake_by_ref();
                        return Poll::Ready(Some(item));
                    }
                    return Poll::Pending;
                }
            }
        }
    }
}

fn enqueue(queue: &Mutex<VecDeque<Result<StepEvent>>>, item: Result<StepEvent>) {
    match queue.lock() {
        Ok(mut guard) => guard.push_back(item),
        Err(poison) => poison.into_inner().push_back(item),
    }
}

fn dequeue(queue: &Mutex<VecDeque<Result<StepEvent>>>) -> Option<Result<StepEvent>> {
    match queue.lock() {
        Ok(mut guard) => guard.pop_front(),
        Err(poison) => poison.into_inner().pop_front(),
    }
}
