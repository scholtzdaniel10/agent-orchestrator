use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use legate_core::{JobRecord, JobStatus, JobType, LeadMessage, LeadRole, LeadStatus, ProviderId};
use rusqlite::params;

use crate::{
    ChatRow, NewWindow, RoutineRow, RunOutcome, RunRow, Store, TERMINAL_SCROLLBACK_CAP,
    TerminalRow, WorktreeNameRow,
};

fn run(provider: &str, started_at: i64) -> RunRow {
    RunRow {
        id: None,
        job_id: "job".to_string(),
        provider: provider.to_string(),
        job_type: "planning".to_string(),
        started_at,
        duration_ms: Some(10),
        cost_usd: None,
        tokens: None,
        utilization: None,
        outcome: RunOutcome::Ok,
        session_id: None,
    }
}

fn chat_row(id: &str, project: &str, title: &str, created_at: i64, updated_at: i64) -> ChatRow {
    ChatRow {
        id: id.to_string(),
        project: project.to_string(),
        title: title.to_string(),
        created_at,
        updated_at,
        session_id: None,
        session_provider: None,
        direct: None,
    }
}

fn terminal(id: &str, project: &str) -> TerminalRow {
    TerminalRow {
        id: id.to_string(),
        project: project.to_string(),
        provider: "claude".to_string(),
        model: None,
        session_id: None,
        title: "claude 1".to_string(),
        created_at: 10,
        updated_at: 10,
        scrollback: String::new(),
        cwd: None,
        change_id: None,
    }
}

fn job(id: &str) -> JobRecord {
    JobRecord {
        id: id.to_string(),
        job_type: JobType::Planning,
        prompt: "go".to_string(),
        provider: Some(ProviderId::Claude),
        status: JobStatus::Done,
        output: String::new(),
        failed_over: Vec::new(),
        steps: None,
        steps_dropped: None,
        model: None,
        reason: None,
        edit: None,
        access: None,
        change: None,
        group: None,
        lead_message: None,
        routine: None,
        title: None,
        session_id: None,
        goal: None,
        base: None,
        error: None,
        owner_id: None,
        heartbeat_at: None,
        cancel_requested: None,
        extra: BTreeMap::new(),
    }
}

fn message(id: &str, role: LeadRole, text: &str) -> LeadMessage {
    LeadMessage {
        id: id.to_string(),
        role,
        text: text.to_string(),
        provider: None,
        model: None,
        status: LeadStatus::Done,
        kind: None,
        proposal: None,
        extra: BTreeMap::new(),
    }
}

struct Temp(PathBuf);

impl Temp {
    fn new(prefix: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn with_store(f: impl FnOnce(&Store)) {
    let store = Store::open(":memory:").unwrap();
    f(&store);
    store.close().unwrap();
}

#[test]
fn inserts_runs_and_returns_them_oldest_first() {
    with_store(|store| {
        let mut first = run("claude", 300);
        first.cost_usd = Some(0.0);
        first.tokens = Some(9);
        first.outcome = RunOutcome::Error;
        store.insert_run(&first).unwrap();

        let mut second = run("cursor", 100);
        second.job_id = "c".to_string();
        second.utilization = Some(0.2);
        second.session_id = Some("s".to_string());
        store.insert_run(&second).unwrap();

        let mut third = run("claude", 200);
        third.job_id = "b".to_string();
        third.tokens = Some(4);
        third.outcome = RunOutcome::Limit;
        store.insert_run(&third).unwrap();

        let all = store.runs(None).unwrap();
        assert_eq!(
            all.iter().map(|row| row.started_at).collect::<Vec<_>>(),
            vec![100, 200, 300]
        );
        assert_eq!(all[0].provider, "cursor");
        assert_eq!(all[0].job_id, "c");
        assert_eq!(all[0].utilization, Some(0.2));
        assert_eq!(all[0].session_id.as_deref(), Some("s"));
        assert_eq!(all[0].cost_usd, None);
        assert_eq!(all[2].cost_usd, Some(0.0));
        assert_eq!(all[2].tokens, Some(9));
        assert_eq!(all[2].outcome, RunOutcome::Error);
        assert_eq!(
            store
                .runs(Some("claude"))
                .unwrap()
                .iter()
                .map(|row| row.job_id.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "job"]
        );
        assert_eq!(
            store
                .runs_since("claude", 200)
                .unwrap()
                .iter()
                .map(|row| row.started_at)
                .collect::<Vec<_>>(),
            vec![200, 300]
        );
        assert_eq!(
            store
                .runs_since("claude", 250)
                .unwrap()
                .iter()
                .map(|row| row.started_at)
                .collect::<Vec<_>>(),
            vec![300]
        );
        assert_eq!(
            store
                .runs_since_all(250)
                .unwrap()
                .iter()
                .map(|row| row.started_at)
                .collect::<Vec<_>>(),
            vec![300]
        );
    });
}

#[test]
fn last_utilization_picks_the_newest_non_null_value_inside_the_window() {
    with_store(|store| {
        let mut row = run("claude", 50);
        row.utilization = Some(0.9);
        store.insert_run(&row).unwrap();
        row.started_at = 100;
        row.utilization = Some(0.2);
        store.insert_run(&row).unwrap();
        row.started_at = 300;
        row.utilization = Some(0.4);
        store.insert_run(&row).unwrap();
        row.utilization = None;
        row.cost_usd = Some(8.0);
        store.insert_run(&row).unwrap();
        row.started_at = 400;
        row.cost_usd = None;
        store.insert_run(&row).unwrap();
        let mut cursor = run("cursor", 500);
        cursor.utilization = Some(0.1);
        store.insert_run(&cursor).unwrap();

        assert_eq!(store.last_utilization("claude", 100).unwrap(), Some(0.4));
        assert_eq!(store.last_utilization("claude", 0).unwrap(), Some(0.4));
        assert_eq!(store.last_utilization("claude", 401).unwrap(), None);
        assert_eq!(store.last_utilization("claude", 1000).unwrap(), None);
        assert_eq!(store.last_utilization("cursor", 0).unwrap(), Some(0.1));
        cursor.started_at = 600;
        cursor.utilization = Some(0.0);
        store.insert_run(&cursor).unwrap();
        assert_eq!(store.last_utilization("cursor", 0).unwrap(), Some(0.0));
    });
}

#[test]
fn save_windows_upserts_by_provider_and_name() {
    with_store(|store| {
        store
            .save_windows(
                "claude",
                &[
                    NewWindow {
                        name: "seven_day".to_string(),
                        utilization: 0.4,
                        resets_at: Some(20),
                    },
                    NewWindow {
                        name: "five_hour".to_string(),
                        utilization: 0.2,
                        resets_at: None,
                    },
                ],
                5,
            )
            .unwrap();
        store
            .save_windows(
                "cursor",
                &[NewWindow {
                    name: "five_hour".to_string(),
                    utilization: 0.9,
                    resets_at: Some(30),
                }],
                6,
            )
            .unwrap();
        store
            .save_windows(
                "claude",
                &[NewWindow {
                    name: "five_hour".to_string(),
                    utilization: 0.3,
                    resets_at: Some(25),
                }],
                7,
            )
            .unwrap();
        assert_eq!(
            store.windows("claude").unwrap(),
            vec![
                crate::StoredWindow {
                    name: "five_hour".to_string(),
                    utilization: 0.3,
                    resets_at: Some(25),
                    observed_at: 7
                },
                crate::StoredWindow {
                    name: "seven_day".to_string(),
                    utilization: 0.4,
                    resets_at: Some(20),
                    observed_at: 5
                },
            ]
        );
        assert_eq!(
            store.windows("cursor").unwrap(),
            vec![crate::StoredWindow {
                name: "five_hour".to_string(),
                utilization: 0.9,
                resets_at: Some(30),
                observed_at: 6,
            }]
        );
        assert!(store.windows("missing").unwrap().is_empty());
        store.delete_window("claude", "five_hour").unwrap();
        assert_eq!(store.windows("claude").unwrap().len(), 1);
    });
}

#[test]
fn resting_is_set_replaced_and_cleared() {
    with_store(|store| {
        assert_eq!(store.resting_until("claude").unwrap(), None);
        store.set_resting("claude", 50).unwrap();
        assert_eq!(store.resting_until("claude").unwrap(), Some(50));
        store.set_resting("claude", 80).unwrap();
        assert_eq!(store.resting_until("claude").unwrap(), Some(80));
        store.set_resting("cursor", 10).unwrap();
        store.clear_resting("claude").unwrap();
        assert_eq!(store.resting_until("claude").unwrap(), None);
        assert_eq!(store.resting_until("cursor").unwrap(), Some(10));
        store.clear_resting("claude").unwrap();
        assert_eq!(store.resting_until("missing").unwrap(), None);
    });
}

#[test]
fn opens_an_old_database_keeps_its_rows_and_reaches_the_latest_schema() {
    let temp = Temp::new("ao-store-mig");
    let path = temp.0.join("old.sqlite");
    {
        let legacy = rusqlite::Connection::open(&path).unwrap();
        legacy
            .execute_batch(
                "
                CREATE TABLE IF NOT EXISTS runs (
                  id INTEGER PRIMARY KEY AUTOINCREMENT,
                  job_id TEXT NOT NULL,
                  provider TEXT NOT NULL,
                  job_type TEXT NOT NULL,
                  started_at INTEGER NOT NULL,
                  duration_ms INTEGER,
                  cost_usd REAL,
                  tokens INTEGER,
                  utilization REAL,
                  outcome TEXT NOT NULL,
                  session_id TEXT
                );
                CREATE TABLE IF NOT EXISTS resting (
                  provider TEXT PRIMARY KEY,
                  until INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS usage_windows (
                  provider TEXT NOT NULL,
                  name TEXT NOT NULL,
                  utilization REAL NOT NULL,
                  resets_at INTEGER,
                  observed_at INTEGER NOT NULL,
                  PRIMARY KEY (provider, name)
                );
                PRAGMA user_version = 0;
                ",
            )
            .unwrap();
        legacy
            .execute(
                "INSERT INTO runs (
                   job_id, provider, job_type, started_at, duration_ms,
                   cost_usd, tokens, utilization, outcome, session_id
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    "kept",
                    "claude",
                    "planning",
                    10,
                    1,
                    None::<f64>,
                    None::<i64>,
                    None::<f64>,
                    "ok",
                    None::<String>
                ],
            )
            .unwrap();
        legacy
            .execute(
                "INSERT INTO resting (provider, until) VALUES (?1, ?2)",
                params!["claude", 99],
            )
            .unwrap();
    }

    let store = Store::open(&path).unwrap();
    assert_eq!(store.runs(None).unwrap().len(), 1);
    assert_eq!(store.runs(None).unwrap()[0].job_id, "kept");
    assert_eq!(store.runs(None).unwrap()[0].provider, "claude");
    assert_eq!(store.resting_until("claude").unwrap(), Some(99));
    store.save_chat(&chat_row("c1", "/p", "hi", 1, 2)).unwrap();
    assert_eq!(store.chats("/p").unwrap().len(), 1);
    assert_eq!(store.chat_count("/p").unwrap(), 1);
    assert_eq!(store.chat_count("/other").unwrap(), 0);
    store.close().unwrap();

    let again = Store::open(&path).unwrap();
    assert_eq!(again.runs(None).unwrap()[0].job_id, "kept");
    assert_eq!(again.chats("/p").unwrap().len(), 1);
    again.close().unwrap();

    let check = rusqlite::Connection::open(&path).unwrap();
    let version: i64 = check
        .query_row("PRAGMA user_version", (), |row| row.get(0))
        .unwrap();
    assert_eq!(version, 11);
    for name in ["terminals", "worktree_names", "project_setup", "routines"] {
        let found: String = check
            .query_row(
                "SELECT name FROM sqlite_master WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(found, name);
    }
    let mut stmt = check
        .prepare("SELECT name FROM pragma_table_info('terminals')")
        .unwrap();
    let cols: Vec<String> = stmt
        .query_map((), |row| row.get(0))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    for name in ["cwd", "change_id", "id", "project", "scrollback"] {
        assert!(cols.iter().any(|col| col == name), "{cols:?}");
    }
}

#[test]
fn opening_a_current_database_twice_is_a_no_op_for_migrations() {
    let temp = Temp::new("ao-store-twice");
    let path = temp.0.join("db.sqlite");
    let first = Store::open(&path).unwrap();
    first.insert_run(&run("claude", 1)).unwrap();
    first.close().unwrap();
    let second = Store::open(&path).unwrap();
    assert_eq!(second.runs(None).unwrap().len(), 1);
    second.close().unwrap();
    let third = Store::open(&path).unwrap();
    assert_eq!(third.runs(None).unwrap().len(), 1);
    third.close().unwrap();
}

#[test]
fn chats_messages_and_jobs_round_trip_with_order_and_upsert_rules() {
    with_store(|store| {
        let mut first = chat_row("c1", "/a", "first", 10, 20);
        first.session_id = Some("s1".to_string());
        first.session_provider = Some("claude".to_string());
        store.save_chat(&first).unwrap();
        store
            .save_chat(&chat_row("c2", "/a", "second", 11, 30))
            .unwrap();
        store
            .save_chat(&chat_row("c3", "/b", "other", 12, 40))
            .unwrap();
        assert_eq!(ids(&store.chats("/a").unwrap()), vec!["c2", "c1"]);
        let loaded = store.chat("c1").unwrap().unwrap();
        assert_eq!(loaded.title, "first");
        assert_eq!(loaded.session_id.as_deref(), Some("s1"));
        assert_eq!(loaded.session_provider.as_deref(), Some("claude"));

        let mut updated = first;
        updated.title = "first updated".to_string();
        updated.updated_at = 50;
        updated.session_id = Some("s2".to_string());
        updated.session_provider = Some("cursor".to_string());
        store.save_chat(&updated).unwrap();
        assert_eq!(store.chat("c1").unwrap().unwrap().title, "first updated");
        assert_eq!(ids(&store.chats("/a").unwrap()), vec!["c1", "c2"]);

        store
            .save_message("c1", 1, &message("m2", LeadRole::Lead, "b"))
            .unwrap();
        store
            .save_message("c1", 0, &message("m1", LeadRole::User, "a"))
            .unwrap();
        assert_eq!(
            store
                .messages("c1")
                .unwrap()
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            vec!["m1", "m2"]
        );
        store
            .save_message("c1", 1, &message("m2", LeadRole::Lead, "b2"))
            .unwrap();
        assert_eq!(store.messages("c1").unwrap()[1].text, "b2");
        assert_eq!(store.chat_for_message("m1").unwrap().as_deref(), Some("c1"));
        assert_eq!(store.chat_for_message("missing").unwrap(), None);

        let mut j1 = job("j1");
        j1.output = "one".to_string();
        let mut j2 = job("j2");
        j2.output = "two".to_string();
        let mut j3 = job("j3");
        j3.output = "three".to_string();
        let mut j4 = job("j4");
        j4.output = "other".to_string();
        store.save_job("/a", 100, &j1).unwrap();
        store.save_job("/a", 200, &j2).unwrap();
        store.save_job("/a", 300, &j3).unwrap();
        store.save_job("/b", 400, &j4).unwrap();
        assert_eq!(
            store
                .jobs("/a", 2)
                .unwrap()
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            vec!["j2", "j3"]
        );
        j2.output = "two-again".to_string();
        j2.status = JobStatus::Failed;
        store.save_job("/a", 200, &j2).unwrap();
        let found = store
            .jobs("/a", 10)
            .unwrap()
            .into_iter()
            .find(|row| row.id == "j2")
            .unwrap();
        assert_eq!(found.output, "two-again");
        assert_eq!(found.status, JobStatus::Failed);
    });
}

fn ids(chats: &[ChatRow]) -> Vec<&str> {
    chats.iter().map(|chat| chat.id.as_str()).collect()
}

#[test]
fn delete_chat_removes_messages_and_keeps_other_chats_and_jobs() {
    with_store(|store| {
        store.save_chat(&chat_row("c1", "/a", "one", 1, 1)).unwrap();
        store.save_chat(&chat_row("c2", "/a", "two", 2, 2)).unwrap();
        store
            .save_message("c1", 0, &message("m1", LeadRole::User, "hi"))
            .unwrap();
        store
            .save_message("c1", 1, &message("m2", LeadRole::Lead, "yo"))
            .unwrap();
        store
            .save_message("c2", 0, &message("m3", LeadRole::User, "other"))
            .unwrap();
        let mut kept = job("j1");
        kept.lead_message = Some("m2".to_string());
        store.save_job("/a", 1, &kept).unwrap();
        store.delete_chat("c1").unwrap();
        assert!(store.chat("c1").unwrap().is_none());
        assert!(store.messages("c1").unwrap().is_empty());
        assert_eq!(ids(&store.chats("/a").unwrap()), vec!["c2"]);
        assert_eq!(store.messages("c2").unwrap()[0].id, "m3");
        assert_eq!(store.jobs("/a", 10).unwrap()[0].id, "j1");
        store.rename_chat("c2", "renamed").unwrap();
        assert_eq!(store.chat("c2").unwrap().unwrap().title, "renamed");
    });
}

#[test]
fn terminals_upsert_list_oldest_first_update_scrollback_delete_and_cap() {
    with_store(|store| {
        let mut t2 = terminal("t2", "/a");
        t2.title = "claude 2".to_string();
        t2.created_at = 20;
        t2.session_id = Some("s2".to_string());
        t2.model = Some("opus".to_string());
        store.save_terminal(&t2).unwrap();
        let mut t1 = terminal("t1", "/a");
        t1.title = "claude 1".to_string();
        t1.created_at = 10;
        t1.provider = "cursor".to_string();
        store.save_terminal(&t1).unwrap();
        let mut t3 = terminal("t3", "/b");
        t3.title = "other".to_string();
        store.save_terminal(&t3).unwrap();

        let listed = store.terminals("/a").unwrap();
        assert_eq!(
            listed.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            vec!["t1", "t2"]
        );
        assert_eq!(listed[1].provider, "claude");
        assert_eq!(listed[1].model.as_deref(), Some("opus"));
        assert_eq!(listed[1].session_id.as_deref(), Some("s2"));
        assert_eq!(listed[1].title, "claude 2");
        assert_eq!(listed[1].cwd, None);
        assert_eq!(listed[1].change_id, None);

        let mut replaced = terminal("t1", "/a");
        replaced.title = "cursor 1".to_string();
        replaced.created_at = 10;
        replaced.updated_at = 40;
        replaced.provider = "cursor".to_string();
        replaced.session_id = Some("chat-1".to_string());
        replaced.scrollback = "hi".to_string();
        store.save_terminal(&replaced).unwrap();
        let listed = store.terminals("/a").unwrap();
        assert_eq!(listed[0].title, "cursor 1");
        assert_eq!(listed[0].session_id.as_deref(), Some("chat-1"));
        assert_eq!(listed[0].scrollback, "hi");
        assert_eq!(listed[0].updated_at, 40);

        store.update_terminal_scrollback("t2", "later", 50).unwrap();
        assert_eq!(store.terminals("/a").unwrap()[1].scrollback, "later");
        assert_eq!(store.terminals("/a").unwrap()[1].updated_at, 50);

        let long = "x".repeat(TERMINAL_SCROLLBACK_CAP + 20);
        let mut t4 = terminal("t4", "/a");
        t4.created_at = 30;
        t4.scrollback = long.clone();
        store.save_terminal(&t4).unwrap();
        let tail = &long[long.len() - TERMINAL_SCROLLBACK_CAP..];
        assert_eq!(store.terminals("/a").unwrap()[2].scrollback, tail);
        let longer = format!("ab{long}");
        store.update_terminal_scrollback("t4", &longer, 60).unwrap();
        let longer_tail = &longer[longer.len() - TERMINAL_SCROLLBACK_CAP..];
        assert_eq!(store.terminals("/a").unwrap()[2].scrollback, longer_tail);
        assert_eq!(
            store.terminals("/a").unwrap()[2].scrollback.len(),
            TERMINAL_SCROLLBACK_CAP
        );

        store.delete_terminal("t1").unwrap();
        assert_eq!(
            store
                .terminals("/a")
                .unwrap()
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            vec!["t2", "t4"]
        );
        assert_eq!(store.terminals("/b").unwrap()[0].id, "t3");
        store.delete_terminal("missing").unwrap();
        assert_eq!(store.terminals("/a").unwrap().len(), 2);

        let mut t5 = terminal("t5", "/a");
        t5.created_at = 40;
        t5.cwd = Some("/wt/3fa9c1d2".to_string());
        t5.change_id = Some("3fa9c1d2".to_string());
        store.save_terminal(&t5).unwrap();
        let last = &store.terminals("/a").unwrap()[2];
        assert_eq!(last.id, "t5");
        assert_eq!(last.cwd.as_deref(), Some("/wt/3fa9c1d2"));
        assert_eq!(last.change_id.as_deref(), Some("3fa9c1d2"));
    });
}

#[test]
fn worktree_names_set_list_newest_first_rename_and_delete() {
    with_store(|store| {
        store
            .set_worktree_name(&WorktreeNameRow {
                id: "aaaaaaaa".to_string(),
                project: "/a".to_string(),
                name: "older".to_string(),
                created_at: 10,
            })
            .unwrap();
        store
            .set_worktree_name(&WorktreeNameRow {
                id: "bbbbbbbb".to_string(),
                project: "/a".to_string(),
                name: "newer".to_string(),
                created_at: 20,
            })
            .unwrap();
        store
            .set_worktree_name(&WorktreeNameRow {
                id: "cccccccc".to_string(),
                project: "/b".to_string(),
                name: "other".to_string(),
                created_at: 30,
            })
            .unwrap();
        let names = store.worktree_names("/a").unwrap();
        assert_eq!(
            names.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            vec!["bbbbbbbb", "aaaaaaaa"]
        );
        assert_eq!(names[0].name, "newer");
        assert_eq!(names[0].project, "/a");
        assert_eq!(names[0].created_at, 20);
        assert_eq!(store.worktree_names("/b").unwrap()[0].name, "other");
        assert!(store.worktree_names("/missing").unwrap().is_empty());

        store
            .set_worktree_name(&WorktreeNameRow {
                id: "aaaaaaaa".to_string(),
                project: "/a".to_string(),
                name: "renamed".to_string(),
                created_at: 99,
            })
            .unwrap();
        let names = store.worktree_names("/a").unwrap();
        assert_eq!(names[1].id, "aaaaaaaa");
        assert_eq!(names[1].name, "renamed");
        assert_eq!(names[1].created_at, 10);

        store.delete_worktree_name("bbbbbbbb").unwrap();
        assert_eq!(store.worktree_names("/a").unwrap()[0].id, "aaaaaaaa");
        store.delete_worktree_name("missing").unwrap();
        assert_eq!(store.worktree_names("/a").unwrap().len(), 1);
    });
}

#[test]
fn project_setup_upserts_a_json_command_list_per_project() {
    with_store(|store| {
        assert!(store.get_project_setup("/a").unwrap().is_empty());
        store
            .set_project_setup(
                "/a",
                &["pnpm install".to_string(), "pnpm build".to_string()],
            )
            .unwrap();
        store
            .set_project_setup("/b", &["echo hi".to_string()])
            .unwrap();
        assert_eq!(
            store.get_project_setup("/a").unwrap(),
            vec!["pnpm install", "pnpm build"]
        );
        assert_eq!(store.get_project_setup("/b").unwrap(), vec!["echo hi"]);
        store
            .set_project_setup("/a", &["pnpm test".to_string()])
            .unwrap();
        assert_eq!(store.get_project_setup("/a").unwrap(), vec!["pnpm test"]);
        store.set_project_setup("/a", &[]).unwrap();
        assert!(store.get_project_setup("/a").unwrap().is_empty());
    });
}

#[test]
fn routines_list_in_position_order_upsert_and_delete_per_project() {
    with_store(|store| {
        store
            .save_routine(&RoutineRow {
                id: "r2".to_string(),
                project: "/a".to_string(),
                position: 1,
                data: "{\"name\":\"two\"}".to_string(),
            })
            .unwrap();
        store
            .save_routine(&RoutineRow {
                id: "r1".to_string(),
                project: "/a".to_string(),
                position: 0,
                data: "{\"name\":\"one\"}".to_string(),
            })
            .unwrap();
        store
            .save_routine(&RoutineRow {
                id: "r3".to_string(),
                project: "/b".to_string(),
                position: 0,
                data: "{\"name\":\"other\"}".to_string(),
            })
            .unwrap();
        let rows = store.list_routines("/a").unwrap();
        assert_eq!(
            rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            vec!["r1", "r2"]
        );
        assert_eq!(rows[0].project, "/a");
        assert_eq!(rows[0].position, 0);
        assert_eq!(rows[0].data, "{\"name\":\"one\"}");
        assert_eq!(store.list_routines("/b").unwrap()[0].id, "r3");
        store
            .save_routine(&RoutineRow {
                id: "r1".to_string(),
                project: "/a".to_string(),
                position: 0,
                data: "{\"name\":\"one-again\"}".to_string(),
            })
            .unwrap();
        assert_eq!(
            store.list_routines("/a").unwrap()[0].data,
            "{\"name\":\"one-again\"}"
        );
        store.delete_routine("r1").unwrap();
        assert_eq!(store.list_routines("/a").unwrap()[0].id, "r2");
        store.delete_routine("missing").unwrap();
        assert_eq!(store.list_routines("/a").unwrap().len(), 1);
        assert!(store.list_routines("/missing").unwrap().is_empty());
    });
}

#[test]
fn a_corrupt_data_row_is_skipped() {
    let temp = Temp::new("ao-store-bad");
    let path = temp.0.join("db.sqlite");
    let store = Store::open(&path).unwrap();
    store.save_chat(&chat_row("c1", "/a", "t", 1, 1)).unwrap();
    store
        .save_message("c1", 0, &message("m1", LeadRole::User, "ok"))
        .unwrap();
    let mut kept = job("j1");
    kept.output = "ok".to_string();
    store.save_job("/a", 1, &kept).unwrap();
    store.close().unwrap();

    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute(
        "INSERT INTO messages (id, chat_id, seq, data) VALUES (?1, ?2, ?3, ?4)",
        params!["bad", "c1", 1, "{not-json"],
    )
    .unwrap();
    raw.execute(
        "INSERT INTO messages (id, chat_id, seq, data) VALUES (?1, ?2, ?3, ?4)",
        params!["noid", "c1", 2, "{\"text\":\"x\"}"],
    )
    .unwrap();
    raw.execute(
        "INSERT INTO jobs (id, project, created_at, data) VALUES (?1, ?2, ?3, ?4)",
        params!["badj", "/a", 2, "nope"],
    )
    .unwrap();
    drop(raw);

    let again = Store::open(&path).unwrap();
    assert_eq!(again.messages("c1").unwrap()[0].id, "m1");
    assert_eq!(again.messages("c1").unwrap().len(), 1);
    assert_eq!(again.jobs("/a", 10).unwrap()[0].id, "j1");
    assert_eq!(again.jobs("/a", 10).unwrap().len(), 1);
    again.close().unwrap();
}

#[test]
fn a_busy_database_waits_up_to_5_seconds() {
    let store = Store::open(":memory:").unwrap();
    assert_eq!(store.busy_timeout_ms().unwrap(), 5000);
    store.close().unwrap();
}

#[test]
fn request_cancel_survives_a_later_save_job_of_the_same_row() {
    with_store(|store| {
        let mut running = job("j1");
        running.status = JobStatus::Running;
        running.owner_id = Some("aaa".to_string());
        running.heartbeat_at = Some(10);
        store.save_job("/a", 1, &running).unwrap();
        assert!(!store.is_cancel_requested("j1").unwrap());
        store.request_cancel("j1").unwrap();
        assert!(store.is_cancel_requested("j1").unwrap());
        assert_eq!(
            store.jobs("/a", 10).unwrap()[0].cancel_requested,
            Some(true)
        );
        running.output = "more".to_string();
        running.heartbeat_at = Some(20);
        running.cancel_requested = None;
        store.save_job("/a", 1, &running).unwrap();
        assert!(store.is_cancel_requested("j1").unwrap());
        let loaded = &store.jobs("/a", 10).unwrap()[0];
        assert_eq!(loaded.output, "more");
        assert_eq!(loaded.owner_id.as_deref(), Some("aaa"));
        assert_eq!(loaded.heartbeat_at, Some(20));
        assert_eq!(loaded.cancel_requested, Some(true));
        assert!(!store.is_cancel_requested("missing").unwrap());
    });
}

#[test]
fn jobs_newest_returns_the_newest_jobs_first_with_their_times() {
    let store = Store::open(":memory:").unwrap();
    store.save_job("/p", 10, &job("old")).unwrap();
    store.save_job("/p", 30, &job("new")).unwrap();
    store.save_job("/p", 20, &job("mid")).unwrap();
    let rows = store.jobs_newest("/p", 2).unwrap();
    assert_eq!(rows[0].job.id, "new");
    assert_eq!(rows[0].created_at, 30);
    assert_eq!(rows[1].job.id, "mid");
    assert_eq!(rows[1].created_at, 20);
    store.close().unwrap();
}

#[test]
fn goals_and_triggers_round_trip() {
    with_store(|store| {
        store.save_goal("g1", "/a", 10, "{\"id\":\"g1\"}").unwrap();
        store.save_goal("g2", "/a", 30, "{\"id\":\"g2\"}").unwrap();
        store.save_goal("g3", "/b", 20, "{\"id\":\"g3\"}").unwrap();
        assert_eq!(
            store.goals("/a", None).unwrap(),
            vec!["{\"id\":\"g2\"}", "{\"id\":\"g1\"}"]
        );
        assert_eq!(
            store.goal("g3").unwrap().as_deref(),
            Some("{\"id\":\"g3\"}")
        );
        store.delete_goal("g1").unwrap();
        assert_eq!(
            store.goals("/a", Some(10)).unwrap(),
            vec!["{\"id\":\"g2\"}"]
        );
        store.save_trigger("t1", "/a", "{\"id\":\"t1\"}").unwrap();
        store.save_trigger("t1", "/a", "{\"id\":\"t1b\"}").unwrap();
        store.save_trigger("t2", "/b", "{\"id\":\"t2\"}").unwrap();
        let mut triggers = store.triggers().unwrap();
        triggers.sort();
        assert_eq!(triggers, vec!["{\"id\":\"t1b\"}", "{\"id\":\"t2\"}"]);
        store.delete_trigger("t2").unwrap();
        assert_eq!(store.triggers().unwrap(), vec!["{\"id\":\"t1b\"}"]);
    });
}

#[test]
fn sqlite_round_trips_a_row() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("create table t (a integer)").unwrap();
    conn.execute("insert into t values (?1)", params![7])
        .unwrap();
    let value: i64 = conn
        .query_row("select a from t", (), |row| row.get(0))
        .unwrap();
    assert_eq!(value, 7);
}

#[test]
fn multi_process_interleaved_writes() {
    if std::env::var("LEGATE_MP_CHILD").ok().as_deref() == Some("1") {
        child_writer();
        return;
    }

    let temp = Temp::new("legate-mp");
    let db = temp.0.join("shared.sqlite");
    let ready = temp.0.join("ready");
    let go = temp.0.join("go");
    let store = Store::open(&db).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tests::multi_process_interleaved_writes"])
        .env("LEGATE_MP_CHILD", "1")
        .env("LEGATE_MP_DB", &db)
        .env("LEGATE_MP_READY", &ready)
        .env("LEGATE_MP_GO", &go)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let started = Instant::now();
    while !ready.exists() {
        if started.elapsed() > Duration::from_secs(20) {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "child did not open the database\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
    fs::write(&go, b"go").unwrap();
    for index in 0..40 {
        let mut row = run("parent", index);
        row.job_id = format!("p-{index}");
        store.insert_run(&row).unwrap();
    }
    store.set_resting("parent", 2).unwrap();

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "child failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let runs = store.runs(None).unwrap();
    let parents = runs.iter().filter(|row| row.provider == "parent").count();
    let children = runs.iter().filter(|row| row.provider == "child").count();
    assert_eq!(parents, 40, "parent rows missing from the shared file");
    assert_eq!(children, 40, "child rows missing from the shared file");
    assert_eq!(store.resting_until("parent").unwrap(), Some(2));
    assert_eq!(store.resting_until("child").unwrap(), Some(1));
    store.close().unwrap();
}

fn child_writer() {
    let db = std::env::var("LEGATE_MP_DB").unwrap();
    let ready = std::env::var("LEGATE_MP_READY").unwrap();
    let go = std::env::var("LEGATE_MP_GO").unwrap();
    let store = Store::open(Path::new(&db)).unwrap();
    fs::write(&ready, b"ready").unwrap();
    let started = Instant::now();
    while !Path::new(&go).exists() {
        if started.elapsed() > Duration::from_secs(20) {
            panic!("go signal missing");
        }
        thread::sleep(Duration::from_millis(10));
    }
    for index in 0..40 {
        let mut row = run("child", 1_000 + index);
        row.job_id = format!("c-{index}");
        store.insert_run(&row).unwrap();
    }
    store.set_resting("child", 1).unwrap();
    store.close().unwrap();
}
