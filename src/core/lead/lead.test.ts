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
  store.insertRun({
    job_id: `seed-${provider}`,
    provider,
    job_type: 'planning',
    started_at: startedAt,
    duration_ms: 1,
    cost_usd: null,
    tokens: null,
    utilization,
    outcome: 'ok',
    session_id: null
  })
}

function leadRuns(store: Store): ReturnType<Store['runs']> {
  return store.runs().filter((row) => row.job_type === 'lead')
}

async function withLead(
  adapters: ProviderAdapter[],
  fn: (lead: Lead, store: Store) => Promise<void>,
  opts?: { now?: () => number; prefer?: ProviderId }
): Promise<void> {
  const store = new Store(':memory:')
  try {
    const lead = new Lead({
      adapters,
      store,
      rules: loadRules(),
      bridge,
      dir: DIR,
      prefer: opts?.prefer,
      now: opts?.now
    })
    await lead.init()
    await fn(lead, store)
  } finally {
    store.close()
  }
}

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
        { kind: 'usage', utilization: 0.99 },
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
