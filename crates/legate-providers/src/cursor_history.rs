//! Reads the Cursor CLI's own chats so Legate can show chats started in `cursor-agent` and keep
//! showing new turns as they are written.
//!
//! The Cursor CLI keeps every chat in `~/.cursor/chats/<md5 of the folder>/<chat id>/`:
//! `meta.json` (the folder, timestamps, whether it has a conversation) and `store.db`, a SQLite
//! file with a `meta` row (hex-encoded JSON: the chat's name, last model, and the id of its
//! latest root record) and content-addressed `blobs`. The root record is protobuf; its field 1
//! repeats the 32-byte ids of the chat's messages in order, and each message is a JSON blob with
//! a `role` and `content`.
//!
//! This module only reads those files, read-only. It keeps the user's prompts, the agent's text
//! and short tool names; system prompts, context records, tool results, reasoning and the chat's
//! encryption key never leave it.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use legate_core::{LeadMessage, LeadRole, LeadStatus, ProviderId};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use crate::note::LEGATE_NOTE;

/// The name the Cursor CLI gives a chat it never titled: every headless (`-p`) run.
const UNTITLED: &str = "New Agent";

/// One Cursor CLI chat, ready to show as a chat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorChat {
    pub id: String,
    /// The folder the chat ran in.
    pub cwd: Option<String>,
    /// The title Cursor generated, when it made one.
    pub title: Option<String>,
    pub first_prompt: Option<String>,
    /// Started in the interactive CLI. Headless runs (`cursor-agent -p`, Legate's jobs, other
    /// tools' automation) are never titled and never record a model.
    pub interactive: bool,
    /// Epoch ms of the last change Cursor recorded.
    pub updated_at: Option<i64>,
    pub messages: Vec<LeadMessage>,
}

impl CursorChat {
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
            None => "Cursor chat".to_string(),
        }
    }
}

/// `~/.cursor/chats`.
pub fn chats_root() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE")
        .filter(|dir| !dir.is_empty())
        .or_else(|| std::env::var_os("HOME").filter(|dir| !dir.is_empty()))?;
    Some(PathBuf::from(home).join(".cursor").join("chats"))
}

/// The per-folder directories under the root (one per folder Cursor ran in).
pub fn folder_dirs(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    dirs.sort();
    dirs
}

/// The chat directories in one folder directory.
pub fn chat_dirs(folder_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder_dir) else {
        return Vec::new();
    };
    let mut dirs = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .filter(|dir| dir.join("meta.json").is_file())
        .collect::<Vec<_>>();
    dirs.sort();
    dirs
}

/// The folder a folder directory's chats ran in, from any of its chats' `meta.json`.
pub fn folder_cwd(folder_dir: &Path) -> Option<String> {
    chat_dirs(folder_dir)
        .iter()
        .find_map(|dir| read_meta_json(dir).and_then(|meta| string(&meta, "cwd")))
}

/// Changes whenever the chat does: its `meta.json`, `store.db` and write-ahead log.
pub fn chat_sig(chat_dir: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for name in ["meta.json", "store.db", "store.db-wal"] {
        match std::fs::metadata(chat_dir.join(name)) {
            Ok(meta) => {
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |since| since.as_millis());
                parts.push(format!("{}:{modified}", meta.len()));
            }
            Err(_) if name == "store.db-wal" => parts.push("-".to_string()),
            Err(_) => return None,
        }
    }
    Some(parts.join("/"))
}

/// Epoch ms of the chat's latest write.
pub fn chat_modified_ms(chat_dir: &Path) -> Option<i64> {
    ["meta.json", "store.db", "store.db-wal"]
        .iter()
        .filter_map(|name| std::fs::metadata(chat_dir.join(name)).ok())
        .filter_map(|meta| meta.modified().ok())
        .filter_map(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .filter_map(|since| i64::try_from(since.as_millis()).ok())
        .max()
}

/// Reads one chat directory. `None` when it cannot be read or has no conversation.
pub fn read_chat(chat_dir: &Path) -> Option<CursorChat> {
    let meta_json = read_meta_json(chat_dir)?;
    if meta_json.get("hasConversation").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let id = chat_dir.file_name()?.to_string_lossy().into_owned();
    let db = Connection::open_with_flags(
        chat_dir.join("store.db"),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let _ = db.busy_timeout(std::time::Duration::from_millis(500));
    let store_meta = read_store_meta(&db)?;
    let root_id = string(&store_meta, "latestRootBlobId")?;
    let blob = |id: &str| -> Option<Vec<u8>> {
        db.query_row("SELECT data FROM blobs WHERE id = ?1", [id], |row| {
            row.get::<_, Vec<u8>>(0)
        })
        .ok()
    };
    let root = blob(&root_id)?;
    let mut blobs = HashMap::new();
    for message_id in message_ids(&root) {
        if !blobs.contains_key(&message_id)
            && let Some(data) = blob(&message_id)
        {
            blobs.insert(message_id, data);
        }
    }
    let order = message_ids(&root);
    let (messages, first_prompt) = parse_messages(&order, &blobs);
    let title = string(&store_meta, "name").filter(|name| name != UNTITLED);
    let interactive = title.is_some() || store_meta.get("lastUsedModel").is_some();
    Some(CursorChat {
        id,
        cwd: string(&meta_json, "cwd"),
        title,
        first_prompt,
        interactive,
        updated_at: meta_json.get("updatedAtMs").and_then(Value::as_i64),
        messages,
    })
}

fn read_meta_json(chat_dir: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(chat_dir.join("meta.json")).ok()?).ok()
}

/// The `meta` row: hex-encoded JSON (some versions store the JSON as text or bytes).
fn read_store_meta(db: &Connection) -> Option<Value> {
    let raw: rusqlite::types::Value = db
        .query_row("SELECT value FROM meta LIMIT 1", [], |row| row.get(0))
        .ok()?;
    let bytes = match raw {
        rusqlite::types::Value::Text(text) => {
            decode_hex(&text).unwrap_or_else(|| text.into_bytes())
        }
        rusqlite::types::Value::Blob(bytes) => bytes,
        _ => return None,
    };
    serde_json::from_slice(&bytes).ok()
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).ok())
        .collect()
}

/// The message ids a root record lists, in order: every 32-byte value of protobuf field 1.
pub fn message_ids(root: &[u8]) -> Vec<String> {
    let mut ids = Vec::new();
    let mut at = 0;
    while at < root.len() {
        let Some((key, next)) = varint(root, at) else {
            break;
        };
        at = next;
        match key & 7 {
            0 => match varint(root, at) {
                Some((_, next)) => at = next,
                None => break,
            },
            1 => at += 8,
            2 => {
                let Some((len, next)) = varint(root, at) else {
                    break;
                };
                let Ok(len) = usize::try_from(len) else { break };
                let end = next.saturating_add(len);
                if end > root.len() {
                    break;
                }
                if key >> 3 == 1 && len == 32 {
                    ids.push(root[next..end].iter().map(|b| format!("{b:02x}")).collect());
                }
                at = end;
            }
            5 => at += 4,
            _ => break,
        }
    }
    ids
}

fn varint(bytes: &[u8], mut at: usize) -> Option<(u64, usize)> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *bytes.get(at)?;
        at += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some((value, at));
        }
    }
    None
}

/// Turns the ordered message blobs into prompts and one reply per turn. Returns the messages
/// and the first prompt.
pub fn parse_messages(
    order: &[String],
    blobs: &HashMap<String, Vec<u8>>,
) -> (Vec<LeadMessage>, Option<String>) {
    let mut messages: Vec<LeadMessage> = Vec::new();
    let mut first_prompt = None;
    let mut turn: Option<Turn> = None;
    for (index, blob_id) in order.iter().enumerate() {
        let Some(record) = blobs
            .get(blob_id)
            .and_then(|data| serde_json::from_slice::<Value>(data).ok())
        else {
            continue;
        };
        let message_id = format!("{}-{index}", &blob_id[..blob_id.len().min(16)]);
        match record.get("role").and_then(Value::as_str) {
            Some("user") => {
                let Some(prompt) = user_prompt(record.get("content")) else {
                    continue;
                };
                if let Some(done) = turn.take() {
                    messages.push(done.into_message());
                }
                if first_prompt.is_none() {
                    first_prompt = Some(prompt.clone());
                }
                messages.push(LeadMessage {
                    id: message_id,
                    role: LeadRole::User,
                    text: prompt,
                    provider: None,
                    model: None,
                    status: LeadStatus::Done,
                    kind: None,
                    proposal: None,
                    extra: BTreeMap::new(),
                });
            }
            Some("assistant") => {
                let current = turn.get_or_insert_with(|| Turn {
                    id: message_id,
                    ..Turn::default()
                });
                match record.get("content") {
                    Some(Value::String(text)) => current.push_text(text),
                    Some(Value::Array(parts)) => {
                        for part in parts {
                            match part.get("type").and_then(Value::as_str) {
                                Some("text") => {
                                    if let Some(text) = part.get("text").and_then(Value::as_str) {
                                        current.push_text(text);
                                    }
                                }
                                Some("tool-call") => {
                                    if let Some(name) = part.get("toolName").and_then(Value::as_str)
                                        && !current.tools.iter().any(|tool| tool == name)
                                    {
                                        current.tools.push(name.to_string());
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    if let Some(done) = turn.take() {
        messages.push(done.into_message());
    }
    messages.retain(|message| !message.text.is_empty());
    (messages, first_prompt)
}

#[derive(Default)]
struct Turn {
    id: String,
    texts: Vec<String>,
    tools: Vec<String>,
}

impl Turn {
    fn push_text(&mut self, text: &str) {
        let text = text.trim();
        if !text.is_empty() {
            self.texts.push(text.to_string());
        }
    }

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
            provider: Some(ProviderId::Cursor),
            model: None,
            status: LeadStatus::Done,
            kind: None,
            proposal: None,
            extra: BTreeMap::new(),
        }
    }
}

/// What the user asked in a `user` record: the text inside `<user_query>`, without the note
/// Legate puts before its own prompts. Context records Cursor writes in the user's place
/// (`<user_info>`, rules, git status) are `None`.
fn user_prompt(content: Option<&Value>) -> Option<String> {
    let text = match content? {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => return None,
    };
    let prompt = match between(&text, "<user_query>", "</user_query>") {
        Some(query) => query,
        None if text.trim_start().starts_with('<') => return None,
        None => text.trim().to_string(),
    };
    let prompt = prompt
        .strip_prefix(LEGATE_NOTE)
        .map(str::trim_start)
        .unwrap_or(&prompt)
        .trim()
        .to_string();
    (!prompt.is_empty()).then_some(prompt)
}

fn between(text: &str, open: &str, close: &str) -> Option<String> {
    let start = text.find(open)? + open.len();
    let end = text[start..].find(close)? + start;
    Some(text[start..end].trim().to_string())
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

    fn id(n: u8) -> String {
        format!("{:02x}", n).repeat(32)
    }

    fn root_for(ids: &[String]) -> Vec<u8> {
        let mut root = Vec::new();
        for id in ids {
            root.extend([0x0a, 0x20]);
            root.extend((0..32).map(|at| u8::from_str_radix(&id[at * 2..at * 2 + 2], 16).unwrap()));
        }
        // Other fields after the list: a varint and a string, as Cursor writes them.
        root.extend([0x10, 0x96, 0x01, 0x1a, 0x03, b'a', b'b', b'c']);
        root
    }

    fn blobs_for(records: &[(String, &str)]) -> HashMap<String, Vec<u8>> {
        records
            .iter()
            .map(|(id, json)| (id.clone(), json.as_bytes().to_vec()))
            .collect()
    }

    #[test]
    fn the_root_lists_message_ids_in_order() {
        let ids = vec![id(1), id(2), id(3)];
        assert_eq!(message_ids(&root_for(&ids)), ids);
        assert!(message_ids(&[0x0a]).is_empty());
    }

    #[test]
    fn a_chat_becomes_prompts_and_one_reply_per_turn() {
        let ids = (1..=7).map(id).collect::<Vec<_>>();
        let blobs = blobs_for(&[
            (
                ids[0].clone(),
                r#"{"role":"system","content":"You are an AI coding assistant"}"#,
            ),
            (
                ids[1].clone(),
                r#"{"role":"user","content":[{"type":"text","text":"<user_info>OS</user_info><rules>r</rules>"}]}"#,
            ),
            (
                ids[2].clone(),
                r#"{"role":"user","content":[{"type":"text","text":"<timestamp>now</timestamp>\n<user_query>\nFix the bug\n</user_query>"}]}"#,
            ),
            (
                ids[3].clone(),
                r#"{"role":"assistant","content":[{"type":"redacted-reasoning","data":"x"},{"type":"text","text":"Looking."},{"type":"tool-call","toolName":"Read","args":{}}]}"#,
            ),
            (
                ids[4].clone(),
                r#"{"role":"tool","content":[{"type":"tool-result","result":"secret file text"}]}"#,
            ),
            (
                ids[5].clone(),
                r#"{"role":"assistant","content":[{"type":"tool-call","toolName":"Edit"},{"type":"text","text":"Fixed."}]}"#,
            ),
            (
                ids[6].clone(),
                r#"{"role":"user","content":"<user_query>Thanks</user_query>"}"#,
            ),
        ]);
        let (messages, first) = parse_messages(&ids, &blobs);
        let shown = messages
            .iter()
            .map(|message| (message.role, message.text.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            shown,
            vec![
                (LeadRole::User, "Fix the bug"),
                (LeadRole::Lead, "Looking.\n\nFixed.\n\nUsed Read, Edit."),
                (LeadRole::User, "Thanks"),
            ]
        );
        assert_eq!(first.as_deref(), Some("Fix the bug"));
        assert_eq!(messages[1].provider, Some(ProviderId::Cursor));
        let all = messages
            .iter()
            .map(|message| serde_json::to_string(message).unwrap())
            .collect::<String>();
        assert!(!all.contains("secret file text"));
        assert!(!all.contains("You are an AI"));
        assert!(!all.contains("<user_info>"));
    }

    #[test]
    fn legates_note_is_taken_off_its_own_prompts() {
        let ids = vec![id(1)];
        let json = serde_json::json!({
            "role": "user",
            "content": format!("<user_query>{LEGATE_NOTE}\n\nWrite the tests</user_query>"),
        })
        .to_string();
        let blobs = blobs_for(&[(ids[0].clone(), json.as_str())]);
        let (messages, _) = parse_messages(&ids, &blobs);
        assert_eq!(messages[0].text, "Write the tests");
    }

    #[test]
    fn a_repeated_message_keeps_its_place() {
        let ids = vec![id(1), id(2), id(1), id(2)];
        let blobs = blobs_for(&[
            (
                id(1),
                r#"{"role":"user","content":"<user_query>Again</user_query>"}"#,
            ),
            (id(2), r#"{"role":"assistant","content":"Yes"}"#),
        ]);
        let (messages, _) = parse_messages(&ids, &blobs);
        assert_eq!(messages.len(), 4);
        let unique = messages
            .iter()
            .map(|m| m.id.clone())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), 4);
    }

    #[test]
    fn reads_a_chat_directory_like_cursor_writes_it() {
        let dir = std::env::temp_dir()
            .join(format!("legate-cursor-history-{}", std::process::id()))
            .join("0123")
            .join("chat-1");
        let _ = std::fs::remove_dir_all(dir.parent().unwrap().parent().unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("meta.json"),
            r#"{"schemaVersion":1,"createdAtMs":1,"hasConversation":true,"updatedAtMs":5,"cwd":"C:\\work\\app"}"#,
        )
        .unwrap();
        let ids = vec![id(1), id(2)];
        {
            let db = Connection::open(dir.join("store.db")).unwrap();
            db.execute_batch(
                "CREATE TABLE blobs (id TEXT PRIMARY KEY, data BLOB); CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);",
            )
            .unwrap();
            let root_id = id(9);
            let meta = serde_json::json!({
                "agentId": "chat-1", "latestRootBlobId": root_id, "name": "Probe Interactive",
                "lastUsedModel": "grok", "blobEncryptionKey": "never-shown",
            })
            .to_string();
            let hex = meta.bytes().map(|b| format!("{b:02x}")).collect::<String>();
            db.execute("INSERT INTO meta VALUES ('0', ?1)", [hex])
                .unwrap();
            db.execute(
                "INSERT INTO blobs VALUES (?1, ?2)",
                rusqlite::params![root_id, root_for(&ids)],
            )
            .unwrap();
            db.execute(
                "INSERT INTO blobs VALUES (?1, ?2)",
                rusqlite::params![
                    ids[0],
                    br#"{"role":"user","content":"<user_query>Hi</user_query>"}"#.to_vec()
                ],
            )
            .unwrap();
            db.execute(
                "INSERT INTO blobs VALUES (?1, ?2)",
                rusqlite::params![
                    ids[1],
                    br#"{"role":"assistant","content":"Hello"}"#.to_vec()
                ],
            )
            .unwrap();
        }
        let chat = read_chat(&dir).unwrap();
        assert_eq!(chat.id, "chat-1");
        assert_eq!(chat.cwd.as_deref(), Some(r"C:\work\app"));
        assert!(chat.interactive);
        assert_eq!(chat.display_title(), "Probe Interactive");
        assert_eq!(chat.updated_at, Some(5));
        assert_eq!(chat.messages.len(), 2);
        assert!(chat_sig(&dir).is_some());
        assert_eq!(
            folder_cwd(dir.parent().unwrap()).as_deref(),
            Some(r"C:\work\app")
        );
        let _ = std::fs::remove_dir_all(dir.parent().unwrap().parent().unwrap());
    }
}
