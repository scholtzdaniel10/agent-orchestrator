import { randomUUID } from 'node:crypto'
import type { AgentEvent, JobType, ProviderAdapter, ProviderId, RouterRules } from '../types'
import { pickProvider } from './router'
import type { Store } from './store'

export type JobStatus = 'queued' | 'running' | 'done' | 'failed'

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
  error?: string
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
  error?: string
}

type ResultEvent = Extract<AgentEvent, { kind: 'result' }>
type LimitEvent = Extract<AgentEvent, { kind: 'limit' }>

export class Orchestrator {
  private readonly adapters: ProviderAdapter[]
  private readonly byId: Map<ProviderId, ProviderAdapter>
  private readonly store: Store
  private readonly rules: RouterRules
  private readonly cwd: string
  private readonly now: () => number
  private candidates: ProviderId[] = []
  private readonly jobs: InternalJob[] = []
  private readonly byJob = new Map<string, InternalJob>()
  private readonly queues = new Map<ProviderId, string[]>()
  private readonly running = new Set<ProviderId>()
  private readonly listeners = new Set<(job: JobRecord) => void>()
  private idleWaiters: Array<() => void> = []

  constructor(opts: {
    adapters: ProviderAdapter[]
    store: Store
    rules: RouterRules
    cwd: string
    now?: () => number
  }) {
    this.adapters = opts.adapters
    this.byId = new Map(opts.adapters.map((adapter) => [adapter.id, adapter]))
    this.store = opts.store
    this.rules = opts.rules
    this.cwd = opts.cwd
    this.now = opts.now ?? ((): number => Date.now())
  }

  /** Providers that are installed and signed in become the candidate set. */
  async init(): Promise<void> {
    const available: ProviderId[] = []
    for (const adapter of this.adapters) {
      if ((await adapter.isInstalled()) && (await adapter.isSignedIn())) {
        available.push(adapter.id)
      }
    }
    this.candidates = available
  }

  submit(type: JobType, prompt: string): JobRecord {
    const job: InternalJob = {
      id: randomUUID(),
      type,
      originalPrompt: prompt,
      prompt,
      provider: null,
      status: 'queued',
      output: '',
      failedOver: []
    }
    this.jobs.push(job)
    this.byJob.set(job.id, job)

    const provider = pickProvider(type, this.candidates, this.store, this.rules, this.now())
    if (!provider) {
      job.status = 'failed'
      job.error = 'no provider available'
      this.emit(job)
      this.resolveIdle()
      return copy(job)
    }

    job.provider = provider
    this.enqueue(provider, job.id)
    this.emit(job)
    this.pump(provider)
    return copy(job)
  }

  list(): JobRecord[] {
    return this.jobs.map((job) => copy(job))
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

  private busy(): boolean {
    return this.jobs.some((job) => job.status === 'queued' || job.status === 'running')
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

  /** Start the next queued job for `provider` when it is free. */
  private pump(provider: ProviderId): void {
    if (this.running.has(provider)) return
    const queue = this.queues.get(provider)
    if (!queue?.length) return
    const id = queue.shift()
    if (id === undefined) return
    const job = this.byJob.get(id)
    if (!job || job.status !== 'queued') {
      this.pump(provider)
      return
    }
    this.running.add(provider)
    job.status = 'running'
    job.provider = provider
    this.emit(job)
    void this.execute(job, provider)
      .catch((err: unknown) => {
        if (job.status === 'running') {
          job.status = 'failed'
          job.error = err instanceof Error ? err.message : String(err)
          this.emit(job)
        }
      })
      .finally(() => {
        this.running.delete(provider)
        this.pump(provider)
        this.resolveIdle()
      })
  }

  private async execute(job: InternalJob, provider: ProviderId): Promise<void> {
    const startedAt = this.now()
    const t0 = Date.now()
    let utilization: number | null = null
    let result: ResultEvent | null = null
    let limitEv: LimitEvent | null = null
    let handle: ReturnType<ProviderAdapter['run']> | undefined
    try {
      const adapter = this.byId.get(provider)
      if (!adapter) throw new Error(`no adapter for ${provider}`)
      handle = adapter.run({ id: job.id, prompt: job.prompt }, this.cwd)
      for await (const event of handle.events) {
        if (event.kind === 'text') {
          this.appendText(job, event.text)
        } else if (event.kind === 'usage') {
          utilization = event.utilization
        } else if (event.kind === 'result') {
          result = event
          if (event.text !== '' && job.output === '') {
            job.output = event.text
            this.emit(job)
          }
        } else if (event.kind === 'limit') {
          limitEv = event
          break
        }
      }
      // Kill before waiting for exit so a limit stops the process.
      if (limitEv) safeKill(handle)
      let exit: { code: number | null; stderr: string }
      try {
        exit = await handle.exit
      } catch (err) {
        if (!limitEv) throw err
        const message = err instanceof Error ? err.message : String(err)
        exit = { code: null, stderr: message }
      }
      const duration = Date.now() - t0
      const limited = limitEv !== null || (exit.code !== 0 && adapter.isLimitError(exit.stderr))
      if (limited) {
        if (!limitEv) safeKill(handle)
        this.applyLimit(job, provider, limitEv, startedAt, duration, utilization, result)
        return
      }
      this.complete(job, provider, result, exit.stderr, startedAt, duration, utilization)
    } catch (err) {
      safeKill(handle)
      this.failThrown(job, provider, err, startedAt, Date.now() - t0, utilization, result)
    }
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
    this.emit(job)
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
    const output = job.output
    this.requeue(job, provider, true, output)
    this.rerouteQueued(provider)
  }

  /** Move jobs still queued on a provider that just went to rest. */
  private rerouteQueued(provider: ProviderId): void {
    const waiting = this.queues.get(provider) ?? []
    this.queues.set(provider, [])
    for (const id of waiting) {
      const job = this.byJob.get(id)
      if (!job || job.status !== 'queued' || job.provider !== provider) continue
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
      job.failedOver
    )
    if (!next) {
      job.status = 'failed'
      job.provider = null
      job.error = 'all providers are at their limit'
      this.emit(job)
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
    }
    job.output = ''
    job.status = 'queued'
    job.provider = next
    delete job.error
    this.enqueue(next, job.id)
    this.emit(job)
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
    this.emit(job)
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

  private emit(job: InternalJob): void {
    const snap = copy(job)
    for (const cb of this.listeners) cb(snap)
  }
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
    failedOver: [...job.failedOver]
  }
  if (job.error !== undefined) record.error = job.error
  return record
}

function failureMessage(resultText: string | undefined, stderr: string): string {
  if (resultText) return resultText
  const trimmed = stderr.trim()
  if (trimmed.length <= 500) return trimmed
  return trimmed.slice(-500)
}
