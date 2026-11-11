//! Claude Code CLI. Arguments and stream-json mapping follow `src/core/providers/claude.ts`.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use async_trait::async_trait;
use legate_core::{
    AgentEvent, BridgeInfo, FileEdit, JobAccess, ModelOption, ProviderId, RunOptions, StepPhase,
    StepTool, UsageWindow,
};
use regex::Regex;
use serde_json::{Value, value::RawValue};

use crate::adapter::{
    Bin, InteractiveCommand, InteractiveOptions, LimitError, ProviderAdapter, RunHandle,
};
use crate::edit_size::{
    cap_detail, edit_size, first_line, relative_to_cwd, sum_edit_size, write_size,
};
use crate::error::ProviderError;
use crate::lenient::{self, Block, Message};
use crate::note::LEGATE_NOTE;
use crate::plugins::{AgentPlugins, plugin_args};
use crate::process::{self, LineParser, VersionCache};
use crate::step_state::{PendingStep, StepParseState};
use crate::util::{job_access, lock, non_empty, path_string};
use crate::values::{empty_object, epoch_secs, number_f64, string_arg, token_total};

static LIMIT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)rate.?limit|usage limit|limit reached|hit your (usage )?limit|out of (usage|credits)",
    )
    .expect("claude limit pattern")
});

const SHELL_TOOLS: &str = "Bash,PowerShell";
const WRITE_TOOLS: &str = "Edit,Write,NotebookEdit";

pub struct ClaudeAdapter {
    found: Mutex<Option<Bin>>,
    plugins: Mutex<AgentPlugins>,
    versions: VersionCache,
    steps: Arc<Mutex<StepParseState>>,
}

impl ClaudeAdapter {
    pub fn new(bin: Option<Bin>) -> Self {
        Self {
            found: Mutex::new(bin),
            plugins: Mutex::new(AgentPlugins::default()),
            versions: VersionCache::new(),
            steps: Arc::new(Mutex::new(StepParseState::default())),
        }
    }

    pub fn set_plugins(&self, plugins: AgentPlugins) {
        *lock(&self.plugins) = plugins;
    }

    pub async fn list_models(&self) -> Vec<ModelOption> {
        let _ = self;
        vec![
            model("opus", "Opus, newest this CLI knows"),
            model("claude-opus-5-5", "Opus 5.5"),
            model("claude-opus-5", "Opus 5"),
            model("sonnet", "Sonnet, newest this CLI knows"),
            model("claude-sonnet-5-5", "Sonnet 5.5"),
            model("claude-sonnet-5", "Sonnet 5"),
            model("haiku", "Haiku, newest this CLI knows"),
            model("claude-haiku-5-5", "Haiku 5.5"),
            model("fable", "Fable, needs usage credits on Pro"),
            model("claude-fable-5-1", "Fable 5.1, needs usage credits on Pro"),
        ]
    }

    pub fn interactive(&self, opts: &InteractiveOptions) -> InteractiveCommand {
        let bin = self.resolved_bin();
        let plugins = lock(&self.plugins).clone();
        let mut args = bin.args.clone();
        if let Some(resume) = non_empty(&opts.resume) {
            args.push("--resume".to_string());
            args.push(resume.to_string());
        } else if let Some(session) = non_empty(&opts.session_id) {
            args.push("--session-id".to_string());
            args.push(session.to_string());
        }
        if let Some(model) = non_empty(&opts.model) {
            args.push("--model".to_string());
            args.push(model.to_string());
        }
        args.push("--append-system-prompt".to_string());
        args.push(LEGATE_NOTE.to_string());
        args.extend(plugin_args(&plugins.worker));
        if non_empty(&opts.resume).is_none()
            && let Some(prompt) = non_empty(&opts.prompt)
        {
            args.push(prompt.to_string());
        }
        InteractiveCommand {
            command: bin.command,
            args,
            env: None,
        }
    }

    /// Smallest turn that still reports the plan's real usage windows.
    pub fn usage_probe(&self, cwd: &Path) -> RunHandle {
        let args = vec![
            "-p".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--verbose".to_string(),
            "--model".to_string(),
            "haiku".to_string(),
            "--system-prompt".to_string(),
            "Reply with the single word ok.".to_string(),
            "--setting-sources".to_string(),
            String::new(),
            "--strict-mcp-config".to_string(),
            "--tools".to_string(),
            String::new(),
            "--max-turns".to_string(),
            "1".to_string(),
        ];
        process::spawn_cli(process::SpawnSpec {
            bin: self.resolved_bin(),
            args,
            prompt: "ok".to_string(),
            cwd: cwd.to_path_buf(),
            env: None,
            parse: parser(&self.steps),
            cleanup: None,
        })
    }

    pub fn parse_events(&self, line: &str) -> Vec<AgentEvent> {
        parse_line(line, &self.steps)
    }

    fn resolved_bin(&self) -> Bin {
        let mut found = lock(&self.found);
        if let Some(bin) = found.as_ref() {
            return bin.clone();
        }
        let bin = resolve_claude_bin();
        if bin.command != "claude" {
            *found = Some(bin.clone());
        }
        bin
    }

    fn spawn(&self, prompt: &str, cwd: &Path, opts: &RunOptions) -> RunHandle {
        match self.prepare(prompt, cwd, opts) {
            Ok(spec) => process::spawn_cli(spec),
            Err(err) => RunHandle::failed(err.to_string()),
        }
    }

    fn prepare(
        &self,
        prompt: &str,
        cwd: &Path,
        opts: &RunOptions,
    ) -> Result<process::SpawnSpec, ProviderError> {
        let mut args = vec![
            "-p".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--verbose".to_string(),
        ];
        if opts.bridge.is_none() {
            args.extend(worker_args(job_access(opts)));
        }
        if let Some(resume) = non_empty(&opts.resume) {
            args.push("--resume".to_string());
            args.push(resume.to_string());
        }
        if let Some(bridge) = &opts.bridge {
            let config = write_mcp_config(cwd, bridge)?;
            let read_dirs = opts.read_dirs.as_deref().unwrap_or(&[]);
            let mut tools: Vec<String> = bridge
                .tools
                .iter()
                .map(|tool| format!("mcp__orchestrator__{tool}"))
                .collect();
            if !read_dirs.is_empty() {
                tools.extend(["Read".to_string(), "Grep".to_string(), "Glob".to_string()]);
            }
            for dir in read_dirs {
                args.push("--add-dir".to_string());
                args.push(dir.clone());
            }
            args.push("--mcp-config".to_string());
            args.push(path_string(&config));
            args.push("--strict-mcp-config".to_string());
            args.push("--allowedTools".to_string());
            args.push(tools.join(","));
            args.push("--setting-sources".to_string());
            args.push("project".to_string());
        }
        if let Some(model) = non_empty(&opts.model) {
            args.push("--model".to_string());
            args.push(model.to_string());
        }
        args.push("--append-system-prompt".to_string());
        args.push(LEGATE_NOTE.to_string());
        let plugins = lock(&self.plugins).clone();
        let dirs = if opts.bridge.is_some() {
            &plugins.lead
        } else {
            &plugins.worker
        };
        args.extend(plugin_args(dirs));
        Ok(process::SpawnSpec {
            bin: self.resolved_bin(),
            args,
            prompt: prompt.to_string(),
            cwd: cwd.to_path_buf(),
            env: None,
            parse: parser(&self.steps),
            cleanup: None,
        })
    }
}

#[async_trait]
impl ProviderAdapter for ClaudeAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::Claude
    }

    async fn is_installed(&self) -> bool {
        self.versions.installed(&self.resolved_bin()).await
    }

    async fn version(&self) -> Option<String> {
        let stdout = self.versions.stdout(&self.resolved_bin()).await;
        crate::versions::parse_cli_version(ProviderId::Claude, &stdout)
    }

    async fn is_signed_in(&self) -> bool {
        let run = process::run_captured(
            &self.resolved_bin(),
            &["auth".to_string(), "status".to_string()],
        )
        .await;
        let Ok(value) = serde_json::from_str::<Value>(run.stdout.trim()) else {
            return false;
        };
        matches!(value.get("loggedIn"), Some(Value::Bool(true)))
    }

    fn run(&self, _job_id: &str, prompt: &str, cwd: &Path, opts: &RunOptions) -> RunHandle {
        self.spawn(prompt, cwd, opts)
    }

    fn parse_event(&self, line: &str) -> Option<AgentEvent> {
        self.parse_events(line).into_iter().next()
    }

    fn is_limit_error(&self, error: LimitError<'_>) -> bool {
        match error {
            LimitError::Event(AgentEvent::Limit { .. }) => true,
            LimitError::Event(_) => false,
            LimitError::Text(text) => LIMIT_RE.is_match(text),
        }
    }
}

fn model(id: &str, label: &str) -> ModelOption {
    ModelOption {
        id: id.to_string(),
        label: label.to_string(),
    }
}

fn parser(steps: &Arc<Mutex<StepParseState>>) -> LineParser {
    let steps = Arc::clone(steps);
    Arc::new(move |line| parse_line(line, &steps))
}

fn parse_line(line: &str, steps: &Mutex<StepParseState>) -> Vec<AgentEvent> {
    let Ok(parsed) = serde_json::from_str::<ClaudeLine<'_>>(line) else {
        return Vec::new();
    };
    map_claude(&parsed, &mut lock(steps))
}

fn worker_args(access: JobAccess) -> Vec<String> {
    if access == JobAccess::Full {
        return vec![
            "--strict-mcp-config".to_string(),
            "--permission-mode".to_string(),
            "bypassPermissions".to_string(),
        ];
    }
    let denied = if access == JobAccess::Edit {
        SHELL_TOOLS.to_string()
    } else {
        format!("{SHELL_TOOLS},{WRITE_TOOLS}")
    };
    let mut args = vec![
        "--disallowedTools".to_string(),
        denied,
        "--strict-mcp-config".to_string(),
    ];
    if access == JobAccess::Edit {
        args.push("--permission-mode".to_string());
        args.push("acceptEdits".to_string());
    }
    args
}

fn write_mcp_config(cwd: &Path, bridge: &BridgeInfo) -> Result<PathBuf, ProviderError> {
    std::fs::create_dir_all(cwd)?;
    let path = cwd.join("orchestrator-mcp.json");
    // Key order matches JSON.stringify in claude.ts. serde_json::Map sorts keys.
    let url = serde_json::to_string(&bridge.url)?;
    let auth = serde_json::to_string(&format!("Bearer {}", bridge.token))?;
    let body = format!(
        r#"{{"mcpServers":{{"orchestrator":{{"type":"http","url":{url},"headers":{{"Authorization":{auth}}}}}}}}}"#
    );
    std::fs::write(&path, body)?;
    Ok(path)
}

fn resolve_claude_bin() -> Bin {
    let file = if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    };
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            if dir.as_os_str().is_empty() {
                continue;
            }
            let candidate = dir.join(file);
            if candidate.exists() {
                return Bin::new(path_string(&candidate));
            }
        }
    }
    #[cfg(windows)]
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let fallback = PathBuf::from(appdata)
            .join("npm")
            .join("node_modules")
            .join("@anthropic-ai")
            .join("claude-code")
            .join("bin")
            .join("claude.exe");
        if fallback.exists() {
            return Bin::new(path_string(&fallback));
        }
    }
    Bin::new("claude")
}

#[derive(Debug, serde::Deserialize)]
#[serde(bound(deserialize = "'de: 'a"))]
struct ClaudeLine<'a> {
    #[serde(rename = "type", default, deserialize_with = "lenient::de_opt_cow")]
    kind: Option<Cow<'a, str>>,
    #[serde(default, deserialize_with = "lenient::de_opt_cow")]
    subtype: Option<Cow<'a, str>>,
    #[serde(default, deserialize_with = "lenient::de_opt_cow")]
    session_id: Option<Cow<'a, str>>,
    #[serde(default, deserialize_with = "lenient::de_opt_cow")]
    cwd: Option<Cow<'a, str>>,
    #[serde(default, deserialize_with = "lenient::de_opt_cow")]
    model: Option<Cow<'a, str>>,
    #[serde(default, deserialize_with = "lenient::de_opt_cow")]
    error: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    message: Option<&'a RawValue>,
    #[serde(borrow, default)]
    rate_limit_info: Option<&'a RawValue>,
    #[serde(default, deserialize_with = "lenient::de_opt_bool")]
    is_error: Option<bool>,
    #[serde(default, deserialize_with = "lenient::de_opt_cow")]
    result: Option<Cow<'a, str>>,
    #[serde(default, deserialize_with = "lenient::de_opt_f64")]
    duration_ms: Option<f64>,
    #[serde(default, deserialize_with = "lenient::de_opt_f64")]
    total_cost_usd: Option<f64>,
    #[serde(borrow, default)]
    usage: Option<&'a RawValue>,
}

fn map_claude(line: &ClaudeLine<'_>, state: &mut StepParseState) -> Vec<AgentEvent> {
    let kind = line.kind.as_deref();
    let subtype = line.subtype.as_deref();
    if kind == Some("system") && subtype == Some("init") {
        if let (Some(session), Some(cwd)) = (&line.session_id, &line.cwd) {
            state.remember_cwd(session.to_string(), cwd.to_string());
        }
        return init_event(line).into_iter().collect();
    }
    if kind == Some("system")
        && subtype == Some("api_retry")
        && line.error.as_deref() == Some("rate_limit")
    {
        return vec![limit_event(None, "rate_limit")];
    }
    if kind == Some("assistant") {
        let Some(message) = line.message else {
            return Vec::new();
        };
        return assistant_events(message, line.session_id.as_deref(), state);
    }
    if kind == Some("user") {
        let Some(message) = line.message else {
            return Vec::new();
        };
        return user_events(message, state);
    }
    if kind == Some("rate_limit_event") {
        return line
            .rate_limit_info
            .and_then(rate_limit_event)
            .into_iter()
            .collect();
    }
    if kind == Some("result") {
        return vec![result_event(line)];
    }
    Vec::new()
}

fn init_event(line: &ClaudeLine<'_>) -> Option<AgentEvent> {
    let session_id = line.session_id.as_ref()?.to_string();
    Some(AgentEvent::Init {
        session_id,
        model: line.model.as_ref().map(|model| model.to_string()),
    })
}

fn assistant_events(
    message: &RawValue,
    session_id: Option<&str>,
    state: &mut StepParseState,
) -> Vec<AgentEvent> {
    let Some(message) = lenient::parse_raw::<Message<'_>>(message) else {
        return Vec::new();
    };
    let cwd = state.cwd(session_id);
    let mut events = Vec::new();
    let mut text_parts: Vec<&str> = Vec::new();
    let mut starts = Vec::new();
    for block in &message.content {
        let block_type = block.kind.as_deref();
        if block_type == Some("thinking") {
            if let Some(thinking) = readable_thinking(block) {
                let title = first_line(&thinking).to_string();
                let detail = cap_detail(&thinking).to_string();
                events.push(step(
                    state.next_id("think"),
                    StepPhase::End,
                    StepTool::Think,
                    title,
                    Some(detail),
                    None,
                    None,
                ));
            }
            continue;
        }
        if block_type == Some("text")
            && let Some(text) = block.text.as_deref()
        {
            text_parts.push(text);
            continue;
        }
        if block_type != Some("tool_use") {
            continue;
        }
        let id = match &block.id {
            Some(id) => id.to_string(),
            None => state.next_id("tool"),
        };
        let name = block.name.as_deref().unwrap_or("tool");
        let input = record_or_empty(block.input);
        let tool = claude_tool(name);
        let cwd_ref = cwd.as_deref();
        let title = claude_title(name, tool, &input, cwd_ref);
        let edit = claude_edit(name, &input, cwd_ref);
        let detail = claude_start_detail(tool, &input);
        state.remember(
            id.clone(),
            PendingStep {
                tool,
                title: title.clone(),
                edit: edit.clone(),
            },
        );
        starts.push(step(id, StepPhase::Start, tool, title, detail, None, edit));
    }
    let text = text_parts.join("");
    if !text.is_empty() {
        let say_id = match &message.id {
            Some(id) => id.to_string(),
            None => state.next_id("say"),
        };
        let title = first_line(&text).to_string();
        let detail = cap_detail(&text).to_string();
        events.push(AgentEvent::Text { text: text.clone() });
        events.push(step(
            say_id,
            StepPhase::End,
            StepTool::Say,
            title,
            Some(detail),
            None,
            None,
        ));
    }
    events.extend(starts);
    events
}

fn user_events(message: &RawValue, state: &mut StepParseState) -> Vec<AgentEvent> {
    let Some(message) = lenient::parse_raw::<Message<'_>>(message) else {
        return Vec::new();
    };
    let mut events = Vec::new();
    for block in &message.content {
        if block.kind.as_deref() != Some("tool_result") {
            continue;
        }
        let id = match &block.tool_use_id {
            Some(id) => id.to_string(),
            None => state.next_id("tool"),
        };
        let pending = state.tool(&id);
        let text = tool_result_text(block.content);
        let detail = if text.is_empty() {
            None
        } else {
            Some(cap_detail(&text).to_string())
        };
        let (tool, title, edit) = match pending {
            Some(pending) => (pending.tool, pending.title, pending.edit),
            None => (StepTool::Other, "tool".to_string(), None),
        };
        events.push(step(
            id,
            StepPhase::End,
            tool,
            title,
            detail,
            Some(block.is_error != Some(true)),
            edit,
        ));
    }
    events
}

fn claude_tool(name: &str) -> StepTool {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "read" => StepTool::Read,
        "edit" | "write" | "multiedit" | "notebookedit" => StepTool::Edit,
        "bash" | "powershell" | "bashoutput" => StepTool::Shell,
        "grep" | "glob" | "ls" => StepTool::Search,
        "websearch" | "webfetch" => StepTool::Web,
        _ if lower.starts_with("mcp__") || lower.starts_with("mcp_") => StepTool::Mcp,
        _ => StepTool::Other,
    }
}

fn claude_title(name: &str, tool: StepTool, input: &Value, cwd: Option<&str>) -> String {
    let path = path_from_input(input, cwd);
    match tool {
        StepTool::Read => path
            .map(|path| format!("Read {path}"))
            .unwrap_or_else(|| "Read".to_string()),
        StepTool::Edit => path
            .map(|path| format!("Edited {path}"))
            .unwrap_or_else(|| "Edited".to_string()),
        StepTool::Shell => string_arg(input, &["command", "cmd"])
            .map(|cmd| format!("Ran {}", first_line(cmd)))
            .unwrap_or_else(|| name.to_string()),
        StepTool::Search => string_arg(input, &["pattern", "query", "glob_pattern", "globPattern"])
            .map(|query| format!("Searched for {}", first_line(query)))
            .unwrap_or_else(|| name.to_string()),
        StepTool::Web => string_arg(input, &["query", "url"])
            .map(|query| first_line(query).to_string())
            .unwrap_or_else(|| name.to_string()),
        StepTool::Mcp => name.rsplit("__").next().unwrap_or(name).to_string(),
        _ => name.to_string(),
    }
}

fn claude_start_detail(tool: StepTool, input: &Value) -> Option<String> {
    if tool != StepTool::Shell {
        return None;
    }
    string_arg(input, &["command", "cmd"]).map(|cmd| cap_detail(cmd).to_string())
}

fn claude_edit(name: &str, input: &Value, cwd: Option<&str>) -> Option<FileEdit> {
    if claude_tool(name) != StepTool::Edit {
        return None;
    }
    let path = path_from_input(input, cwd)?;
    let lower = name.to_ascii_lowercase();
    if lower == "write" {
        let content = match input.get("content") {
            Some(Value::String(text)) => text.as_str(),
            _ => "",
        };
        let size = write_size(content);
        return Some(FileEdit {
            path,
            added: size.added,
            removed: size.removed,
        });
    }
    if lower == "multiedit" {
        let parts = input
            .get("edits")
            .and_then(Value::as_array)
            .map(|edits| {
                edits
                    .iter()
                    .map(|item| {
                        let Some(obj) = item.as_object() else {
                            return edit_size("", "");
                        };
                        let old = obj.get("old_string").and_then(Value::as_str).unwrap_or("");
                        let new = obj.get("new_string").and_then(Value::as_str).unwrap_or("");
                        edit_size(old, new)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if input.get("edits").and_then(Value::as_array).is_some() {
            let size = sum_edit_size(&parts);
            return Some(FileEdit {
                path,
                added: size.added,
                removed: size.removed,
            });
        }
    }
    if lower == "notebookedit" {
        let content = match input.get("new_source") {
            Some(Value::String(text)) => Some(text.as_str()),
            _ => match input.get("content") {
                Some(Value::String(text)) => Some(text.as_str()),
                _ => None,
            },
        };
        if let Some(content) = content.filter(|text| !text.is_empty()) {
            let size = write_size(content);
            return Some(FileEdit {
                path,
                added: size.added,
                removed: size.removed,
            });
        }
    }
    let old = input
        .get("old_string")
        .and_then(Value::as_str)
        .unwrap_or("");
    let new = input
        .get("new_string")
        .and_then(Value::as_str)
        .unwrap_or("");
    let size = edit_size(old, new);
    Some(FileEdit {
        path,
        added: size.added,
        removed: size.removed,
    })
}

fn path_from_input(input: &Value, cwd: Option<&str>) -> Option<String> {
    let raw = string_arg(input, &["file_path", "path", "filePath"])?;
    Some(relative_to_cwd(raw, cwd))
}

fn readable_thinking(block: &Block<'_>) -> Option<String> {
    let text = block
        .thinking
        .as_deref()
        .or(block.text.as_deref())
        .unwrap_or("");
    if text.trim().is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn tool_result_text(content: Option<&RawValue>) -> String {
    let Some(content) = content else {
        return String::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(content.get()) else {
        return String::new();
    };
    match value {
        Value::String(text) => text,
        Value::Array(items) => {
            let mut out = String::new();
            for item in items {
                match item {
                    Value::String(text) => out.push_str(&text),
                    Value::Object(map) => {
                        if let Some(Value::String(text)) = map.get("text") {
                            out.push_str(text);
                        }
                    }
                    _ => {}
                }
            }
            out
        }
        _ => String::new(),
    }
}

fn rate_limit_event(raw: &RawValue) -> Option<AgentEvent> {
    let info: Value = serde_json::from_str(raw.get()).ok()?;
    let info = info.as_object()?;
    if info.get("status").and_then(Value::as_str) == Some("rejected") {
        let resets = info
            .get("resetsAt")
            .and_then(number_f64)
            .and_then(epoch_secs);
        return Some(limit_event(resets, "rate limit rejected"));
    }
    let windows_value = info.get("unifiedWindows")?.as_object()?;
    let mut windows = Vec::new();
    let mut best: Option<(f64, Option<i64>)> = None;
    for (name, window) in windows_value {
        let Some(window) = window.as_object() else {
            continue;
        };
        let Some(utilization) = window.get("utilization").and_then(number_f64) else {
            continue;
        };
        let resets = window
            .get("resetsAt")
            .and_then(number_f64)
            .and_then(epoch_secs);
        windows.push(UsageWindow {
            name: name.clone(),
            utilization,
            resets_at: resets,
        });
        let replace = match best {
            Some((best_util, _)) => utilization > best_util,
            None => true,
        };
        if replace {
            best = Some((utilization, resets));
        }
    }
    let (utilization, resets_at) = best?;
    Some(AgentEvent::Usage {
        utilization,
        resets_at,
        windows: Some(windows),
    })
}

fn result_event(line: &ClaudeLine<'_>) -> AgentEvent {
    let text = line.result.as_deref().unwrap_or("").to_string();
    if line.is_error == Some(true) && LIMIT_RE.is_match(&text) {
        return limit_event(None, text);
    }
    let tokens = line
        .usage
        .and_then(|usage| token_sum(usage, "input_tokens", "output_tokens"));
    AgentEvent::Result {
        ok: line.is_error != Some(true),
        text,
        session_id: line.session_id.as_ref().map(|id| id.to_string()),
        duration_ms: line.duration_ms,
        cost_usd: line.total_cost_usd,
        tokens,
    }
}

fn token_sum(raw: &RawValue, input_key: &str, output_key: &str) -> Option<i64> {
    let value: Value = serde_json::from_str(raw.get()).ok()?;
    let obj = value.as_object()?;
    let input = obj.get(input_key).and_then(number_f64)?;
    let output = obj.get(output_key).and_then(number_f64)?;
    token_total(input, output)
}

fn record_or_empty(raw: Option<&RawValue>) -> Value {
    let Some(raw) = raw else {
        return empty_object();
    };
    match serde_json::from_str::<Value>(raw.get()) {
        Ok(Value::Object(map)) => Value::Object(map),
        _ => empty_object(),
    }
}

fn limit_event(resets_at: Option<i64>, message: impl Into<String>) -> AgentEvent {
    AgentEvent::Limit {
        resets_at,
        message: message.into(),
    }
}

fn step(
    id: String,
    phase: StepPhase,
    tool: StepTool,
    title: String,
    detail: Option<String>,
    ok: Option<bool>,
    edit: Option<FileEdit>,
) -> AgentEvent {
    AgentEvent::Step {
        id,
        phase,
        tool,
        title,
        detail,
        ok,
        edit,
    }
}
