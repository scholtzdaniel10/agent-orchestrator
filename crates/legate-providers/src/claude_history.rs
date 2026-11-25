//! Reads Claude Code's own session files so Legate can show chats started elsewhere (the CLI,
//! the Claude app, the IDE extensions) and keep showing new turns as they are written.
//!
//! Claude Code saves every session as `<config>/projects/<slug>/<session-id>.jsonl`, one JSON
//! record per line, where `<slug>` is the session's folder with every character that is not a
//! letter or digit replaced by `-`. This module only reads those files; it never writes them.
//! It keeps message text and tool names and drops everything else: account ids, tool results
//! and file snapshots never leave this module.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use legate_core::{LeadMessage, LeadRole, LeadStatus, ProviderId};
use serde_json::Value;

/// One Claude Code session, ready to show as a chat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeSession {
    pub id: String,
    /// The folder the session ran in.
    pub cwd: Option<String>,
    /// The session's own title: a custom title, else the one Claude generated.
    pub title: Option<String>,
    /// The first thing the user typed, for a title when there is none.
    pub first_prompt: Option<String>,
    /// Which program started it: `claude-desktop`, `cli`, `sdk-cli` (headless runs).
    pub entrypoint: Option<String>,
    pub messages: Vec<LeadMessage>,
}

impl ClaudeSession {
    /// A title for the chat list.
    pub fn display_title(&self) -> String {
        if let Some(title) = self
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            return clip(title, 80);
        }
        match self.first_prompt.as_deref() {
            Some(prompt) => clip(prompt.lines().next().unwrap_or(prompt).trim(), 60),
            None => "Claude Code chat".to_string(),
        }
    }
}

/// `<config>/projects`, where `<config>` is `CLAUDE_CONFIG_DIR` or `~/.claude`.
pub fn projects_root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|dir| !dir.is_empty()) {
        return Some(PathBuf::from(dir).join("projects"));
    }
    let home = std::env::var_os("USERPROFILE")
        .filter(|dir| !dir.is_empty())
        .or_else(|| std::env::var_os("HOME").filter(|dir| !dir.is_empty()))?;
    Some(PathBuf::from(home).join(".claude").join("projects"))
}

/// The folder name Claude Code files a session under: every character that is not an ASCII
/// letter or digit becomes `-`. `C:\Users\me\app` → `C--Users-me-app`.
pub fn slug(folder: &str) -> String {
    folder
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// The session folders that belong to `project`: its own, and those of its Claude worktrees
/// (`<project>/.claude/worktrees/<name>`).
pub fn session_dirs(root: &Path, project: &str) -> Vec<PathBuf> {
    let own = slug(project.trim_end_matches(['\\', '/']));
    if own.is_empty() {
        return Vec::new();
    }
    let worktrees = format!("{own}--claude-worktrees-");
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.eq_ignore_ascii_case(&own) || name.starts_with(&worktrees)
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    dirs.sort();
    dirs
}

/// The session files in `dir`.
pub fn session_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .collect::<Vec<_>>();
    files.sort();
    files
}

/// Where a session's file is, given the folder it ran in.
pub fn session_file(root: &Path, cwd: &str, id: &str) -> PathBuf {
    root.join(slug(cwd.trim_end_matches(['\\', '/'])))
        .join(format!("{id}.jsonl"))
}

/// Size and modified time, as a string that changes whenever the file does.
pub fn file_sig(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    Some(format!("{}:{}", meta.len(), modified_ms(&meta)?))
}

/// Epoch ms of the file's last write.
pub fn file_modified_ms(path: &Path) -> Option<i64> {
    modified_ms(&std::fs::metadata(path).ok()?)
}

fn modified_ms(meta: &std::fs::Metadata) -> Option<i64> {
    let since = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(since.as_millis()).ok()
}

/// Reads one session file. `None` when it cannot be read or holds no session.
pub fn read_session(path: &Path) -> Option<ClaudeSession> {
    let text = std::fs::read_to_string(path).ok()?;
    let fallback = path.file_stem()?.to_string_lossy().into_owned();
    parse_session(&text, &fallback)
}

/// Parses the text of a session file. `fallback_id` is used when no line names the session.
pub fn parse_session(text: &str, fallback_id: &str) -> Option<ClaudeSession> {
    let mut id = None;
    let mut cwd = None;
    let mut entrypoint = None;
    let mut custom_title = None;
    let mut ai_title = None;
    let mut first_prompt = None;
    let mut messages: Vec<LeadMessage> = Vec::new();
    let mut turn: Option<Turn> = None;

    for line in text.lines() {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let kind = record.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "custom-title" => {
                custom_title = string(&record, "customTitle").or(custom_title);
                continue;
            }
            "ai-title" => {
                ai_title = string(&record, "aiTitle").or(ai_title);
                continue;
            }
            _ => {}
        }
        if record.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        if id.is_none() {
            id = string(&record, "sessionId");
        }
        if cwd.is_none() {
            cwd = string(&record, "cwd");
        }
        if entrypoint.is_none() {
            entrypoint = string(&record, "entrypoint");
        }
        match kind {
            "user" => {
                if record.get("isMeta").and_then(Value::as_bool) == Some(true) {
                    continue;
                }
                let text = match user_record(&record) {
                    UserRecord::Text(text) => text,
                    UserRecord::ToolResult => continue,
                    // Something the CLI wrote in the user's place (a slash command, a
                    // notification): not shown, but what Claude says next is a new reply.
                    UserRecord::Hidden => {
                        if let Some(done) = turn.take() {
                            messages.push(done.into_message());
                        }
                        continue;
                    }
                };
                if let Some(done) = turn.take() {
                    messages.push(done.into_message());
                }
                if first_prompt.is_none() {
                    first_prompt = Some(text.clone());
                }
                messages.push(LeadMessage {
                    id: string(&record, "uuid").unwrap_or_else(|| format!("u{}", messages.len())),
                    role: LeadRole::User,
                    text,
                    provider: None,
                    model: None,
                    status: LeadStatus::Done,
                    kind: None,
                    proposal: None,
                    extra: BTreeMap::new(),
                });
            }
            "assistant" => {
                let Some(message) = record.get("message") else {
                    continue;
                };
                let current = turn.get_or_insert_with(|| Turn {
                    id: string(&record, "uuid").unwrap_or_else(|| format!("a{}", messages.len())),
                    ..Turn::default()
                });
                if current.model.is_none() {
                    current.model = message
                        .get("model")
                        .and_then(Value::as_str)
                        .filter(|model| !model.starts_with('<'))
                        .map(str::to_string);
                }
                for block in message
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            if let Some(text) = block
                                .get("text")
                                .and_then(Value::as_str)
                                .map(str::trim)
                                .filter(|text| !text.is_empty())
                            {
                                current.texts.push(text.to_string());
                            }
                        }
                        Some("tool_use") => {
                            if let Some(name) =
                                block.get("name").and_then(Value::as_str).map(tool_label)
                                && !current.tools.contains(&name)
                            {
                                current.tools.push(name);
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(done) = turn.take() {
        messages.push(done.into_message());
    }
    messages.retain(|message| !message.text.is_empty());
    let id = id.unwrap_or_else(|| fallback_id.to_string());
    if id.is_empty() {
        return None;
    }
    Some(ClaudeSession {
        id,
        cwd,
        title: custom_title.or(ai_title),
        first_prompt,
        entrypoint,
        messages,
    })
}

/// Everything Claude said between two prompts, shown as one reply.
#[derive(Default)]
struct Turn {
    id: String,
    model: Option<String>,
    texts: Vec<String>,
    tools: Vec<String>,
}

impl Turn {
    fn into_message(self) -> LeadMessage {
        let mut text = self.texts.join("\n\n");
        if !self.tools.is_empty() {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(&format!("Used {}.", self.tools.join(", ")));
        }
        LeadMessage {
            id: self.id,
            role: LeadRole::Lead,
            text,
            provider: Some(ProviderId::Claude),
            model: self.model,
            status: LeadStatus::Done,
            kind: None,
            proposal: None,
            extra: BTreeMap::new(),
        }
    }
}

/// What the user typed in a `user` record, or `None` for tool results, slash-command echoes
/// and other wrappers the CLI writes as user records.
enum UserRecord {
    Text(String),
    ToolResult,
    Hidden,
}

/// Wrappers whose inner text is a real prompt: a message another Claude session sent.
const SHOWN_WRAPPERS: &[&str] = &["cross-session-message"];

fn user_record(record: &Value) -> UserRecord {
    let Some(content) = record
        .get("message")
        .and_then(|message| message.get("content"))
    else {
        return UserRecord::Hidden;
    };
    let text = match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => {
            let texts = blocks
                .iter()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>();
            if texts.is_empty() {
                return UserRecord::ToolResult;
            }
            texts.join("\n\n")
        }
        _ => return UserRecord::Hidden,
    };
    let text = text.trim();
    if text.is_empty() {
        return UserRecord::Hidden;
    }
    match wrapper_tag(text) {
        None => UserRecord::Text(text.to_string()),
        Some(tag) if SHOWN_WRAPPERS.contains(&tag) => match inner_text(text, tag) {
            Some(inner) => UserRecord::Text(inner),
            None => UserRecord::Hidden,
        },
        Some(_) => UserRecord::Hidden,
    }
}

/// The tag of a record the CLI writes in the user's place (`<command-name>`,
/// `<local-command-stdout>`, `<system-reminder>`, task notifications and the like), if the
/// text is one.
fn wrapper_tag(text: &str) -> Option<&str> {
    let rest = text.strip_prefix('<')?;
    let tag = rest
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .next()
        .unwrap_or("");
    (!tag.is_empty() && text.contains(&format!("</{tag}>"))).then_some(tag)
}

/// The text between `<tag …>` and `</tag>`.
fn inner_text(text: &str, tag: &str) -> Option<String> {
    let open_end = text.find('>')? + 1;
    let close = text.rfind(&format!("</{tag}>"))?;
    let inner = text.get(open_end..close)?.trim();
    (!inner.is_empty()).then(|| inner.to_string())
}

/// A tool's name as people know it: `mcp__server__tool` shows as `tool`.
fn tool_label(name: &str) -> String {
    match name.strip_prefix("mcp__") {
        Some(rest) => rest.rsplit("__").next().unwrap_or(rest).to_string(),
        None => name.to_string(),
    }
}

fn string(record: &Value, key: &str) -> Option<String> {
    record
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out = text.chars().take(max - 1).collect::<String>();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: &str = r#"{"type":"user","sessionId":"s-1","cwd":"C:\\work\\app","entrypoint":"claude-desktop","uuid":"u1","message":{"role":"user","content":"Fix the login bug\nin auth.rs"}}
{"type":"assistant","sessionId":"s-1","uuid":"a1","message":{"id":"m1","model":"claude-opus-5-5","content":[{"type":"text","text":"Looking at it."},{"type":"tool_use","name":"Read","input":{}}]}}
{"type":"user","sessionId":"s-1","uuid":"t1","message":{"role":"user","content":[{"type":"tool_result","content":"secret file text"}]}}
{"type":"assistant","sessionId":"s-1","uuid":"a2","message":{"id":"m2","model":"claude-opus-5-5","content":[{"type":"tool_use","name":"Edit","input":{}},{"type":"tool_use","name":"Read","input":{}},{"type":"text","text":"Fixed."}]}}
{"type":"user","sessionId":"s-1","uuid":"meta","isMeta":true,"message":{"role":"user","content":"hidden"}}
{"type":"user","sessionId":"s-1","uuid":"cmd","message":{"role":"user","content":"<command-name>/compact</command-name>"}}
{"type":"assistant","sessionId":"s-1","uuid":"side","isSidechain":true,"message":{"content":[{"type":"text","text":"subagent"}]}}
{"type":"ai-title","sessionId":"s-1","aiTitle":"Login bug"}
{"type":"user","sessionId":"s-1","uuid":"u2","ownerAccountUuid":"acct","message":{"role":"user","content":[{"type":"text","text":"Thanks"}]}}
not json
"#;

    #[test]
    fn a_hidden_record_starts_a_new_reply_and_session_messages_are_shown() {
        let text = [
            r#"{"type":"user","sessionId":"s","uuid":"u1","message":{"content":"First"}}"#,
            r#"{"type":"assistant","sessionId":"s","uuid":"a1","message":{"content":[{"type":"text","text":"One"}]}}"#,
            r#"{"type":"user","sessionId":"s","uuid":"n","message":{"content":"<task-notification>done</task-notification>"}}"#,
            r#"{"type":"assistant","sessionId":"s","uuid":"a2","message":{"content":[{"type":"text","text":"Two"}]}}"#,
            r#"{"type":"user","sessionId":"s","uuid":"x","message":{"content":"<cross-session-message from=\"local_1\">\nHello from the app\n</cross-session-message>"}}"#,
            r#"{"type":"assistant","sessionId":"s","uuid":"a3","message":{"content":[{"type":"text","text":"Three"}]}}"#,
        ]
        .join("\n");
        let shown = parse_session(&text, "s")
            .unwrap()
            .messages
            .into_iter()
            .map(|message| message.text)
            .collect::<Vec<_>>();
        assert_eq!(
            shown,
            vec!["First", "One", "Two", "Hello from the app", "Three"]
        );
    }

    #[test]
    fn mcp_tools_show_by_their_short_name() {
        assert_eq!(
            tool_label("mcp__ccd_session_mgmt__archive_session"),
            "archive_session"
        );
        assert_eq!(tool_label("Bash"), "Bash");
    }

    #[test]
    fn slug_matches_claude_codes_folder_names() {
        assert_eq!(
            slug(r"C:\Users\Jean\Documents\LPR"),
            "C--Users-Jean-Documents-LPR"
        );
        assert_eq!(
            slug(r"C:\Users\Jean\Documents\agent-orchestrator\.claude\worktrees\a-b"),
            "C--Users-Jean-Documents-agent-orchestrator--claude-worktrees-a-b"
        );
        assert_eq!(slug("/home/me/my_app"), "-home-me-my-app");
    }

    #[test]
    fn a_session_becomes_prompts_and_one_reply_per_turn() {
        let session = parse_session(SESSION, "file-id").unwrap();
        assert_eq!(session.id, "s-1");
        assert_eq!(session.cwd.as_deref(), Some(r"C:\work\app"));
        assert_eq!(session.entrypoint.as_deref(), Some("claude-desktop"));
        assert_eq!(session.title.as_deref(), Some("Login bug"));
        let shown = session
            .messages
            .iter()
            .map(|message| (message.role, message.text.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            shown,
            vec![
                (LeadRole::User, "Fix the login bug\nin auth.rs"),
                (
                    LeadRole::Lead,
                    "Looking at it.\n\nFixed.\n\nUsed Read, Edit."
                ),
                (LeadRole::User, "Thanks"),
            ]
        );
        assert_eq!(
            session.messages[1].model.as_deref(),
            Some("claude-opus-5-5")
        );
        assert_eq!(session.messages[1].provider, Some(ProviderId::Claude));
        assert_eq!(session.messages[0].id, "u1");
        assert_eq!(session.messages[1].id, "a1");
    }

    #[test]
    fn tool_results_and_account_fields_never_reach_a_message() {
        let session = parse_session(SESSION, "file-id").unwrap();
        let all = session
            .messages
            .iter()
            .map(|message| serde_json::to_string(message).unwrap())
            .collect::<String>();
        assert!(!all.contains("secret file text"));
        assert!(!all.contains("acct"));
        assert!(!all.contains("subagent"));
        assert!(!all.contains("hidden"));
    }

    #[test]
    fn titles_prefer_the_custom_one_then_the_first_prompt() {
        let custom = format!(
            "{SESSION}{}\n",
            r#"{"type":"custom-title","sessionId":"s-1","customTitle":"My name"}"#
        );
        assert_eq!(
            parse_session(&custom, "x").unwrap().display_title(),
            "My name"
        );
        let untitled = r#"{"type":"user","sessionId":"s-2","uuid":"u","message":{"content":"First line of a prompt\nsecond"}}"#;
        assert_eq!(
            parse_session(untitled, "x").unwrap().display_title(),
            "First line of a prompt"
        );
        assert_eq!(
            parse_session("", "x").unwrap().display_title(),
            "Claude Code chat"
        );
    }

    #[test]
    fn session_dirs_are_the_project_and_its_claude_worktrees() {
        let root =
            std::env::temp_dir().join(format!("legate-claude-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for name in [
            "C--work-app",
            "C--work-app--claude-worktrees-fix-1",
            "C--work-app2",
            "C--work-other",
        ] {
            std::fs::create_dir_all(root.join(name)).unwrap();
        }
        std::fs::write(root.join("C--work-app").join("s-1.jsonl"), SESSION).unwrap();
        let dirs = session_dirs(&root, r"C:\work\app\")
            .iter()
            .map(|dir| dir.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            dirs,
            vec!["C--work-app", "C--work-app--claude-worktrees-fix-1"]
        );
        let files = session_files(&root.join("C--work-app"));
        assert_eq!(files.len(), 1);
        assert_eq!(read_session(&files[0]).unwrap().id, "s-1");
        assert!(file_sig(&files[0]).is_some());
        assert_eq!(
            session_file(&root, r"C:\work\app", "s-1"),
            root.join("C--work-app").join("s-1.jsonl")
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
