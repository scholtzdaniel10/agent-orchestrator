//! Local MCP bridge.
//!
//! JSON-RPC 2.0 over `POST /mcp` on `127.0.0.1`, with a new bearer token every
//! start. The server is [`Send`] and runs on a multi-thread Tokio runtime.
//! Tool calls go through [`BridgeBackend`], which the app implements by
//! forwarding work onto the orchestrator's `LocalSet` thread.
//!
//! The TypeScript server does not use server-sent events or an MCP session
//! header. This port does not add them.

mod backend;
mod error;
mod server;
mod tools;

pub use backend::{BackendError, BridgeBackend, NewJob, WorkerSnapshot};
pub use error::{BridgeError, ToolError};
pub use legate_core::BridgeInfo;
pub use server::{Bridge, BridgeTool, start_bridge};
pub use tools::{
    GET_RESULT, GET_RESULT_DESCRIPTION, GET_RESULT_SCHEMA, GET_STATUS, GET_STATUS_DESCRIPTION,
    GET_STATUS_SCHEMA, LIST_WORKERS, LIST_WORKERS_DESCRIPTION, LIST_WORKERS_SCHEMA, PROPOSE_JOBS,
    PROPOSE_JOBS_DESCRIPTION, PROPOSE_JOBS_SCHEMA, SEND_JOB, SEND_JOB_DESCRIPTION, SEND_JOB_SCHEMA,
    ToolConfig, orchestrator_tools, proposed_jobs,
};
