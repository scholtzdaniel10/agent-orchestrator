import { randomUUID } from 'node:crypto'
import type {
  AgentEvent,
  BridgeInfo,
  LeadMessage,
  ProviderAdapter,
  ProviderId,
  RouterRules,
  RunHandle
} from '../types'
import { headroom } from '../router/router'
import type { Store } from '../router/store'

export const LEAD_INSTRUCTIONS: string = [
  "You are the lead of a small team of coding agents. Do not do the work yourself: split the user's request into independent jobs and hand each one to a worker with the send_job tool of the orchestrator MCP server. Pick the job type that fits (planning, debugging, review, refactor or boilerplate); the router chooses which worker runs it. Each job prompt must be self-contained, because the worker sees nothing else. A worker only reads files unless you pass edit: true; then it may change files, in a separate worktree, and the user reviews and merges that change themselves. Use edit: true only when the request asks for code or files to be changed, and say in your report which jobs left a change waiting for review.",
  'Send every job first, then collect each one with get_result, which waits for the job to finish. If get_result says a job is still queued or running, call it again. Use get_status to see the whole job board, including jobs the user started directly, and list_workers only if you need to know who is available.',
  'When every result is in, report to the user in a few lines: for each job, which worker ran it and what it answered. Keep it short and do not repeat full outputs. If the request is a simple question you can answer in one sentence without a worker, just answer it.'
].join('\n\n')

const NO_PLAN = 'No plan is available for the lead right now.'

type ResultEvent = Extract<AgentEvent, { kind: 'result' }>
type LimitEvent = Extract<AgentEvent, { kind: 'limit' }>
type Outcome = 'ok' | 'error' | 'limit'

interface Plan {
  provider: ProviderId
  resume: string | undefined
  prompt: string
}

export class Lead {
  private readonly adapters: ProviderAdapter[]
  private readonly byId: Map<ProviderId, ProviderAdapter>
  private readonly store: Store
  private readonly rules: RouterRules
  private readonly bridge: BridgeInfo
  private readonly dir: string
  private readonly prefer?: ProviderId | (() => ProviderId | undefined)
  private readonly now: () => number
  private readonly modelFor?: (provider: ProviderId) => string | undefined
  private available: ProviderId[] = []
  private history: LeadMessage[] = []
  private readonly listeners = new Set<(message: LeadMessage) => void>()
  private sessionId: string | undefined
  private sessionProvider: ProviderId | null = null
  private busy = false
  /** Id of the lead message whose turn is running, or null. */
  private turnId: string | null = null

  constructor(opts: {
    adapters: ProviderAdapter[]
    store: Store
    rules: RouterRules
    bridge: BridgeInfo
    dir: string
    prefer?: ProviderId | (() => ProviderId | undefined)
    now?: () => number
    modelFor?: (provider: ProviderId) => string | undefined
  }) {
    this.adapters = opts.adapters
    this.byId = new Map(opts.adapters.map((adapter) => [adapter.id, adapter]))
    this.store = opts.store
    this.rules = opts.rules
    this.bridge = opts.bridge
    this.dir = opts.dir
    this.prefer = opts.prefer
    this.now = opts.now ?? ((): number => Date.now())
    this.modelFor = opts.modelFor
  }

  /** Providers that are installed and signed in become the candidate set. */
  async init(): Promise<void> {
    const available: ProviderId[] = []
    for (const adapter of this.adapters) {
      if ((await adapter.isInstalled()) && (await adapter.isSignedIn())) available.push(adapter.id)
    }
    this.available = available
  }

  async send(text: string): Promise<LeadMessage> {
    if (text.trim() === '') throw new Error('empty message')
    if (this.busy) throw new Error('lead is busy')
    this.busy = true
    const user: LeadMessage = { id: randomUUID(), role: 'user', text, status: 'done' }
    const lead: LeadMessage = { id: randomUUID(), role: 'lead', text: '', status: 'streaming' }
    this.history.push(user, lead)
    this.turnId = lead.id
    try {
      this.emit(user)
      this.emit(lead)
      await this.turn(lead, text)
    } catch (err) {
      lead.status = 'error'
      if (lead.text === '') lead.text = errorText(err)
      try {
        this.emit(lead)
      } catch (emitErr) {
        console.error(emitErr)
      }
    } finally {
      this.busy = false
      this.turnId = null
    }
    return copyMessage(lead)
  }

  /** Id of the lead message whose turn is running right now, or null. */
  currentTurn(): string | null {
    return this.turnId
  }

  messages(): LeadMessage[] {
    return this.history.map((message) => copyMessage(message))
  }

  onUpdate(cb: (message: LeadMessage) => void): () => void {
    this.listeners.add(cb)
    return (): void => {
      this.listeners.delete(cb)
    }
  }

  provider(): ProviderId | null {
    return this.sessionProvider
  }

  reset(): void {
    if (this.busy) throw new Error('lead is busy')
    this.history = []
    this.sessionId = undefined
    this.sessionProvider = null
  }

  private preferredPlan(): ProviderId | undefined {
    if (typeof this.prefer === 'function') return this.prefer()
    return this.prefer
  }

  /**
   * A new preference for a different available plan with headroom starts this
   * turn fresh, so the lead instructions are sent again. Shown messages stay.
   */
  private followPreference(): void {
    if (this.sessionProvider === null || !this.sessionId) return
    const prefer = this.preferredPlan()
    if (prefer === undefined || prefer === this.sessionProvider) return
    if (!this.available.includes(prefer)) return
    if (headroom(prefer, this.store, this.rules, this.now()) <= 0) return
    this.sessionId = undefined
    this.sessionProvider = null
  }

  private planFor(text: string): Plan | null {
    if (this.sessionProvider !== null && this.sessionId) {
      return { provider: this.sessionProvider, resume: this.sessionId, prompt: text }
    }
    const provider = this.chooseProvider()
    if (!provider) return null
    return {
      provider,
      resume: undefined,
      prompt: `${LEAD_INSTRUCTIONS}\n\nUser request:\n${text}`
    }
  }

  private chooseProvider(): ProviderId | null {
    const rooms = new Map<ProviderId, number>()
    let any = false
    for (const id of this.available) {
      const room = headroom(id, this.store, this.rules, this.now())
      rooms.set(id, room)
      if (room > 0) any = true
    }
    if (!any) return null
    const prefer = this.preferredPlan()
    if (prefer !== undefined && (rooms.get(prefer) ?? 0) > 0) return prefer
    let best: ProviderId | null = null
    let bestRoom = 0
    for (const [id, room] of rooms) {
      if (room > bestRoom || (room === bestRoom && room > 0 && id === 'cursor')) {
        best = id
        bestRoom = room
      }
    }
    return best
  }

  private async turn(lead: LeadMessage, text: string): Promise<void> {
    let provider: ProviderId | null = null
    let startedAt = this.now()
    let t0 = Date.now()
    let costUsd: number | null = null
    let tokens: number | null = null
    let utilization: number | null = null
    let sessionId: string | null = null
    let logged = false
    let handle: RunHandle | undefined

    const record = (outcome: Outcome): void => {
      if (logged || provider === null) return
      this.store.insertRun({
        job_id: lead.id,
        provider,
        job_type: 'lead',
        started_at: startedAt,
        duration_ms: Date.now() - t0,
        cost_usd: costUsd,
        tokens,
        utilization,
        outcome,
        session_id: sessionId
      })
      logged = true
    }

    try {
      this.followPreference()
      const plan = this.planFor(text)
      if (!plan) {
        lead.status = 'error'
        lead.text = NO_PLAN
        this.emit(lead)
        return
      }
      provider = plan.provider
      sessionId = plan.resume ?? null
      lead.provider = provider
      this.emit(lead)
      startedAt = this.now()
      t0 = Date.now()

      const adapter = this.byId.get(provider)
      if (!adapter) throw new Error(`no adapter for ${provider}`)

      let result: ResultEvent | null = null
      let limitEv: LimitEvent | null = null
      handle = adapter.run({ id: lead.id, prompt: plan.prompt }, this.dir, {
        resume: plan.resume,
        bridge: this.bridge,
        model: this.modelFor?.(provider)
      })
      for await (const event of handle.events) {
        if (event.kind === 'init') {
          sessionId = event.sessionId
          this.sessionId = event.sessionId
          this.sessionProvider = provider
          if (event.model !== undefined) {
            lead.model = event.model
            this.emit(lead)
          }
        } else if (event.kind === 'text') {
          this.append(lead, event.text)
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
          if (event.sessionId) {
            sessionId = event.sessionId
            this.sessionId = event.sessionId
            this.sessionProvider = provider
          }
          if (event.costUsd !== undefined) costUsd = event.costUsd
          if (event.tokens !== undefined) tokens = event.tokens
          if (lead.text === '' && event.text !== '') this.append(lead, event.text)
          if (!event.ok) {
            lead.status = 'error'
            this.emit(lead)
          }
        } else if (event.kind === 'limit') {
          limitEv = event
          break
        }
      }

      if (limitEv) this.kill(handle)
      const exit = await handle.exit
      const stderrLimit =
        limitEv === null && result === null && exit.code !== 0 && adapter.isLimitError(exit.stderr)
      if (limitEv || stderrLimit) {
        if (stderrLimit) this.kill(handle)
        this.applyLimit(lead, provider, limitEv?.resetsAt)
        record('limit')
        return
      }

      if (result && !result.ok) {
        lead.status = 'error'
        if (lead.text === '') lead.text = result.text
        this.keepSession(provider, sessionId)
        this.emit(lead)
        record('error')
        return
      }

      if (!result && exit.code !== 0) {
        lead.status = 'error'
        if (lead.text === '') lead.text = stderrTail(exit.stderr)
        this.keepSession(provider, sessionId)
        this.emit(lead)
        record('error')
        return
      }

      lead.status = 'done'
      this.keepSession(provider, sessionId)
      this.emit(lead)
      record('ok')
    } catch (err) {
      this.kill(handle)
      lead.status = 'error'
      if (lead.text === '') lead.text = errorText(err)
      this.emit(lead)
      try {
        record('error')
      } catch (insertErr) {
        console.error(insertErr)
      }
    }
  }

  private applyLimit(lead: LeadMessage, provider: ProviderId, resetsAt: number | undefined): void {
    const until = resetsAt ? resetsAt * 1000 : this.now() + this.rules.defaultRestHours * 3600_000
    this.store.setResting(provider, until)
    lead.status = 'error'
    const notice = `[The lead's plan (${provider}) hit its usage limit. Send your message again to continue on another plan.]`
    lead.text = lead.text === '' ? notice : `${lead.text}\n${notice}`
    this.sessionId = undefined
    this.sessionProvider = null
    this.emit(lead)
  }

  private keepSession(provider: ProviderId, sessionId: string | null): void {
    if (sessionId) {
      this.sessionId = sessionId
      this.sessionProvider = provider
    } else {
      this.sessionId = undefined
      this.sessionProvider = null
    }
  }

  private append(lead: LeadMessage, text: string): void {
    lead.text = lead.text === '' ? text : `${lead.text}\n${text}`
    this.emit(lead)
  }

  private emit(message: LeadMessage): void {
    const snap = copyMessage(message)
    for (const cb of this.listeners) cb(snap)
  }

  private kill(handle: RunHandle | undefined): void {
    if (!handle) return
    try {
      handle.kill()
    } catch (err) {
      console.error(err)
    }
  }
}

function copyMessage(message: LeadMessage): LeadMessage {
  const copy: LeadMessage = {
    id: message.id,
    role: message.role,
    text: message.text,
    status: message.status
  }
  if (message.provider !== undefined) copy.provider = message.provider
  if (message.model !== undefined) copy.model = message.model
  return copy
}

function stderrTail(stderr: string): string {
  const trimmed = stderr.trim()
  if (trimmed.length <= 500) return trimmed
  return trimmed.slice(-500)
}

function errorText(err: unknown): string {
  return err instanceof Error ? err.message : String(err)
}
