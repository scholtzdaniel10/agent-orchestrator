//! Mirrors Claude Code sessions and Cursor CLI chats into the open project's chat list, both
//! ways and as they happen. Every few seconds it looks at the project's session files:
//! Claude Code's `~/.claude/projects/<slug>` (the project folder and its Claude worktrees) and
//! the Cursor CLI's `~/.cursor/chats/<md5 of folder>` (the project folder and folders under it).
//!
//! - a session started in the Claude app, the interactive Claude Code CLI or the interactive
//!   Cursor CLI becomes a direct chat with that plan;
//! - a chat that follows a session (mirrored, or one of Legate's own direct Claude or Cursor
//!   chats) gets the session's turns again whenever its files change, so turns typed elsewhere
//!   show up in Legate.
//!
//! The other direction needs nothing here: Legate's direct chats are ordinary CLI sessions in
//! the project folder, so each CLI's own resume list shows them; "Open in Claude app"
//! (`claude --desktop --resume`) and "Open in terminal" (`cursor-agent --resume`) hand one over.
//!
//! Headless sessions (Claude's `sdk-cli`, Cursor's untitled `-p` runs: Legate's jobs, other
//! tools' automation) are only followed when a chat already owns them. A chat the user deletes
//! stays deleted. Session files are only read.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use legate_core::LeadMessage;
use legate_lead::Lead;
use legate_providers::{claude_history, cursor_history};
use legate_store::{ChatRow, Store};
use tokio::sync::broadcast;

use crate::event::AppEvent;

/// How often the session files are checked.
const TICK: Duration = Duration::from_secs(3);
/// A session written this recently, by something other than Legate, is still busy in another
/// app.
const BUSY_MS: i64 = 5_000;

/// Where each CLI keeps its sessions. `None` turns that CLI's mirror off.
#[derive(Debug, Clone, Default)]
pub(crate) struct Roots {
    pub claude: Option<PathBuf>,
    pub cursor: Option<PathBuf>,
}

impl Roots {
    pub fn from_home() -> Self {
        Self {
            claude: claude_history::projects_root(),
            cursor: cursor_history::chats_root(),
        }
    }
}

pub(crate) struct ChatSync {
    store: Rc<Store>,
    project: Rc<RefCell<String>>,
    leads: Rc<RefCell<HashMap<String, Rc<Lead>>>>,
    events: broadcast::Sender<AppEvent>,
    roots: Roots,
    /// The signature each session had when it was last looked at, so unchanged ones are skipped.
    seen: RefCell<HashMap<PathBuf, String>>,
    /// The folder each Cursor folder directory belongs to (read once from a chat's meta.json).
    cursor_folders: RefCell<HashMap<PathBuf, String>>,
    /// When Legate last had a turn running in a chat, epoch ms.
    legate_turns: RefCell<HashMap<String, i64>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Claude,
    Cursor,
}

/// A session found on disk: a Claude Code file or a Cursor chat directory.
struct Found {
    path: PathBuf,
    sig: String,
    kind: Kind,
}

/// A session from either CLI, in the shape a chat needs.
struct External {
    provider: &'static str,
    id: String,
    cwd: Option<String>,
    title: String,
    /// Where it started (`claude-app`, `claude-code`, `cursor-cli`); `None` for a headless run.
    source: Option<&'static str>,
    messages: Vec<LeadMessage>,
    modified: i64,
}

enum Outcome {
    Changed(String),
    Skipped,
    /// Try again next tick (a Legate turn is writing this session).
    Later,
}

impl ChatSync {
    pub fn new(
        store: Rc<Store>,
        project: Rc<RefCell<String>>,
        leads: Rc<RefCell<HashMap<String, Rc<Lead>>>>,
        events: broadcast::Sender<AppEvent>,
        roots: Roots,
    ) -> Rc<Self> {
        Rc::new(Self {
            store,
            project,
            leads,
            events,
            roots,
            seen: RefCell::new(HashMap::new()),
            cursor_folders: RefCell::new(HashMap::new()),
            legate_turns: RefCell::new(HashMap::new()),
        })
    }

    pub fn spawn(self: &Rc<Self>) {
        let this = Rc::clone(self);
        tokio::task::spawn_local(async move {
            loop {
                this.tick().await;
                tokio::time::sleep(TICK).await;
            }
        });
    }

    /// Notes that Legate is running a turn in `chat_id` now.
    pub fn note_legate_turn(&self, chat_id: &str) {
        self.legate_turns
            .borrow_mut()
            .insert(chat_id.to_string(), now_ms());
    }

    /// `Some(reason)` when the chat's session was written to moments ago by another app, so
    /// sending now would have two programs driving one session.
    pub fn busy_elsewhere(&self, chat_id: &str) -> Option<String> {
        let row = self.store.chat(chat_id).ok().flatten()?;
        let session = row.session_id.as_deref()?;
        let cwd = row.cwd.clone().unwrap_or_else(|| row.project.clone());
        let modified = match row.direct.as_deref()? {
            "claude" => claude_history::file_modified_ms(&claude_history::session_file(
                self.roots.claude.as_ref()?,
                &cwd,
                session,
            ))?,
            "cursor" => self
                .cursor_folders
                .borrow()
                .iter()
                .filter(|(_, folder)| same_folder(folder, &cwd))
                .find_map(|(dir, _)| cursor_history::chat_modified_ms(&dir.join(session)))?,
            _ => return None,
        };
        let legate = self
            .legate_turns
            .borrow()
            .get(chat_id)
            .copied()
            .unwrap_or(0);
        let now = now_ms();
        (now - modified < BUSY_MS && modified - legate > BUSY_MS).then(|| {
            "This chat is running in another app right now. Wait until it finishes there, then send."
                .to_string()
        })
    }

    /// One pass over the open project's sessions. Returns the chats it changed.
    pub async fn tick(&self) -> Vec<String> {
        let project = self.project.borrow().clone();
        if project.is_empty() {
            return Vec::new();
        }
        let lead = self.leads.borrow().get(&project).cloned();
        if let Some(chat) = lead.as_ref().and_then(|lead| lead.busy_chat()) {
            self.note_legate_turn(&chat);
        }
        let roots = self.roots.clone();
        let scan_project = project.clone();
        let known = self.cursor_folders.borrow().clone();
        let Ok((found, folders)) =
            tokio::task::spawn_blocking(move || list_sessions(&roots, &scan_project, known)).await
        else {
            return Vec::new();
        };
        *self.cursor_folders.borrow_mut() = folders;
        let mut changed = Vec::new();
        for item in found {
            if self.seen.borrow().get(&item.path) == Some(&item.sig) {
                continue;
            }
            let read_path = item.path.clone();
            let kind = item.kind;
            let Ok(Some(session)) =
                tokio::task::spawn_blocking(move || read_session(kind, &read_path)).await
            else {
                self.seen.borrow_mut().insert(item.path, item.sig);
                continue;
            };
            if *self.project.borrow() != project {
                return changed;
            }
            match self.apply(&project, lead.as_deref(), &session, &item.sig) {
                Outcome::Changed(id) => {
                    changed.push(id);
                    self.seen.borrow_mut().insert(item.path, item.sig);
                }
                Outcome::Skipped => {
                    self.seen.borrow_mut().insert(item.path, item.sig);
                }
                Outcome::Later => {}
            }
        }
        if !changed.is_empty() {
            let _ = self.events.send(AppEvent::Chats {
                changed: changed.clone(),
            });
        }
        changed
    }

    fn apply(&self, project: &str, lead: Option<&Lead>, session: &External, sig: &str) -> Outcome {
        if session.messages.is_empty() {
            return Outcome::Skipped;
        }
        let existing = self
            .store
            .chat_by_session(project, &session.id)
            .ok()
            .flatten();
        let cwd = session.cwd.clone().filter(|dir| !same_folder(dir, project));
        let row = match existing {
            Some(row) => {
                if row.direct.as_deref() != Some(session.provider)
                    || row.source_sig.as_deref() == Some(sig)
                {
                    return Outcome::Skipped;
                }
                if lead.and_then(Lead::busy_chat).as_deref() == Some(row.id.as_str()) {
                    return Outcome::Later;
                }
                ChatRow {
                    updated_at: row.updated_at.max(session.modified),
                    cwd: row.cwd.clone().or(cwd),
                    source_sig: Some(sig.to_string()),
                    ..row
                }
            }
            None => {
                let Some(source) = session.source else {
                    return Outcome::Skipped;
                };
                if self.store.is_session_dismissed(&session.id).unwrap_or(true) {
                    return Outcome::Skipped;
                }
                ChatRow {
                    id: chat_id_for(session.provider, &session.id),
                    project: project.to_string(),
                    title: session.title.clone(),
                    created_at: session.modified,
                    updated_at: session.modified,
                    session_id: Some(session.id.clone()),
                    session_provider: Some(session.provider.to_string()),
                    direct: Some(session.provider.to_string()),
                    cwd,
                    source: Some(source.to_string()),
                    source_sig: Some(sig.to_string()),
                }
            }
        };
        if let Err(err) = self.store.save_chat(&row) {
            tracing::error!("chat sync: {err}");
            return Outcome::Skipped;
        }
        if let Err(err) = self.store.replace_messages(&row.id, &session.messages) {
            tracing::error!("chat sync: {err}");
            return Outcome::Skipped;
        }
        if let Some(lead) = lead {
            lead.refresh_chat(&row.id);
        }
        Outcome::Changed(row.id)
    }
}

/// The project's sessions with their signatures, and the Cursor folder map grown by any folder
/// directory seen for the first time.
fn list_sessions(
    roots: &Roots,
    project: &str,
    mut folders: HashMap<PathBuf, String>,
) -> (Vec<Found>, HashMap<PathBuf, String>) {
    let mut found = Vec::new();
    if let Some(root) = &roots.claude {
        for dir in claude_history::session_dirs(root, project) {
            for path in claude_history::session_files(&dir) {
                if let Some(sig) = claude_history::file_sig(&path) {
                    found.push(Found {
                        path,
                        sig,
                        kind: Kind::Claude,
                    });
                }
            }
        }
    }
    if let Some(root) = &roots.cursor {
        for dir in cursor_history::folder_dirs(root) {
            if !folders.contains_key(&dir) {
                match cursor_history::folder_cwd(&dir) {
                    Some(cwd) => {
                        folders.insert(dir.clone(), cwd);
                    }
                    None => continue,
                }
            }
            if !in_project(&folders[&dir], project) {
                continue;
            }
            for chat in cursor_history::chat_dirs(&dir) {
                if let Some(sig) = cursor_history::chat_sig(&chat) {
                    found.push(Found {
                        path: chat,
                        sig,
                        kind: Kind::Cursor,
                    });
                }
            }
        }
    }
    (found, folders)
}

fn read_session(kind: Kind, path: &Path) -> Option<External> {
    match kind {
        Kind::Claude => {
            let session = claude_history::read_session(path)?;
            let source = match session.entrypoint.as_deref() {
                Some("claude-desktop") => Some("claude-app"),
                Some("cli") => Some("claude-code"),
                _ => None,
            };
            Some(External {
                provider: "claude",
                title: session.display_title(),
                id: session.id,
                cwd: session.cwd,
                source,
                messages: session.messages,
                modified: claude_history::file_modified_ms(path).unwrap_or_else(now_ms),
            })
        }
        Kind::Cursor => {
            let chat = cursor_history::read_chat(path)?;
            Some(External {
                provider: "cursor",
                title: chat.display_title(),
                source: chat.interactive.then_some("cursor-cli"),
                modified: chat
                    .updated_at
                    .or_else(|| cursor_history::chat_modified_ms(path))
                    .unwrap_or_else(now_ms),
                id: chat.id,
                cwd: chat.cwd,
                messages: chat.messages,
            })
        }
    }
}

/// A stable chat id for a mirrored session (chat ids are at most 64 characters).
fn chat_id_for(provider: &str, session_id: &str) -> String {
    let id = format!("{provider}-{session_id}");
    id.chars().take(64).collect()
}

fn norm(path: &str) -> String {
    path.trim_end_matches(['\\', '/'])
        .replace('/', "\\")
        .to_lowercase()
}

fn same_folder(a: &str, b: &str) -> bool {
    norm(a) == norm(b)
}

/// The folder is the project or inside it (a worktree under `.claude/worktrees`, a subfolder).
fn in_project(folder: &str, project: &str) -> bool {
    let folder = norm(folder);
    let project = norm(project);
    folder == project || folder.starts_with(&format!("{project}\\"))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(kind: &str, extra: &str) -> String {
        format!(r#"{{"type":"{kind}","sessionId":"s-1",{extra}}}"#)
    }

    fn write_session(root: &Path, cwd: &str, entrypoint: &str, turns: &[(&str, &str)]) -> PathBuf {
        let dir = root.join(claude_history::slug(cwd));
        std::fs::create_dir_all(&dir).unwrap();
        let mut text = String::new();
        for (index, (prompt, reply)) in turns.iter().enumerate() {
            text.push_str(&line(
                "user",
                &format!(
                    r#""cwd":{},"entrypoint":"{entrypoint}","uuid":"u{index}","message":{{"content":"{prompt}"}}"#,
                    serde_json::to_string(cwd).unwrap()
                ),
            ));
            text.push('\n');
            text.push_str(&line(
                "assistant",
                &format!(
                    r#""uuid":"a{index}","message":{{"content":[{{"type":"text","text":"{reply}"}}]}}"#
                ),
            ));
            text.push('\n');
        }
        let path = dir.join("s-1.jsonl");
        std::fs::write(&path, text).unwrap();
        path
    }

    fn sync_for(project: &str, root: &Path) -> (Rc<ChatSync>, Rc<Store>) {
        sync_with(
            project,
            Roots {
                claude: Some(root.to_path_buf()),
                cursor: None,
            },
        )
    }

    fn sync_with(project: &str, roots: Roots) -> (Rc<ChatSync>, Rc<Store>) {
        let store = Rc::new(Store::open(":memory:").unwrap());
        let (events, _) = broadcast::channel(16);
        let sync = ChatSync::new(
            Rc::clone(&store),
            Rc::new(RefCell::new(project.to_string())),
            Rc::new(RefCell::new(HashMap::new())),
            events,
            roots,
        );
        (sync, store)
    }

    /// Writes a Cursor CLI chat the way `cursor-agent` stores it.
    fn write_cursor_chat(
        root: &Path,
        folder: &str,
        cwd: &str,
        id: &str,
        name: &str,
        turns: &[(&str, &str)],
    ) -> PathBuf {
        let dir = root.join(folder).join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("meta.json"),
            serde_json::json!({ "schemaVersion": 1, "hasConversation": true, "updatedAtMs": 7, "cwd": cwd })
                .to_string(),
        )
        .unwrap();
        let db_path = dir.join("store.db");
        let _ = std::fs::remove_file(&db_path);
        let db = rusqlite::Connection::open(&db_path).unwrap();
        db.execute_batch(
            "CREATE TABLE blobs (id TEXT PRIMARY KEY, data BLOB); CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);",
        )
        .unwrap();
        let mut root_bytes = Vec::new();
        let mut n = 0u8;
        let mut put = |json: String| {
            n += 1;
            let id = format!("{n:02x}").repeat(32);
            db.execute(
                "INSERT INTO blobs VALUES (?1, ?2)",
                rusqlite::params![id, json.into_bytes()],
            )
            .unwrap();
            root_bytes.extend([0x0a, 0x20]);
            root_bytes.extend(std::iter::repeat_n(n, 32));
        };
        for (prompt, reply) in turns {
            put(serde_json::json!({ "role": "user", "content": format!("<user_query>{prompt}</user_query>") }).to_string());
            put(serde_json::json!({ "role": "assistant", "content": reply }).to_string());
        }
        let root_id = "ff".repeat(32);
        db.execute(
            "INSERT INTO blobs VALUES (?1, ?2)",
            rusqlite::params![root_id, root_bytes],
        )
        .unwrap();
        let meta = serde_json::json!({ "latestRootBlobId": root_id, "name": name }).to_string();
        let hex = meta.bytes().map(|b| format!("{b:02x}")).collect::<String>();
        db.execute("INSERT INTO meta VALUES ('0', ?1)", [hex])
            .unwrap();
        dir
    }

    #[tokio::test(flavor = "current_thread")]
    async fn an_interactive_cursor_chat_becomes_a_chat_and_headless_runs_stay_out() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let root = temp_root("cursor");
                let project = r"C:\work\app";
                write_cursor_chat(
                    &root,
                    "aaaa",
                    project,
                    "c-1",
                    "Fix Login",
                    &[("Fix login", "Done")],
                );
                write_cursor_chat(
                    &root,
                    "aaaa",
                    project,
                    "c-2",
                    "New Agent",
                    &[("Job prompt", "Job reply")],
                );
                write_cursor_chat(
                    &root,
                    "bbbb",
                    r"C:\work\other",
                    "c-3",
                    "Other",
                    &[("Hi", "Hello")],
                );
                let (sync, store) = sync_with(
                    project,
                    Roots {
                        claude: None,
                        cursor: Some(root.clone()),
                    },
                );
                assert_eq!(sync.tick().await, vec!["cursor-c-1".to_string()]);
                let row = store.chat("cursor-c-1").unwrap().unwrap();
                assert_eq!(row.source.as_deref(), Some("cursor-cli"));
                assert_eq!(row.direct.as_deref(), Some("cursor"));
                assert_eq!(row.session_provider.as_deref(), Some("cursor"));
                assert_eq!(row.title, "Fix Login");
                assert_eq!(store.messages("cursor-c-1").unwrap().len(), 2);
                assert_eq!(store.chats(project).unwrap().len(), 1);

                std::thread::sleep(Duration::from_millis(20));
                write_cursor_chat(
                    &root,
                    "aaaa",
                    project,
                    "c-1",
                    "Fix Login",
                    &[("Fix login", "Done"), ("Also logout", "Done too")],
                );
                assert_eq!(sync.tick().await, vec!["cursor-c-1".to_string()]);
                assert_eq!(store.messages("cursor-c-1").unwrap().len(), 4);
                assert!(sync.busy_elsewhere("cursor-c-1").is_some());
                let _ = std::fs::remove_dir_all(&root);
            })
            .await;
    }

    fn temp_root(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("legate-chat-sync-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[tokio::test(flavor = "current_thread")]
    async fn an_app_session_becomes_a_chat_and_new_turns_follow() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let root = temp_root("follow");
                let project = r"C:\work\app";
                write_session(&root, project, "claude-desktop", &[("Hello", "Hi there")]);
                let (sync, store) = sync_for(project, &root);
                assert_eq!(sync.tick().await, vec!["claude-s-1".to_string()]);
                let row = store.chat("claude-s-1").unwrap().unwrap();
                assert_eq!(row.source.as_deref(), Some("claude-app"));
                assert_eq!(row.direct.as_deref(), Some("claude"));
                assert_eq!(row.session_id.as_deref(), Some("s-1"));
                assert_eq!(row.cwd, None);
                assert_eq!(row.title, "Hello");
                assert_eq!(store.messages("claude-s-1").unwrap().len(), 2);

                assert!(
                    sync.tick().await.is_empty(),
                    "an unchanged file is not read again"
                );

                std::thread::sleep(Duration::from_millis(20));
                write_session(
                    &root,
                    project,
                    "claude-desktop",
                    &[("Hello", "Hi there"), ("And now?", "Done")],
                );
                assert_eq!(sync.tick().await, vec!["claude-s-1".to_string()]);
                let texts = store
                    .messages("claude-s-1")
                    .unwrap()
                    .into_iter()
                    .map(|message| message.text)
                    .collect::<Vec<_>>();
                assert_eq!(texts, vec!["Hello", "Hi there", "And now?", "Done"]);
                let _ = std::fs::remove_dir_all(&root);
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn worktree_sessions_keep_their_folder_and_headless_ones_are_left_out() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let root = temp_root("worktree");
                let project = r"C:\work\app";
                let worktree = r"C:\work\app\.claude\worktrees\fix";
                write_session(&root, worktree, "cli", &[("Fix it", "Fixed")]);
                let (sync, store) = sync_for(project, &root);
                assert_eq!(sync.tick().await.len(), 1);
                let row = store.chat("claude-s-1").unwrap().unwrap();
                assert_eq!(row.source.as_deref(), Some("claude-code"));
                assert_eq!(row.cwd.as_deref(), Some(worktree));

                let other = temp_root("headless");
                write_session(&other, project, "sdk-cli", &[("Job prompt", "Job reply")]);
                let (sync, store) = sync_for(project, &other);
                assert!(sync.tick().await.is_empty());
                assert_eq!(store.chats(project).unwrap().len(), 0);
                let _ = std::fs::remove_dir_all(&root);
                let _ = std::fs::remove_dir_all(&other);
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_deleted_mirrored_chat_stays_deleted() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let root = temp_root("dismiss");
                let project = r"C:\work\app";
                let path = write_session(&root, project, "cli", &[("Hello", "Hi")]);
                let (sync, store) = sync_for(project, &root);
                assert_eq!(sync.tick().await.len(), 1);
                store.delete_chat("claude-s-1").unwrap();
                std::thread::sleep(Duration::from_millis(20));
                write_session(&root, project, "cli", &[("Hello", "Hi"), ("More", "Yes")]);
                assert!(path.exists());
                assert!(sync.tick().await.is_empty());
                assert_eq!(store.chat("claude-s-1").unwrap(), None);
                let _ = std::fs::remove_dir_all(&root);
            })
            .await;
    }

    #[test]
    fn a_session_written_moments_ago_by_another_app_is_busy() {
        let root = temp_root("busy");
        let project = r"C:\work\app";
        write_session(&root, project, "cli", &[("Hello", "Hi")]);
        let (sync, store) = sync_for(project, &root);
        let row = ChatRow {
            id: "c1".into(),
            project: project.into(),
            title: "t".into(),
            created_at: 0,
            updated_at: 0,
            session_id: Some("s-1".into()),
            session_provider: Some("claude".into()),
            direct: Some("claude".into()),
            cwd: None,
            source: None,
            source_sig: None,
        };
        store.save_chat(&row).unwrap();
        assert!(sync.busy_elsewhere("c1").is_some());
        sync.note_legate_turn("c1");
        assert!(
            sync.busy_elsewhere("c1").is_none(),
            "Legate's own turn wrote it"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn folders_compare_without_case_or_trailing_slashes() {
        assert!(same_folder(r"C:\Work\App\", r"c:\work\app"));
        assert!(!same_folder(r"C:\work\app2", r"C:\work\app"));
        assert_eq!(chat_id_for("claude", "s-1"), "claude-s-1");
        assert!(in_project(
            r"C:\work\app\.claude\worktrees\x",
            r"C:\Work\App"
        ));
        assert!(!in_project(r"C:\work\app2", r"C:\work\app"));
    }
}
