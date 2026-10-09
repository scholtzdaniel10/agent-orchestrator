import { randomUUID } from 'node:crypto'
import type {
  AgentEvent,
  BridgeInfo,
  LeadChat,
  LeadMessage,
  ProviderAdapter,
  ProviderId,
  RouterRules,
  RunHandle
} from '../types'
import type { JobRecord, Orchestrator } from '../router/orchestrator'
import { headroom } from '../router/router'
import type { Store } from '../router/store'

export const LEAD_INSTRUCTIONS: string = [
  "You are the lead of a small team of coding agents. Do not do the work yourself: split the user's request into independent jobs and hand each one to a worker with the send_job tool of the orchestrator MCP server. Pick the job type that fits (planning, debugging, review, refactor or boilerplate); the router chooses which worker runs it. Each job prompt must be self-contained, because the worker sees nothing else. A worker only reads files unless you pass edit: true; then it may change files, in a separate worktree, and the user reviews and merges that change themselves. Use edit: true only when the request asks for code or files to be changed.",
  'Send every job with send_job, then stop. Do not wait for results: never call get_result to block until a job finishes. You may use get_status to inspect the board, including jobs the user started directly, and list_workers only if you need to know who is available. After sending, tell the user in one or two lines what you sent and to whom.',
  'If the request is a simple question you can answer in one sentence without a worker, just answer it. The app will prompt you again later with worker results when you need to report them.'
].join('\n\n')

const REPORT_OUTPUT_MAX = 4000

const NO_PLAN = 'No plan is available for the lead right now.'

type ResultEvent = Extract<AgentEvent, { kind: 'result' }>
type LimitEvent = Extract<AgentEvent, { kind: 'limit' }>
type Outcome = 'ok' | 'error' | 'limit'

interface Plan {
  provider: ProviderId
  resume: string | undefined
  prompt: string
}

interface ChatContext {
  chatId: string | null
  history: LeadMessage[]
  sessionId: string | undefined
  sessionProvider: ProviderId | null
  chatTitle: string
  chatCreatedAt: number
}

interface PendingFollowUp {
  chatId: string
  leadMessageId: string
  order: number
}

export class Lead {
  private readonly adapters: ProviderAdapter[]
  private readonly byId: Map<ProviderId, ProviderAdapter>
  private readonly store: Store
  private readonly rules: RouterRules
  private readonly bridge: BridgeInfo
  private readonly dir: string
  private readonly project: () => string
  private readonly prefer?: ProviderId | (() => ProviderId | undefined)
  private readonly now: () => number
  private readonly modelFor?: (provider: ProviderId) => string | undefined
  private readonly listJobs: () => JobRecord[]
  private available: ProviderId[] = []
  private history: LeadMessage[] = []
  private readonly listeners = new Set<(message: LeadMessage, chatId: string | null) => void>()
  private sessionId: string | undefined
  private sessionProvider: ProviderId | null = null
  private busy = false
  private drainingFollowUps = false
  /** Id of the lead message whose turn is running, or null. */
  private turnId: string | null = null
  /** Id of the chat currently shown, or null before the first message. */
  private chatId: string | null = null
  /** Chat the user is viewing; unchanged while a report runs on another chat. */
  private viewChatId: string | null = null
  private chatTitle = ''
  private chatCreatedAt = 0
  private unsubJobs: (() => void) | null = null
  private pendingFollowUps: PendingFollowUp[] = []
  private followUpScheduled = new Set<string>()
  private readonly skipFollowUp = new Set<string>()
  private followUpOrder = 0
  /** Lead message id to the chat that owns it. */
  private readonly turnChat = new Map<string, string>()

  constructor(opts: {
    adapters: ProviderAdapter[]
    store: Store
    rules: RouterRules
    bridge: BridgeInfo
    dir: string
    project: () => string
    prefer?: ProviderId | (() => ProviderId | undefined)
    now?: () => number
    modelFor?: (provider: ProviderId) => string | undefined
    listJobs?: () => JobRecord[]
  }) {
    this.adapters = opts.adapters
    this.byId = new Map(opts.adapters.map((adapter) => [adapter.id, adapter]))
    this.store = opts.store
    this.rules = opts.rules
    this.bridge = opts.bridge
    this.dir = opts.dir
    this.project = opts.project
    this.prefer = opts.prefer
    this.now = opts.now ?? ((): number => Date.now())
    this.modelFor = opts.modelFor
    this.listJobs = opts.listJobs ?? ((): JobRecord[] => [])
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
    this.ensureChat(text)
    this.history.push(user, lead)
    this.turnId = lead.id
    if (this.chatId !== null) this.turnChat.set(lead.id, this.chatId)
    try {
      this.persistMessage(user, this.history.length - 2)
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
      void this.drainFollowUps()
    }
    return copyMessage(lead)
  }

  /** Chat the user has open in the UI. */
  viewChat(): string | null {
    return this.viewChatId
  }

  attachOrchestrator(orch: Orchestrator): void {
    this.unsubJobs?.()
    this.unsubJobs = orch.onUpdate((job) => {
      this.onJobUpdate(job)
    })
  }

  /** After restore, do not follow up for turns whose jobs were interrupted. */
  noteRestoredJobs(jobs: JobRecord[]): void {
    for (const job of jobs) {
      if (job.leadMessage === undefined) continue
      if (job.error === 'interrupted when the app closed') {
        this.skipFollowUp.add(job.leadMessage)
      }
    }
  }

  /** Wait until the lead and any queued report turns are idle. For tests. */
  async whenQuiet(): Promise<void> {
    while (this.busy || this.drainingFollowUps || this.pendingFollowUps.length > 0) {
      await new Promise<void>((resolve) => {
        setTimeout(resolve, 5)
      })
      if (!this.busy && !this.drainingFollowUps && this.pendingFollowUps.length > 0) {
        void this.drainFollowUps()
      }
    }
  }

  /** Id of the lead message whose turn is running right now, or null. */
  currentTurn(): string | null {
    return this.turnId
  }

  messages(): LeadMessage[] {
    return this.history.map((message) => copyMessage(message))
  }

  chats(): LeadChat[] {
    const active = this.chatId
    return this.store.chats(this.project()).map((row) => ({
      id: row.id,
      title: row.title,
      updatedAt: row.updated_at,
      active: row.id === active,
      ...(this.chatHasPendingWork(row.id) ? { pending: true } : {})
    }))
  }

  open(id: string): void {
    if (this.busy) throw new Error('lead is busy')
    const row = this.store.chat(id)
    if (row === null || row.project !== this.project()) throw new Error('unknown chat')
    this.chatId = row.id
    this.viewChatId = row.id
    this.chatTitle = row.title
    this.chatCreatedAt = row.created_at
    this.history = this.store.messages(id).map((message) => copyMessage(message))
    this.indexTurnChats(id)
    this.sessionId = row.session_id ?? undefined
    this.sessionProvider = isProviderId(row.session_provider) ? row.session_provider : null
  }

  openLatest(): void {
    if (this.busy) throw new Error('lead is busy')
    const newest = this.store.chats(this.project())[0]
    if (newest === undefined) {
      this.history = []
      this.sessionId = undefined
      this.sessionProvider = null
      this.chatId = null
      this.viewChatId = null
      this.chatTitle = ''
      this.chatCreatedAt = 0
      return
    }
    this.open(newest.id)
  }

  onUpdate(cb: (message: LeadMessage, chatId: string | null) => void): () => void {
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
    this.chatId = null
    this.viewChatId = null
    this.chatTitle = ''
    this.chatCreatedAt = 0
  }

  private ensureChat(text: string): void {
    if (this.chatId !== null) {
      const row = this.store.chat(this.chatId)
      if (row !== null && row.project === this.project()) return
      // Active project changed while a chat was still open — start a new one.
      this.history = []
      this.sessionId = undefined
      this.sessionProvider = null
      this.chatId = null
      this.viewChatId = null
      this.chatTitle = ''
      this.chatCreatedAt = 0
    }
    const id = randomUUID()
    const title = chatTitle(text)
    const now = this.now()
    this.chatId = id
    this.viewChatId = id
    this.chatTitle = title
    this.chatCreatedAt = now
    this.persistChat()
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
    this.persistChat()
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
          this.persistChat()
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
            this.persistChat()
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
    this.persistChat()
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
    this.persistChat()
  }

  private append(lead: LeadMessage, text: string): void {
    lead.text = lead.text === '' ? text : `${lead.text}\n${text}`
    this.emit(lead)
  }

  private emit(message: LeadMessage): void {
    if (message.status !== 'streaming') {
      const seq = this.history.findIndex((item) => item.id === message.id)
      if (seq >= 0) this.persistMessage(message, seq)
    }
    const snap = copyMessage(message)
    for (const cb of this.listeners) cb(snap, this.chatId)
  }

  private onJobUpdate(job: JobRecord): void {
    const leadMessageId = job.leadMessage
    if (leadMessageId === undefined) return
    if (this.skipFollowUp.has(leadMessageId)) return
    if (this.followUpScheduled.has(leadMessageId)) return
    const group = this.jobsForLeadMessage(leadMessageId)
    if (group.length === 0) return
    if (!group.every((item) => item.status === 'done' || item.status === 'failed')) return
    const chatId = this.turnChat.get(leadMessageId) ?? this.store.chatForMessage(leadMessageId)
    if (chatId === null) return
    this.followUpScheduled.add(leadMessageId)
    this.pendingFollowUps.push({ chatId, leadMessageId, order: this.followUpOrder++ })
    this.pendingFollowUps.sort((a, b) => a.order - b.order)
    void this.drainFollowUps()
  }

  private jobsForLeadMessage(leadMessageId: string): JobRecord[] {
    return this.listJobs().filter((item) => item.leadMessage === leadMessageId)
  }

  private async drainFollowUps(): Promise<void> {
    if (this.drainingFollowUps || this.busy) return
    this.drainingFollowUps = true
    try {
      while (this.pendingFollowUps.length > 0 && !this.busy) {
        const next = this.pendingFollowUps.shift()
        if (!next) break
        await this.runReportTurn(next.chatId, next.leadMessageId)
      }
    } finally {
      this.drainingFollowUps = false
      if (this.pendingFollowUps.length > 0 && !this.busy) void this.drainFollowUps()
    }
  }

  private async runReportTurn(chatId: string, sourceLeadMessageId: string): Promise<void> {
    const jobs = this.jobsForLeadMessage(sourceLeadMessageId)
    if (jobs.length === 0) return
    const saved = this.saveChatContext()
    this.busy = true
    const lead: LeadMessage = {
      id: randomUUID(),
      role: 'lead',
      kind: 'report',
      text: '',
      status: 'streaming'
    }
    try {
      this.loadChat(chatId)
      this.history.push(lead)
      this.turnId = lead.id
      this.turnChat.set(lead.id, chatId)
      this.emit(lead)
      await this.turn(lead, buildReportPrompt(jobs))
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
      if (saved.chatId === chatId) {
        this.loadChat(chatId)
      } else {
        this.restoreChatContext(saved)
      }
      void this.drainFollowUps()
    }
  }

  private loadChat(id: string): void {
    const row = this.store.chat(id)
    if (row === null || row.project !== this.project()) throw new Error('unknown chat')
    this.chatId = row.id
    this.chatTitle = row.title
    this.chatCreatedAt = row.created_at
    this.history = this.store.messages(id).map((message) => copyMessage(message))
    this.indexTurnChats(id)
    this.sessionId = row.session_id ?? undefined
    this.sessionProvider = isProviderId(row.session_provider) ? row.session_provider : null
  }

  private indexTurnChats(chatId: string): void {
    for (const message of this.store.messages(chatId)) {
      if (message.role === 'lead') this.turnChat.set(message.id, chatId)
    }
  }

  private saveChatContext(): ChatContext {
    return {
      chatId: this.chatId,
      history: this.history,
      sessionId: this.sessionId,
      sessionProvider: this.sessionProvider,
      chatTitle: this.chatTitle,
      chatCreatedAt: this.chatCreatedAt
    }
  }

  private restoreChatContext(ctx: ChatContext): void {
    this.chatId = ctx.chatId
    this.history = ctx.history
    this.sessionId = ctx.sessionId
    this.sessionProvider = ctx.sessionProvider
    this.chatTitle = ctx.chatTitle
    this.chatCreatedAt = ctx.chatCreatedAt
  }

  private chatHasPendingWork(chatId: string): boolean {
    if (this.busy && this.chatId === chatId) return true
    if (this.pendingFollowUps.some((item) => item.chatId === chatId)) return true
    const leadIds = new Set(
      this.store
        .messages(chatId)
        .filter((message) => message.role === 'lead')
        .map((message) => message.id)
    )
    return this.listJobs().some(
      (job) =>
        job.leadMessage !== undefined &&
        leadIds.has(job.leadMessage) &&
        job.status !== 'done' &&
        job.status !== 'failed'
    )
  }

  private persistMessage(message: LeadMessage, seq: number): void {
    if (this.chatId === null) return
    try {
      this.store.saveMessage(this.chatId, seq, copyMessage(message))
      this.persistChat()
    } catch (err) {
      console.error(err)
    }
  }

  private persistChat(): void {
    if (this.chatId === null) return
    try {
      this.store.saveChat({
        id: this.chatId,
        project: this.project(),
        title: this.chatTitle,
        created_at: this.chatCreatedAt,
        updated_at: this.now(),
        session_id: this.sessionId ?? null,
        session_provider: this.sessionProvider
      })
    } catch (err) {
      console.error(err)
    }
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

function chatTitle(text: string): string {
  const line = text.split(/\r?\n/, 1)[0] ?? ''
  return line.trim().slice(0, 60)
}

function isProviderId(value: string | null): value is ProviderId {
  return value === 'claude' || value === 'cursor'
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
  if (message.kind !== undefined) copy.kind = message.kind
  return copy
}

function buildReportPrompt(jobs: JobRecord[]): string {
  const lines = [
    'Worker jobs from an earlier turn have finished. Report to the user in a few lines.',
    ''
  ]
  for (const job of jobs) {
    lines.push(
      `Job ${job.id} · type ${job.type} · worker ${job.provider ?? 'none'} · status ${job.status}`
    )
    if (job.error !== undefined) lines.push(`Error: ${job.error}`)
    if (job.edit === true && job.change !== undefined) {
      lines.push('This job left a change waiting for review.')
    }
    const output = job.output
    if (output !== '') {
      if (output.length > REPORT_OUTPUT_MAX) {
        lines.push(`Output (cut to ${REPORT_OUTPUT_MAX} characters):`)
        lines.push(output.slice(-REPORT_OUTPUT_MAX))
      } else {
        lines.push('Output:')
        lines.push(output)
      }
    }
    lines.push('')
  }
  lines.push('Summarize for the user; keep it short.')
  return lines.join('\n')
}

function stderrTail(stderr: string): string {
  const trimmed = stderr.trim()
  if (trimmed.length <= 500) return trimmed
  return trimmed.slice(-500)
}

function errorText(err: unknown): string {
  return err instanceof Error ? err.message : String(err)
}
