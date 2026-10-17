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
  /** Plan this chat talks to directly; null or absent for a lead chat. */
  direct?: string | null
}

export interface TerminalRow {
  id: string
  project: string
  provider: string
  model: string | null
  session_id: string | null
  title: string
  created_at: number
  updated_at: number
  scrollback: string
  cwd: string | null
  change_id: string | null
}

export interface WorktreeNameRow {
  id: string
  project: string
  name: string
  created_at: number
}

export interface RoutineRow {
  id: string
  project: string
  position: number
  data: string
}

export const TERMINAL_SCROLLBACK_CAP = 200_000

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
    `,
  `
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
    `,
  `
      ALTER TABLE terminals ADD COLUMN cwd TEXT;
      ALTER TABLE terminals ADD COLUMN change_id TEXT;
    `,
  `
      CREATE TABLE worktree_names (
        id TEXT PRIMARY KEY,
        project TEXT NOT NULL,
        name TEXT NOT NULL,
        created_at INTEGER NOT NULL
      );
    `,
  `
      CREATE TABLE project_setup (
        project TEXT PRIMARY KEY,
        commands TEXT NOT NULL
      );
    `,
  `
      CREATE TABLE routines (
        id TEXT PRIMARY KEY,
        project TEXT NOT NULL,
        position INTEGER NOT NULL,
        data TEXT NOT NULL
      );
      CREATE INDEX routines_project ON routines (project, position);
    `,
  `
      ALTER TABLE jobs ADD COLUMN owner_id TEXT;
      ALTER TABLE jobs ADD COLUMN heartbeat_at INTEGER;
      ALTER TABLE jobs ADD COLUMN cancel_requested INTEGER NOT NULL DEFAULT 0;
    `,
  'ALTER TABLE chats ADD COLUMN direct TEXT;',
  `
      CREATE TABLE goals (
        id TEXT PRIMARY KEY,
        project TEXT NOT NULL,
        updated_at INTEGER NOT NULL,
        data TEXT NOT NULL
      );
      CREATE INDEX goals_project ON goals (project, updated_at);
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
    // The app and the command can have this file open together. Wait instead of failing at once.
    this.db.exec('PRAGMA busy_timeout = 5000')
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
           id, project, title, created_at, updated_at, session_id, session_provider, direct
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
           project = excluded.project,
           title = excluded.title,
           created_at = excluded.created_at,
           updated_at = excluded.updated_at,
           session_id = excluded.session_id,
           session_provider = excluded.session_provider,
           direct = excluded.direct`
      )
      .run(
        row.id,
        row.project,
        row.title,
        row.created_at,
        row.updated_at,
        row.session_id,
        row.session_provider,
        row.direct ?? null
      )
  }

  chatCount(project: string): number {
    const row = this.db.prepare('SELECT COUNT(*) AS n FROM chats WHERE project = ?').get(project)
    return Number((row as { n?: unknown } | undefined)?.n ?? 0)
  }

  /** Newest first. */
  chats(project: string): ChatRow[] {
    const rows = this.db
      .prepare(
        `SELECT id, project, title, created_at, updated_at, session_id, session_provider, direct
         FROM chats WHERE project = ? ORDER BY updated_at DESC`
      )
      .all(project)
    return rows.map((row) => mapChat(row as Row))
  }

  chat(id: string): ChatRow | null {
    const row = this.db
      .prepare(
        `SELECT id, project, title, created_at, updated_at, session_id, session_provider, direct
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

  saveTerminal(row: TerminalRow): void {
    const scrollback = capScrollback(row.scrollback)
    this.db
      .prepare(
        `INSERT INTO terminals (
           id, project, provider, model, session_id, title, created_at, updated_at, scrollback,
           cwd, change_id
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
           change_id = excluded.change_id`
      )
      .run(
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
        row.change_id
      )
  }

  updateTerminalScrollback(id: string, scrollback: string, updatedAt: number): void {
    this.db
      .prepare(`UPDATE terminals SET scrollback = ?, updated_at = ? WHERE id = ?`)
      .run(capScrollback(scrollback), updatedAt, id)
  }

  /** Oldest first. */
  terminals(project: string): TerminalRow[] {
    const rows = this.db
      .prepare(
        `SELECT id, project, provider, model, session_id, title, created_at, updated_at, scrollback,
                cwd, change_id
         FROM terminals WHERE project = ? ORDER BY created_at ASC`
      )
      .all(project)
    return rows.map((row) => mapTerminal(row as Row))
  }

  deleteTerminal(id: string): void {
    this.db.prepare('DELETE FROM terminals WHERE id = ?').run(id)
  }

  setWorktreeName(row: WorktreeNameRow): void {
    this.db
      .prepare(
        `INSERT INTO worktree_names (id, project, name, created_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
           project = excluded.project,
           name = excluded.name`
      )
      .run(row.id, row.project, row.name, row.created_at)
  }

  /** Newest first. */
  worktreeNames(project: string): WorktreeNameRow[] {
    const rows = this.db
      .prepare(
        `SELECT id, project, name, created_at
         FROM worktree_names WHERE project = ? ORDER BY created_at DESC`
      )
      .all(project)
    return rows.map((row) => mapWorktreeName(row as Row))
  }

  deleteWorktreeName(id: string): void {
    this.db.prepare('DELETE FROM worktree_names WHERE id = ?').run(id)
  }

  getProjectSetup(project: string): string[] {
    const row = this.db.prepare(`SELECT commands FROM project_setup WHERE project = ?`).get(project)
    if (!row) return []
    try {
      const parsed: unknown = JSON.parse(String((row as Row).commands))
      if (!Array.isArray(parsed)) return []
      const out: string[] = []
      for (const item of parsed) {
        if (typeof item !== 'string' || item.length < 1 || item.length > 300) continue
        if (/[\r\n]/.test(item)) continue
        out.push(item)
        if (out.length >= 5) break
      }
      return out
    } catch {
      return []
    }
  }

  setProjectSetup(project: string, commands: string[]): void {
    this.db
      .prepare(
        `INSERT INTO project_setup (project, commands) VALUES (?, ?)
         ON CONFLICT(project) DO UPDATE SET commands = excluded.commands`
      )
      .run(project, JSON.stringify(commands))
  }

  /** Lowest position first. */
  listRoutines(project: string): RoutineRow[] {
    const rows = this.db
      .prepare(
        `SELECT id, project, position, data
         FROM routines WHERE project = ? ORDER BY position ASC`
      )
      .all(project)
    return rows.map((row) => mapRoutine(row as Row))
  }

  saveRoutine(row: RoutineRow): void {
    this.db
      .prepare(
        `INSERT INTO routines (id, project, position, data) VALUES (?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
           project = excluded.project,
           position = excluded.position,
           data = excluded.data`
      )
      .run(row.id, row.project, row.position, row.data)
  }

  deleteRoutine(id: string): void {
    this.db.prepare('DELETE FROM routines WHERE id = ?').run(id)
  }

  /** A goal as JSON text; the goals module owns its shape. */
  saveGoal(id: string, project: string, updatedAt: number, data: string): void {
    this.db
      .prepare(
        `INSERT INTO goals (id, project, updated_at, data) VALUES (?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
           project = excluded.project,
           updated_at = excluded.updated_at,
           data = excluded.data`
      )
      .run(id, project, updatedAt, data)
  }

  /** Newest first. */
  goals(project: string, limit = 50): string[] {
    const rows = this.db
      .prepare('SELECT data FROM goals WHERE project = ? ORDER BY updated_at DESC LIMIT ?')
      .all(project, limit)
    return rows.map((row) => String((row as { data: unknown }).data))
  }

  goal(id: string): string | null {
    const row = this.db.prepare('SELECT data FROM goals WHERE id = ?').get(id)
    return row === undefined ? null : String((row as { data: unknown }).data)
  }

  deleteGoal(id: string): void {
    this.db.prepare('DELETE FROM goals WHERE id = ?').run(id)
  }

  saveJob(project: string, createdAt: number, job: JobRecord): void {
    this.db
      .prepare(
        `INSERT INTO jobs (id, project, created_at, data, owner_id, heartbeat_at, cancel_requested)
         VALUES (?, ?, ?, ?, ?, ?, 0)
         ON CONFLICT(id) DO UPDATE SET
           project = excluded.project,
           data = excluded.data,
           owner_id = excluded.owner_id,
           heartbeat_at = excluded.heartbeat_at`
      )
      .run(
        job.id,
        project,
        createdAt,
        JSON.stringify(job),
        job.ownerId ?? null,
        job.heartbeatAt ?? null
      )
  }

  /** Sets the cancel flag without rewriting the job JSON the owner is saving. */
  requestCancel(id: string): void {
    this.db.prepare('UPDATE jobs SET cancel_requested = 1 WHERE id = ?').run(id)
  }

  isCancelRequested(id: string): boolean {
    const row = this.db.prepare('SELECT cancel_requested FROM jobs WHERE id = ?').get(id)
    if (!row) return false
    return Number((row as Row).cancel_requested) === 1
  }

  /**
   * The newest `limit` jobs for the project, returned oldest first.
   * Corrupt or id-less rows are skipped.
   */
  jobs(project: string, limit: number): JobRecord[] {
    const rows = this.db
      .prepare(
        `SELECT data, owner_id, heartbeat_at, cancel_requested FROM jobs
         WHERE project = ?
         ORDER BY created_at DESC
         LIMIT ?`
      )
      .all(project, limit)
    const newest: JobRecord[] = []
    for (const row of rows) {
      const parsed = jobFromRow(row as Row)
      if (parsed !== null) newest.push(parsed)
    }
    newest.reverse()
    return newest
  }

  /** Newest `limit` jobs for the project, newest first. Corrupt rows are skipped. */
  jobsNewest(project: string, limit: number): { createdAt: number; job: JobRecord }[] {
    const rows = this.db
      .prepare(
        `SELECT data, created_at, owner_id, heartbeat_at, cancel_requested FROM jobs
         WHERE project = ?
         ORDER BY created_at DESC
         LIMIT ?`
      )
      .all(project, limit)
    const out: { createdAt: number; job: JobRecord }[] = []
    for (const row of rows) {
      const record = row as Row
      const parsed = jobFromRow(record)
      if (parsed === null) continue
      const created = record.created_at
      const createdAt = typeof created === 'number' ? created : Number(created)
      out.push({
        createdAt: Number.isFinite(createdAt) ? createdAt : 0,
        job: parsed
      })
    }
    return out
  }

  close(): void {
    this.db.close()
  }
}

type Row = Record<string, string | number | bigint | null | Uint8Array>

function jobFromRow(row: Row): JobRecord | null {
  const parsed = parseJsonRow(row)
  if (parsed === null) return null
  const job = parsed as JobRecord
  const owner = row.owner_id
  if (typeof owner === 'string' && owner !== '') job.ownerId = owner
  const beat = row.heartbeat_at
  if (beat !== null && beat !== undefined) {
    const n = Number(beat)
    if (Number.isFinite(n)) job.heartbeatAt = n
  }
  if (Number(row.cancel_requested) === 1) job.cancelRequested = true
  return job
}

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

function capScrollback(text: string): string {
  if (text.length <= TERMINAL_SCROLLBACK_CAP) return text
  return text.slice(-TERMINAL_SCROLLBACK_CAP)
}

function mapTerminal(row: Row): TerminalRow {
  return {
    id: String(row.id),
    project: String(row.project),
    provider: String(row.provider),
    model: row.model === null || row.model === undefined ? null : String(row.model),
    session_id:
      row.session_id === null || row.session_id === undefined ? null : String(row.session_id),
    title: String(row.title),
    created_at: Number(row.created_at),
    updated_at: Number(row.updated_at),
    scrollback: String(row.scrollback ?? ''),
    cwd: row.cwd === null || row.cwd === undefined ? null : String(row.cwd),
    change_id: row.change_id === null || row.change_id === undefined ? null : String(row.change_id)
  }
}

function mapWorktreeName(row: Row): WorktreeNameRow {
  return {
    id: String(row.id),
    project: String(row.project),
    name: String(row.name),
    created_at: Number(row.created_at)
  }
}

function mapRoutine(row: Row): RoutineRow {
  return {
    id: String(row.id),
    project: String(row.project),
    position: Number(row.position),
    data: String(row.data)
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
        : String(row.session_provider),
    direct: row.direct === null || row.direct === undefined ? null : String(row.direct)
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
