//! Official Codex CLI, authenticated with the person's ChatGPT account.
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use legate_core::{
    AgentEvent, FileEdit, JobAccess, ModelOption, ProviderId, RunOptions, StepPhase, StepTool,
};
use serde_json::Value;

use crate::adapter::{
    Bin, InteractiveCommand, InteractiveOptions, LimitError, ProviderAdapter, RunHandle,
};
use crate::edit_size::{cap_detail, first_line};
use crate::process::{self, VersionCache};
use crate::util::lock;

pub struct CodexAdapter {
    bin: Bin,
    version: VersionCache,
    parse_state: Mutex<ParseState>,
}

#[derive(Default)]
struct ParseState {
    session: Option<String>,
}

impl CodexAdapter {
    pub fn new(bin: Option<Bin>) -> Self {
        Self {
            bin: bin.unwrap_or_else(resolve_bin),
            version: VersionCache::new(),
            parse_state: Mutex::new(ParseState::default()),
        }
    }

    pub fn parse_events(&self, line: &str) -> Vec<AgentEvent> {
        parse_line(line, &mut lock(&self.parse_state))
    }

    pub async fn list_models(&self) -> Vec<ModelOption> {
        let home = std::env::var_os("CODEX_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("USERPROFILE")
                    .or_else(|| std::env::var_os("HOME"))
                    .map(|p| std::path::PathBuf::from(p).join(".codex"))
            });
        let Some(home) = home else {
            return Vec::new();
        };
        let Ok(bytes) = tokio::fs::read(home.join("models_cache.json")).await else {
            return Vec::new();
        };
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            return Vec::new();
        };
        value["models"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|model| model["hidden"].as_bool() != Some(true))
            .filter_map(|model| {
                let id = model["slug"].as_str()?;
                Some(ModelOption {
                    id: id.into(),
                    label: model["display_name"].as_str().unwrap_or(id).into(),
                })
            })
            .collect()
    }

    pub fn interactive(&self, opts: &InteractiveOptions) -> InteractiveCommand {
        let mut args = self.bin.args.clone();
        if let Some(model) = &opts.model {
            args.extend(["--model".into(), model.clone()]);
        }
        if let Some(session) = &opts.resume {
            args.extend(["resume".into(), session.clone()]);
        }
        if let Some(prompt) = &opts.prompt {
            args.push(prompt.clone());
        }
        InteractiveCommand {
            command: self.bin.command.clone(),
            args,
            env: Some(codex_env()),
        }
    }
}

#[async_trait]
impl ProviderAdapter for CodexAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::Codex
    }
    async fn is_installed(&self) -> bool {
        self.version.installed(&self.bin).await
    }
    async fn version(&self) -> Option<String> {
        crate::versions::parse_cli_version(self.id(), &self.version.stdout(&self.bin).await)
    }
    async fn is_signed_in(&self) -> bool {
        let result = process::run_captured(&self.bin, &["login".into(), "status".into()]).await;
        chatgpt_login(result.code, &result.stdout, &result.stderr)
    }
    fn run(&self, _job_id: &str, prompt: &str, cwd: &Path, opts: &RunOptions) -> RunHandle {
        let state = Arc::new(Mutex::new(ParseState::default()));
        let mut env = codex_env();
        if let Some(bridge) = &opts.bridge {
            env.insert("LEGATE_MCP_TOKEN".into(), bridge.token.clone());
        }
        process::spawn_cli(process::SpawnSpec {
            bin: self.bin.clone(),
            args: run_args(opts),
            prompt: crate::note::with_legate_note(prompt),
            cwd: cwd.to_path_buf(),
            env: Some(env),
            parse: Arc::new(move |line| parse_line(line, &mut lock(&state))),
            cleanup: None,
        })
    }
    fn parse_event(&self, line: &str) -> Option<AgentEvent> {
        self.parse_events(line).into_iter().next()
    }
    fn is_limit_error(&self, error: LimitError<'_>) -> bool {
        match error {
            LimitError::Event(event) => matches!(event, AgentEvent::Limit { .. }),
            LimitError::Text(text) => is_limit(text),
        }
    }
}

fn resolve_bin() -> Bin {
    if cfg!(windows) {
        if let Some(path) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path) {
                let exe = dir.join("codex.exe");
                if exe.is_file() {
                    return Bin::new(exe.to_string_lossy());
                }
            }
        }
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let script = std::path::PathBuf::from(appdata)
                .join("npm/node_modules/@openai/codex/bin/codex.js");
            if script.is_file() {
                return Bin::with_args("node", [script.to_string_lossy().to_string()]);
            }
        }
    }
    Bin::new("codex")
}
fn codex_env() -> BTreeMap<String, String> {
    subscription_env(std::env::vars().collect())
}
fn subscription_env(source: BTreeMap<String, String>) -> BTreeMap<String, String> {
    source
        .into_iter()
        .filter(|(key, _)| {
            !matches!(
                key.to_ascii_uppercase().as_str(),
                "OPENAI_API_KEY" | "CODEX_API_KEY" | "CODEX_THREAD_ID"
            )
        })
        .collect()
}
fn chatgpt_login(code: Option<i32>, stdout: &str, stderr: &str) -> bool {
    code == Some(0)
        && format!("{stdout}\n{stderr}")
            .to_lowercase()
            .contains("chatgpt")
}
fn run_args(opts: &RunOptions) -> Vec<String> {
    // Ignore the user's permission/MCP defaults, while retaining the official CLI's saved auth.
    // Codex uses its native sandbox: workspace-write permits commands inside that sandbox.
    let access = if opts.bridge.is_some() {
        JobAccess::Read
    } else {
        opts.access.unwrap_or(if opts.edit == Some(true) {
            JobAccess::Edit
        } else {
            JobAccess::Read
        })
    };
    let sandbox = match access {
        JobAccess::Read => "read-only",
        JobAccess::Edit => "workspace-write",
        JobAccess::Full => "danger-full-access",
    };
    let mut args: Vec<String> = [
        "exec",
        "--json",
        "--ignore-user-config",
        "--skip-git-repo-check",
        "--color",
        "never",
        "--sandbox",
        sandbox,
        "-c",
        "approval_policy=\"never\"",
        "-c",
        "mcp_servers={}",
    ]
    .iter()
    .map(|s| (*s).into())
    .collect();
    // The native Windows CLI requires an explicit backend when user config is ignored.
    // Use the supported legacy backend; MXC is not available on all installations.
    if cfg!(windows) {
        args.extend(["-c".into(), "windows.sandbox=\"unelevated\"".into()]);
    }
    if let Some(model) = &opts.model {
        args.extend(["--model".into(), model.clone()]);
    }
    if let Some(bridge) = &opts.bridge {
        args.extend([
            "-c".into(),
            format!(
                "mcp_servers.orchestrator.url={}",
                serde_json::to_string(&bridge.url).unwrap_or_default()
            ),
            "-c".into(),
            "mcp_servers.orchestrator.bearer_token_env_var=\"LEGATE_MCP_TOKEN\"".into(),
            "-c".into(),
            format!(
                "mcp_servers.orchestrator.enabled_tools={}",
                serde_json::to_string(&bridge.tools).unwrap_or_default()
            ),
        ]);
    }
    if let Some(session) = &opts.resume {
        args.extend(["resume".into(), session.clone()]);
    }
    args.push("-".into());
    args
}

fn parse_line(line: &str, state: &mut ParseState) -> Vec<AgentEvent> {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    match value["type"].as_str().unwrap_or("") {
        "thread.started" => {
            let Some(id) = value["thread_id"].as_str() else {
                return Vec::new();
            };
            state.session = Some(id.into());
            vec![AgentEvent::Init {
                session_id: id.into(),
                model: None,
            }]
        }
        "turn.completed" => {
            let usage = &value["usage"];
            let tokens = usage["input_tokens"]
                .as_i64()
                .zip(usage["output_tokens"].as_i64())
                .map(|(a, b)| a + b);
            vec![AgentEvent::Result {
                ok: true,
                text: String::new(),
                session_id: state.session.clone(),
                duration_ms: None,
                cost_usd: None,
                tokens,
            }]
        }
        "turn.failed" => {
            let message = value["error"]["message"]
                .as_str()
                .unwrap_or("Codex turn failed");
            if is_limit(message) {
                vec![AgentEvent::Limit {
                    message: message.into(),
                    resets_at: None,
                }]
            } else {
                vec![AgentEvent::Result {
                    ok: false,
                    text: message.into(),
                    session_id: state.session.clone(),
                    duration_ms: None,
                    cost_usd: None,
                    tokens: None,
                }]
            }
        }
        "error" if is_limit(value["message"].as_str().unwrap_or("")) => vec![AgentEvent::Limit {
            message: value["message"].as_str().unwrap_or_default().into(),
            resets_at: None,
        }],
        kind @ ("item.started" | "item.updated" | "item.completed") => {
            item_events(&value["item"], kind == "item.completed")
        }
        _ => Vec::new(),
    }
}

fn item_events(item: &Value, ended: bool) -> Vec<AgentEvent> {
    let id = item["id"].as_str().unwrap_or("codex-item");
    let kind = item["type"].as_str().unwrap_or("");
    let text = item["text"].as_str().unwrap_or("");
    if kind == "agent_message" || kind == "reasoning" {
        if !ended || text.is_empty() {
            return Vec::new();
        }
        let mut events = Vec::new();
        if kind == "agent_message" {
            events.push(AgentEvent::Text { text: text.into() });
        }
        events.push(step(
            id,
            ended,
            if kind == "reasoning" {
                StepTool::Think
            } else {
                StepTool::Say
            },
            first_line(text),
            Some(text),
            true,
            None,
        ));
        return events;
    }
    if kind == "file_change" {
        return item["changes"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(i, change)| {
                let path = change["path"].as_str()?;
                Some(step(
                    &format!("{id}-{i}"),
                    ended,
                    StepTool::Edit,
                    &format!("Edited {path}"),
                    None,
                    item["status"] != "failed",
                    Some(FileEdit {
                        path: path.into(),
                        added: 0,
                        removed: 0,
                    }),
                ))
            })
            .collect();
    }
    let (tool, title, detail) = match kind {
        "command_execution" => (
            StepTool::Shell,
            format!(
                "Ran {}",
                first_line(item["command"].as_str().unwrap_or("command"))
            ),
            item["aggregated_output"]
                .as_str()
                .or_else(|| item["command"].as_str()),
        ),
        "mcp_tool_call" => (
            StepTool::Mcp,
            item["tool"].as_str().unwrap_or("MCP tool").into(),
            None,
        ),
        "web_search" => (
            StepTool::Web,
            item["query"].as_str().unwrap_or("Web search").into(),
            None,
        ),
        _ => return Vec::new(),
    };
    let ok = item["status"] != "failed" && item["exit_code"].as_i64().is_none_or(|code| code == 0);
    vec![step(id, ended, tool, &title, detail, ok, None)]
}

fn step(
    id: &str,
    ended: bool,
    tool: StepTool,
    title: &str,
    detail: Option<&str>,
    ok: bool,
    edit: Option<FileEdit>,
) -> AgentEvent {
    AgentEvent::Step {
        id: id.into(),
        phase: if ended {
            StepPhase::End
        } else {
            StepPhase::Start
        },
        tool,
        title: cap_detail(title).into(),
        detail: detail.map(|s| cap_detail(s).into()),
        ok: ended.then_some(ok),
        edit,
    }
}
fn is_limit(text: &str) -> bool {
    let text = text.to_lowercase();
    [
        "usage limit",
        "rate limit",
        "rate_limit",
        "insufficient_quota",
        "quota exceeded",
    ]
    .iter()
    .any(|word| text.contains(word))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn codex_parses_thread_messages_and_token_usage() {
        let adapter = CodexAdapter::new(None);
        assert!(
            matches!(adapter.parse_events(r#"{"type":"thread.started","thread_id":"session-1"}"#).first(), Some(AgentEvent::Init { session_id, .. }) if session_id == "session-1")
        );
        assert!(
            matches!(adapter.parse_events(r#"{"type":"item.completed","item":{"id":"m1","type":"agent_message","text":"done"}}"#).first(), Some(AgentEvent::Text { text }) if text == "done")
        );
        assert!(
            matches!(adapter.parse_events(r#"{"type":"turn.completed","usage":{"input_tokens":10,"cached_input_tokens":7,"output_tokens":2}}"#).first(), Some(AgentEvent::Result { ok:true, tokens:Some(12), session_id:Some(id), .. }) if id == "session-1")
        );
    }
    #[test]
    fn codex_uses_explicit_sandbox_and_never_requests_approvals() {
        for (access, sandbox) in [
            (JobAccess::Read, "read-only"),
            (JobAccess::Edit, "workspace-write"),
            (JobAccess::Full, "danger-full-access"),
        ] {
            let args = run_args(&RunOptions {
                access: Some(access),
                ..RunOptions::default()
            });
            assert!(args.windows(2).any(|a| a == ["--sandbox", sandbox]));
            assert!(args.iter().any(|a| a == "approval_policy=\"never\""));
            assert!(args.iter().any(|a| a == "--ignore-user-config"));
            assert_eq!(
                args.iter().any(|a| a == "windows.sandbox=\"unelevated\""),
                cfg!(windows)
            );
        }
    }
    #[test]
    fn codex_only_accepts_chatgpt_subscription_authentication() {
        assert!(chatgpt_login(Some(0), "Logged in using ChatGPT", ""));
        assert!(!chatgpt_login(Some(0), "Logged in using an API key", ""));
        assert!(!chatgpt_login(Some(1), "Logged in using ChatGPT", ""));
    }
    #[test]
    fn codex_reports_limit_and_failed_turns() {
        let adapter = CodexAdapter::new(None);
        assert!(matches!(
            adapter
                .parse_events(
                    r#"{"type":"turn.failed","error":{"message":"You've hit your usage limit"}}"#
                )
                .first(),
            Some(AgentEvent::Limit { .. })
        ));
        assert!(matches!(
            adapter
                .parse_events(r#"{"type":"turn.failed","error":{"message":"server unavailable"}}"#)
                .first(),
            Some(AgentEvent::Result { ok: false, .. })
        ));
    }
    #[test]
    fn codex_does_not_forward_api_keys_or_another_thread_context() {
        let env = subscription_env(BTreeMap::from([
            ("OPENAI_API_KEY".into(), "private".into()),
            ("CODEX_API_KEY".into(), "private".into()),
            ("CODEX_THREAD_ID".into(), "parent-thread".into()),
            ("PATH".into(), "tools".into()),
        ]));
        assert_eq!(env, BTreeMap::from([("PATH".into(), "tools".into())]));
    }
    #[test]
    fn codex_streams_commands_edits_and_failures_as_steps() {
        let adapter = CodexAdapter::new(None);
        let start=adapter.parse_events(r#"{"type":"item.started","item":{"id":"c1","type":"command_execution","command":"cargo test"}}"#);
        assert!(matches!(
            &start[0],
            AgentEvent::Step {
                phase: StepPhase::Start,
                tool: StepTool::Shell,
                ..
            }
        ));
        let end=adapter.parse_events(r#"{"type":"item.completed","item":{"id":"c1","type":"command_execution","command":"cargo test","exit_code":1,"status":"failed","aggregated_output":"FAIL"}}"#);
        assert!(matches!(
            &end[0],
            AgentEvent::Step {
                phase: StepPhase::End,
                ok: Some(false),
                ..
            }
        ));
        let edit=adapter.parse_events(r#"{"type":"item.completed","item":{"id":"e1","type":"file_change","status":"completed","changes":[{"path":"src/lib.rs","kind":"update"}]}}"#);
        assert!(
            matches!(&edit[0],AgentEvent::Step{tool:StepTool::Edit,edit:Some(file),..} if file.path=="src/lib.rs")
        );
        assert!(adapter.parse_events("not JSON").is_empty());
    }
}
