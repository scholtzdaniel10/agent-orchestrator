import { expect, test } from 'vitest'
import type {
  AgentEvent,
  BridgeInfo,
  Job,
  LeadMessage,
  ProviderAdapter,
  ProviderId,
  RunHandle,
  RunOptions
} from '../types'
import { createOrchestratorTools, startBridge, type Bridge } from '../bridge'
import { Orchestrator } from '../router/orchestrator'
import { headroom, loadRules } from '../router/router'
import { Store } from '../router/store'
import { Lead, LEAD_INSTRUCTIONS } from './lead'

const DIR = 'lead-work'
const NO_PLAN = 'No plan is available for the lead right now.'

function limitNotice(provider: ProviderId): string {
  return `[The lead's plan (${provider}) hit its usage limit. Send your message again to continue on another plan.]`
}

const bridge: BridgeInfo = {
  url: 'http://127.0.0.1:53124/mcp',
  token: 'bridge-token',
  tools: ['list_workers', 'send_job', 'get_result', 'get_status']
}

interface Script {
  events?: AgentEvent[]
  gate?: Promise<void>
  exitCode?: number | null
  stderr?: string
  throwOnRun?: boolean
}

interface Call {
  job: { id: string; prompt: string }
  cwd: string
  opts: RunOptions | undefined
}

class FakeAdapter implements ProviderAdapter {
  readonly id: ProviderId
  readonly calls: Call[] = []
  kills = 0
  installed = true
  signedIn = true
  private readonly scripts: Script[]

  constructor(id: ProviderId, scripts: Script[]) {
    this.id = id
    this.scripts = [...scripts]
  }

  async isInstalled(): Promise<boolean> {
    return this.installed
  }

  async version(): Promise<string | null> {
    return null
  }

  async isSignedIn(): Promise<boolean> {
    return this.signedIn
  }

  run(job: Pick<Job, 'id' | 'prompt'>, cwd: string, opts?: RunOptions): RunHandle {
    this.calls.push({ job: { id: job.id, prompt: job.prompt }, cwd, opts })
    const script = this.scripts.shift()
    if (!script) throw new Error(`no script left for ${this.id}`)
    if (script.throwOnRun) throw new Error('adapter exploded')
    let killed = false
    const events = (async function* (): AsyncGenerator<AgentEvent> {
      if (script.gate) await script.gate
      if (killed) return
      for (const event of script.events ?? []) {
        if (killed) return
        yield event
      }
    })()
    return {
      events,
      exit: Promise.resolve({
        code: script.exitCode === undefined ? 0 : script.exitCode,
        stderr: script.stderr ?? ''
      }),
      kill: (): void => {
        killed = true
        this.kills += 1
      }
    }
  }

  parseEvent(): AgentEvent | null {
    return null
  }

  isLimitError(event: AgentEvent | string): boolean {
    if (typeof event === 'string') return /usage limit|rate.?limit|limit reached/i.test(event)
    return event.kind === 'limit'
  }
}

function done(sessionId: string, text = 'ok'): Script {
  return {
    events: [
      { kind: 'init', sessionId },
      { kind: 'usage', utilization: 0.2 },
      { kind: 'result', ok: true, text, sessionId, costUsd: 1, tokens: 1 }
    ]
  }
}

function gate(): { promise: Promise<void>; open: () => void } {
  let open: () => void = () => undefined
  const promise = new Promise<void>((resolve) => {
    open = resolve
  })
  return { promise, open }
}

function seed(store: Store, provider: ProviderId, utilization: number, startedAt: number): void {
  store.saveWindows(
    provider,
    [{ name: 'reported', utilization, resetsAt: startedAt + 3_600_000 }],
    startedAt
  )
}

function leadRuns(store: Store): ReturnType<Store['runs']> {
  return store.runs().filter((row) => row.job_type === 'lead')
}

async function withLead(
  adapters: ProviderAdapter[],
  fn: (lead: Lead, store: Store) => Promise<void>,
  opts?: {
    now?: () => number
    prefer?: ProviderId | (() => ProviderId | undefined)
    modelFor?: (provider: ProviderId) => string | undefined
    project?: () => string
    listJobs?: () => import('../router/orchestrator').JobRecord[]
  }
): Promise<void> {
  const store = new Store(':memory:')
  try {
    const lead = new Lead({
      adapters,
      store,
      rules: loadRules(),
      bridge,
      dir: DIR,
      project: opts?.project ?? ((): string => '/project'),
      prefer: opts?.prefer,
      now: opts?.now,
      modelFor: opts?.modelFor,
      listJobs: opts?.listJobs
    })
    await lead.init()
    await fn(lead, store)
  } finally {
    store.close()
  }
}

async function withLeadOrch(
  adapters: ProviderAdapter[],
  fn: (lead: Lead, orch: Orchestrator, store: Store) => Promise<void>,
  opts?: {
    now?: () => number
    prefer?: ProviderId | (() => ProviderId | undefined)
    project?: () => string
  }
): Promise<void> {
  const store = new Store(':memory:')
  let bridgeHandle: Bridge | undefined
  try {
    const rules = loadRules()
    const orch = new Orchestrator({
      adapters,
      store,
      rules,
      cwd: opts?.project?.() ?? '/project',
      now: opts?.now
    })
    await orch.init()
    const leadRef: { current: Lead | null } = { current: null }
    bridgeHandle = await startBridge(
      createOrchestratorTools(orch, (): string | null => leadRef.current?.currentTurn() ?? null)
    )
    const lead = new Lead({
      adapters,
      store,
      rules,
      bridge: bridgeHandle.info,
      dir: DIR,
      project: opts?.project ?? ((): string => '/project'),
      prefer: opts?.prefer,
      now: opts?.now,
      listJobs: (): import('../router/orchestrator').JobRecord[] => orch.list()
    })
    leadRef.current = lead
    await lead.init()
    lead.attachOrchestrator(orch)
    await fn(lead, orch, store)
  } finally {
    try {
      if (bridgeHandle) await bridgeHandle.close()
    } finally {
      store.close()
    }
  }
}

function waitUntil(check: () => boolean): Promise<void> {
  return new Promise((resolve, reject) => {
    const start = Date.now()
    const tick = (): void => {
      if (check()) {
        resolve()
        return
      }
      if (Date.now() - start > 5000) {
        reject(new Error('timeout'))
        return
      }
      setTimeout(tick, 5)
    }
    tick()
  })
}

test('instructions tell the lead when a worker may edit and not to wait on jobs', () => {
  expect(LEAD_INSTRUCTIONS).toContain(
    "A worker's access is read, edit, or full, and cannot go above the ceiling the person set (see list_workers)."
  )
  expect(LEAD_INSTRUCTIONS).toContain('Do not wait for results')
  expect(LEAD_INSTRUCTIONS).not.toContain('read-only for now')
})

test('picks the provider with more headroom', async () => {
  const now = 1_700_000_000_000
  const claude = new FakeAdapter('claude', [done('c')])
  const cursor = new FakeAdapter('cursor', [done('u')])
  await withLead(
    [claude, cursor],
    async (lead, store) => {
      seed(store, 'claude', 0.2, now)
      seed(store, 'cursor', 0.7, now)
      const msg = await lead.send('plan this')
      expect(msg.provider).toBe('claude')
      expect(claude.calls).toHaveLength(1)
      expect(cursor.calls).toHaveLength(0)
    },
    { now: () => now }
  )
})

test('a headroom tie goes to cursor', async () => {
  const now = 1_700_000_000_000
  const claude = new FakeAdapter('claude', [done('c')])
  const cursor = new FakeAdapter('cursor', [done('u')])
  await withLead(
    [claude, cursor],
    async (lead, store) => {
      seed(store, 'claude', 0.4, now)
      seed(store, 'cursor', 0.4, now)
      const msg = await lead.send('plan this')
      expect(msg.provider).toBe('cursor')
      expect(cursor.calls).toHaveLength(1)
      expect(claude.calls).toHaveLength(0)
    },
    { now: () => now }
  )
})

test('prefer wins when available and is ignored when unavailable', async () => {
  const now = 1_700_000_000_000
  const preferred = new FakeAdapter('claude', [done('c')])
  const other = new FakeAdapter('cursor', [done('u')])
  await withLead(
    [preferred, other],
    async (lead, store) => {
      seed(store, 'claude', 0.8, now)
      seed(store, 'cursor', 0.1, now)
      const msg = await lead.send('use the preferred plan')
      expect(msg.provider).toBe('claude')
      expect(preferred.calls).toHaveLength(1)
      expect(other.calls).toHaveLength(0)
    },
    { now: () => now, prefer: 'claude' }
  )

  const unavailable = new FakeAdapter('claude', [done('no')])
  const cursor = new FakeAdapter('cursor', [done('yes')])
  unavailable.signedIn = false
  await withLead(
    [unavailable, cursor],
    async (lead) => {
      const msg = await lead.send('fallback')
      expect(msg.provider).toBe('cursor')
      expect(unavailable.calls).toHaveLength(0)
      expect(cursor.calls).toHaveLength(1)
    },
    { prefer: 'claude' }
  )
})

test('a resting preferred plan is skipped for one with headroom', async () => {
  const now = 1_700_000_000_000
  const claude = new FakeAdapter('claude', [done('c')])
  const cursor = new FakeAdapter('cursor', [done('u')])
  await withLead(
    [claude, cursor],
    async (lead, store) => {
      store.setResting('claude', now + 3600_000)
      const msg = await lead.send('carry on')
      expect(msg.provider).toBe('cursor')
      expect(claude.calls).toHaveLength(0)
      expect(cursor.calls).toHaveLength(1)
    },
    { now: () => now, prefer: 'claude' }
  )
})

test('no plan with headroom reports an error and spawns nothing', async () => {
  const now = 1_700_000_000_000
  const claude = new FakeAdapter('claude', [done('c')])
  const cursor = new FakeAdapter('cursor', [done('u')])
  await withLead(
    [claude, cursor],
    async (lead, store) => {
      seed(store, 'claude', 1, now)
      seed(store, 'cursor', 1, now)
      const msg = await lead.send('hello')
      expect(msg).toMatchObject({ status: 'error', text: NO_PLAN })
      expect(msg.provider).toBeUndefined()
      expect(claude.calls).toHaveLength(0)
      expect(cursor.calls).toHaveLength(0)
      expect(leadRuns(store)).toHaveLength(0)
      expect(lead.provider()).toBeNull()
      expect(lead.messages().map((message) => message.role)).toEqual(['user', 'lead'])
    },
    { now: () => now }
  )
})

test('the first turn sends instructions, the bridge, and no resume', async () => {
  const cursor = new FakeAdapter('cursor', [done('sess-1', 'started')])
  await withLead([cursor], async (lead) => {
    const text = 'plan the work'
    const msg = await lead.send(text)
    expect(cursor.calls).toHaveLength(1)
    const call = cursor.calls[0]
    expect(call.job.id).toBe(msg.id)
    expect(call.job.prompt.startsWith(LEAD_INSTRUCTIONS)).toBe(true)
    expect(call.job.prompt.endsWith(text)).toBe(true)
    expect(call.job.prompt).toBe(`${LEAD_INSTRUCTIONS}\n\nUser request:\n${text}`)
    expect(call.opts?.resume).toBeUndefined()
    expect(call.opts?.bridge).toBe(bridge)
    expect(call.cwd).toBe(DIR)
    expect(msg.status).toBe('done')
    expect(lead.provider()).toBe('cursor')
  })
})

test('a later turn resumes the same provider even when headroom changes', async () => {
  const now = 1_700_000_000_000
  const claude = new FakeAdapter('claude', [
    {
      events: [
        { kind: 'init', sessionId: 'sess-1' },
        {
          kind: 'usage',
          utilization: 0.99,
          windows: [{ name: 'reported', utilization: 0.99 }]
        },
        { kind: 'result', ok: true, text: 'first', sessionId: 'sess-1', costUsd: 1, tokens: 2 }
      ]
    },
    {
      events: [
        { kind: 'result', ok: true, text: 'second', sessionId: 'sess-1', costUsd: 1, tokens: 2 }
      ]
    }
  ])
  const cursor = new FakeAdapter('cursor', [done('other')])
  await withLead(
    [claude, cursor],
    async (lead, store) => {
      seed(store, 'claude', 0.15, now)
      seed(store, 'cursor', 0.55, now)
      await lead.send('plan the work')
      const rules = loadRules()
      expect(headroom('cursor', store, rules, now)).toBeGreaterThan(
        headroom('claude', store, rules, now)
      )
      const second = 'what did they say'
      const msg = await lead.send(second)
      expect(claude.calls).toHaveLength(2)
      expect(cursor.calls).toHaveLength(0)
      expect(claude.calls[1].job.prompt).toBe(second)
      expect(claude.calls[1].opts?.resume).toBe('sess-1')
      expect(claude.calls[1].opts?.bridge).toBe(bridge)
      expect(claude.calls[1].cwd).toBe(DIR)
      expect(msg.provider).toBe('claude')
      expect(lead.provider()).toBe('claude')
      expect(leadRuns(store).map((row) => row.session_id)).toEqual(['sess-1', 'sess-1'])
    },
    { now: () => now }
  )
})

test('streams text updates and messages() returns copies in order', async () => {
  const cursor = new FakeAdapter('cursor', [
    {
      events: [
        { kind: 'text', text: 'alpha' },
        { kind: 'text', text: 'beta' },
        { kind: 'result', ok: true, text: 'ignored', sessionId: 's', costUsd: 1, tokens: 1 }
      ]
    }
  ])
  await withLead([cursor], async (lead) => {
    const seen: LeadMessage[] = []
    lead.onUpdate((message) => seen.push(message))
    const msg = await lead.send('please split this')
    expect(msg.status).toBe('done')
    expect(msg.text).toBe('alpha\nbeta')
    expect(seen.some((message) => message.role === 'lead' && message.text === 'alpha')).toBe(true)
    expect(seen.some((message) => message.role === 'lead' && message.text === 'alpha\nbeta')).toBe(
      true
    )
    const streamed = seen.find((message) => message.role === 'lead' && message.text === 'alpha')
    expect(streamed).toBeTruthy()
    if (streamed) streamed.text = 'changed'
    const messages = lead.messages()
    expect(messages.map((message) => message.role)).toEqual(['user', 'lead'])
    expect(messages[0]).toMatchObject({ text: 'please split this', status: 'done' })
    expect(messages[1]).toMatchObject({ text: 'alpha\nbeta', status: 'done', provider: 'cursor' })
    expect(messages[0].id).not.toBe(messages[1].id)
    messages[1].text = 'mutated'
    expect(lead.messages()[1].text).toBe('alpha\nbeta')
    expect(lead.messages()[1]).not.toBe(messages[1])
  })
})

test('writes one lead run row with cost, tokens, utilization, and session id', async () => {
  const now = 1_700_000_000_000
  const cursor = new FakeAdapter('cursor', [
    {
      events: [
        { kind: 'init', sessionId: 'sess-1' },
        { kind: 'usage', utilization: 0.33 },
        {
          kind: 'result',
          ok: true,
          text: 'from-result',
          sessionId: 'sess-1',
          costUsd: 1.25,
          tokens: 40
        }
      ]
    }
  ])
  await withLead(
    [cursor],
    async (lead, store) => {
      const msg = await lead.send('question')
      expect(msg.text).toBe('from-result')
      expect(leadRuns(store)).toHaveLength(1)
      expect(leadRuns(store)[0]).toMatchObject({
        job_id: msg.id,
        provider: 'cursor',
        job_type: 'lead',
        started_at: now,
        cost_usd: 1.25,
        tokens: 40,
        utilization: 0.33,
        outcome: 'ok',
        session_id: 'sess-1'
      })
      expect(leadRuns(store)[0].duration_ms).toBeGreaterThanOrEqual(0)
    },
    { now: () => now }
  )
})

test('a limit event rests the provider and the next turn switches plan', async () => {
  const now = 1_700_000_000_000
  const resetsAt = 1_800_000_000
  const claude = new FakeAdapter('claude', [
    {
      events: [
        { kind: 'init', sessionId: 'old' },
        { kind: 'text', text: 'partial' },
        { kind: 'usage', utilization: 0.4 },
        { kind: 'limit', resetsAt, message: 'spent' }
      ]
    }
  ])
  const cursor = new FakeAdapter('cursor', [done('fresh', 'continued')])
  await withLead(
    [claude, cursor],
    async (lead, store) => {
      seed(store, 'claude', 0.1, now)
      seed(store, 'cursor', 0.6, now)
      const first = await lead.send('do it')
      expect(first.status).toBe('error')
      expect(first.text).toBe(`partial\n${limitNotice('claude')}`)
      expect(first.provider).toBe('claude')
      expect(store.restingUntil('claude')).toBe(resetsAt * 1000)
      expect(claude.kills).toBe(1)
      expect(lead.provider()).toBeNull()
      expect(leadRuns(store)[0]).toMatchObject({
        outcome: 'limit',
        provider: 'claude',
        utilization: 0.4,
        job_id: first.id
      })

      const second = await lead.send('again')
      expect(cursor.calls).toHaveLength(1)
      expect(claude.calls).toHaveLength(1)
      expect(cursor.calls[0].opts?.resume).toBeUndefined()
      expect(cursor.calls[0].job.prompt).toBe(`${LEAD_INSTRUCTIONS}\n\nUser request:\nagain`)
      expect(cursor.calls[0].opts?.bridge).toBe(bridge)
      expect(second).toMatchObject({ status: 'done', provider: 'cursor', text: 'continued' })
      expect(leadRuns(store).map((row) => row.outcome)).toEqual(['limit', 'ok'])
    },
    { now: () => now }
  )
})

test('a limit on stderr uses the default rest hours', async () => {
  const now = 5_000_000
  const claude = new FakeAdapter('claude', [
    { events: [], exitCode: 1, stderr: 'usage limit reached' }
  ])
  await withLead(
    [claude],
    async (lead, store) => {
      const msg = await lead.send('do it')
      expect(msg.status).toBe('error')
      expect(msg.text).toBe(limitNotice('claude'))
      expect(claude.kills).toBe(1)
      expect(lead.provider()).toBeNull()
      expect(store.restingUntil('claude')).toBe(now + loadRules().defaultRestHours * 3600_000)
      expect(leadRuns(store)[0]).toMatchObject({ outcome: 'limit', provider: 'claude' })
    },
    { now: () => now }
  )
})

test('a failed result and a stderr tail become errors', async () => {
  const stderr = ` ${'b'.repeat(600)}END `
  const claude = new FakeAdapter('claude', [
    {
      events: [{ kind: 'result', ok: false, text: 'bad result', costUsd: 2, tokens: 3 }],
      exitCode: 1,
      stderr: 'stderr message'
    },
    { events: [], exitCode: 1, stderr }
  ])
  await withLead([claude], async (lead, store) => {
    const first = await lead.send('one')
    const second = await lead.send('two')
    expect(first).toMatchObject({ status: 'error', text: 'bad result' })
    const trimmed = stderr.trim()
    expect(second.status).toBe('error')
    expect(second.text).toBe(trimmed.slice(-500))
    expect(second.text.endsWith('END')).toBe(true)
    expect(leadRuns(store).map((row) => row.outcome)).toEqual(['error', 'error'])
    expect(leadRuns(store)[0]).toMatchObject({ cost_usd: 2, tokens: 3 })
    expect(store.restingUntil('claude')).toBeNull()
  })
})

test('a thrown adapter becomes an error and the next send works', async () => {
  const claude = new FakeAdapter('claude', [
    { throwOnRun: true },
    {
      events: [
        { kind: 'result', ok: true, text: 'recovered', sessionId: 's', costUsd: 1, tokens: 1 }
      ]
    }
  ])
  await withLead([claude], async (lead, store) => {
    const failed = await lead.send('boom')
    expect(failed).toMatchObject({ status: 'error', text: 'adapter exploded' })
    expect(leadRuns(store).map((row) => row.outcome)).toEqual(['error'])
    const next = await lead.send('next')
    expect(next).toMatchObject({ status: 'done', text: 'recovered' })
    expect(claude.calls[1].opts?.resume).toBeUndefined()
    expect(claude.calls[1].job.prompt.startsWith(LEAD_INSTRUCTIONS)).toBe(true)
    expect(leadRuns(store).map((row) => row.outcome)).toEqual(['error', 'ok'])
  })
})

test('currentTurn is the lead message id while streaming and null otherwise', async () => {
  const held = gate()
  const cursor = new FakeAdapter('cursor', [
    {
      gate: held.promise,
      events: [
        { kind: 'init', sessionId: 's1' },
        { kind: 'result', ok: true, text: 'ok', sessionId: 's1', costUsd: 1, tokens: 1 }
      ]
    }
  ])
  await withLead([cursor], async (lead) => {
    expect(lead.currentTurn()).toBeNull()
    const pending = lead.send('hello')
    const during = lead.messages().find((message) => message.role === 'lead')
    expect(during).toBeDefined()
    expect(lead.currentTurn()).toBe(during!.id)
    held.open()
    const done = await pending
    expect(done.id).toBe(during!.id)
    expect(lead.currentTurn()).toBeNull()
  })
})

test('rejects blank text and a send while busy; reset and unsubscribe behave', async () => {
  const held = gate()
  const cursor = new FakeAdapter('cursor', [
    {
      gate: held.promise,
      events: [
        { kind: 'init', sessionId: 's1' },
        { kind: 'result', ok: true, text: 'ok', sessionId: 's1', costUsd: 1, tokens: 1 }
      ]
    },
    {
      events: [{ kind: 'result', ok: true, text: 'after', sessionId: 's2', costUsd: 1, tokens: 1 }]
    }
  ])
  await withLead([cursor], async (lead) => {
    await expect(lead.send('')).rejects.toThrow('empty message')
    await expect(lead.send(' \n ')).rejects.toThrow('empty message')
    expect(lead.messages()).toEqual([])

    const seen: LeadMessage[] = []
    const unsub = lead.onUpdate((message) => seen.push(message))
    const pending = lead.send('hello')
    await expect(lead.send('more')).rejects.toThrow('lead is busy')
    expect(() => lead.reset()).toThrow('lead is busy')
    expect(cursor.calls).toHaveLength(1)
    held.open()
    const first = await pending
    expect(first.status).toBe('done')
    const n = seen.length
    expect(n).toBeGreaterThan(0)
    unsub()
    lead.reset()
    expect(lead.messages()).toEqual([])
    expect(lead.provider()).toBeNull()
    const second = await lead.send('next')
    expect(second.status).toBe('done')
    expect(seen).toHaveLength(n)
    expect(cursor.calls[1].opts?.resume).toBeUndefined()
    expect(cursor.calls[1].job.prompt.startsWith(LEAD_INSTRUCTIONS)).toBe(true)
    expect(lead.messages()).toHaveLength(2)
  })
})

test('modelFor reaches the run with bridge and resume, and init sets the message model', async () => {
  let chosen = 'opus'
  const cursor = new FakeAdapter('cursor', [
    {
      events: [
        { kind: 'init', sessionId: 'sess-1', model: 'claude-opus-4-8' },
        { kind: 'result', ok: true, text: 'first', sessionId: 'sess-1', costUsd: 1, tokens: 1 }
      ]
    },
    {
      events: [
        { kind: 'init', sessionId: 'sess-1', model: 'claude-sonnet-4-6' },
        { kind: 'result', ok: true, text: 'second', sessionId: 'sess-1', costUsd: 1, tokens: 1 }
      ]
    }
  ])
  await withLead(
    [cursor],
    async (lead) => {
      const seen: LeadMessage[] = []
      lead.onUpdate((message) => seen.push(message))
      const first = await lead.send('plan the work')
      expect(cursor.calls[0]?.opts).toMatchObject({ bridge, model: 'opus' })
      expect(cursor.calls[0]?.opts?.resume).toBeUndefined()
      expect(first.model).toBe('claude-opus-4-8')
      expect(lead.messages()[1]?.model).toBe('claude-opus-4-8')
      expect(seen.some((message) => message.model === 'claude-opus-4-8')).toBe(true)
      const copied = lead.messages()[1]
      if (!copied) throw new Error('expected a lead message')
      copied.model = 'mutated'
      expect(lead.messages()[1]?.model).toBe('claude-opus-4-8')

      chosen = 'sonnet'
      const second = await lead.send('continue')
      expect(cursor.calls[1]?.opts).toMatchObject({
        bridge,
        resume: 'sess-1',
        model: 'sonnet'
      })
      expect(second.model).toBe('claude-sonnet-4-6')
      expect(lead.messages()[3]?.model).toBe('claude-sonnet-4-6')
      expect(lead.messages()[1]?.model).toBe('claude-opus-4-8')
    },
    { modelFor: () => chosen }
  )
})

test('a usage event with windows is saved', async () => {
  const now = 1_800_000_000_000
  const cursor = new FakeAdapter('cursor', [
    {
      events: [
        { kind: 'init', sessionId: 'sess-1' },
        {
          kind: 'usage',
          utilization: 0.33,
          windows: [
            { name: 'five_hour', utilization: 0.2, resetsAt: 1_800_000_050 },
            { name: 'seven_day', utilization: 0.33, resetsAt: 1_800_000_100 }
          ]
        },
        { kind: 'result', ok: true, text: 'ok', sessionId: 'sess-1', costUsd: 1, tokens: 1 }
      ]
    }
  ])
  await withLead(
    [cursor],
    async (lead, store) => {
      await lead.send('hi')
      expect(store.windows('cursor')).toEqual([
        { name: 'five_hour', utilization: 0.2, resetsAt: 1_800_000_050_000, observedAt: now },
        { name: 'seven_day', utilization: 0.33, resetsAt: 1_800_000_100_000, observedAt: now }
      ])
      expect(leadRuns(store)[0]?.utilization).toBe(0.33)
    },
    { now: () => now }
  )
})

test('a function prefer is read each turn, and a change drops the session', async () => {
  let prefer: ProviderId = 'cursor'
  let reads = 0
  const claude = new FakeAdapter('claude', [done('c-sess', 'from claude')])
  const cursor = new FakeAdapter('cursor', [done('u-sess', 'from cursor')])
  await withLead(
    [claude, cursor],
    async (lead) => {
      const first = await lead.send('one')
      expect(first.provider).toBe('cursor')
      expect(cursor.calls[0]?.opts?.resume).toBeUndefined()
      expect(cursor.calls[0]?.job.prompt.startsWith(LEAD_INSTRUCTIONS)).toBe(true)
      const kept = lead.messages()
      const afterFirst = reads
      prefer = 'claude'
      const second = await lead.send('two')
      expect(reads).toBeGreaterThan(afterFirst)
      expect(second.provider).toBe('claude')
      expect(claude.calls).toHaveLength(1)
      expect(claude.calls[0]?.opts?.resume).toBeUndefined()
      expect(claude.calls[0]?.job.prompt).toBe(`${LEAD_INSTRUCTIONS}\n\nUser request:\ntwo`)
      expect(cursor.calls).toHaveLength(1)
      expect(lead.messages().slice(0, 2)).toEqual(kept)
      expect(lead.messages()).toHaveLength(4)
    },
    {
      prefer: () => {
        reads += 1
        return prefer
      }
    }
  )
})

test('changing preference does not drop the session when the plan is unavailable or resting', async () => {
  const now = 1_800_000_000_000
  let prefer: ProviderId = 'cursor'
  const unavailable = new FakeAdapter('claude', [done('no')])
  unavailable.signedIn = false
  const cursor = new FakeAdapter('cursor', [done('u1', 'first'), done('u1', 'second')])
  await withLead(
    [unavailable, cursor],
    async (lead) => {
      await lead.send('one')
      prefer = 'claude'
      await lead.send('two')
      expect(cursor.calls).toHaveLength(2)
      expect(cursor.calls[1]?.opts?.resume).toBe('u1')
      expect(cursor.calls[1]?.job.prompt).toBe('two')
      expect(unavailable.calls).toHaveLength(0)
    },
    { prefer: () => prefer }
  )

  prefer = 'cursor'
  const claude = new FakeAdapter('claude', [done('c')])
  const restingCursor = new FakeAdapter('cursor', [done('u2', 'first'), done('u2', 'second')])
  await withLead(
    [claude, restingCursor],
    async (lead, store) => {
      await lead.send('one')
      store.setResting('claude', now + 10_000)
      prefer = 'claude'
      await lead.send('two')
      expect(restingCursor.calls).toHaveLength(2)
      expect(restingCursor.calls[1]?.opts?.resume).toBe('u2')
      expect(restingCursor.calls[1]?.job.prompt).toBe('two')
      expect(claude.calls).toHaveLength(0)
    },
    { now: () => now, prefer: () => prefer }
  )
})

test('messages and session survive a new Lead on the same store after openLatest', async () => {
  const store = new Store(':memory:')
  try {
    const first = new FakeAdapter('cursor', [done('sess-keep', 'remembered')])
    const lead = new Lead({
      adapters: [first],
      store,
      rules: loadRules(),
      bridge,
      dir: DIR,
      project: () => '/proj'
    })
    await lead.init()
    await lead.send('keep this chat')
    expect(lead.messages()).toHaveLength(2)
    expect(lead.provider()).toBe('cursor')

    const resumed = new FakeAdapter('cursor', [done('sess-keep', 'again')])
    const again = new Lead({
      adapters: [resumed],
      store,
      rules: loadRules(),
      bridge,
      dir: DIR,
      project: () => '/proj'
    })
    await again.init()
    again.openLatest()
    expect(again.messages().map((message) => message.text)).toEqual([
      'keep this chat',
      'remembered'
    ])
    expect(again.provider()).toBe('cursor')
    await again.send('continue')
    expect(resumed.calls[0]?.opts?.resume).toBe('sess-keep')
    expect(resumed.calls[0]?.job.prompt).toBe('continue')
  } finally {
    store.close()
  }
})

test('reset then send creates a second chat; both are listed newest first', async () => {
  let t = 1000
  const cursor = new FakeAdapter('cursor', [done('s1', 'one'), done('s2', 'two')])
  await withLead(
    [cursor],
    async (lead) => {
      await lead.send('first chat topic')
      const firstId = lead.chats()[0]?.id
      expect(firstId).toBeDefined()
      lead.reset()
      expect(lead.messages()).toEqual([])
      expect(lead.provider()).toBeNull()
      await lead.send('second chat topic')
      const listed = lead.chats()
      expect(listed).toHaveLength(2)
      expect(listed.map((chat) => chat.title)).toEqual(['second chat topic', 'first chat topic'])
      expect(listed[0]?.active).toBe(true)
      expect(listed[1]?.active).toBe(false)
      expect(listed[1]?.id).toBe(firstId)
    },
    { now: (): number => ((t += 1), t) }
  )
})

test('chats of another project are not listed and cannot be opened', async () => {
  const store = new Store(':memory:')
  try {
    let project = '/a'
    const cursor = new FakeAdapter('cursor', [done('sa', 'a'), done('sb', 'b')])
    const lead = new Lead({
      adapters: [cursor],
      store,
      rules: loadRules(),
      bridge,
      dir: DIR,
      project: () => project
    })
    await lead.init()
    await lead.send('only in a')
    const id = lead.chats()[0]?.id
    expect(id).toBeDefined()
    project = '/b'
    expect(lead.chats()).toEqual([])
    expect(() => lead.open(id!)).toThrow('unknown chat')
    await lead.send('only in b')
    expect(lead.chats().map((chat) => chat.title)).toEqual(['only in b'])
  } finally {
    store.close()
  }
})

test('a turn that sends two jobs ends without waiting', async () => {
  const leadHeld = gate()
  const workerHeld = gate()
  const cursor = new FakeAdapter('cursor', [
    {
      gate: leadHeld.promise,
      events: [
        { kind: 'init', sessionId: 'lead-s' },
        { kind: 'result', ok: true, text: 'handed off', sessionId: 'lead-s', costUsd: 0, tokens: 0 }
      ]
    },
    {
      gate: workerHeld.promise,
      events: [
        { kind: 'result', ok: true, text: 'worker-one', sessionId: 'w1', costUsd: 0, tokens: 0 }
      ]
    },
    {
      gate: workerHeld.promise,
      events: [
        { kind: 'result', ok: true, text: 'worker-two', sessionId: 'w2', costUsd: 0, tokens: 0 }
      ]
    },
    done('lead-s', 'both finished')
  ])
  const claude = new FakeAdapter('claude', [])
  claude.signedIn = false
  await withLeadOrch([cursor, claude], async (lead, orch) => {
    const pending = lead.send('split this')
    await waitUntil(() => lead.currentTurn() !== null)
    const turn = lead.currentTurn()
    expect(turn).toBeTruthy()
    orch.submit('planning', 'job one', undefined, undefined, { leadMessage: turn! })
    orch.submit('review', 'job two', undefined, undefined, { leadMessage: turn! })
    leadHeld.open()
    const msg = await pending
    expect(msg.status).toBe('done')
    expect(orch.list().some((job) => job.status === 'running' || job.status === 'queued')).toBe(
      true
    )
    workerHeld.open()
    await orch.idle()
    await lead.whenQuiet()
    const reports = lead.messages().filter((message) => message.kind === 'report')
    expect(reports).toHaveLength(1)
    expect(reports[0]?.status).toBe('done')
    expect(cursor.calls).toHaveLength(4)
  })
})

test('when both jobs finish exactly one follow-up runs and its prompt contains both outputs', async () => {
  const cursor = new FakeAdapter('cursor', [
    done('lead-s', 'sent'),
    {
      events: [
        { kind: 'result', ok: true, text: 'alpha output', sessionId: 'w1', costUsd: 0, tokens: 0 }
      ]
    },
    {
      events: [
        { kind: 'result', ok: true, text: 'beta output', sessionId: 'w2', costUsd: 0, tokens: 0 }
      ]
    },
    done('lead-s', 'summary')
  ])
  const claude = new FakeAdapter('claude', [])
  claude.signedIn = false
  await withLeadOrch([cursor, claude], async (lead, orch) => {
    const turn = await lead.send('two jobs')
    orch.submit('planning', 'a', undefined, undefined, { leadMessage: turn.id })
    orch.submit('planning', 'b', undefined, undefined, { leadMessage: turn.id })
    await orch.idle()
    await lead.whenQuiet()
    expect(cursor.calls).toHaveLength(4)
    const reportCall = cursor.calls[3]
    expect(reportCall?.job.prompt).toContain('alpha output')
    expect(reportCall?.job.prompt).toContain('beta output')
    expect(reportCall?.opts?.resume).toBe('lead-s')
    expect(lead.messages().filter((message) => message.kind === 'report')).toHaveLength(1)
  })
})

test('a failed job is reported as failed in the follow-up prompt', async () => {
  const cursor = new FakeAdapter('cursor', [
    done('s', 'go'),
    { events: [{ kind: 'result', ok: false, text: 'bad', costUsd: 0, tokens: 0 }], exitCode: 1 },
    done('s', 'report')
  ])
  const claude = new FakeAdapter('claude', [])
  claude.signedIn = false
  await withLeadOrch([cursor, claude], async (lead, orch) => {
    const turn = await lead.send('one job')
    orch.submit('debugging', 'fail me', undefined, undefined, { leadMessage: turn.id })
    await orch.idle()
    await lead.whenQuiet()
    expect(cursor.calls[2]?.job.prompt).toContain('status failed')
  })
})

test('follow-up outputs are cut in the prompt', async () => {
  const long = 'z'.repeat(5000)
  const cursor = new FakeAdapter('cursor', [
    done('s', 'go'),
    { events: [{ kind: 'result', ok: true, text: long, sessionId: 'w', costUsd: 0, tokens: 0 }] },
    done('s', 'report')
  ])
  const claude = new FakeAdapter('claude', [])
  claude.signedIn = false
  await withLeadOrch([cursor, claude], async (lead, orch) => {
    const turn = await lead.send('one job')
    orch.submit('planning', 'long', undefined, undefined, { leadMessage: turn.id })
    await orch.idle()
    await lead.whenQuiet()
    const prompt = cursor.calls[2]?.job.prompt ?? ''
    expect(prompt).toContain('cut to 4000 characters')
    expect(prompt.length).toBeLessThan(long.length + 500)
  })
})

test('a follow-up waits for a busy lead', async () => {
  const leadHeld = gate()
  const workerHeld = gate()
  const cursor = new FakeAdapter('cursor', [
    {
      gate: leadHeld.promise,
      events: [
        { kind: 'init', sessionId: 's1' },
        { kind: 'result', ok: true, text: 'first', sessionId: 's1', costUsd: 0, tokens: 0 }
      ]
    },
    {
      gate: workerHeld.promise,
      events: [{ kind: 'result', ok: true, text: 'job out', sessionId: 'w', costUsd: 0, tokens: 0 }]
    },
    done('s1', 'report')
  ])
  const claude = new FakeAdapter('claude', [])
  claude.signedIn = false
  await withLeadOrch([cursor, claude], async (lead, orch) => {
    const pending = lead.send('hold')
    await waitUntil(() => lead.currentTurn() !== null)
    const turn = lead.currentTurn()!
    orch.submit('planning', 'quick', undefined, undefined, { leadMessage: turn })
    workerHeld.open()
    await orch.idle()
    expect(lead.messages().some((message) => message.kind === 'report')).toBe(false)
    leadHeld.open()
    await pending
    await lead.whenQuiet()
    expect(lead.messages().some((message) => message.kind === 'report')).toBe(true)
    expect(cursor.calls).toHaveLength(3)
  })
})

test('a follow-up that sends no jobs causes no further turns', async () => {
  const cursor = new FakeAdapter('cursor', [
    done('s', 'go'),
    { events: [{ kind: 'result', ok: true, text: 'done', sessionId: 'w', costUsd: 0, tokens: 0 }] },
    done('s', 'report only')
  ])
  const claude = new FakeAdapter('claude', [])
  claude.signedIn = false
  await withLeadOrch([cursor, claude], async (lead, orch) => {
    const turn = await lead.send('one job')
    orch.submit('planning', 'solo', undefined, undefined, { leadMessage: turn.id })
    await orch.idle()
    await lead.whenQuiet()
    expect(cursor.calls).toHaveLength(3)
  })
})

test('a report is saved to the chat that owns the jobs when another chat is open', async () => {
  const workerHeld = gate()
  const cursor = new FakeAdapter('cursor', [
    done('s1', 'chat a'),
    {
      gate: workerHeld.promise,
      events: [{ kind: 'result', ok: true, text: 'worker', sessionId: 'w', costUsd: 0, tokens: 0 }]
    },
    done('s1', 'report for a'),
    done('s2', 'chat b reply')
  ])
  const claude = new FakeAdapter('claude', [])
  claude.signedIn = false
  await withLeadOrch([cursor, claude], async (lead, orch, store) => {
    const turnA = await lead.send('work in chat a')
    const chatA = lead.chats().find((chat) => chat.active)?.id
    expect(chatA).toBeDefined()
    orch.submit('planning', 'job', undefined, undefined, { leadMessage: turnA.id })
    lead.reset()
    await lead.send('chat b')
    expect(lead.chats()).toHaveLength(2)
    const chatB = lead.chats().find((chat) => chat.active)?.id
    expect(chatB).toBeDefined()
    expect(chatB).not.toBe(chatA)
    workerHeld.open()
    await orch.idle()
    await lead.whenQuiet()
    const messagesA = store.messages(chatA!)
    expect(messagesA.some((message) => message.kind === 'report')).toBe(true)
    expect(lead.messages().some((message) => message.kind === 'report')).toBe(false)
  })
})

test('restore skips follow-ups for interrupted jobs', async () => {
  const cursor = new FakeAdapter('cursor', [done('s', 'first')])
  const claude = new FakeAdapter('claude', [])
  claude.signedIn = false
  await withLeadOrch([cursor, claude], async (lead, orch) => {
    const turn = await lead.send('before restart')
    lead.noteRestoredJobs([
      {
        id: 'job-old',
        type: 'planning',
        prompt: 'stale',
        provider: 'cursor',
        status: 'failed',
        output: '',
        failedOver: [],
        leadMessage: turn.id,
        error: 'interrupted when the app closed'
      }
    ])
    orch.submit('planning', 'after restore', 'cursor', undefined, { leadMessage: turn.id })
    await orch.idle()
    await lead.whenQuiet()
    expect(cursor.calls).toHaveLength(2)
    expect(lead.messages().some((message) => message.kind === 'report')).toBe(false)
  })
})

test('open while busy throws', async () => {
  const held = gate()
  const cursor = new FakeAdapter('cursor', [
    {
      gate: held.promise,
      events: [
        { kind: 'init', sessionId: 's1' },
        { kind: 'result', ok: true, text: 'ok', sessionId: 's1', costUsd: 1, tokens: 1 }
      ]
    }
  ])
  await withLead([cursor], async (lead) => {
    const pending = lead.send('hello')
    const id = lead.chats()[0]?.id
    expect(id).toBeDefined()
    expect(() => lead.open(id!)).toThrow('lead is busy')
    expect(() => lead.openLatest()).toThrow('lead is busy')
    held.open()
    await pending
  })
})

test('rename validates title and project; remove current chat resets to empty', async () => {
  let project = '/a'
  const cursor = new FakeAdapter('cursor', [done('s1', 'a'), done('s2', 'b')])
  await withLead(
    [cursor],
    async (lead, store) => {
      await lead.send('first topic')
      const id = lead.chats()[0]?.id
      expect(id).toBeDefined()
      expect(() => lead.rename(id!, '')).toThrow('invalid title')
      expect(() => lead.rename(id!, '   ')).toThrow('invalid title')
      expect(() => lead.rename(id!, 'x'.repeat(81))).toThrow('invalid title')
      lead.rename(id!, '  Renamed chat  ')
      expect(lead.chats()[0]?.title).toBe('Renamed chat')
      expect(store.chat(id!)?.title).toBe('Renamed chat')

      project = '/b'
      expect(() => lead.rename(id!, 'nope')).toThrow('unknown chat')
      expect(() => lead.remove(id!)).toThrow('unknown chat')
      project = '/a'
      lead.open(id!)
      lead.remove(id!)
      expect(lead.messages()).toEqual([])
      expect(lead.chats()).toEqual([])
      expect(store.chat(id!)).toBeNull()
    },
    { project: (): string => project }
  )
})

test('remove while its turn runs throws; other chats stay', async () => {
  const held = gate()
  const cursor = new FakeAdapter('cursor', [
    {
      gate: held.promise,
      events: [
        { kind: 'init', sessionId: 's1' },
        { kind: 'result', ok: true, text: 'ok', sessionId: 's1', costUsd: 1, tokens: 1 }
      ]
    },
    done('s2', 'kept')
  ])
  await withLead([cursor], async (lead, store) => {
    const pending = lead.send('busy chat')
    const busyId = lead.chats()[0]?.id
    expect(busyId).toBeDefined()
    expect(() => lead.remove(busyId!)).toThrow('lead is busy')
    held.open()
    await pending
    lead.reset()
    await lead.send('keep me')
    const keepId = lead.chats()[0]?.id
    expect(keepId).toBeDefined()
    lead.open(busyId!)
    lead.remove(busyId!)
    expect(store.chat(busyId!)).toBeNull()
    expect(lead.chats().map((chat) => chat.id)).toEqual([keepId])
  })
})
