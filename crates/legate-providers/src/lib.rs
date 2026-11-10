//! Headless Claude, Cursor, Grok, and local model adapters.

mod adapter;
mod claude;
mod codex;
mod cursor;
mod edit_size;
mod error;
mod lenient;
mod local;
mod note;
mod plugins;
mod process;
mod step_state;
mod util;
mod values;
mod versions;

pub use adapter::{
    Bin, EventStream, InteractiveCommand, InteractiveOptions, LimitError, ProcessExit,
    ProviderAdapter, RunHandle,
};
pub use claude::ClaudeAdapter;
pub use codex::CodexAdapter;
pub use cursor::{
    CursorAdapter, ModelSource, cursor_env, cursor_version_dirs, node_platform,
    parse_cursor_models, resolve_cursor_bin,
};
pub use edit_size::{
    LineChange, cap_detail, edit_size, first_line, line_count, relative_to_cwd, reported_edit_size,
    sum_edit_size, write_size,
};
pub use error::ProviderError;
pub use legate_core::{
    AgentEvent, BridgeInfo, FileEdit, GROK_DEFAULT_MODEL, JobAccess, ModelOption, ProviderId,
    RunOptions, StepPhase, StepTool, UsageWindow, VersionStatus,
};
pub use local::{LocalAdapter, LocalConfig};
pub use note::{LEGATE_NOTE, with_legate_note};
pub use plugins::{AgentPlugins, plugin_args};
pub use versions::{
    TESTED_VERSIONS, TestedVersions, compare_to_tested, parse_cli_version, unknown_format_message,
};
