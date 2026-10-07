import { spawn } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { spawn as spawnPty } from 'node-pty'
import type { Store } from '../router/store'
import type { ProviderId, TerminalInfo } from '../types'

export interface Launch {
  command: string
  args: string[]
  env?: NodeJS.ProcessEnv
}

export interface PtyProcess {
  pid: number
  onData(cb: (data: string) => void): unknown
  onExit(cb: (e: { exitCode: number }) => void): unknown
  write(data: string): void
  resize(cols: number, rows: number): void
  kill(): void
}

export type SpawnPty = (
  command: string,
  args: string[],
  opts: { name: string; cols: number; rows: number; cwd: string; env: Record<string, string> }
) => PtyProcess

const FLUSH_MS = 12
const SNAPSHOT_MAX = 256_000
const WRITE_MAX = 65_536
const DEFAULT_MAX_TERMINALS = 8
const DEFAULT_COLS = 100
const DEFAULT_ROWS = 30

interface Session {
  info: TerminalInfo
  proc: PtyProcess
  snapshot: string
  pending: string
  timer: ReturnType<typeof setTimeout> | null
  removed: boolean
  exited: boolean
  closedByHost: boolean
  logged: boolean
}

interface PtyHostOptions {
  launch: (provider: ProviderId, model: string | undefined) => Launch
  cwd: string | (() => string)
  modelFor?: (provider: ProviderId) => string | undefined
  store?: Store
  spawn?: SpawnPty
  killTree?: (proc: PtyProcess) => void
  now?: () => number
  maxTerminals?: number
}

/** Drop a "no colour" choice inherited from whoever launched the app, and ask for true colour. */
export function terminalEnv(source: NodeJS.ProcessEnv): Record<string, string> {
  const env: Record<string, string> = {}
  for (const [key, value] of Object.entries(source)) {
    if (value === undefined) continue
    const upper = key.toUpperCase()
    if (upper === 'NO_COLOR') continue
    if ((upper === 'FORCE_COLOR' || upper === 'CLICOLOR') && value === '0') continue
    env[key] = value
  }
  env.COLORTERM = 'truecolor'
  return env
}

export class PtyHost {
  private readonly launch: PtyHostOptions['launch']
  private readonly cwd: string | (() => string)
  private readonly modelFor: PtyHostOptions['modelFor']
  private readonly store: Store | undefined
  private readonly spawnPty: SpawnPty
  private readonly killTree: (proc: PtyProcess) => void
  private readonly now: () => number
  private readonly maxTerminals: number
  private readonly sessions: Session[] = []
  private readonly byId = new Map<string, Session>()
  private readonly opened: Record<ProviderId, number> = { claude: 0, cursor: 0 }
  private readonly dataListeners: Array<(id: string, data: string) => void> = []
  private readonly updateListeners: Array<(info: TerminalInfo, removed: boolean) => void> = []

  constructor(opts: PtyHostOptions) {
    this.launch = opts.launch
    this.cwd = opts.cwd
    this.modelFor = opts.modelFor
    this.store = opts.store
    this.spawnPty = opts.spawn ?? defaultSpawn
    this.killTree = opts.killTree ?? defaultKillTree
    this.now = opts.now ?? Date.now
    this.maxTerminals = opts.maxTerminals ?? DEFAULT_MAX_TERMINALS
  }

  open(provider: ProviderId, cols: number, rows: number): TerminalInfo {
    if (this.sessions.filter((session) => !session.removed).length >= this.maxTerminals) {
      throw new Error('too many terminals')
    }
    const size = clampSize(cols, rows)
    const model = this.modelFor?.(provider)
    const spec = this.launch(provider, model)
    let proc: PtyProcess
    try {
      proc = this.spawnPty(spec.command, spec.args, {
        name: 'xterm-256color',
        cols: size.cols,
        rows: size.rows,
        cwd: typeof this.cwd === 'function' ? this.cwd() : this.cwd,
        env: terminalEnv(spec.env ?? process.env)
      })
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err)
      throw new Error(`could not start ${provider}: ${message}`)
    }
    this.opened[provider] += 1
    const info: TerminalInfo = {
      id: randomUUID(),
      provider,
      title: `${provider} ${this.opened[provider]}`,
      status: 'running',
      exitCode: null,
      model: model ?? null,
      startedAt: this.now()
    }
    const session: Session = {
      info,
      proc,
      snapshot: '',
      pending: '',
      timer: null,
      removed: false,
      exited: false,
      closedByHost: false,
      logged: false
    }
    this.sessions.push(session)
    this.byId.set(info.id, session)
    this.attach(session)
    this.emitUpdate(copyInfo(info), false)
    return copyInfo(info)
  }

  write(id: string, data: string): void {
    if (typeof data !== 'string' || data.length < 1 || data.length > WRITE_MAX) return
    const session = this.running(id)
    if (!session) return
    session.proc.write(data)
  }

  resize(id: string, cols: number, rows: number): void {
    const session = this.running(id)
    if (!session) return
    const size = clampSize(cols, rows)
    try {
      session.proc.resize(size.cols, size.rows)
    } catch {
      // A resize the pty rejects must not take down the host.
    }
  }

  close(id: string): void {
    const session = this.byId.get(id)
    if (!session || session.removed) return
    if (session.info.status === 'running') {
      session.closedByHost = true
      try {
        this.killTree(session.proc)
      } catch (err) {
        console.error(err)
      }
    }
    if (session.removed) return
    session.removed = true
    this.emitUpdate(copyInfo(session.info), true)
    this.forget(session)
  }

  /** Free a session that is both closed and exited; its output buffer is the bulk of it. */
  private forget(session: Session): void {
    if (!session.removed || !session.exited) return
    this.byId.delete(session.info.id)
    const index = this.sessions.indexOf(session)
    if (index >= 0) this.sessions.splice(index, 1)
    session.snapshot = ''
  }

  closeAll(): void {
    for (const info of this.list()) this.close(info.id)
  }

  list(): TerminalInfo[] {
    const out: TerminalInfo[] = []
    for (const session of this.sessions) {
      if (!session.removed) out.push(copyInfo(session.info))
    }
    return out
  }

  snapshot(id: string): string {
    const session = this.byId.get(id)
    if (!session || session.removed) return ''
    return session.snapshot
  }

  onData(cb: (id: string, data: string) => void): () => void {
    this.dataListeners.push(cb)
    return () => {
      const index = this.dataListeners.indexOf(cb)
      if (index >= 0) this.dataListeners.splice(index, 1)
    }
  }

  onUpdate(cb: (info: TerminalInfo, removed: boolean) => void): () => void {
    this.updateListeners.push(cb)
    return () => {
      const index = this.updateListeners.indexOf(cb)
      if (index >= 0) this.updateListeners.splice(index, 1)
    }
  }

  private running(id: string): Session | undefined {
    const session = this.byId.get(id)
    if (!session || session.removed || session.info.status !== 'running') return undefined
    return session
  }

  private attach(session: Session): void {
    session.proc.onData((data) => {
      try {
        this.onPtyData(session, data)
      } catch (err) {
        console.error(err)
      }
    })
    session.proc.onExit((event) => {
      try {
        this.onPtyExit(session, event.exitCode)
      } catch (err) {
        console.error(err)
      }
    })
  }

  private onPtyData(session: Session, data: string): void {
    if (session.exited || typeof data !== 'string' || data === '') return
    session.snapshot += data
    if (session.snapshot.length > SNAPSHOT_MAX) {
      session.snapshot = session.snapshot.slice(-SNAPSHOT_MAX)
    }
    session.pending += data
    if (session.timer !== null) return
    session.timer = setTimeout(() => {
      session.timer = null
      try {
        this.flush(session)
      } catch (err) {
        console.error(err)
      }
    }, FLUSH_MS)
  }

  private onPtyExit(session: Session, exitCode: number): void {
    if (session.exited) return
    session.exited = true
    this.flush(session)
    session.info.status = 'exited'
    session.info.exitCode =
      typeof exitCode === 'number' && Number.isFinite(exitCode) ? exitCode : null
    if (!session.removed) this.emitUpdate(copyInfo(session.info), false)
    this.writeRun(session)
    this.forget(session)
  }

  private flush(session: Session): void {
    if (session.timer !== null) {
      clearTimeout(session.timer)
      session.timer = null
    }
    if (session.pending === '') return
    const data = session.pending
    session.pending = ''
    this.emitData(session.info.id, data)
  }

  private writeRun(session: Session): void {
    if (session.logged) return
    session.logged = true
    if (!this.store) return
    const code = session.info.exitCode
    const outcome = session.closedByHost || code === 0 ? 'ok' : 'error'
    this.store.insertRun({
      job_id: session.info.id,
      provider: session.info.provider,
      job_type: 'terminal',
      started_at: session.info.startedAt,
      duration_ms: this.now() - session.info.startedAt,
      cost_usd: null,
      tokens: null,
      utilization: null,
      outcome,
      session_id: null
    })
  }

  private emitData(id: string, data: string): void {
    for (const cb of [...this.dataListeners]) {
      try {
        cb(id, data)
      } catch (err) {
        console.error(err)
      }
    }
  }

  private emitUpdate(info: TerminalInfo, removed: boolean): void {
    for (const cb of [...this.updateListeners]) {
      try {
        cb(info, removed)
      } catch (err) {
        console.error(err)
      }
    }
  }
}

function copyInfo(info: TerminalInfo): TerminalInfo {
  return { ...info }
}

function clampSize(cols: number, rows: number): { cols: number; rows: number } {
  if (!Number.isFinite(cols) || !Number.isFinite(rows)) {
    return { cols: DEFAULT_COLS, rows: DEFAULT_ROWS }
  }
  return { cols: clampInt(cols, 2, 500), rows: clampInt(rows, 1, 300) }
}

function clampInt(value: number, min: number, max: number): number {
  const n = Math.trunc(value)
  if (n < min) return min
  if (n > max) return max
  return n
}

function defaultSpawn(
  command: string,
  args: string[],
  opts: { name: string; cols: number; rows: number; cwd: string; env: Record<string, string> }
): PtyProcess {
  const proc = spawnPty(command, args, opts)
  return {
    pid: proc.pid,
    onData: (cb) => proc.onData(cb),
    onExit: (cb) => proc.onExit((event) => cb({ exitCode: event.exitCode })),
    write: (data) => {
      proc.write(data)
    },
    resize: (cols, rows) => {
      proc.resize(cols, rows)
    },
    kill: () => {
      proc.kill()
    }
  }
}

function defaultKillTree(proc: PtyProcess): void {
  if (process.platform === 'win32') {
    const killer = spawn('taskkill', ['/pid', String(proc.pid), '/T', '/F'], {
      windowsHide: true,
      shell: false,
      stdio: 'ignore'
    })
    killer.on('error', () => {})
    killer.unref()
    return
  }
  try {
    proc.kill()
  } catch {
    // Already gone.
  }
}
