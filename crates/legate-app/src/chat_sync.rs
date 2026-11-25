//! Mirrors Claude Code sessions into the open project's chat list, both ways and as they
//! happen. Every few seconds it looks at the project's Claude Code session files (the project
//! folder and its Claude worktrees):
//!
//! - a session started in the Claude app or the interactive CLI becomes a direct Claude chat;
//! - a chat that follows a session (mirrored, or one of Legate's own direct Claude chats) gets
//!   the session's turns again whenever its file changes, so turns typed in the Claude app or
//!   the CLI show up in Legate.
//!
//! The other direction needs nothing here: Legate's direct Claude chats are ordinary Claude
//! Code sessions in the project folder, so `claude --resume` and the IDE extensions list them,
//! and "Open in Claude app" (`claude --desktop --resume`) puts one in the app's sidebar.
//!
//! Headless sessions (`sdk-cli`: Legate's jobs, other tools' automation) are only followed when
//! a chat already owns them. A chat the user deletes stays deleted. Session files are only read.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use legate_lead::Lead;
use legate_providers::claude_history::{self, ClaudeSession};
use legate_store::{ChatRow, Store};
use tokio::sync::broadcast;

use crate::event::AppEvent;

/// How often the session files are checked.
const TICK: Duration = Duration::from_secs(3);
/// A session file written this recently, by something other than Legate, is still busy in
/// another app.
const BUSY_MS: i64 = 5_000;

pub(crate) struct ChatSync {
    store: Rc<Store>,
    project: Rc<RefCell<String>>,
    leads: Rc<RefCell<HashMap<String, Rc<Lead>>>>,
    events: broadcast::Sender<AppEvent>,
    root: Option<PathBuf>,
    /// The signature each file had when it was last looked at, so unchanged files are skipped.
    seen: RefCell<HashMap<PathBuf, String>>,
    /// When Legate last had a turn running in a chat, epoch ms.
    legate_turns: RefCell<HashMap<String, i64>>,
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
        root: Option<PathBuf>,
    ) -> Rc<Self> {
        Rc::new(Self {
            store,
            project,
            leads,
            events,
            root,
            seen: RefCell::new(HashMap::new()),
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
        let root = self.root.as_ref()?;
        let row = self.store.chat(chat_id).ok().flatten()?;
        if row.direct.as_deref() != Some("claude") {
            return None;
        }
        let session = row.session_id.as_deref()?;
        let cwd = row.cwd.clone().unwrap_or_else(|| row.project.clone());
        let modified =
            claude_history::file_modified_ms(&claude_history::session_file(root, &cwd, session))?;
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

    /// One pass over the open project's session files. Returns the chats it changed.
    pub async fn tick(&self) -> Vec<String> {
        let project = self.project.borrow().clone();
        let Some(root) = self.root.clone() else {
            return Vec::new();
        };
        if project.is_empty() {
            return Vec::new();
        }
        let lead = self.leads.borrow().get(&project).cloned();
        if let Some(chat) = lead.as_ref().and_then(|lead| lead.busy_chat()) {
            self.note_legate_turn(&chat);
        }
        let scan_project = project.clone();
        let files = tokio::task::spawn_blocking(move || list_files(&root, &scan_project))
            .await
            .unwrap_or_default();
        let mut changed = Vec::new();
        for (path, sig) in files {
            if self.seen.borrow().get(&path) == Some(&sig) {
                continue;
            }
            let read_path = path.clone();
            let Ok(Some(session)) =
                tokio::task::spawn_blocking(move || claude_history::read_session(&read_path)).await
            else {
                self.seen.borrow_mut().insert(path, sig);
                continue;
            };
            if *self.project.borrow() != project {
                return changed;
            }
            let modified = claude_history::file_modified_ms(&path).unwrap_or_else(now_ms);
            match self.apply(&project, lead.as_deref(), &session, &sig, modified) {
                Outcome::Changed(id) => {
                    changed.push(id);
                    self.seen.borrow_mut().insert(path, sig);
                }
                Outcome::Skipped => {
                    self.seen.borrow_mut().insert(path, sig);
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

    fn apply(
        &self,
        project: &str,
        lead: Option<&Lead>,
        session: &ClaudeSession,
        sig: &str,
        modified: i64,
    ) -> Outcome {
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
                if row.direct.as_deref() != Some("claude") || row.source_sig.as_deref() == Some(sig)
                {
                    return Outcome::Skipped;
                }
                if lead.and_then(Lead::busy_chat).as_deref() == Some(row.id.as_str()) {
                    return Outcome::Later;
                }
                ChatRow {
                    updated_at: row.updated_at.max(modified),
                    cwd: row.cwd.clone().or(cwd),
                    source_sig: Some(sig.to_string()),
                    ..row
                }
            }
            None => {
                let source = match session.entrypoint.as_deref() {
                    Some("claude-desktop") => "claude-app",
                    Some("cli") => "claude-code",
                    _ => return Outcome::Skipped,
                };
                if self.store.is_session_dismissed(&session.id).unwrap_or(true) {
                    return Outcome::Skipped;
                }
                ChatRow {
                    id: chat_id_for(&session.id),
                    project: project.to_string(),
                    title: session.display_title(),
                    created_at: modified,
                    updated_at: modified,
                    session_id: Some(session.id.clone()),
                    session_provider: Some("claude".to_string()),
                    direct: Some("claude".to_string()),
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

/// The project's session files with their signatures.
fn list_files(root: &Path, project: &str) -> Vec<(PathBuf, String)> {
    claude_history::session_dirs(root, project)
        .iter()
        .flat_map(|dir| claude_history::session_files(dir))
        .filter_map(|path| {
            let sig = claude_history::file_sig(&path)?;
            Some((path, sig))
        })
        .collect()
}

/// A stable chat id for a mirrored session (chat ids are at most 64 characters).
fn chat_id_for(session_id: &str) -> String {
    let id = format!("claude-{session_id}");
    id.chars().take(64).collect()
}

fn same_folder(a: &str, b: &str) -> bool {
    let norm = |path: &str| {
        path.trim_end_matches(['\\', '/'])
            .replace('/', "\\")
            .to_lowercase()
    };
    norm(a) == norm(b)
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
        let store = Rc::new(Store::open(":memory:").unwrap());
        let (events, _) = broadcast::channel(16);
        let sync = ChatSync::new(
            Rc::clone(&store),
            Rc::new(RefCell::new(project.to_string())),
            Rc::new(RefCell::new(HashMap::new())),
            events,
            Some(root.to_path_buf()),
        );
        (sync, store)
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
        assert_eq!(chat_id_for("s-1"), "claude-s-1");
    }
}
