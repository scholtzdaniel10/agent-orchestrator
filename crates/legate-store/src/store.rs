//! Store methods. Semantics follow `src/core/router/store.ts`.

use std::path::Path;
use std::time::Duration;

use legate_core::{JobRecord, LeadMessage};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use crate::migrate;

pub const TERMINAL_SCROLLBACK_CAP: usize = 200_000;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    Ok,
    Error,
    Limit,
}

impl RunOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Limit => "limit",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ok" => Some(Self::Ok),
            "error" => Some(Self::Error),
            "limit" => Some(Self::Limit),
            _ => None,
        }
    }
}

impl rusqlite::types::FromSql for RunOutcome {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        Self::parse(value.as_str()?).ok_or(rusqlite::types::FromSqlError::InvalidType)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunRow {
    pub id: Option<i64>,
    pub job_id: String,
    pub provider: String,
    pub job_type: String,
    pub started_at: i64,
    pub duration_ms: Option<i64>,
    pub cost_usd: Option<f64>,
    pub tokens: Option<i64>,
    pub utilization: Option<f64>,
    pub outcome: RunOutcome,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredWindow {
    pub name: String,
    pub utilization: f64,
    pub resets_at: Option<i64>,
    pub observed_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewWindow {
    pub name: String,
    pub utilization: f64,
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatRow {
    pub id: String,
    pub project: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub session_id: Option<String>,
    pub session_provider: Option<String>,
    pub direct: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalRow {
    pub id: String,
    pub project: String,
    pub provider: String,
    pub model: Option<String>,
    pub session_id: Option<String>,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub scrollback: String,
    pub cwd: Option<String>,
    pub change_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeNameRow {
    pub id: String,
    pub project: String,
    pub name: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineRow {
    pub id: String,
    pub project: String,
    pub position: i64,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredJob {
    pub created_at: i64,
    pub job: JobRecord,
}

/// Usage ledger shared by the desktop app and the CLI.
///
/// `path` is a filesystem path or `":memory:"`. The connection sets
/// `PRAGMA busy_timeout = 5000`, same as the TypeScript store.
pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref();
        let conn = if path == Path::new(":memory:") {
            Connection::open_in_memory()?
        } else {
            Connection::open(path)?
        };
        conn.busy_timeout(Duration::from_millis(5000))?;
        conn.execute_batch("PRAGMA busy_timeout = 5000")?;
        migrate::migrate(&conn)?;
        Ok(Self { conn })
    }

    pub fn busy_timeout_ms(&self) -> Result<i64, StoreError> {
        Ok(self
            .conn
            .query_row("PRAGMA busy_timeout", (), |row| row.get(0))?)
    }

    pub fn close(self) -> Result<(), StoreError> {
        self.conn
            .close()
            .map_err(|(_, err)| StoreError::Sqlite(err))
    }

    pub fn insert_run(&self, row: &RunRow) -> Result<(), StoreError> {
        self.exec(
            "INSERT INTO runs (
               job_id, provider, job_type, started_at, duration_ms,
               cost_usd, tokens, utilization, outcome, session_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                row.job_id,
                row.provider,
                row.job_type,
                row.started_at,
                row.duration_ms,
                row.cost_usd,
                row.tokens,
                row.utilization,
                row.outcome.as_str(),
                row.session_id,
            ],
        )
    }

    /// All runs, oldest first. `provider` limits the rows to one worker.
    pub fn runs(&self, provider: Option<&str>) -> Result<Vec<RunRow>, StoreError> {
        if let Some(provider) = provider {
            self.query_all(
                "SELECT * FROM runs WHERE provider = ?1 ORDER BY started_at ASC, id ASC",
                params![provider],
                map_run,
            )
        } else {
            self.query_all(
                "SELECT * FROM runs ORDER BY started_at ASC, id ASC",
                (),
                map_run,
            )
        }
    }

    pub fn runs_since(&self, provider: &str, since_ms: i64) -> Result<Vec<RunRow>, StoreError> {
        self.query_all(
            "SELECT * FROM runs
             WHERE provider = ?1 AND started_at >= ?2
             ORDER BY started_at ASC, id ASC",
            params![provider, since_ms],
            map_run,
        )
    }

    /// Every provider's runs since `since_ms`, oldest first, in one query.
    pub fn runs_since_all(&self, since_ms: i64) -> Result<Vec<RunRow>, StoreError> {
        self.query_all(
            "SELECT * FROM runs WHERE started_at >= ?1 ORDER BY started_at ASC, id ASC",
            params![since_ms],
            map_run,
        )
    }

    pub fn save_windows(
        &self,
        provider: &str,
        windows: &[NewWindow],
        observed_at: i64,
    ) -> Result<(), StoreError> {
        let mut stmt = self.conn.prepare(
            "INSERT INTO usage_windows (provider, name, utilization, resets_at, observed_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(provider, name) DO UPDATE SET
               utilization = excluded.utilization,
               resets_at = excluded.resets_at,
               observed_at = excluded.observed_at",
        )?;
        for window in windows {
            stmt.execute(params![
                provider,
                window.name,
                window.utilization,
                window.resets_at,
                observed_at
            ])?;
        }
        Ok(())
    }

    pub fn delete_window(&self, provider: &str, name: &str) -> Result<(), StoreError> {
        self.exec(
            "DELETE FROM usage_windows WHERE provider = ?1 AND name = ?2",
            params![provider, name],
        )
    }

    /// One row per window name, ordered by name.
    pub fn windows(&self, plan: &str) -> Result<Vec<StoredWindow>, StoreError> {
        self.query_all(
            "SELECT name, utilization, resets_at, observed_at
             FROM usage_windows WHERE provider = ?1 ORDER BY name ASC",
            params![plan],
            map_window,
        )
    }

    /// Utilization of the newest in-window row that has one, else null.
    pub fn last_utilization(
        &self,
        provider: &str,
        since_ms: i64,
    ) -> Result<Option<f64>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT utilization FROM runs
             WHERE provider = ?1 AND started_at >= ?2 AND utilization IS NOT NULL
             ORDER BY started_at DESC, id DESC
             LIMIT 1",
                params![provider, since_ms],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn set_resting(&self, provider: &str, until_ms: i64) -> Result<(), StoreError> {
        self.exec(
            "INSERT INTO resting (provider, until) VALUES (?1, ?2)
             ON CONFLICT(provider) DO UPDATE SET until = excluded.until",
            params![provider, until_ms],
        )
    }

    pub fn resting_until(&self, plan: &str) -> Result<Option<i64>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT until FROM resting WHERE provider = ?1",
                params![plan],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn clear_resting(&self, provider: &str) -> Result<(), StoreError> {
        self.exec("DELETE FROM resting WHERE provider = ?1", params![provider])
    }

    pub fn save_chat(&self, row: &ChatRow) -> Result<(), StoreError> {
        self.exec(
            "INSERT INTO chats (
               id, project, title, created_at, updated_at, session_id, session_provider, direct
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
               project = excluded.project,
               title = excluded.title,
               created_at = excluded.created_at,
               updated_at = excluded.updated_at,
               session_id = excluded.session_id,
               session_provider = excluded.session_provider,
               direct = excluded.direct",
            params![
                row.id,
                row.project,
                row.title,
                row.created_at,
                row.updated_at,
                row.session_id,
                row.session_provider,
                row.direct,
            ],
        )
    }

    pub fn chat_count(&self, project: &str) -> Result<i64, StoreError> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) AS n FROM chats WHERE project = ?1",
            params![project],
            |row| row.get(0),
        )?)
    }

    /// Newest first.
    pub fn chats(&self, project: &str) -> Result<Vec<ChatRow>, StoreError> {
        self.query_all(
            "SELECT id, project, title, created_at, updated_at, session_id, session_provider, direct
             FROM chats WHERE project = ?1 ORDER BY updated_at DESC",
            params![project],
            map_chat,
        )
    }

    pub fn chat(&self, id: &str) -> Result<Option<ChatRow>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, project, title, created_at, updated_at, session_id, session_provider, direct
                 FROM chats WHERE id = ?1",
                params![id],
                map_chat,
            )
            .optional()?)
    }

    pub fn rename_chat(&self, id: &str, title: &str) -> Result<(), StoreError> {
        self.exec(
            "UPDATE chats SET title = ?1 WHERE id = ?2",
            params![title, id],
        )
    }

    /// Deletes the chat and its messages. Jobs are kept.
    pub fn delete_chat(&self, id: &str) -> Result<(), StoreError> {
        self.conn.execute_batch("BEGIN")?;
        let deleted = (|| -> Result<(), StoreError> {
            self.exec("DELETE FROM messages WHERE chat_id = ?1", params![id])?;
            self.exec("DELETE FROM chats WHERE id = ?1", params![id])?;
            self.conn.execute_batch("COMMIT")?;
            Ok(())
        })();
        if deleted.is_err() {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
        deleted
    }

    /// Which chat owns a message, or null when the id is unknown.
    pub fn chat_for_message(&self, message_id: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT chat_id FROM messages WHERE id = ?1",
                params![message_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn save_message(
        &self,
        chat_id: &str,
        seq: i64,
        message: &LeadMessage,
    ) -> Result<(), StoreError> {
        self.exec(
            "INSERT INTO messages (id, chat_id, seq, data) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
               chat_id = excluded.chat_id,
               seq = excluded.seq,
               data = excluded.data",
            params![message.id, chat_id, seq, serde_json::to_string(message)?],
        )
    }

    /// In seq order. Corrupt or id-less rows are skipped.
    pub fn messages(&self, chat_id: &str) -> Result<Vec<LeadMessage>, StoreError> {
        let rows = self.query_all(
            "SELECT data FROM messages WHERE chat_id = ?1 ORDER BY seq ASC",
            params![chat_id],
            |row| row.get::<_, String>(0),
        )?;
        Ok(rows.iter().filter_map(|data| parse_message(data)).collect())
    }

    pub fn save_terminal(&self, row: &TerminalRow) -> Result<(), StoreError> {
        let scrollback = cap_scrollback(&row.scrollback);
        self.exec(
            "INSERT INTO terminals (
               id, project, provider, model, session_id, title, created_at, updated_at, scrollback,
               cwd, change_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(id) DO UPDATE SET
               project = excluded.project,
               provider = excluded.provider,
               model = excluded.model,
               session_id = excluded.session_id,
               title = excluded.title,
               created_at = excluded.created_at,
               updated_at = excluded.updated_at,
               scrollback = excluded.scrollback,
               cwd = excluded.cwd,
               change_id = excluded.change_id",
            params![
                row.id,
                row.project,
                row.provider,
                row.model,
                row.session_id,
                row.title,
                row.created_at,
                row.updated_at,
                scrollback,
                row.cwd,
                row.change_id,
            ],
        )
    }

    pub fn update_terminal_scrollback(
        &self,
        id: &str,
        scrollback: &str,
        updated_at: i64,
    ) -> Result<(), StoreError> {
        self.exec(
            "UPDATE terminals SET scrollback = ?1, updated_at = ?2 WHERE id = ?3",
            params![cap_scrollback(scrollback), updated_at, id],
        )
    }

    /// Oldest first.
    pub fn terminals(&self, project: &str) -> Result<Vec<TerminalRow>, StoreError> {
        self.query_all(
            "SELECT id, project, provider, model, session_id, title, created_at, updated_at,
                    scrollback, cwd, change_id
             FROM terminals WHERE project = ?1 ORDER BY created_at ASC",
            params![project],
            map_terminal,
        )
    }

    pub fn delete_terminal(&self, id: &str) -> Result<(), StoreError> {
        self.exec("DELETE FROM terminals WHERE id = ?1", params![id])
    }

    pub fn set_worktree_name(&self, row: &WorktreeNameRow) -> Result<(), StoreError> {
        self.exec(
            "INSERT INTO worktree_names (id, project, name, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
               project = excluded.project,
               name = excluded.name",
            params![row.id, row.project, row.name, row.created_at],
        )
    }

    /// Newest first.
    pub fn worktree_names(&self, project: &str) -> Result<Vec<WorktreeNameRow>, StoreError> {
        self.query_all(
            "SELECT id, project, name, created_at
             FROM worktree_names WHERE project = ?1 ORDER BY created_at DESC",
            params![project],
            map_worktree,
        )
    }

    pub fn delete_worktree_name(&self, id: &str) -> Result<(), StoreError> {
        self.exec("DELETE FROM worktree_names WHERE id = ?1", params![id])
    }

    pub fn get_project_setup(&self, project: &str) -> Result<Vec<String>, StoreError> {
        let raw: Option<String> = self
            .conn
            .query_row(
                "SELECT commands FROM project_setup WHERE project = ?1",
                params![project],
                |row| row.get(0),
            )
            .optional()?;
        Ok(raw.map(|text| filter_commands(&text)).unwrap_or_default())
    }

    pub fn set_project_setup(&self, project: &str, commands: &[String]) -> Result<(), StoreError> {
        self.exec(
            "INSERT INTO project_setup (project, commands) VALUES (?1, ?2)
             ON CONFLICT(project) DO UPDATE SET commands = excluded.commands",
            params![project, serde_json::to_string(commands)?],
        )
    }

    /// Lowest position first.
    pub fn list_routines(&self, project: &str) -> Result<Vec<RoutineRow>, StoreError> {
        self.query_all(
            "SELECT id, project, position, data
             FROM routines WHERE project = ?1 ORDER BY position ASC",
            params![project],
            map_routine,
        )
    }

    pub fn save_routine(&self, row: &RoutineRow) -> Result<(), StoreError> {
        self.exec(
            "INSERT INTO routines (id, project, position, data) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
               project = excluded.project,
               position = excluded.position,
               data = excluded.data",
            params![row.id, row.project, row.position, row.data],
        )
    }

    pub fn delete_routine(&self, id: &str) -> Result<(), StoreError> {
        self.exec("DELETE FROM routines WHERE id = ?1", params![id])
    }

    pub fn save_goal(
        &self,
        id: &str,
        project: &str,
        updated_at: i64,
        data: &str,
    ) -> Result<(), StoreError> {
        self.exec(
            "INSERT INTO goals (id, project, updated_at, data) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
               project = excluded.project,
               updated_at = excluded.updated_at,
               data = excluded.data",
            params![id, project, updated_at, data],
        )
    }

    /// Newest first. `limit` defaults to 50 when `None`, matching the TypeScript signature.
    pub fn goals(&self, project: &str, limit: Option<i64>) -> Result<Vec<String>, StoreError> {
        self.query_all(
            "SELECT data FROM goals WHERE project = ?1 ORDER BY updated_at DESC LIMIT ?2",
            params![project, limit.unwrap_or(50)],
            |row| row.get(0),
        )
    }

    pub fn goal(&self, id: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT data FROM goals WHERE id = ?1", params![id], |row| {
                row.get(0)
            })
            .optional()?)
    }

    pub fn delete_goal(&self, id: &str) -> Result<(), StoreError> {
        self.exec("DELETE FROM goals WHERE id = ?1", params![id])
    }

    pub fn save_trigger(&self, id: &str, project: &str, data: &str) -> Result<(), StoreError> {
        self.exec(
            "INSERT INTO triggers (id, project, data) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET project = excluded.project, data = excluded.data",
            params![id, project, data],
        )
    }

    /// Every trigger, of every project.
    pub fn triggers(&self) -> Result<Vec<String>, StoreError> {
        self.query_all("SELECT data FROM triggers", (), |row| row.get(0))
    }

    pub fn delete_trigger(&self, id: &str) -> Result<(), StoreError> {
        self.exec("DELETE FROM triggers WHERE id = ?1", params![id])
    }

    pub fn save_job(
        &self,
        project: &str,
        created_at: i64,
        job: &JobRecord,
    ) -> Result<(), StoreError> {
        self.exec(
            "INSERT INTO jobs (id, project, created_at, data, owner_id, heartbeat_at, cancel_requested)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)
             ON CONFLICT(id) DO UPDATE SET
               project = excluded.project,
               data = excluded.data,
               owner_id = excluded.owner_id,
               heartbeat_at = excluded.heartbeat_at",
            params![
                job.id,
                project,
                created_at,
                serde_json::to_string(job)?,
                job.owner_id,
                job.heartbeat_at,
            ],
        )
    }

    /// Sets the cancel flag without rewriting the job JSON the owner is saving.
    pub fn request_cancel(&self, id: &str) -> Result<(), StoreError> {
        self.exec(
            "UPDATE jobs SET cancel_requested = 1 WHERE id = ?1",
            params![id],
        )
    }

    pub fn is_cancel_requested(&self, id: &str) -> Result<bool, StoreError> {
        let flag: Option<i64> = self
            .conn
            .query_row(
                "SELECT cancel_requested FROM jobs WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(flag == Some(1))
    }

    /// The newest `limit` jobs for the project, returned oldest first.
    /// Corrupt or id-less rows are skipped.
    pub fn jobs(&self, project: &str, limit: i64) -> Result<Vec<JobRecord>, StoreError> {
        let mut newest = self.jobs_newest(project, limit)?;
        newest.reverse();
        Ok(newest.into_iter().map(|row| row.job).collect())
    }

    /// Newest `limit` jobs for the project, newest first. Corrupt rows are skipped.
    pub fn jobs_newest(&self, project: &str, limit: i64) -> Result<Vec<StoredJob>, StoreError> {
        let rows = self.query_all(
            "SELECT data, created_at, owner_id, heartbeat_at, cancel_requested FROM jobs
             WHERE project = ?1
             ORDER BY created_at DESC
             LIMIT ?2",
            params![project, limit],
            map_job_row,
        )?;
        Ok(rows.into_iter().flatten().collect())
    }

    fn exec(&self, sql: &str, params: impl rusqlite::Params) -> Result<(), StoreError> {
        self.conn.execute(sql, params)?;
        Ok(())
    }

    fn query_all<T, P, F>(&self, sql: &str, params: P, map: F) -> Result<Vec<T>, StoreError>
    where
        P: rusqlite::Params,
        F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(params, map)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
}

fn map_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunRow> {
    Ok(RunRow {
        id: Some(row.get("id")?),
        job_id: row.get("job_id")?,
        provider: row.get("provider")?,
        job_type: row.get("job_type")?,
        started_at: row.get("started_at")?,
        duration_ms: row.get("duration_ms")?,
        cost_usd: row.get("cost_usd")?,
        tokens: row.get("tokens")?,
        utilization: row.get("utilization")?,
        outcome: row.get("outcome")?,
        session_id: row.get("session_id")?,
    })
}

fn map_window(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredWindow> {
    Ok(StoredWindow {
        name: row.get("name")?,
        utilization: row.get("utilization")?,
        resets_at: row.get("resets_at")?,
        observed_at: row.get("observed_at")?,
    })
}

fn map_chat(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatRow> {
    Ok(ChatRow {
        id: row.get("id")?,
        project: row.get("project")?,
        title: row.get("title")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        session_id: row.get("session_id")?,
        session_provider: row.get("session_provider")?,
        direct: row.get("direct")?,
    })
}

fn map_terminal(row: &rusqlite::Row<'_>) -> rusqlite::Result<TerminalRow> {
    Ok(TerminalRow {
        id: row.get("id")?,
        project: row.get("project")?,
        provider: row.get("provider")?,
        model: row.get("model")?,
        session_id: row.get("session_id")?,
        title: row.get("title")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        scrollback: row.get("scrollback")?,
        cwd: row.get("cwd")?,
        change_id: row.get("change_id")?,
    })
}

fn map_worktree(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorktreeNameRow> {
    Ok(WorktreeNameRow {
        id: row.get("id")?,
        project: row.get("project")?,
        name: row.get("name")?,
        created_at: row.get("created_at")?,
    })
}

fn map_routine(row: &rusqlite::Row<'_>) -> rusqlite::Result<RoutineRow> {
    Ok(RoutineRow {
        id: row.get("id")?,
        project: row.get("project")?,
        position: row.get("position")?,
        data: row.get("data")?,
    })
}

fn map_job_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<StoredJob>> {
    let data: String = row.get("data")?;
    let created_at: Option<i64> = row.get("created_at")?;
    let owner_id: Option<String> = row.get("owner_id")?;
    let heartbeat_at: Option<i64> = row.get("heartbeat_at")?;
    let cancel: Option<i64> = row.get("cancel_requested")?;
    let created_at = created_at.unwrap_or(0);
    Ok(
        job_from_parts(&data, owner_id, heartbeat_at, cancel.unwrap_or(0))
            .map(|job| StoredJob { created_at, job }),
    )
}

fn job_from_parts(
    data: &str,
    owner_id: Option<String>,
    heartbeat_at: Option<i64>,
    cancel: i64,
) -> Option<JobRecord> {
    let value: Value = serde_json::from_str(data).ok()?;
    value.get("id").and_then(Value::as_str)?;
    let mut job: JobRecord = serde_json::from_value(value).ok()?;
    if let Some(owner) = owner_id
        && !owner.is_empty()
    {
        job.owner_id = Some(owner);
    }
    if let Some(beat) = heartbeat_at {
        job.heartbeat_at = Some(beat);
    }
    if cancel == 1 {
        job.cancel_requested = Some(true);
    }
    Some(job)
}

fn parse_message(data: &str) -> Option<LeadMessage> {
    let value: Value = serde_json::from_str(data).ok()?;
    value.get("id").and_then(Value::as_str)?;
    serde_json::from_value(value).ok()
}

fn filter_commands(text: &str) -> Vec<String> {
    let Ok(parsed) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let Some(items) = parsed.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items {
        let Some(command) = item.as_str() else {
            continue;
        };
        let len = command.chars().count();
        if !(1..=300).contains(&len) || command.chars().any(|ch| ch == '\r' || ch == '\n') {
            continue;
        }
        out.push(command.to_string());
        if out.len() >= 5 {
            break;
        }
    }
    out
}

pub fn cap_scrollback(text: &str) -> String {
    if text.len() <= TERMINAL_SCROLLBACK_CAP {
        return text.to_string();
    }
    let total = text.chars().count();
    if total <= TERMINAL_SCROLLBACK_CAP {
        return text.to_string();
    }
    let skip = total - TERMINAL_SCROLLBACK_CAP;
    text.chars().skip(skip).collect()
}
