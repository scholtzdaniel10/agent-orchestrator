//! Local agent runtime for llama-server and Ollama.
//!
//! The loop only appends messages, so a llama-server slot can reuse the KV cache
//! of the system prompt and tool list. Tool calls are forced through one JSON
//! schema. A small model can try each step and hand the hard ones to a big model.
//! A repeated tool call is not run again.

mod agent;
mod client;
mod error;
mod prefix_store;
mod pyjson;
mod toolpath;
mod tools;

#[cfg(test)]
mod fake_server;
#[cfg(test)]
mod tests;

pub use agent::{Agent, AgentOutcome, EscalationReason, RunStats, RunStream, StepEvent};
pub use client::Message;
pub use client::{Backend, CallStats, ServerKind};
pub use error::{Error, Result};
pub use prefix_store::{PrefixStats, PrefixStore};
pub use toolpath::{
    ESCALATE, FINAL, Step, Tool, ToolSpec, object_schema, parse_step, step_schema, tools_prompt,
};
pub use tools::{DEFAULT_SYSTEM, default_tools};
