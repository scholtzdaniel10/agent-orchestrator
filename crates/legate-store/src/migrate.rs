//! Schema migrations, in the same order as `src/core/router/store.ts`.
//! `user_version` is the count of applied migrations. A current file is version 11.

use rusqlite::{Connection, Result};

const MIGRATIONS: [&str; 11] = [
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
    ",
    "
      CREATE TABLE chats (
        id TEXT PRIMARY KEY,
        project TEXT NOT NULL,
        title TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        session_id TEXT,
        session_provider TEXT
      );
      CREATE INDEX chats_project ON chats (project, updated_at);
      CREATE TABLE messages (
        id TEXT PRIMARY KEY,
        chat_id TEXT NOT NULL,
        seq INTEGER NOT NULL,
        data TEXT NOT NULL
      );
      CREATE INDEX messages_chat ON messages (chat_id, seq);
      CREATE TABLE jobs (
        id TEXT PRIMARY KEY,
        project TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        data TEXT NOT NULL
      );
      CREATE INDEX jobs_project ON jobs (project, created_at);
    ",
    "
      CREATE TABLE terminals (
        id TEXT PRIMARY KEY,
        project TEXT NOT NULL,
        provider TEXT NOT NULL,
        model TEXT,
        session_id TEXT,
        title TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        scrollback TEXT NOT NULL DEFAULT ''
      );
      CREATE INDEX terminals_project ON terminals (project, created_at);
    ",
    "
      ALTER TABLE terminals ADD COLUMN cwd TEXT;
      ALTER TABLE terminals ADD COLUMN change_id TEXT;
    ",
    "
      CREATE TABLE worktree_names (
        id TEXT PRIMARY KEY,
        project TEXT NOT NULL,
        name TEXT NOT NULL,
        created_at INTEGER NOT NULL
      );
    ",
    "
      CREATE TABLE project_setup (
        project TEXT PRIMARY KEY,
        commands TEXT NOT NULL
      );
    ",
    "
      CREATE TABLE routines (
        id TEXT PRIMARY KEY,
        project TEXT NOT NULL,
        position INTEGER NOT NULL,
        data TEXT NOT NULL
      );
      CREATE INDEX routines_project ON routines (project, position);
    ",
    "
      ALTER TABLE jobs ADD COLUMN owner_id TEXT;
      ALTER TABLE jobs ADD COLUMN heartbeat_at INTEGER;
      ALTER TABLE jobs ADD COLUMN cancel_requested INTEGER NOT NULL DEFAULT 0;
    ",
    "ALTER TABLE chats ADD COLUMN direct TEXT;",
    "
      CREATE TABLE goals (
        id TEXT PRIMARY KEY,
        project TEXT NOT NULL,
        updated_at INTEGER NOT NULL,
        data TEXT NOT NULL
      );
      CREATE INDEX goals_project ON goals (project, updated_at);
    ",
    "
      CREATE TABLE triggers (
        id TEXT PRIMARY KEY,
        project TEXT NOT NULL,
        data TEXT NOT NULL
      );
    ",
];

pub(crate) fn migrate(conn: &Connection) -> Result<()> {
    let mut version: i64 = conn.query_row("PRAGMA user_version", (), |row| row.get(0))?;
    while version < MIGRATIONS.len() as i64 {
        let sql = MIGRATIONS[version as usize];
        conn.execute_batch("BEGIN")?;
        let applied = apply(conn, sql, version + 1);
        if let Err(err) = applied {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(err);
        }
        version += 1;
    }
    Ok(())
}

fn apply(conn: &Connection, sql: &str, version: i64) -> Result<()> {
    conn.execute_batch(sql)?;
    conn.execute_batch(&format!("PRAGMA user_version = {version}"))?;
    conn.execute_batch("COMMIT")?;
    Ok(())
}
