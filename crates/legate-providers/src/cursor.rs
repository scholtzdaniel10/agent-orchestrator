//! Cursor CLI, and Grok through that same CLI. Follows `src/core/providers/cursor.ts`.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use async_trait::async_trait;
use legate_core::{
    AgentEvent, BridgeInfo, FileEdit, GROK_DEFAULT_MODEL, JobAccess, ModelOption, ProviderId,
    RunOptions, StepPhase, StepTool,
};
use regex::Regex;
use serde_json::{Value, value::RawValue};

use crate::adapter::{
    Bin, InteractiveCommand, InteractiveOptions, LimitError, ProviderAdapter, RunHandle,
};
use crate::edit_size::{
    cap_detail, edit_size, first_line, relative_to_cwd, reported_edit_size, write_size,
};
use crate::error::ProviderError;
use crate::lenient::{self, Message};
use crate::note::with_legate_note;
use crate::plugins::{AgentPlugins, plugin_args};
use crate::process::{self, LineParser, VersionCache};
use crate::step_state::{PendingStep, StepParseState};
use crate::util::{job_access, lock, non_empty, path_string};
use crate::values::{empty_object, js_number, number_f64, string_arg, token_total};

static LIMIT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)usage limit|rate.?limit|limit reached|out of (usage|credits)|spend limit|quota",
    )
    .expect("cursor limit pattern")
});

static SIGNED_IN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)logged in").expect("cursor sign-in pattern"));

const EDIT_CLI: &str = r#"{"permissions":{"allow":["Read(**)","Write(**)"],"deny":["Shell(*)"]}}"#;

const ZERO_WIDTH: &[char] = &['\u{200B}', '\u{200C}', '\u{200D}', '\u{FEFF}'];

/// Called when a Grok run needs a model and the request did not name one.
pub type ModelSource = Arc<dyn Fn() -> Option<String> + Send + Sync>;

pub struct CursorAdapter {
    provider: ProviderId,
    found: Mutex<Option<Bin>>,
    model_for: Option<ModelSource>,
    plugins: Mutex<AgentPlugins>,
    cached_models: Mutex<Option<Vec<ModelOption>>>,
    versions: VersionCache,
    steps: Arc<Mutex<StepParseState>>,
}

impl CursorAdapter {
    pub fn new(bin: Option<Bin>) -> Self {
        Self::build(ProviderId::Cursor, bin, None)
    }

    pub fn grok(bin: Option<Bin>, model_for: Option<ModelSource>) -> Self {
        Self::build(ProviderId::Grok, bin, model_for)
    }

    fn build(provider: ProviderId, bin: Option<Bin>, model_for: Option<ModelSource>) -> Self {
        let provider = if provider == ProviderId::Grok {
            ProviderId::Grok
        } else {
            ProviderId::Cursor
        };
        Self {
            provider,
            found: Mutex::new(bin),
            model_for,
            plugins: Mutex::new(AgentPlugins::default()),
            cached_models: Mutex::new(None),
            versions: VersionCache::new(),
            steps: Arc::new(Mutex::new(StepParseState::default())),
        }
    }

    pub fn set_plugins(&self, plugins: AgentPlugins) {
        *lock(&self.plugins) = plugins;
    }

    pub async fn list_models(&self) -> Vec<ModelOption> {
        if let Some(cached) = lock(&self.cached_models).clone() {
            return cached;
        }
        let run = process::run_captured(&self.resolved_bin(), &["models".to_string()]).await;
        if run.code != Some(0) {
            return Vec::new();
        }
        let mut models = parse_cursor_models(&run.stdout);
        if self.provider == ProviderId::Grok {
            models.retain(|model| model.id.starts_with("grok"));
        }
        if models.is_empty() {
            return Vec::new();
        }
        *lock(&self.cached_models) = Some(models.clone());
        models
    }

    pub fn interactive(&self, opts: &InteractiveOptions) -> InteractiveCommand {
        let bin = self.resolved_bin();
        let plugins = lock(&self.plugins).clone();
        let mut args = bin.args.clone();
        if let Some(resume) = resume_flag(opts) {
            args.push("--resume".to_string());
            args.push(resume);
        }
        if let Some(model) = self.cli_model(opts.model.as_deref()) {
            args.push("--model".to_string());
            args.push(model);
        }
        args.extend(plugin_args(&plugins.worker));
        if non_empty(&opts.resume).is_none()
            && let Some(prompt) = non_empty(&opts.prompt)
        {
            args.push("--trust".to_string());
            args.push(with_legate_note(prompt));
        }
        InteractiveCommand {
            command: bin.command,
            args,
            env: current_cursor_env(),
        }
    }

    pub fn parse_events(&self, line: &str) -> Vec<AgentEvent> {
        parse_line(line, &self.steps)
    }

    fn cli_model(&self, requested: Option<&str>) -> Option<String> {
        if self.provider != ProviderId::Grok {
            return requested
                .filter(|text| !text.is_empty())
                .map(str::to_string);
        }
        if let Some(requested) = requested.filter(|text| text.starts_with("grok")) {
            return Some(requested.to_string());
        }
        if let Some(given) = self.model_for.as_ref().and_then(|source| source())
            && given.starts_with("grok")
        {
            return Some(given);
        }
        Some(GROK_DEFAULT_MODEL.to_string())
    }

    fn resolved_bin(&self) -> Bin {
        let mut found = lock(&self.found);
        if let Some(bin) = found.as_ref() {
            return bin.clone();
        }
        let bin = default_cursor_bin();
        if bin.command != "agent" {
            *found = Some(bin.clone());
        }
        bin
    }

    fn prepare(
        &self,
        prompt: &str,
        cwd: &Path,
        opts: &RunOptions,
    ) -> Result<process::SpawnSpec, ProviderError> {
        let mut args = vec![
            "-p".to_string(),
            "--trust".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
        ];
        let resume = non_empty(&opts.resume);
        if let Some(resume) = resume {
            args.push("--resume".to_string());
            args.push(resume.to_string());
        }
        let access = job_access(opts);
        if access == JobAccess::Full && opts.bridge.is_none() {
            args.push("--force".to_string());
        }
        let cleanup = if access == JobAccess::Edit && opts.bridge.is_none() {
            allow_edits(cwd)?
        } else {
            None
        };
        if let Some(bridge) = &opts.bridge {
            write_bridge(cwd, bridge)?;
            args.push("--approve-mcps".to_string());
        }
        if let Some(model) = self.cli_model(opts.model.as_deref()) {
            args.push("--model".to_string());
            args.push(model);
        }
        let plugins = lock(&self.plugins).clone();
        let dirs = if opts.bridge.is_some() {
            &plugins.lead
        } else {
            &plugins.worker
        };
        args.extend(plugin_args(dirs));
        let prompt = if resume.is_some() {
            prompt.to_string()
        } else {
            with_legate_note(prompt)
        };
        Ok(process::SpawnSpec {
            bin: self.resolved_bin(),
            args,
            prompt,
            cwd: cwd.to_path_buf(),
            env: current_cursor_env(),
            parse: parser(&self.steps),
            cleanup,
        })
    }
}

#[async_trait]
impl ProviderAdapter for CursorAdapter {
    fn id(&self) -> ProviderId {
        self.provider
    }

    async fn is_installed(&self) -> bool {
        self.versions.installed(&self.resolved_bin()).await
    }

    async fn version(&self) -> Option<String> {
        let stdout = self.versions.stdout(&self.resolved_bin()).await;
        crate::versions::parse_cli_version(self.provider, &stdout)
    }

    async fn is_signed_in(&self) -> bool {
        let run = process::run_captured(&self.resolved_bin(), &["status".to_string()]).await;
        run.code == Some(0) && SIGNED_IN.is_match(&format!("{}{}", run.stdout, run.stderr))
    }

    fn run(&self, _job_id: &str, prompt: &str, cwd: &Path, opts: &RunOptions) -> RunHandle {
        match self.prepare(prompt, cwd, opts) {
            Ok(spec) => process::spawn_cli(spec),
            Err(err) => RunHandle::failed(err.to_string()),
        }
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

pub fn parse_cursor_models(stdout: &str) -> Vec<ModelOption> {
    let mut models = Vec::new();
    for raw in stdout.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        let cleaned: String = raw.chars().filter(|c| !ZERO_WIDTH.contains(c)).collect();
        let line = cleaned.trim();
        let Some(sep) = line.find(" - ") else {
            continue;
        };
        if sep == 0 {
            continue;
        }
        let id = &line[..sep];
        let label = &line[sep + 3..];
        if label.is_empty() || id.chars().any(char::is_whitespace) {
            continue;
        }
        models.push(ModelOption {
            id: id.to_string(),
            label: label.to_string(),
        });
    }
    models
}

/// On Windows, drop Git Bash markers so Cursor hooks stay PowerShell. Elsewhere, inherit.
pub fn cursor_env(
    platform: &str,
    source: &BTreeMap<String, String>,
) -> Option<BTreeMap<String, String>> {
    if platform != "win32" {
        return None;
    }
    let mut env = BTreeMap::new();
    for (key, value) in source {
        let upper = key.to_uppercase();
        if upper != "SHELL" && upper != "MSYSTEM" {
            env.insert(key.clone(), value.clone());
        }
    }
    Some(env)
}

pub fn node_platform() -> &'static str {
    match std::env::consts::OS {
        "windows" => "win32",
        "macos" => "darwin",
        other => other,
    }
}

pub fn cursor_version_dirs(local_app_data: Option<&Path>, app_data: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(local) = local_app_data {
        dirs.push(local.join("cursor-agent").join("versions"));
    }
    if let Some(app) = app_data {
        dirs.push(
            app.join("Cursor")
                .join("User")
                .join("globalStorage")
                .join("anysphere.cursor-agent-worker")
                .join("agent-cli")
                .join(".local")
                .join("share")
                .join("cursor-agent")
                .join("versions"),
        );
    }
    dirs
}

pub fn resolve_cursor_bin(
    local_app_data: Option<&Path>,
    app_data: Option<&Path>,
    platform: &str,
) -> Bin {
    if platform != "win32" {
        return Bin::new("agent");
    }
    for dir in cursor_version_dirs(local_app_data, app_data) {
        if let Some(bin) = newest_in(&dir) {
            return bin;
        }
    }
    Bin::new("agent")
}

fn current_cursor_env() -> Option<BTreeMap<String, String>> {
    let platform = node_platform();
    if platform != "win32" {
        return None;
    }
    let source: BTreeMap<String, String> = std::env::vars().collect();
    cursor_env(platform, &source)
}

fn default_cursor_bin() -> Bin {
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let app = std::env::var_os("APPDATA").map(PathBuf::from);
    resolve_cursor_bin(local.as_deref(), app.as_deref(), node_platform())
}

fn newest_in(dir: &Path) -> Option<Bin> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut names = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            names.push(entry.file_name());
        }
    }
    names.sort_by(|left, right| right.cmp(left));
    for name in names {
        let dir = dir.join(name);
        let node = dir.join("node.exe");
        let index = dir.join("index.js");
        if node.exists() && index.exists() {
            return Some(Bin::with_args(path_string(&node), [path_string(&index)]));
        }
    }
    None
}

fn resume_flag(opts: &InteractiveOptions) -> Option<String> {
    if opts.resume.is_some() {
        non_empty(&opts.resume).map(str::to_string)
    } else {
        non_empty(&opts.session_id).map(str::to_string)
    }
}

fn allow_edits(cwd: &Path) -> Result<Option<Box<dyn FnOnce() + Send>>, ProviderError> {
    let cursor_dir = cwd.join(".cursor");
    let cli_path = cursor_dir.join("cli.json");
    if cli_path.exists() {
        return Ok(None);
    }
    let created_dir = !cursor_dir.exists();
    std::fs::create_dir_all(&cursor_dir)?;
    std::fs::write(&cli_path, EDIT_CLI)?;
    Ok(Some(Box::new(move || {
        let _ = std::fs::remove_file(&cli_path);
        if !created_dir {
            return;
        }
        let empty = std::fs::read_dir(&cursor_dir)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(false);
        if empty {
            let _ = std::fs::remove_dir(&cursor_dir);
        }
    })))
}

fn write_bridge(cwd: &Path, bridge: &BridgeInfo) -> Result<(), ProviderError> {
    let cursor_dir = cwd.join(".cursor");
    std::fs::create_dir_all(&cursor_dir)?;
    // Key order matches JSON.stringify in cursor.ts. serde_json::Map sorts keys.
    let url = serde_json::to_string(&bridge.url)?;
    let auth = serde_json::to_string(&format!("Bearer {}", bridge.token))?;
    let mcp = format!(
        r#"{{"mcpServers":{{"orchestrator":{{"url":{url},"headers":{{"Authorization":{auth}}}}}}}}}"#
    );
    const CLI: &str =
        r#"{"permissions":{"allow":["Mcp(orchestrator:*)"],"deny":["Shell(*)","Write(**)"]}}"#;
    std::fs::write(cursor_dir.join("mcp.json"), mcp)?;
    std::fs::write(cursor_dir.join("cli.json"), CLI)?;
    Ok(())
}

fn parser(steps: &Arc<Mutex<StepParseState>>) -> LineParser {
    let steps = Arc::clone(steps);
    Arc::new(move |line| parse_line(line, &steps))
}

fn parse_line(line: &str, steps: &Mutex<StepParseState>) -> Vec<AgentEvent> {
    let Ok(parsed) = serde_json::from_str::<CursorLine<'_>>(line) else {
        return Vec::new();
    };
    map_cursor(&parsed, &mut lock(steps))
}

#[derive(Debug, serde::Deserialize)]
#[serde(bound(deserialize = "'de: 'a"))]
struct CursorLine<'a> {
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
    text: Option<Cow<'a, str>>,
    #[serde(default, deserialize_with = "lenient::de_opt_f64")]
    timestamp_ms: Option<f64>,
    #[serde(borrow, default)]
    message: Option<&'a RawValue>,
    #[serde(default, deserialize_with = "lenient::de_opt_cow")]
    model_call_id: Option<Cow<'a, str>>,
    #[serde(default, deserialize_with = "lenient::de_opt_cow")]
    call_id: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    tool_call: Option<&'a RawValue>,
    #[serde(default, deserialize_with = "lenient::de_opt_bool")]
    is_error: Option<bool>,
    #[serde(default, deserialize_with = "lenient::de_opt_cow")]
    result: Option<Cow<'a, str>>,
    #[serde(default, deserialize_with = "lenient::de_opt_f64")]
    duration_ms: Option<f64>,
    #[serde(borrow, default)]
    usage: Option<&'a RawValue>,
}

fn map_cursor(line: &CursorLine<'_>, state: &mut StepParseState) -> Vec<AgentEvent> {
    let kind = line.kind.as_deref();
    if kind == Some("system") && line.subtype.as_deref() == Some("init") {
        if let (Some(session), Some(cwd)) = (&line.session_id, &line.cwd) {
            state.remember_cwd(session.to_string(), cwd.to_string());
        }
        return init_event(line).into_iter().collect();
    }
    if kind == Some("thinking") {
        return thinking_events(line, state);
    }
    if kind == Some("assistant") {
        let Some(message) = line.message else {
            return Vec::new();
        };
        return assistant_events(message, line);
    }
    if kind == Some("tool_call") {
        return tool_call_events(line, state);
    }
    if kind == Some("result") {
        return vec![result_event(line)];
    }
    Vec::new()
}

fn init_event(line: &CursorLine<'_>) -> Option<AgentEvent> {
    Some(AgentEvent::Init {
        session_id: line.session_id.as_ref()?.to_string(),
        model: line.model.as_ref().map(|model| model.to_string()),
    })
}

fn thinking_events(line: &CursorLine<'_>, state: &mut StepParseState) -> Vec<AgentEvent> {
    let raw = line.text.as_deref().unwrap_or("");
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let id = match line.timestamp_ms {
        Some(stamp) => format!("think-{}", js_number(stamp)),
        None => state.next_id("think"),
    };
    vec![step(
        id,
        StepPhase::End,
        StepTool::Think,
        first_line(trimmed).to_string(),
        Some(cap_detail(raw).to_string()),
        None,
        None,
    )]
}

fn assistant_events(message: &RawValue, line: &CursorLine<'_>) -> Vec<AgentEvent> {
    let Some(message) = lenient::parse_raw::<Message<'_>>(message) else {
        return Vec::new();
    };
    let mut parts = Vec::new();
    for block in &message.content {
        if block.kind.as_deref() == Some("text")
            && let Some(text) = block.text.as_deref()
        {
            parts.push(text);
        }
    }
    let text = parts.join("");
    if text.is_empty() {
        return Vec::new();
    }
    let session = line.session_id.as_deref().unwrap_or("x");
    let say_id = if let Some(id) = message.id.as_deref() {
        id.to_string()
    } else if let Some(call) = line.model_call_id.as_deref() {
        format!("say-{call}")
    } else {
        format!("say-{session}-{}", first_line(&text))
    };
    vec![
        AgentEvent::Text { text: text.clone() },
        step(
            say_id,
            StepPhase::End,
            StepTool::Say,
            first_line(&text).to_string(),
            Some(cap_detail(&text).to_string()),
            None,
            None,
        ),
    ]
}

fn tool_call_events(line: &CursorLine<'_>, state: &mut StepParseState) -> Vec<AgentEvent> {
    let tool_call = record_or_empty(line.tool_call);
    let entry = cursor_tool_entry(&tool_call);
    if entry.is_none() && line.call_id.is_none() {
        return Vec::new();
    }
    let id = match &line.call_id {
        Some(id) => id.to_string(),
        None => state.next_id("tool"),
    };
    let cwd = state.cwd(line.session_id.as_deref());
    let phase = if line.subtype.as_deref() == Some("completed")
        || line.subtype.as_deref() == Some("error")
    {
        StepPhase::End
    } else {
        StepPhase::Start
    };
    let Some((key, body)) = entry else {
        let ok = if phase == StepPhase::End {
            Some(line.subtype.as_deref() != Some("error"))
        } else {
            None
        };
        return vec![step(
            id,
            phase,
            StepTool::Other,
            "tool".to_string(),
            None,
            ok,
            None,
        )];
    };
    let args = match body.get("args") {
        Some(Value::Object(_)) => body.get("args").cloned().unwrap_or_else(empty_object),
        _ => empty_object(),
    };
    let result = body.get("result").cloned().unwrap_or(Value::Null);
    let tool = cursor_tool_kind(key);
    let title = cursor_title(key, tool, &args, cwd.as_deref());
    let edit = cursor_edit(tool, &args, &result, cwd.as_deref());
    if phase == StepPhase::Start {
        let detail = cursor_start_detail(tool, &args);
        state.remember(
            id.clone(),
            PendingStep {
                tool,
                title: title.clone(),
                edit: edit.clone(),
            },
        );
        return vec![step(id, phase, tool, title, detail, None, edit)];
    }
    let pending = state.tool(&id);
    let edit = edit.or_else(|| pending.and_then(|pending| pending.edit));
    let ok = cursor_result_ok(&result) && line.subtype.as_deref() != Some("error");
    vec![step(
        id,
        phase,
        tool,
        title,
        cursor_result_detail(&result),
        Some(ok),
        edit,
    )]
}

fn cursor_tool_entry(tool_call: &Value) -> Option<(&str, &Value)> {
    let obj = tool_call.as_object()?;
    for (key, value) in obj {
        if key.ends_with("ToolCall") && value.is_object() {
            return Some((key.as_str(), value));
        }
    }
    None
}

fn cursor_tool_kind(key: &str) -> StepTool {
    let name = strip_tool_call(key).to_ascii_lowercase();
    match name.as_str() {
        "read" => StepTool::Read,
        "write" | "edit" | "applypatch" | "strreplace" | "delete" => StepTool::Edit,
        "shell" | "awaitshell" | "bash" => StepTool::Shell,
        "glob" | "grep" | "ls" | "semsearch" | "search" => StepTool::Search,
        "websearch" | "webfetch" | "web" => StepTool::Web,
        _ if name == "mcp" || name.starts_with("mcp") => StepTool::Mcp,
        _ => StepTool::Other,
    }
}

fn cursor_title(key: &str, tool: StepTool, args: &Value, cwd: Option<&str>) -> String {
    let path = string_arg(args, &["path", "filePath"]).map(|path| relative_to_cwd(path, cwd));
    match tool {
        StepTool::Read => path
            .map(|path| format!("Read {path}"))
            .unwrap_or_else(|| "Read".to_string()),
        StepTool::Edit => path
            .map(|path| format!("Edited {path}"))
            .unwrap_or_else(|| "Edited".to_string()),
        StepTool::Shell => string_arg(args, &["command", "cmd"])
            .map(|cmd| format!("Ran {}", first_line(cmd)))
            .unwrap_or_else(|| "Ran".to_string()),
        StepTool::Search => string_arg(args, &["globPattern", "pattern", "query", "glob_pattern"])
            .map(|query| format!("Searched for {}", first_line(query)))
            .unwrap_or_else(|| "Searched".to_string()),
        StepTool::Web => string_arg(args, &["searchTerm", "query", "url"])
            .map(|query| first_line(query).to_string())
            .unwrap_or_else(|| "Web".to_string()),
        StepTool::Mcp => string_arg(args, &["name", "toolName"])
            .map(str::to_string)
            .unwrap_or_else(|| strip_tool_call(key).to_string()),
        _ => strip_tool_call(key).to_string(),
    }
}

fn cursor_start_detail(tool: StepTool, args: &Value) -> Option<String> {
    if tool != StepTool::Shell {
        return None;
    }
    string_arg(args, &["command", "cmd"]).map(|cmd| cap_detail(cmd).to_string())
}

fn cursor_edit(
    tool: StepTool,
    args: &Value,
    result: &Value,
    cwd: Option<&str>,
) -> Option<FileEdit> {
    if tool != StepTool::Edit {
        return None;
    }
    let path = relative_to_cwd(string_arg(args, &["path", "filePath"])?, cwd);
    if let Some(size) = reported_edit_size(result) {
        return Some(FileEdit {
            path,
            added: size.added,
            removed: size.removed,
        });
    }
    let old = string_arg(args, &["old_string", "oldString"]).unwrap_or("");
    let new = string_arg(args, &["new_string", "newString"]).unwrap_or("");
    if !old.is_empty() || !new.is_empty() {
        let size = edit_size(old, new);
        return Some(FileEdit {
            path,
            added: size.added,
            removed: size.removed,
        });
    }
    if let Some(content) = string_arg(
        args,
        &["contents", "fileContent", "streamContent", "content"],
    ) {
        let size = write_size(content);
        return Some(FileEdit {
            path,
            added: size.added,
            removed: size.removed,
        });
    }
    Some(FileEdit {
        path,
        added: 0,
        removed: 0,
    })
}

fn cursor_result_ok(result: &Value) -> bool {
    let Some(obj) = result.as_object() else {
        return true;
    };
    if obj.contains_key("error") || obj.contains_key("failure") || obj.contains_key("rejected") {
        return false;
    }
    obj.get("success") != Some(&Value::Bool(false))
}

fn cursor_result_detail(result: &Value) -> Option<String> {
    if result.is_null() {
        return None;
    }
    if let Some(success) = result.get("success").and_then(Value::as_object) {
        if let Some(text) = success.get("content").and_then(Value::as_str) {
            return Some(cap_detail(text).to_string());
        }
        if let Some(text) = success.get("stdout").and_then(Value::as_str) {
            return Some(cap_detail(text).to_string());
        }
        if let Some(text) = success.get("output").and_then(Value::as_str) {
            return Some(cap_detail(text).to_string());
        }
        if let Some(files) = success.get("files").and_then(Value::as_array) {
            let joined = files.iter().map(js_string).collect::<Vec<_>>().join("\n");
            return Some(cap_detail(&joined).to_string());
        }
    }
    if let Some(error) = result.as_object().and_then(|obj| obj.get("error"))
        && error.is_object()
    {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| serde_json::to_string(error).ok())?;
        return Some(cap_detail(&message).to_string());
    }
    serde_json::to_string(result)
        .ok()
        .map(|text| cap_detail(&text).to_string())
}

fn js_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "null".to_string(),
        Value::Bool(true) => "true".to_string(),
        Value::Bool(false) => "false".to_string(),
        Value::Number(number) => number.to_string(),
        Value::Array(items) => items.iter().map(js_string).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

fn result_event(line: &CursorLine<'_>) -> AgentEvent {
    let text = line.result.as_deref().unwrap_or("").to_string();
    if line.is_error == Some(true) && LIMIT_RE.is_match(&text) {
        return AgentEvent::Limit {
            resets_at: None,
            message: text,
        };
    }
    AgentEvent::Result {
        ok: line.is_error != Some(true),
        text,
        session_id: line.session_id.as_ref().map(|id| id.to_string()),
        duration_ms: line.duration_ms,
        cost_usd: None,
        tokens: line
            .usage
            .and_then(|usage| token_sum(usage, "inputTokens", "outputTokens")),
    }
}

fn token_sum(raw: &RawValue, input_key: &str, output_key: &str) -> Option<i64> {
    let value: Value = serde_json::from_str(raw.get()).ok()?;
    let obj = value.as_object()?;
    token_total(
        obj.get(input_key).and_then(number_f64)?,
        obj.get(output_key).and_then(number_f64)?,
    )
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

fn strip_tool_call(key: &str) -> &str {
    const SUFFIX: &[u8] = b"toolcall";
    let bytes = key.as_bytes();
    if bytes.len() >= SUFFIX.len() {
        let start = bytes.len() - SUFFIX.len();
        if key.is_char_boundary(start) && bytes[start..].eq_ignore_ascii_case(SUFFIX) {
            return &key[..start];
        }
    }
    key
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
