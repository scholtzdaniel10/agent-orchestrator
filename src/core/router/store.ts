import { DatabaseSync } from 'node:sqlite'
import type { LeadMessage } from '../types'
import type { JobRecord } from './orchestrator'

export interface StoredWindow {
  name: string
  utilization: number
  /** Epoch ms, or null when the CLI did not say. */
  resetsAt: number | null
  observedAt: number
}

export interface RunRow {
  id?: number
  job_id: string
  provider: string
  job_type: string
  started_at: number
  duration_ms: number | null
  cost_usd: number | null
  tokens: number | null
  utilization: number | null
  outcome: 'ok' | 'error' | 'limit'
  session_id: string | null
}

export interface ChatRow {
  id: string
  project: string
  title: string
  created_at: number
  updated_at: number
  session_id: string | null
  session_provider: string | null
}

const MIGRATIONS: string[] = [
  `
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
    `,
  `
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
    `
]

/**
 * Usage ledger and provider rest flags. One connection, created up front.
 * `path` is a file path or `':memory:'`.
 */
export class Store {
  private readonly db: DatabaseSync

  constructor(path: string) {
    this.db = new DatabaseSync(path)
    this.migrate()
  }

  private migrate(): void {
    const row = this.db.prepare('PRAGMA user_version').get() as { user_version: number } | undefined
    let version = row?.user_version ?? 0
    while (version < MIGRATIONS.length) {
      const sql = MIGRATIONS[version]
      if (sql === undefined) break
      this.db.exec('BEGIN')
      try {
        this.db.exec(sql)
        version += 1
        this.db.exec(`PRAGMA user_version = ${String(version)}`)
        this.db.exec('COMMIT')
      } catch (err) {
        this.db.exec('ROLLBACK')
        throw err
      }
    }
  }

  insertRun(row: RunRow): void {
    this.db
      .prepare(
        `INSERT INTO runs (
           job_id, provider, job_type, started_at, duration_ms,
           cost_usd, tokens, utilization, outcome, session_id
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`
      )
      .run(
        row.job_id,
        row.provider,
        row.job_type,
        row.started_at,
        row.duration_ms,
        row.cost_usd,
        row.tokens,
        row.utilization,
        row.outcome,
        row.session_id
      )
  }

  /** All runs, oldest first. Optionally one provider. */
  runs(provider?: string): RunRow[] {
    const sql = provider
      ? 'SELECT * FROM runs WHERE provider = ? ORDER BY started_at ASC, id ASC'
      : 'SELECT * FROM runs ORDER BY started_at ASC, id ASC'
    const rows = provider ? this.db.prepare(sql).all(provider) : this.db.prepare(sql).all()
    return rows.map((row) => mapRun(row as Row))
  }

  runsSince(provider: string, sinceMs: number): RunRow[] {
    const rows = this.db
      .prepare(
        `SELECT * FROM runs
         WHERE provider = ? AND started_at >= ?
         ORDER BY started_at ASC, id ASC`
      )
      .all(provider, sinceMs)
    return rows.map((row) => mapRun(row as Row))
  }

  saveWindows(
    provider: string,
    windows: { name: string; utilization: number; resetsAt: number | null }[],
    observedAt: number
  ): void {
    const stmt = this.db.prepare(
      `INSERT INTO usage_windows (provider, name, utilization, resets_at, observed_at)
       VALUES (?, ?, ?, ?, ?)
       ON CONFLICT(provider, name) DO UPDATE SET
         utilization = excluded.utilization,
         resets_at = excluded.resets_at,
         observed_at = excluded.observed_at`
    )
    for (const window of windows) {
      stmt.run(provider, window.name, window.utilization, window.resetsAt, observedAt)
    }
  }

  /** One row per window name, ordered by name. */
  windows(provider: string): StoredWindow[] {
    const rows = this.db
      .prepare(
        `SELECT name, utilization, resets_at, observed_at
         FROM usage_windows WHERE provider = ? ORDER BY name ASC`
      )
      .all(provider)
    return rows.map((row) => mapWindow(row as Row))
  }

  /** Utilization of the newest in-window row that has one, else null. */
  lastUtilization(provider: string, sinceMs: number): number | null {
    const row = this.db
      .prepare(
        `SELECT utilization FROM runs
         WHERE provider = ? AND started_at >= ? AND utilization IS NOT NULL
         ORDER BY started_at DESC, id DESC
         LIMIT 1`
      )
      .get(provider, sinceMs)
    if (!row) return null
    return Number(row.utilization)
  }

  setResting(provider: string, untilMs: number): void {
    this.db
      .prepare(
        `INSERT INTO resting (provider, until) VALUES (?, ?)
         ON CONFLICT(provider) DO UPDATE SET until = excluded.until`
      )
      .run(provider, untilMs)
  }

  restingUntil(provider: string): number | null {
    const row = this.db.prepare('SELECT until FROM resting WHERE provider = ?').get(provider)
    if (!row) return null
    return Number(row.until)
  }

  clearResting(provider: string): void {
    this.db.prepare('DELETE FROM resting WHERE provider = ?').run(provider)
  }

  saveChat(row: ChatRow): void {
    this.db
      .prepare(
        `INSERT INTO chats (
           id, project, title, created_at, updated_at, session_id, session_provider
         ) VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
           project = excluded.project,
           title = excluded.title,
           created_at = excluded.created_at,
           updated_at = excluded.updated_at,
           session_id = excluded.session_id,
           session_provider = excluded.session_provider`
      )
      .run(
        row.id,
        row.project,
        row.title,
        row.created_at,
        row.updated_at,
        row.session_id,
        row.session_provider
      )
  }

  /** Newest first. */
  chats(project: string): ChatRow[] {
    const rows = this.db
      .prepare(
        `SELECT id, project, title, created_at, updated_at, session_id, session_provider
         FROM chats WHERE project = ? ORDER BY updated_at DESC`
      )
      .all(project)
    return rows.map((row) => mapChat(row as Row))
  }

  chat(id: string): ChatRow | null {
    const row = this.db
      .prepare(
        `SELECT id, project, title, created_at, updated_at, session_id, session_provider
         FROM chats WHERE id = ?`
      )
      .get(id)
    if (!row) return null
    return mapChat(row as Row)
  }

  renameChat(id: string, title: string): void {
    this.db.prepare('UPDATE chats SET title = ? WHERE id = ?').run(title, id)
  }

  /** Deletes the chat and its messages. Jobs are kept. */
  deleteChat(id: string): void {
    this.db.exec('BEGIN')
    try {
      this.db.prepare('DELETE FROM messages WHERE chat_id = ?').run(id)
      this.db.prepare('DELETE FROM chats WHERE id = ?').run(id)
      this.db.exec('COMMIT')
    } catch (err) {
      this.db.exec('ROLLBACK')
      throw err
    }
  }

  /** Which chat owns a message, or null when the id is unknown. */
  chatForMessage(messageId: string): string | null {
    const row = this.db.prepare(`SELECT chat_id FROM messages WHERE id = ?`).get(messageId)
    if (!row) return null
    const chatId = (row as Row).chat_id
    return typeof chatId === 'string' ? chatId : null
  }

  saveMessage(chatId: string, seq: number, message: LeadMessage): void {
    this.db
      .prepare(
        `INSERT INTO messages (id, chat_id, seq, data) VALUES (?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
           chat_id = excluded.chat_id,
           seq = excluded.seq,
           data = excluded.data`
      )
      .run(message.id, chatId, seq, JSON.stringify(message))
  }

  /** In seq order. Corrupt or id-less rows are skipped. */
  messages(chatId: string): LeadMessage[] {
    const rows = this.db
      .prepare(`SELECT data FROM messages WHERE chat_id = ? ORDER BY seq ASC`)
      .all(chatId)
    const out: LeadMessage[] = []
    for (const row of rows) {
      const parsed = parseJsonRow(row as Row)
      if (parsed !== null) out.push(parsed as LeadMessage)
    }
    return out
  }

  saveJob(project: string, createdAt: number, job: JobRecord): void {
    this.db
      .prepare(
        `INSERT INTO jobs (id, project, created_at, data) VALUES (?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
           project = excluded.project,
           data = excluded.data`
      )
      .run(job.id, project, createdAt, JSON.stringify(job))
  }

  /**
   * The newest `limit` jobs for the project, returned oldest first.
   * Corrupt or id-less rows are skipped.
   */
  jobs(project: string, limit: number): JobRecord[] {
    const rows = this.db
      .prepare(
        `SELECT data FROM jobs
         WHERE project = ?
         ORDER BY created_at DESC
         LIMIT ?`
      )
      .all(project, limit)
    const newest: JobRecord[] = []
    for (const row of rows) {
      const parsed = parseJsonRow(row as Row)
      if (parsed !== null) newest.push(parsed as JobRecord)
    }
    newest.reverse()
    return newest
  }

  close(): void {
    this.db.close()
  }
}

type Row = Record<string, string | number | bigint | null | Uint8Array>

function parseJsonRow(row: Row): { id: string } | null {
  try {
    const value = JSON.parse(String(row.data)) as unknown
    if (value === null || typeof value !== 'object') return null
    const id = (value as { id?: unknown }).id
    if (typeof id !== 'string') return null
    return value as { id: string }
  } catch {
    return null
  }
}

function mapChat(row: Row): ChatRow {
  return {
    id: String(row.id),
    project: String(row.project),
    title: String(row.title),
    created_at: Number(row.created_at),
    updated_at: Number(row.updated_at),
    session_id:
      row.session_id === null || row.session_id === undefined ? null : String(row.session_id),
    session_provider:
      row.session_provider === null || row.session_provider === undefined
        ? null
        : String(row.session_provider)
  }
}

function mapWindow(row: Row): StoredWindow {
  return {
    name: String(row.name),
    utilization: Number(row.utilization),
    resetsAt: row.resets_at === null ? null : Number(row.resets_at),
    observedAt: Number(row.observed_at)
  }
}

function mapRun(row: Row): RunRow {
  return {
    id: Number(row.id),
    job_id: String(row.job_id),
    provider: String(row.provider),
    job_type: String(row.job_type),
    started_at: Number(row.started_at),
    duration_ms: row.duration_ms === null ? null : Number(row.duration_ms),
    cost_usd: row.cost_usd === null ? null : Number(row.cost_usd),
    tokens: row.tokens === null ? null : Number(row.tokens),
    utilization: row.utilization === null ? null : Number(row.utilization),
    outcome: String(row.outcome) as RunRow['outcome'],
    session_id: row.session_id === null ? null : String(row.session_id)
  }
}
