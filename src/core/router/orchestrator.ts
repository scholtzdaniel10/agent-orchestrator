import { randomUUID } from 'node:crypto'
import { compareToTested, unknownFormatMessage } from '../providers/versions'
import {
  type StepTool,
  accessOf,
  isJobAccess,
  type AgentEvent,
  type JobAccess,
  type JobType,
  type ProviderAdapter,
  type ProviderId,
  type RouterRules,
  type VersionStatus
} from '../types'
import { JOB_STALE_MS } from '../worktree-guard'
import { Worktrees } from '../worktrees'
import { headroom, pickProvider, pickWithReason, planUsage } from './router'
import type { Store } from './store'

export const JOB_HEARTBEAT_MS = 5_000
export const JOB_WATCH_MS = 2_000

export type JobStatus = 'queued' | 'running' | 'done' | 'failed'

export interface JobStep {
  id: string
  tool: StepTool
  title: string
  detail?: string
  status: 'running' | 'done' | 'failed'
  startedAt: number
  endedAt?: number
  edit?: { path: string; added: number; removed: number }
}

export interface JobRecord {
  id: string
  type: JobType
  prompt: string
  /** Currently assigned provider. */
  provider: ProviderId | null
  status: JobStatus
  /** Assistant text, appended with '\n' between messages. */
  output: string
  /** Providers that hit a limit on this job, in order. */
  failedOver: ProviderId[]
  /** Live tool calls; older jobs load with []. */
  steps?: JobStep[]
  /** How many steps were dropped after the 400 cap. */
  stepsDropped?: number
  /** Model that ran the job, as the CLI reported it. */
  model?: string
  /** Why this plan got the job, e.g. "first choice", "allowance expiring", "chosen". */
  reason?: string
  /** The job may edit files, in its own worktree. */
  edit?: boolean
  /** read, edit, or full. Absent on old saved jobs; then `edit: true` means edit. */
  access?: JobAccess
  /** Id of the job's change set (see `ChangeSet`), once its worktree exists. */
  change?: string
  /** Jobs submitted together to compare plans share this id. */
  group?: string
  /** Id of the lead chat message whose turn created this job. */
  leadMessage?: string
  /** Name of the routine that started this job, when it did. */
  routine?: string
  /** Id of the goal whose round this job belongs to. */
  goal?: string
  /** Branch the job's worktree starts from; unset means the project's HEAD. */
  base?: string
  error?: string
  /** Process that is running this job. Absent on old saved jobs. */
  ownerId?: string
  /** Epoch ms of the owner's last heartbeat. */
  heartbeatAt?: number
  /** Another process asked the owner to stop this job. */
  cancelRequested?: boolean
}

interface InternalJob {
  id: string
  type: JobType
  originalPrompt: string
  prompt: string
  provider: ProviderId | null
  status: JobStatus
  output: string
  failedOver: ProviderId[]
  /** Model the CLI reported for the attempt now running. Cleared on failover. */
  model?: string
  /** Why this plan got the job. */
  reason?: string
  /** The person named the plan. That job is never moved to another one. */
  chosen: boolean
  /** The job may edit files, in its own worktree. */
  edit?: boolean
  access: JobAccess
  /** Id of the job's change set, once its worktree exists. */
  change?: string
  /** Jobs submitted together to compare plans share this id. */
  group?: string
  /** Id of the lead chat message whose turn created this job. */
  leadMessage?: string
  /** Name of the routine that started this job, when it did. */
  routine?: string
  /** Id of the goal whose round this job belongs to. */
  goal?: string
  /** Branch the job's worktree starts from; unset means the project's HEAD. */
  base?: string
  error?: string
  /** Project folder when the job was submitted. */
  project: string
  /** Epoch ms when the job was submitted. */
  createdAt: number
  steps: JobStep[]
  stepsDropped: number
  ownerId?: string
  heartbeatAt?: number
  cancelRequested?: boolean
}

type ResultEvent = Extract<AgentEvent, { kind: 'result' }>
type LimitEvent = Extract<AgentEvent, { kind: 'limit' }>

export type PlanProblem = 'not-installed' | 'signed-out'

export interface WorkerInfo {
  id: ProviderId
  /** Installed and signed in: present in the candidate set. */
  available: boolean
  /** Why the plan is unusable; absent when available. */
  problem?: PlanProblem
  /** Fraction of allowance left, 0..1. */
  headroom: number
  /** Epoch milliseconds, only while that instant is still ahead. */
  restingUntil: number | null
  /** A job is running on this provider. */
  busy: boolean
  /** Jobs running on this provider now. */
  running: number
  /** Jobs waiting in this provider's queue. */
  queued: number
  /** Epoch ms the worst window resets, when known. */
  resetsAt: number | null
  /** A large share of allowance is about to expire unused. */
  atRisk: boolean
  windows: { name: string; used: number; resetsAt: number | null }[]
  /** Token from the CLI's `--version`, or null when unknown. */
  version: string | null
  versionStatus: VersionStatus
}

export class Orchestrator {
  private readonly adapters: ProviderAdapter[]
  private readonly byId: Map<ProviderId, ProviderAdapter>
  private readonly store: Store
  private readonly rules: RouterRules
  private readonly cwd: string | (() => string)
  private readonly worktrees: Worktrees | undefined
  private readonly onChanges: (() => void) | undefined
  private readonly now: () => number
  private readonly modelFor?: (provider: ProviderId) => string | undefined
  private candidates: ProviderId[] = []
  private readonly problems = new Map<ProviderId, PlanProblem>()
  private readonly versions = new Map<ProviderId, string | null>()
  private readonly jobs: InternalJob[] = []
  private readonly byJob = new Map<string, InternalJob>()
  private readonly queues = new Map<ProviderId, string[]>()
  /** Job ids currently executing on each provider. */
  private readonly running = new Map<ProviderId, Set<string>>()
  private readonly handles = new Map<string, ReturnType<ProviderAdapter['run']>>()
  /** Jobs killed because another job on the same plan hit a limit. */
  private readonly abandoning = new Set<string>()
  /** Jobs stopped by cancel(); they fail with error 'stopped' and never fail over. */
  private readonly cancelling = new Set<string>()
  private readonly listeners = new Set<(job: JobRecord) => void>()
  private idleWaiters: Array<() => void> = []
  private readonly stepFlushAt = new Map<string, number>()
  private readonly stepTimers = new Map<string, ReturnType<typeof setTimeout>>()
  readonly ownerId: string
  private readonly startInterval: (fn: () => void, ms: number) => unknown
  private readonly stopInterval: (id: unknown) => void
  private project: string | null = null
  private watching = false
  private heartbeatTimer: unknown
  private watchTimer: unknown

  constructor(opts: {
    adapters: ProviderAdapter[]
    store: Store
    rules: RouterRules
    cwd: string | (() => string)
    now?: () => number
    modelFor?: (provider: ProviderId) => string | undefined
    worktrees?: Worktrees
    onChanges?: () => void
    ownerId?: string
    setInterval?: (fn: () => void, ms: number) => unknown
    clearInterval?: (id: unknown) => void
  }) {
    this.adapters = opts.adapters
    this.byId = new Map(opts.adapters.map((adapter) => [adapter.id, adapter]))
    this.store = opts.store
    this.rules = opts.rules
    this.cwd = opts.cwd
    this.worktrees = opts.worktrees
    this.onChanges = opts.onChanges
    this.now = opts.now ?? ((): number => Date.now())
    this.modelFor = opts.modelFor
    this.ownerId = opts.ownerId ?? randomUUID()
    this.startInterval = opts.setInterval ?? ((fn, ms) => setInterval(fn, ms))
    this.stopInterval =
      opts.clearInterval ?? ((id) => clearInterval(id as ReturnType<typeof setInterval>))
  }

  /** Providers that are installed and signed in become the candidate set. */
  async init(): Promise<void> {
    await this.recheck()
  }

  /**
   * Re-run installed/signed-in checks. Only adds or removes candidates; running
   * and queued jobs are left alone.
   */
  async recheck(): Promise<void> {
    const available: ProviderId[] = []
    this.problems.clear()
    this.versions.clear()
    for (const adapter of this.adapters) {
      const installed = await adapter.isInstalled()
      this.versions.set(adapter.id, await adapter.version())
      if (!installed) {
        this.problems.set(adapter.id, 'not-installed')
        continue
      }
      if (!(await adapter.isSignedIn())) {
        this.problems.set(adapter.id, 'signed-out')
        continue
      }
      available.push(adapter.id)
    }
    this.candidates = available
  }

  submit(
    type: JobType,
    prompt: string,
    provider?: ProviderId,
    access?: JobAccess | boolean,
    extra?: {
      group?: string
      leadMessage?: string
      routine?: string
      goal?: string
      base?: string
      /** The project the job belongs to; unset means the open project. */
      project?: string
    }
  ): JobRecord {
    const granted = submitAccess(access)
    const job: InternalJob = {
      id: randomUUID(),
      type,
      originalPrompt: prompt,
      prompt,
      provider: null,
      status: 'queued',
      output: '',
      failedOver: [],
      chosen: false,
      project: extra?.project ?? this.workingDir(),
      createdAt: this.now(),
      access: granted,
      steps: [],
      stepsDropped: 0,
      ownerId: this.ownerId,
      heartbeatAt: this.now(),
      ...(granted !== 'read' ? { edit: true } : {}),
      ...(extra?.group !== undefined ? { group: extra.group } : {}),
      ...(extra?.leadMessage !== undefined ? { leadMessage: extra.leadMessage } : {}),
      ...(extra?.routine !== undefined ? { routine: extra.routine } : {}),
      ...(extra?.goal !== undefined ? { goal: extra.goal } : {}),
      ...(extra?.base !== undefined ? { base: extra.base } : {})
    }
    this.jobs.push(job)
    this.byJob.set(job.id, job)

    if (provider !== undefined) {
      job.provider = provider
      if (!this.candidates.includes(provider)) {
        return this.failNow(job, `${provider} is not signed in`)
      }
      const until = this.store.restingUntil(provider)
      if (until !== null && until > this.now()) {
        return this.failNow(job, `${provider} is resting`)
      }
      job.chosen = true
      job.reason = 'chosen'
      this.enqueue(provider, job.id)
      this.emitStatus(job)
      this.pump(provider)
      return copy(job)
    }

    const pick = pickWithReason(
      type,
      this.candidates,
      this.store,
      this.rules,
      this.now(),
      [],
      this.runningCounts()
    )
    if (!pick) return this.failNow(job, 'no provider available')

    job.provider = pick.provider
    job.reason = pick.reason
    this.enqueue(pick.provider, job.id)
    this.emitStatus(job)
    this.pump(pick.provider)
    return copy(job)
  }

  /**
   * Switch to `project`: load its newest stored jobs. Jobs this process is still running or has
   * queued, in any project, stay and keep running in their own folders.
   */
  restore(project: string): void {
    this.project = project
    const active = this.jobs.filter(
      (job) => this.isOurs(job) && (job.status === 'queued' || job.status === 'running')
    )
    this.jobs.length = 0
    this.byJob.clear()
    for (const job of active) {
      this.jobs.push(job)
      this.byJob.set(job.id, job)
    }
    if (active.length === 0) {
      this.queues.clear()
      this.running.clear()
    }
    for (const record of this.store.jobs(project, 200)) {
      if (this.byJob.has(record.id)) continue
      const job = fromRecord(record, project, this.now())
      if (this.shouldInterrupt(job)) {
        job.status = 'failed'
        job.error = 'interrupted when the app closed'
        this.persist(job)
      }
      this.jobs.push(job)
      this.byJob.set(job.id, job)
    }
    this.syncTimers()
  }

  /**
   * Watch the store for jobs another process starts on the current project.
   * The app calls this while a window is open; one-shot commands do not.
   */
  watchStore(): () => void {
    this.watching = true
    if (this.project === null) this.project = this.workingDir()
    this.syncTimers()
    return (): void => {
      this.watching = false
      this.syncTimers()
    }
  }

  /** Jobs of the current project; jobs still running in another project are left out. */
  list(): JobRecord[] {
    const project = this.currentProject()
    return this.jobs.filter((job) => job.project === project).map((job) => copy(job))
  }

  /** Jobs queued or running in `project`, including ones left running after a switch. */
  activeIn(project: string): number {
    return this.jobs.filter(
      (job) => job.project === project && (job.status === 'queued' || job.status === 'running')
    ).length
  }

  /** The project folder a job belongs to, or null for an unknown id. */
  projectOf(id: string): string | null {
    return this.byJob.get(id)?.project ?? null
  }

  private currentProject(): string {
    return this.project ?? this.workingDir()
  }

  get(id: string): JobRecord | null {
    const job = this.byJob.get(id)
    if (!job) return null
    return copy(job)
  }

  workers(): WorkerInfo[] {
    const now = this.now()
    return this.adapters.map((adapter) => {
      const until = this.store.restingUntil(adapter.id)
      const queue = this.queues.get(adapter.id)
      const usage = planUsage(adapter.id, this.store, this.rules, now)
      const problem = this.problems.get(adapter.id)
      const version = this.versions.get(adapter.id) ?? null
      const info: WorkerInfo = {
        id: adapter.id,
        available: this.candidates.includes(adapter.id),
        headroom: headroom(adapter.id, this.store, this.rules, now),
        restingUntil: until !== null && until > now ? until : null,
        busy: this.runningCount(adapter.id) > 0,
        running: this.runningCount(adapter.id),
        queued: queue?.length ?? 0,
        resetsAt: usage.resetsAt,
        atRisk: usage.atRisk,
        windows: usage.windows.map((window) => ({
          name: window.name,
          used: window.used,
          resetsAt: window.resetsAt
        })),
        version,
        versionStatus: compareToTested(adapter.id, version)
      }
      if (problem !== undefined) info.problem = problem
      return info
    })
  }

  /** Stop a queued or running job. Finished or unknown ids throw. */
  cancel(id: string): JobRecord {
    const job = this.byJob.get(id)
    if (!job || (job.status !== 'queued' && job.status !== 'running')) {
      throw new Error('job is not running')
    }
    if (this.isLiveForeign(job)) {
      this.store.requestCancel(job.id)
      job.cancelRequested = true
      return copy(job)
    }
    if (job.status === 'queued') {
      const provider = job.provider
      if (provider !== null) {
        const queue = this.queues.get(provider)
        if (queue) {
          const index = queue.indexOf(id)
          if (index >= 0) queue.splice(index, 1)
        }
      }
      job.status = 'failed'
      job.error = 'stopped'
      this.emitStatus(job)
      this.resolveIdle()
      return copy(job)
    }
    this.cancelling.add(id)
    safeKill(this.handles.get(id))
    return copy(job)
  }

  onUpdate(cb: (job: JobRecord) => void): () => void {
    this.listeners.add(cb)
    return (): void => {
      this.listeners.delete(cb)
    }
  }

  idle(): Promise<void> {
    if (!this.busy()) return Promise.resolve()
    return new Promise((resolve) => {
      this.idleWaiters.push(resolve)
    })
  }

  busy(): boolean {
    return this.jobs.some(
      (job) => this.isOurs(job) && (job.status === 'queued' || job.status === 'running')
    )
  }

  private resolveIdle(): void {
    if (this.busy()) return
    const waiters = this.idleWaiters.splice(0)
    for (const waiter of waiters) waiter()
  }

  private enqueue(provider: ProviderId, id: string): void {
    const queue = this.queues.get(provider)
    if (queue) queue.push(id)
    else this.queues.set(provider, [id])
  }

  private runningCount(provider: ProviderId): number {
    return this.running.get(provider)?.size ?? 0
  }

  private runningCounts(): Partial<Record<ProviderId, number>> {
    const counts: Partial<Record<ProviderId, number>> = {}
    for (const [provider, ids] of this.running) {
      counts[provider] = ids.size
    }
    return counts
  }

  private addRunning(provider: ProviderId, id: string): void {
    const set = this.running.get(provider)
    if (set) set.add(id)
    else this.running.set(provider, new Set([id]))
  }

  private removeRunning(provider: ProviderId, id: string): void {
    const set = this.running.get(provider)
    if (!set) return
    set.delete(id)
    if (set.size === 0) this.running.delete(provider)
  }

  /** Start queued jobs for `provider` while it has free parallel slots. */
  private pump(provider: ProviderId): void {
    const limit = this.rules.maxParallel
    while (this.runningCount(provider) < limit) {
      const queue = this.queues.get(provider)
      if (!queue?.length) return
      const id = queue.shift()
      if (id === undefined) return
      const job = this.byJob.get(id)
      if (!job || job.status !== 'queued') continue
      this.addRunning(provider, id)
      job.status = 'running'
      job.provider = provider
      this.emitStatus(job)
      void this.execute(job, provider)
        .catch((err: unknown) => {
          if (job.status === 'running') {
            job.status = 'failed'
            job.error = err instanceof Error ? err.message : String(err)
            this.emitStatus(job)
          }
        })
        .finally(() => {
          this.removeRunning(provider, id)
          this.handles.delete(id)
          this.abandoning.delete(id)
          this.cancelling.delete(id)
          this.pump(provider)
          this.resolveIdle()
        })
    }
  }

  private workingDir(): string {
    return typeof this.cwd === 'function' ? this.cwd() : this.cwd
  }

  private async editFolder(job: InternalJob, project: string): Promise<string | null> {
    try {
      if (!this.worktrees) throw new Error('no worktrees')
      const created = await this.worktrees.create(project, job.id, job.base)
      job.change = created.id
      this.emit(job)
      return created.path
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err)
      job.status = 'failed'
      job.error = `cannot edit here: ${message}`
      this.emitStatus(job)
      return null
    }
  }

  private async execute(job: InternalJob, provider: ProviderId): Promise<void> {
    const startedAt = this.now()
    const t0 = Date.now()
    let utilization: number | null = null
    let result: ResultEvent | null = null
    let limitEv: LimitEvent | null = null
    let handle: ReturnType<ProviderAdapter['run']> | undefined
    let started = false
    try {
      // The job's own project: the person may have switched to another one since it was queued.
      const dir = job.project
      let runCwd = dir
      if (job.edit) {
        const folder = await this.editFolder(job, dir)
        if (!folder) return
        runCwd = folder
      }
      if (this.finishCancelled(job, provider, startedAt, Date.now() - t0, utilization, result)) {
        return
      }
      const adapter = this.byId.get(provider)
      if (!adapter) throw new Error(`no adapter for ${provider}`)
      started = true
      handle = adapter.run({ id: job.id, prompt: job.prompt }, runCwd, {
        model: this.modelFor?.(provider),
        access: job.access,
        ...(job.edit ? { edit: true } : {})
      })
      this.handles.set(job.id, handle)
      if (this.cancelling.has(job.id)) safeKill(handle)
      for await (const event of handle.events) {
        if (this.abandoning.has(job.id) || this.cancelling.has(job.id)) break
        if (event.kind === 'init') {
          if (event.model !== undefined) {
            job.model = event.model
            this.emit(job)
          }
        } else if (event.kind === 'text') {
          this.appendText(job, event.text)
        } else if (event.kind === 'usage') {
          utilization = event.utilization
          if (event.windows) {
            this.store.saveWindows(
              provider,
              event.windows.map((window) => ({
                name: window.name,
                utilization: window.utilization,
                resetsAt: window.resetsAt === undefined ? null : window.resetsAt * 1000
              })),
              this.now()
            )
          }
        } else if (event.kind === 'result') {
          result = event
          if (event.text !== '' && job.output === '') {
            job.output = event.text
            this.emit(job)
          }
        } else if (event.kind === 'step') {
          this.applyStep(job, event)
        } else if (event.kind === 'limit') {
          limitEv = event
          break
        }
      }
      // Kill before waiting for exit so a limit or cancel stops the process.
      if (limitEv || this.abandoning.has(job.id) || this.cancelling.has(job.id)) safeKill(handle)
      let exit: { code: number | null; stderr: string; unreadable?: boolean }
      try {
        exit = await handle.exit
      } catch (err) {
        if (!limitEv && !this.abandoning.has(job.id) && !this.cancelling.has(job.id)) throw err
        const message = err instanceof Error ? err.message : String(err)
        exit = { code: null, stderr: message }
      }
      const duration = Date.now() - t0
      if (this.finishCancelled(job, provider, startedAt, duration, utilization, result)) return
      if (this.finishAbandoned(job, provider)) return
      const limited = limitEv !== null || (exit.code !== 0 && adapter.isLimitError(exit.stderr))
      if (limited) {
        if (!limitEv) safeKill(handle)
        this.applyLimit(job, provider, limitEv, startedAt, duration, utilization, result)
        return
      }
      if (exit.unreadable === true) {
        this.insertRun(job, provider, startedAt, duration, utilization, result, 'error')
        job.status = 'failed'
        job.error = unknownFormatMessage(provider, this.versions.get(provider) ?? null)
        this.emitStatus(job)
        return
      }
      this.complete(job, provider, result, exit.stderr, startedAt, duration, utilization)
    } catch (err) {
      if (this.finishCancelled(job, provider, startedAt, Date.now() - t0, utilization, result)) {
        return
      }
      if (this.finishAbandoned(job, provider)) return
      safeKill(handle)
      this.failThrown(job, provider, err, startedAt, Date.now() - t0, utilization, result)
    } finally {
      if (started && job.edit) this.onChanges?.()
    }
  }

  /** Finish a job that was stopped by cancel(). */
  private finishCancelled(
    job: InternalJob,
    provider: ProviderId,
    startedAt: number,
    durationMs: number,
    utilization: number | null,
    result: ResultEvent | null
  ): boolean {
    if (!this.cancelling.delete(job.id)) return false
    if (job.status !== 'running') return true
    this.insertRun(job, provider, startedAt, durationMs, utilization, result, 'error')
    job.status = 'failed'
    job.error = 'stopped'
    this.emitStatus(job)
    return true
  }

  /** Finish a job that was killed because its plan hit a limit. */
  private finishAbandoned(job: InternalJob, provider: ProviderId): boolean {
    if (!this.abandoning.delete(job.id)) return false
    if (job.status !== 'running') return true
    if (job.chosen) {
      job.status = 'failed'
      job.error = `${provider} hit its usage limit`
      this.emitStatus(job)
      return true
    }
    this.requeue(job, provider, true, job.output)
    return true
  }

  private appendText(job: InternalJob, text: string): void {
    job.output = job.output === '' ? text : `${job.output}\n${text}`
    this.emit(job)
  }

  private complete(
    job: InternalJob,
    provider: ProviderId,
    result: ResultEvent | null,
    stderr: string,
    startedAt: number,
    durationMs: number,
    utilization: number | null
  ): void {
    const ok = result?.ok === true
    this.insertRun(job, provider, startedAt, durationMs, utilization, result, ok ? 'ok' : 'error')
    if (ok) {
      job.status = 'done'
      delete job.error
    } else {
      job.status = 'failed'
      job.error = failureMessage(result?.text, stderr)
    }
    this.emitStatus(job)
  }

  private applyLimit(
    job: InternalJob,
    provider: ProviderId,
    limitEv: LimitEvent | null,
    startedAt: number,
    durationMs: number,
    utilization: number | null,
    result: ResultEvent | null
  ): void {
    const now = this.now()
    const until = limitEv?.resetsAt
      ? limitEv.resetsAt * 1000
      : now + this.rules.defaultRestHours * 3600_000
    this.store.setResting(provider, until)
    this.insertRun(job, provider, startedAt, durationMs, utilization, result, 'limit')
    this.abandonOthers(provider, job.id)
    if (job.chosen) {
      job.status = 'failed'
      job.error = `${provider} hit its usage limit`
      this.emitStatus(job)
      this.rerouteQueued(provider)
      return
    }
    const output = job.output
    this.requeue(job, provider, true, output)
    this.rerouteQueued(provider)
  }

  /** Kill every other running job on a plan that just hit its limit. */
  private abandonOthers(provider: ProviderId, exceptId: string): void {
    const ids = this.running.get(provider)
    if (!ids) return
    for (const id of ids) {
      if (id === exceptId) continue
      this.abandoning.add(id)
      safeKill(this.handles.get(id))
    }
  }

  /** Move jobs still queued on a provider that just went to rest. */
  private rerouteQueued(provider: ProviderId): void {
    const waiting = this.queues.get(provider) ?? []
    this.queues.set(provider, [])
    for (const id of waiting) {
      const job = this.byJob.get(id)
      if (!job || job.status !== 'queued' || job.provider !== provider) continue
      if (job.chosen) {
        job.status = 'failed'
        job.error = `${provider} hit its usage limit`
        this.emitStatus(job)
        continue
      }
      this.requeue(job, provider, false, '')
    }
  }

  /**
   * Pick a new provider. `recordFailure` pushes `from` onto `failedOver` (the
   * job that actually hit the limit). Queued siblings are re-picked without that.
   * The handoff block is built from the original prompt so a later failover
   * does not nest earlier handoffs.
   */
  private requeue(
    job: InternalJob,
    from: ProviderId,
    recordFailure: boolean,
    output: string
  ): void {
    if (recordFailure) job.failedOver.push(from)
    const next = pickProvider(
      job.type,
      this.candidates,
      this.store,
      this.rules,
      this.now(),
      job.failedOver,
      this.runningCounts()
    )
    if (!next) {
      job.status = 'failed'
      job.provider = null
      job.error = 'all providers are at their limit'
      this.emitStatus(job)
      return
    }
    if (recordFailure) {
      job.prompt = job.originalPrompt
      if (output !== '') {
        const tail = output.slice(-2000)
        job.prompt +=
          `\n\n[Handoff: a previous attempt on ${from} stopped at its usage limit. Its output so far:]\n` +
          tail
      }
      if (job.edit) {
        const note = 'The files you need are already changed in this folder; continue from them.'
        job.prompt += output === '' ? `\n\n${note}` : `\n${note}`
      }
    }
    job.output = ''
    job.status = 'queued'
    job.provider = next
    job.reason = 'failover'
    delete job.model
    delete job.error
    this.enqueue(next, job.id)
    this.emitStatus(job)
    this.pump(next)
  }

  private failThrown(
    job: InternalJob,
    provider: ProviderId,
    err: unknown,
    startedAt: number,
    durationMs: number,
    utilization: number | null,
    result: ResultEvent | null
  ): void {
    this.insertRun(job, provider, startedAt, durationMs, utilization, result, 'error')
    job.status = 'failed'
    job.error = err instanceof Error ? err.message : String(err)
    this.emitStatus(job)
  }

  private insertRun(
    job: InternalJob,
    provider: ProviderId,
    startedAt: number,
    durationMs: number,
    utilization: number | null,
    result: ResultEvent | null,
    outcome: 'ok' | 'error' | 'limit'
  ): void {
    this.store.insertRun({
      job_id: job.id,
      provider,
      job_type: job.type,
      started_at: startedAt,
      duration_ms: durationMs,
      cost_usd: result?.costUsd ?? null,
      tokens: result?.tokens ?? null,
      utilization,
      outcome,
      session_id: result?.sessionId ?? null
    })
  }

  private failNow(job: InternalJob, error: string): JobRecord {
    job.status = 'failed'
    job.error = error
    this.emitStatus(job)
    this.resolveIdle()
    return copy(job)
  }

  private emitStatus(job: InternalJob): void {
    this.clearStepTimer(job.id)
    if (job.status !== 'running') this.closeRunningSteps(job, job.status === 'done')
    this.persist(job)
    this.emit(job)
    this.syncTimers()
  }

  private applyStep(job: InternalJob, event: Extract<AgentEvent, { kind: 'step' }>): void {
    const now = this.now()
    if (event.phase === 'start') {
      const step: JobStep = {
        id: event.id,
        tool: event.tool,
        title: event.title,
        status: 'running',
        startedAt: now
      }
      if (event.detail !== undefined) step.detail = event.detail
      if (event.edit !== undefined) step.edit = { ...event.edit }
      this.pushStep(job, step)
    } else {
      const existing = job.steps.find((step) => step.id === event.id)
      if (existing !== undefined) {
        existing.title = event.title
        existing.tool = event.tool
        existing.status = event.ok === false ? 'failed' : 'done'
        existing.endedAt = now
        if (event.detail !== undefined) existing.detail = event.detail
        if (event.edit !== undefined) existing.edit = { ...event.edit }
      } else if (event.tool === 'think' && this.extendThinking(job, event, now)) {
        // Joined onto the thinking step before it.
      } else {
        const step: JobStep = {
          id: event.id,
          tool: event.tool,
          title: event.title,
          status: event.ok === false ? 'failed' : 'done',
          startedAt: now,
          endedAt: now
        }
        if (event.detail !== undefined) step.detail = event.detail
        if (event.edit !== undefined) step.edit = { ...event.edit }
        this.pushStep(job, step)
      }
    }
    this.pushStepUpdate(job)
  }

  /**
   * A CLI streams its thinking a few words at a time. Shown as they arrive, one thought becomes
   * a dozen steps, so a fragment joins the thinking step right before it.
   */
  private extendThinking(
    job: InternalJob,
    event: Extract<AgentEvent, { kind: 'step' }>,
    now: number
  ): boolean {
    const last = job.steps[job.steps.length - 1]
    if (last === undefined || last.tool !== 'think' || last.status !== 'done') return false
    const text = `${last.detail ?? last.title}${event.detail ?? event.title}`.slice(-4000)
    const firstLine = text.split(/\r?\n/).find((line) => line.trim() !== '') ?? ''
    last.detail = text
    last.title = firstLine.trim().slice(0, 120)
    last.endedAt = now
    return true
  }

  private pushStep(job: InternalJob, step: JobStep): void {
    job.steps.push(step)
    while (job.steps.length > 400) {
      job.steps.shift()
      job.stepsDropped += 1
    }
  }

  private pushStepUpdate(job: InternalJob): void {
    const now = this.now()
    const last = this.stepFlushAt.get(job.id) ?? 0
    if (now - last >= 200) {
      this.stepFlushAt.set(job.id, now)
      this.persist(job)
      this.emit(job)
      return
    }
    if (this.stepTimers.has(job.id)) return
    const wait = Math.max(0, 200 - (now - last))
    const timer = setTimeout(() => {
      this.stepTimers.delete(job.id)
      this.stepFlushAt.set(job.id, this.now())
      this.persist(job)
      this.emit(job)
    }, wait)
    this.stepTimers.set(job.id, timer)
  }

  private clearStepTimer(id: string): void {
    const timer = this.stepTimers.get(id)
    if (timer !== undefined) {
      clearTimeout(timer)
      this.stepTimers.delete(id)
    }
    this.stepFlushAt.delete(id)
  }

  private closeRunningSteps(job: InternalJob, ok: boolean): void {
    const now = this.now()
    for (const step of job.steps) {
      if (step.status !== 'running') continue
      step.status = ok ? 'done' : 'failed'
      step.endedAt = now
    }
  }

  private persist(job: InternalJob): void {
    if (this.isLiveForeign(job)) return
    if (job.ownerId === undefined || job.ownerId === this.ownerId) {
      job.ownerId = this.ownerId
      job.heartbeatAt = this.now()
    }
    try {
      this.store.saveJob(job.project, job.createdAt, copy(job))
    } catch (err) {
      console.error(err)
    }
  }

  private emit(job: InternalJob): void {
    const snap = copy(job)
    for (const cb of this.listeners) cb(snap)
  }

  private isOurs(job: InternalJob): boolean {
    return job.ownerId === this.ownerId
  }

  private isLiveForeign(job: InternalJob): boolean {
    if (job.ownerId === undefined || job.ownerId === this.ownerId) return false
    if (job.heartbeatAt === undefined) return false
    return this.now() - job.heartbeatAt < JOB_STALE_MS
  }

  private shouldInterrupt(job: InternalJob): boolean {
    if (job.status !== 'queued' && job.status !== 'running') return false
    if (job.ownerId === undefined || job.ownerId === this.ownerId) return true
    if (job.heartbeatAt === undefined) return true
    return this.now() - job.heartbeatAt >= JOB_STALE_MS
  }

  private hasOwnedActive(): boolean {
    return this.jobs.some(
      (job) => this.isOurs(job) && (job.status === 'queued' || job.status === 'running')
    )
  }

  private hasForeignUnfinished(): boolean {
    return this.jobs.some(
      (job) =>
        job.ownerId !== undefined &&
        job.ownerId !== this.ownerId &&
        (job.status === 'queued' || job.status === 'running')
    )
  }

  private syncTimers(): void {
    const needBeat = this.hasOwnedActive()
    if (needBeat && this.heartbeatTimer === undefined) {
      this.heartbeatTimer = this.startInterval(() => this.onHeartbeat(), JOB_HEARTBEAT_MS)
    } else if (!needBeat && this.heartbeatTimer !== undefined) {
      this.stopInterval(this.heartbeatTimer)
      this.heartbeatTimer = undefined
    }
    const needWatch = this.watching || this.hasForeignUnfinished()
    if (needWatch && this.watchTimer === undefined) {
      this.watchTimer = this.startInterval(() => this.pollStore(), JOB_WATCH_MS)
    } else if (!needWatch && this.watchTimer !== undefined) {
      this.stopInterval(this.watchTimer)
      this.watchTimer = undefined
    }
  }

  private onHeartbeat(): void {
    for (const job of this.jobs) {
      if (!this.isOurs(job)) continue
      if (job.status !== 'queued' && job.status !== 'running') continue
      if (this.store.isCancelRequested(job.id)) {
        this.cancel(job.id)
        continue
      }
      this.persist(job)
    }
    this.syncTimers()
  }

  private pollStore(): void {
    const project = this.project
    if (project === null) return
    for (const record of this.store.jobs(project, 200)) {
      const existing = this.byJob.get(record.id)
      if (existing === undefined) {
        if (!this.watching) continue
        const job = fromRecord(record, project, this.now())
        this.jobs.push(job)
        this.byJob.set(job.id, job)
        if (this.shouldInterrupt(job)) {
          job.status = 'failed'
          job.error = 'interrupted when the app closed'
          this.emitStatus(job)
        } else {
          this.emit(job)
        }
        continue
      }
      if (this.isOurs(existing)) continue
      const changed = !sameSnapshot(existing, record)
      adopt(existing, record)
      if (this.shouldInterrupt(existing)) {
        existing.status = 'failed'
        existing.error = 'interrupted when the app closed'
        this.emitStatus(existing)
        continue
      }
      if (changed) this.emit(existing)
    }
    this.syncTimers()
  }
}

function sameSnapshot(job: InternalJob, record: JobRecord): boolean {
  return (
    job.status === record.status &&
    job.output === record.output &&
    job.error === record.error &&
    job.provider === record.provider &&
    job.model === record.model &&
    job.change === record.change &&
    JSON.stringify(job.steps) === JSON.stringify(record.steps ?? [])
  )
}

function adopt(job: InternalJob, record: JobRecord): void {
  job.status = record.status
  job.output = record.output
  job.provider = record.provider
  job.failedOver = [...record.failedOver]
  job.steps = stepsFrom(record)
  job.stepsDropped = typeof record.stepsDropped === 'number' ? record.stepsDropped : 0
  if (record.error !== undefined) job.error = record.error
  else delete job.error
  if (record.model !== undefined) job.model = record.model
  else delete job.model
  if (record.reason !== undefined) job.reason = record.reason
  if (record.change !== undefined) job.change = record.change
  if (record.ownerId !== undefined) job.ownerId = record.ownerId
  if (record.heartbeatAt !== undefined) job.heartbeatAt = record.heartbeatAt
  if (record.cancelRequested === true) job.cancelRequested = true
}

function safeKill(handle: { kill(): void } | undefined): void {
  if (!handle) return
  try {
    handle.kill()
  } catch (err) {
    console.error(err)
  }
}

function copy(job: InternalJob): JobRecord {
  const record: JobRecord = {
    id: job.id,
    type: job.type,
    prompt: job.prompt,
    provider: job.provider,
    status: job.status,
    output: job.output,
    failedOver: [...job.failedOver],
    steps: job.steps.map(copyStep),
    stepsDropped: job.stepsDropped
  }
  if (job.error !== undefined) record.error = job.error
  if (job.model !== undefined) record.model = job.model
  if (job.reason !== undefined) record.reason = job.reason
  record.access = job.access
  if (job.access !== 'read') record.edit = true
  if (job.change !== undefined) record.change = job.change
  if (job.group !== undefined) record.group = job.group
  if (job.leadMessage !== undefined) record.leadMessage = job.leadMessage
  if (job.routine !== undefined) record.routine = job.routine
  if (job.goal !== undefined) record.goal = job.goal
  if (job.base !== undefined) record.base = job.base
  if (job.ownerId !== undefined) record.ownerId = job.ownerId
  if (job.heartbeatAt !== undefined) record.heartbeatAt = job.heartbeatAt
  if (job.cancelRequested === true) record.cancelRequested = true
  return record
}

function copyStep(step: JobStep): JobStep {
  const copied: JobStep = {
    id: step.id,
    tool: step.tool,
    title: step.title,
    status: step.status,
    startedAt: step.startedAt
  }
  if (step.detail !== undefined) copied.detail = step.detail
  if (step.endedAt !== undefined) copied.endedAt = step.endedAt
  if (step.edit !== undefined) copied.edit = { ...step.edit }
  return copied
}

function fromRecord(record: JobRecord, project: string, createdAt: number): InternalJob {
  const access = accessOf(record)
  return {
    id: record.id,
    type: record.type,
    originalPrompt: record.prompt,
    prompt: record.prompt,
    provider: record.provider,
    status: record.status,
    output: record.output,
    failedOver: [...record.failedOver],
    chosen: record.reason === 'chosen',
    project,
    createdAt,
    access,
    steps: stepsFrom(record),
    stepsDropped: typeof record.stepsDropped === 'number' ? record.stepsDropped : 0,
    ...(record.error !== undefined ? { error: record.error } : {}),
    ...(record.model !== undefined ? { model: record.model } : {}),
    ...(record.reason !== undefined ? { reason: record.reason } : {}),
    ...(access !== 'read' ? { edit: true } : {}),
    ...(record.change !== undefined ? { change: record.change } : {}),
    ...(record.group !== undefined ? { group: record.group } : {}),
    ...(record.leadMessage !== undefined ? { leadMessage: record.leadMessage } : {}),
    ...(record.routine !== undefined ? { routine: record.routine } : {}),
    ...(record.goal !== undefined ? { goal: record.goal } : {}),
    ...(record.base !== undefined ? { base: record.base } : {}),
    ...(record.ownerId !== undefined ? { ownerId: record.ownerId } : {}),
    ...(record.heartbeatAt !== undefined ? { heartbeatAt: record.heartbeatAt } : {}),
    ...(record.cancelRequested === true ? { cancelRequested: true } : {})
  }
}

function submitAccess(value: JobAccess | boolean | undefined): JobAccess {
  if (value === true) return 'edit'
  if (value === false || value === undefined) return 'read'
  if (isJobAccess(value)) return value
  return 'read'
}

function stepsFrom(record: JobRecord): JobStep[] {
  if (!Array.isArray(record.steps)) return []
  const steps: JobStep[] = []
  for (const item of record.steps) {
    if (!isJobStep(item)) continue
    steps.push(copyStep(item))
  }
  return steps
}

function isJobStep(value: unknown): value is JobStep {
  if (typeof value !== 'object' || value === null) return false
  const step = value as JobStep
  if (typeof step.id !== 'string' || typeof step.title !== 'string') return false
  if (typeof step.startedAt !== 'number') return false
  if (step.status !== 'running' && step.status !== 'done' && step.status !== 'failed') {
    return false
  }
  return typeof step.tool === 'string'
}

function failureMessage(resultText: string | undefined, stderr: string): string {
  if (resultText) return resultText
  const trimmed = stderr.trim()
  const unknownModel = /unrecognized_model\]\s*\{"model":"([^"]{1,80})"/.exec(trimmed)
  if (unknownModel) {
    return `This CLI version does not know the model ${unknownModel[1]}. Update the CLI, or pick another model in Usage.`
  }
  if (trimmed.length <= 500) return trimmed
  return trimmed.slice(-500)
}
