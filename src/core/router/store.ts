import { DatabaseSync } from 'node:sqlite'

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

/**
 * Usage ledger and provider rest flags. One connection, created up front.
 * `path` is a file path or `':memory:'`.
 */
export class Store {
  private readonly db: DatabaseSync

  constructor(path: string) {
    this.db = new DatabaseSync(path)
    this.db.exec(`
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
    `)
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

  close(): void {
    this.db.close()
  }
}

type Row = Record<string, string | number | bigint | null | Uint8Array>

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
