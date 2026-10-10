import { spawn } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { existsSync } from 'node:fs'
import { createRequire } from 'node:module'
import type * as NodePty from 'node-pty'
import { TERMINAL_SCROLLBACK_CAP, type Store } from '../router/store'
import type { ProviderId, TerminalInfo } from '../types'

export interface Launch {
  command: string
  args: string[]
  env?: NodeJS.ProcessEnv
}

export interface LaunchOpts {
  model?: string
  sessionId?: string
  resume?: string
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

export type CreateChat = () => Promise<string>

const FLUSH_MS = 12
const STORE_FLUSH_MS = 2_000
const CREATE_CHAT_MS = 10_000
const SNAPSHOT_MAX = 256_000
const WRITE_MAX = 65_536
const DEFAULT_MAX_TERMINALS = 16
const DEFAULT_COLS = 100
const DEFAULT_ROWS = 30
const CLOSE_CHANGE_MS = 5_000
// A CLI that cannot resume says so and exits within a few seconds, even on a cold start.
const PROBATION_MS = 20_000
const RESTORE_MARK = '\r\n\x1b[2m— restored session —\x1b[0m\r\n'
const FRESH_MARK = '\r\n\x1b[2m— could not resume; started a new session —\x1b[0m\r\n'

interface Session {
  info: TerminalInfo
  proc: PtyProcess
  snapshot: string
  pending: string
  timer: ReturnType<typeof setTimeout> | null
  storeTimer: ReturnType<typeof setTimeout> | null
  cols: number
  rows: number
  project: string
  cwd: string
  exitWaiters: Array<() => void>
  onProbation: boolean
  relaunching: boolean
  removed: boolean
  exited: boolean
  closedByHost: boolean
  logged: boolean
}

interface PtyHostOptions {
  launch: (provider: ProviderId, opts: LaunchOpts) => Launch
  cwd: string | (() => string)
  modelFor?: (provider: ProviderId) => string | undefined
  store?: Store
  spawn?: SpawnPty
  killTree?: (proc: PtyProcess) => void
  now?: () => number
  setTimeout?: (fn: () => void, ms: number) => ReturnType<typeof setTimeout>
  clearTimeout?: (id: ReturnType<typeof setTimeout>) => void
  createChat?: CreateChat
  maxTerminals?: number
  exists?: (path: string) => boolean
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
  private readonly setTimeout: (fn: () => void, ms: number) => ReturnType<typeof setTimeout>
  private readonly clearTimeout: (id: ReturnType<typeof setTimeout>) => void
  private readonly createChat: CreateChat | undefined
  private readonly maxTerminals: number
  private readonly exists: (path: string) => boolean
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
    this.setTimeout = opts.setTimeout ?? setTimeout
    this.clearTimeout = opts.clearTimeout ?? clearTimeout
    this.createChat = opts.createChat
    this.maxTerminals = opts.maxTerminals ?? DEFAULT_MAX_TERMINALS
    this.exists = opts.exists ?? existsSync
  }

  async open(
    provider: ProviderId,
    cols: number,
    rows: number,
    opts?: { cwd?: string; change?: string }
  ): Promise<TerminalInfo> {
    if (this.liveCount() >= this.maxTerminals) {
      throw new Error('too many terminals')
    }
    const model = this.modelFor?.(provider)
    const sessionId = await this.freshSessionId(provider)
    const launchOpts: LaunchOpts = { model }
    if (sessionId !== null) {
      if (provider === 'claude') launchOpts.sessionId = sessionId
      else launchOpts.resume = sessionId
    }
    const n = this.opened[provider] + 1
    const change = opts?.change
    const title = change === undefined ? `${provider} ${n}` : `${provider} ${n} · ${change}`
    const info = this.boot({
      id: randomUUID(),
      provider,
      title,
      model,
      launchOpts,
      cols,
      rows,
      snapshot: '',
      startedAt: this.now(),
      cwd: opts?.cwd,
      change
    })
    this.opened[provider] = n
    this.persistOpen(info, sessionId)
    return info
  }

  restore(project: string, cols: number, rows: number): TerminalInfo[] {
    const store = this.store
    if (!store) return []
    const restored: TerminalInfo[] = []
    for (const row of store.terminals(project)) {
      if (this.liveCount() >= this.maxTerminals) break
      if (this.byId.has(row.id)) continue
      const provider = asProvider(row.provider)
      if (provider === null) continue
      const model = row.model ?? undefined
      let sessionId = row.session_id
      const launchOpts: LaunchOpts = { model }
      if (sessionId !== null) {
        launchOpts.resume = sessionId
      } else if (provider === 'claude') {
        sessionId = randomUUID()
        launchOpts.sessionId = sessionId
      }
      const snapshot = `${row.scrollback}${RESTORE_MARK}`
      const savedCwd = row.cwd
      if (savedCwd !== null && savedCwd !== '') {
        if (!this.exists(savedCwd)) {
          store.deleteTerminal(row.id)
          continue
        }
      }
      try {
        const info = this.boot({
          id: row.id,
          provider,
          title: row.title,
          model,
          launchOpts,
          cols,
          rows,
          snapshot,
          startedAt: this.now(),
          onProbation: row.session_id !== null,
          cwd: savedCwd === null || savedCwd === '' ? undefined : savedCwd,
          change: row.change_id ?? undefined,
          project
        })
        this.opened[provider] += 1
        if (sessionId !== row.session_id) {
          store.saveTerminal({
            ...row,
            session_id: sessionId,
            updated_at: info.startedAt
          })
        }
        restored.push(info)
      } catch (err) {
        console.error(err)
      }
    }
    return restored
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
    this.teardown(session, { keepRow: false })
  }

  async closeChange(id: string): Promise<void> {
    const matching = this.sessions.filter(
      (session) => !session.removed && session.info.change === id
    )
    if (matching.length === 0) return
    const waits = matching.map((session) => this.whenExited(session))
    for (const session of matching) {
      this.teardown(session, { keepRow: false })
    }
    await new Promise<void>((resolve) => {
      let settled = false
      const done = (): void => {
        if (settled) return
        settled = true
        this.clearTimeout(timer)
        resolve()
      }
      const timer = this.setTimeout(done, CLOSE_CHANGE_MS)
      void Promise.all(waits).then(done, done)
    })
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
    for (const session of [...this.sessions]) {
      if (session.removed) continue
      this.flushStore(session)
      this.teardown(session, { keepRow: true })
    }
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

  private liveCount(): number {
    return this.sessions.filter((session) => !session.removed).length
  }

  private projectDir(): string {
    return typeof this.cwd === 'function' ? this.cwd() : this.cwd
  }

  private async freshSessionId(provider: ProviderId): Promise<string | null> {
    if (provider === 'claude') return randomUUID()
    if (!this.createChat) return null
    try {
      const raw = await withTimeout(
        this.createChat(),
        CREATE_CHAT_MS,
        this.setTimeout,
        this.clearTimeout
      )
      return parseSessionId(raw)
    } catch {
      return null
    }
  }

  private boot(args: {
    id: string
    provider: ProviderId
    title: string
    model: string | undefined
    launchOpts: LaunchOpts
    cols: number
    rows: number
    snapshot: string
    startedAt: number
    onProbation?: boolean
    cwd?: string
    change?: string
    project?: string
  }): TerminalInfo {
    const size = clampSize(args.cols, args.rows)
    const cwd = args.cwd ?? this.projectDir()
    const project = args.project ?? this.projectDir()
    const spec = this.launch(args.provider, args.launchOpts)
    let proc: PtyProcess
    try {
      proc = this.spawnPty(spec.command, spec.args, {
        name: 'xterm-256color',
        cols: size.cols,
        rows: size.rows,
        cwd,
        env: terminalEnv(spec.env ?? process.env)
      })
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err)
      throw new Error(`could not start ${args.provider}: ${message}`)
    }
    const info: TerminalInfo = {
      id: args.id,
      provider: args.provider,
      title: args.title,
      status: 'running',
      exitCode: null,
      model: args.model ?? null,
      startedAt: args.startedAt
    }
    if (args.change !== undefined) info.change = args.change
    const session: Session = {
      info,
      proc,
      snapshot: args.snapshot,
      pending: '',
      timer: null,
      storeTimer: null,
      cols: args.cols,
      rows: args.rows,
      project,
      cwd,
      exitWaiters: [],
      onProbation: args.onProbation === true,
      relaunching: false,
      removed: false,
      exited: false,
      closedByHost: false,
      logged: false
    }
    this.sessions.push(session)
    this.byId.set(info.id, session)
    this.attach(session)
    if (args.snapshot !== '') this.emitData(info.id, args.snapshot)
    this.emitUpdate(copyInfo(info), false)
    return copyInfo(info)
  }

  private persistOpen(info: TerminalInfo, sessionId: string | null): void {
    if (!this.store) return
    const session = this.byId.get(info.id)
    this.store.saveTerminal({
      id: info.id,
      project: session?.project ?? this.projectDir(),
      provider: info.provider,
      model: info.model,
      session_id: sessionId,
      title: info.title,
      created_at: info.startedAt,
      updated_at: info.startedAt,
      scrollback: '',
      cwd: session?.cwd ?? this.projectDir(),
      change_id: info.change ?? null
    })
  }

  private whenExited(session: Session): Promise<void> {
    if (session.exited) return Promise.resolve()
    return new Promise((resolve) => {
      session.exitWaiters.push(resolve)
    })
  }

  private markExited(session: Session): void {
    session.exited = true
    const waiters = session.exitWaiters.splice(0)
    for (const waiter of waiters) waiter()
  }

  private teardown(session: Session, opts: { keepRow: boolean }): void {
    session.relaunching = false
    if (session.info.status === 'running') {
      session.closedByHost = true
      try {
        this.killTree(session.proc)
      } catch (err) {
        console.error(err)
      }
    }
    if (session.removed) return
    if (!opts.keepRow) this.store?.deleteTerminal(session.info.id)
    session.removed = true
    this.emitUpdate(copyInfo(session.info), true)
    this.forget(session)
  }

  private running(id: string): Session | undefined {
    const session = this.byId.get(id)
    if (!session || session.removed || session.exited || session.info.status !== 'running') {
      return undefined
    }
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
    this.scheduleStoreFlush(session)
    if (session.timer !== null) return
    session.timer = this.setTimeout(() => {
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
    if (this.shouldRelaunch(session, exitCode)) {
      this.beginRelaunch(session, exitCode)
      return
    }
    this.finishExit(session, exitCode)
  }

  private shouldRelaunch(session: Session, exitCode: number): boolean {
    if (session.removed || session.closedByHost || !session.onProbation) return false
    const code = typeof exitCode === 'number' && Number.isFinite(exitCode) ? exitCode : null
    if (code === 0) return false
    return this.now() - session.info.startedAt < PROBATION_MS
  }

  private beginRelaunch(session: Session, exitCode: number): void {
    this.flush(session)
    this.flushStore(session)
    this.markExited(session)
    session.onProbation = false
    session.relaunching = true
    if (session.info.provider === 'cursor') {
      void this.relaunchCursor(session, exitCode)
      return
    }
    this.relaunchClaude(session, exitCode)
  }

  private relaunchClaude(session: Session, exitCode: number): void {
    const sessionId = randomUUID()
    if (
      !this.applyFresh(session, { model: session.info.model ?? undefined, sessionId }, exitCode)
    ) {
      return
    }
    this.updateRowSessionId(session, sessionId)
  }

  private async relaunchCursor(session: Session, exitCode: number): Promise<void> {
    const sessionId = await this.freshSessionId('cursor')
    if (session.removed || !session.relaunching) return
    const launchOpts: LaunchOpts = { model: session.info.model ?? undefined }
    if (sessionId !== null) launchOpts.resume = sessionId
    if (!this.applyFresh(session, launchOpts, exitCode)) return
    this.updateRowSessionId(session, sessionId)
  }

  private applyFresh(session: Session, launchOpts: LaunchOpts, fallbackCode: number): boolean {
    if (session.removed || !session.relaunching) return false
    try {
      this.respawn(session, launchOpts)
    } catch (err) {
      console.error(err)
      session.relaunching = false
      this.finishExit(session, fallbackCode)
      return false
    }
    session.relaunching = false
    session.exited = false
    session.info.status = 'running'
    session.info.exitCode = null
    session.snapshot += FRESH_MARK
    if (session.snapshot.length > SNAPSHOT_MAX) {
      session.snapshot = session.snapshot.slice(-SNAPSHOT_MAX)
    }
    this.emitData(session.info.id, FRESH_MARK)
    this.emitUpdate(copyInfo(session.info), false)
    return true
  }

  private respawn(session: Session, launchOpts: LaunchOpts): void {
    const size = clampSize(session.cols, session.rows)
    const spec = this.launch(session.info.provider, launchOpts)
    session.proc = this.spawnPty(spec.command, spec.args, {
      name: 'xterm-256color',
      cols: size.cols,
      rows: size.rows,
      cwd: session.cwd,
      env: terminalEnv(spec.env ?? process.env)
    })
    this.attach(session)
  }

  private updateRowSessionId(session: Session, sessionId: string | null): void {
    if (!this.store) return
    const row = this.store.terminals(session.project).find((item) => item.id === session.info.id)
    if (!row) return
    this.store.saveTerminal({
      ...row,
      session_id: sessionId,
      updated_at: this.now()
    })
  }

  private finishExit(session: Session, exitCode: number): void {
    this.markExited(session)
    session.relaunching = false
    this.flush(session)
    this.flushStore(session)
    session.info.status = 'exited'
    session.info.exitCode =
      typeof exitCode === 'number' && Number.isFinite(exitCode) ? exitCode : null
    if (!session.removed) this.emitUpdate(copyInfo(session.info), false)
    this.writeRun(session)
    this.forget(session)
  }

  private flush(session: Session): void {
    if (session.timer !== null) {
      this.clearTimeout(session.timer)
      session.timer = null
    }
    if (session.pending === '') return
    const data = session.pending
    session.pending = ''
    this.emitData(session.info.id, data)
  }

  private scheduleStoreFlush(session: Session): void {
    if (!this.store || session.storeTimer !== null) return
    session.storeTimer = this.setTimeout(() => {
      session.storeTimer = null
      try {
        this.flushStore(session)
      } catch (err) {
        console.error(err)
      }
    }, STORE_FLUSH_MS)
  }

  private flushStore(session: Session): void {
    if (session.storeTimer !== null) {
      this.clearTimeout(session.storeTimer)
      session.storeTimer = null
    }
    if (!this.store) return
    const text =
      session.snapshot.length > TERMINAL_SCROLLBACK_CAP
        ? session.snapshot.slice(-TERMINAL_SCROLLBACK_CAP)
        : session.snapshot
    this.store.updateTerminalScrollback(session.info.id, text, this.now())
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

function asProvider(value: string): ProviderId | null {
  if (value === 'claude' || value === 'cursor') return value
  return null
}

function parseSessionId(text: string): string | null {
  const line = text.trim().split(/\r?\n/, 1)[0]?.trim() ?? ''
  if (line.length < 8 || line.length > 128) return null
  if (!/^[A-Za-z0-9][A-Za-z0-9._:-]*$/.test(line)) return null
  return line
}

function withTimeout<T>(
  promise: Promise<T>,
  ms: number,
  startTimer: (fn: () => void, delay: number) => ReturnType<typeof setTimeout>,
  stopTimer: (id: ReturnType<typeof setTimeout>) => void
): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = startTimer(() => {
      reject(new Error('timed out'))
    }, ms)
    promise.then(
      (value) => {
        stopTimer(timer)
        resolve(value)
      },
      (err: unknown) => {
        stopTimer(timer)
        reject(err)
      }
    )
  })
}

// Loaded on first use. The native module ships no Linux build, and tests never start a real terminal.
let nodePty: typeof NodePty | null = null

function loadNodePty(): typeof NodePty {
  nodePty ??= createRequire(import.meta.url)('node-pty') as typeof NodePty
  return nodePty
}

function defaultSpawn(
  command: string,
  args: string[],
  opts: { name: string; cols: number; rows: number; cwd: string; env: Record<string, string> }
): PtyProcess {
  const proc = loadNodePty().spawn(command, args, opts)
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
