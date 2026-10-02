import { expect, test } from 'vitest'
import type { AgentEvent, Job, ProviderAdapter, ProviderId, RunHandle, RunOptions } from '../types'
import { Orchestrator, type JobRecord } from './orchestrator'
import { headroom, loadRules } from './router'
import { Store } from './store'

const CWD = 'C:\\work'

interface Script {
  events?: AgentEvent[]
  gate?: Promise<void>
  exitCode?: number | null
  stderr?: string
  throwOnRun?: boolean
  onStart?: () => void
}

class FakeAdapter implements ProviderAdapter {
  readonly id: ProviderId
  readonly received: Job[] = []
  readonly cwds: string[] = []
  readonly options: Array<RunOptions | undefined> = []
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

  run(job: Job, cwd: string, opts?: RunOptions): RunHandle {
    this.received.push({ ...job })
    this.cwds.push(cwd)
    this.options.push(opts)
    const script = this.scripts.shift()
    if (!script) throw new Error(`no script left for ${this.id}`)
    script.onStart?.()
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
      kill(): void {
        killed = true
      }
    }
  }

  parseEvent(): AgentEvent | null {
    return null
  }

  isLimitError(event: AgentEvent | string): boolean {
    if (typeof event === 'string') return event.toLowerCase().includes('limit')
    return event.kind === 'limit'
  }
}

function ok(text = 'ok'): Script {
  return {
    events: [{ kind: 'result', ok: true, text, costUsd: 1, tokens: 1, sessionId: 'sess' }]
  }
}

function gate(): { promise: Promise<void>; open: () => void } {
  let open: () => void = () => undefined
  const promise = new Promise<void>((resolve) => {
    open = resolve
  })
  return { promise, open }
}

async function waitFor(predicate: () => boolean): Promise<void> {
  const deadline = Date.now() + 3000
  while (!predicate()) {
    if (Date.now() > deadline) throw new Error('timed out waiting for condition')
    await new Promise((resolve) => setTimeout(resolve, 5))
  }
}

async function withOrch(
  adapters: ProviderAdapter[],
  fn: (orch: Orchestrator, store: Store) => Promise<void>,
  now?: () => number,
  modelFor?: (provider: ProviderId) => string | undefined
): Promise<void> {
  const store = new Store(':memory:')
  try {
    const orch = new Orchestrator({
      adapters,
      store,
      rules: loadRules(),
      cwd: CWD,
      now,
      modelFor
    })
    await orch.init()
    await fn(orch, store)
  } finally {
    store.close()
  }
}

test('routes a job to the provider listed first for its type', async () => {
  const claude = new FakeAdapter('claude', [ok('plan')])
  const cursor = new FakeAdapter('cursor', [ok('files')])
  await withOrch([claude, cursor], async (orch) => {
    await orch.idle()
    orch.submit('planning', 'plan this')
    orch.submit('boilerplate', 'scaffold this')
    await orch.idle()
    expect(claude.received.map((job) => job.prompt)).toEqual(['plan this'])
    expect(cursor.received.map((job) => job.prompt)).toEqual(['scaffold this'])
    expect(claude.cwds).toEqual([CWD])
    expect(orch.list().every((job) => job.status === 'done')).toBe(true)
  })
})

test('runs one job per provider and lets providers run concurrently', async () => {
  const claudeHold = gate()
  const cursorHold = gate()
  const secondStarted = gate()
  const claude = new FakeAdapter('claude', [
    { gate: claudeHold.promise, ...ok('a') },
    { onStart: () => secondStarted.open(), ...ok('b') }
  ])
  const cursor = new FakeAdapter('cursor', [{ gate: cursorHold.promise, ...ok('c') }])

  await withOrch([claude, cursor], async (orch) => {
    orch.submit('planning', 'c1')
    orch.submit('refactor', 'r1')
    orch.submit('planning', 'c2')

    expect(claude.received.map((job) => job.prompt)).toEqual(['c1'])
    expect(cursor.received.map((job) => job.prompt)).toEqual(['r1'])
    expect(orch.list()[2]).toMatchObject({ prompt: 'c2', status: 'queued', provider: 'claude' })

    claudeHold.open()
    await secondStarted.promise
    expect(claude.received.map((job) => job.prompt)).toEqual(['c1', 'c2'])
    expect(orch.list().find((job) => job.prompt === 'r1')?.status).toBe('running')

    cursorHold.open()
    await orch.idle()
    expect(orch.list().map((job) => job.status)).toEqual(['done', 'done', 'done'])
  })
})

test('a successful run writes one ok row and keeps streamed text', async () => {
  const now = 1_700_000_000_000
  const claude = new FakeAdapter('claude', [
    {
      events: [
        { kind: 'text', text: 'hello' },
        { kind: 'text', text: 'world' },
        { kind: 'usage', utilization: 0.2 },
        {
          kind: 'result',
          ok: true,
          text: 'ignored',
          costUsd: 1.5,
          tokens: 80,
          sessionId: 's1'
        }
      ]
    }
  ])
  await withOrch(
    [claude],
    async (orch, store) => {
      orch.submit('debugging', 'find it')
      await orch.idle()
      expect(orch.list()[0]).toMatchObject({ status: 'done', output: 'hello\nworld' })
      expect(store.runs()).toHaveLength(1)
      expect(store.runs()[0]).toMatchObject({
        outcome: 'ok',
        cost_usd: 1.5,
        tokens: 80,
        utilization: 0.2,
        session_id: 's1',
        provider: 'claude',
        job_type: 'debugging',
        started_at: now
      })
      expect(store.runs()[0].duration_ms).toBeGreaterThanOrEqual(0)
    },
    () => now
  )
})

test('uses result text when the stream produced no text', async () => {
  const claude = new FakeAdapter('claude', [
    { events: [{ kind: 'result', ok: true, text: 'from-result', costUsd: 1, tokens: 2 }] }
  ])
  await withOrch([claude], async (orch) => {
    orch.submit('planning', 'go')
    await orch.idle()
    expect(orch.list()[0].output).toBe('from-result')
  })
})

test('a limit event rests claude, records the row, and hands the job to cursor', async () => {
  const now = 1_700_000_000_000
  const resetsAt = 1_800_000_000
  const produced = 'HEAD' + 'y'.repeat(2000)
  const held = gate()
  const claude = new FakeAdapter('claude', [
    {
      events: [
        { kind: 'text', text: produced },
        { kind: 'usage', utilization: 0.4 },
        { kind: 'limit', resetsAt, message: 'spent' }
      ]
    }
  ])
  const cursor = new FakeAdapter('cursor', [
    {
      gate: held.promise,
      events: [{ kind: 'result', ok: true, text: 'done', costUsd: 2, tokens: 3, sessionId: 'c' }]
    }
  ])

  await withOrch(
    [claude, cursor],
    async (orch, store) => {
      await orch.idle()
      orch.submit('planning', 'do the thing')
      const done = orch.idle()
      let settled = false
      void done.then(() => {
        settled = true
      })
      await waitFor(() => cursor.received.length === 1)
      await new Promise((resolve) => setTimeout(resolve, 20))
      expect(settled).toBe(false)

      const handoff =
        '\n\n[Handoff: a previous attempt on claude stopped at its usage limit. Its output so far:]\n'
      expect(claude.received[0].prompt).toBe('do the thing')
      expect(cursor.received[0].prompt).toBe('do the thing' + handoff + produced.slice(-2000))
      expect(cursor.received[0].prompt).not.toContain('HEAD')

      held.open()
      await done
      expect(settled).toBe(true)
      expect(orch.list()[0]).toMatchObject({
        status: 'done',
        provider: 'cursor',
        failedOver: ['claude'],
        output: 'done'
      })
      expect(store.restingUntil('claude')).toBe(resetsAt * 1000)
      expect(store.runs('claude')).toEqual([
        expect.objectContaining({ outcome: 'limit', utilization: 0.4, started_at: now })
      ])
      expect(store.runs('cursor')[0]).toMatchObject({
        outcome: 'ok',
        cost_usd: 2,
        tokens: 3,
        session_id: 'c'
      })
    },
    () => now
  )
})

test('queued jobs of a resting provider are re-picked too', async () => {
  const held = gate()
  const claude = new FakeAdapter('claude', [
    {
      gate: held.promise,
      events: [
        { kind: 'text', text: 'partial' },
        { kind: 'limit', message: 'spent', resetsAt: 1_900_000_000 }
      ]
    }
  ])
  const cursor = new FakeAdapter('cursor', [ok('one'), ok('two')])

  await withOrch([claude, cursor], async (orch) => {
    orch.submit('planning', 'alpha')
    expect(claude.received).toHaveLength(1)
    const second = orch.submit('planning', 'beta')
    expect(second).toMatchObject({ status: 'queued', provider: 'claude' })
    expect(cursor.received).toHaveLength(0)

    held.open()
    await orch.idle()

    expect(claude.received).toHaveLength(1)
    expect(cursor.received.map((job) => job.prompt)[0]).toContain(
      '[Handoff: a previous attempt on claude stopped at its usage limit. Its output so far:]\npartial'
    )
    expect(cursor.received[1].prompt).toBe('beta')
    expect(orch.list().map((job) => job.status)).toEqual(['done', 'done'])
    expect(orch.list()[0].failedOver).toEqual(['claude'])
    expect(orch.list()[1].failedOver).toEqual([])
  })
})

test('both providers limited fails the job', async () => {
  const now = 1_700_000_000_000
  const claude = new FakeAdapter('claude', [
    {
      events: [
        { kind: 'text', text: 'aaa' },
        { kind: 'limit', message: 'no', resetsAt: 1_900_000_000 }
      ]
    }
  ])
  const cursor = new FakeAdapter('cursor', [
    {
      events: [
        { kind: 'text', text: 'bbb' },
        { kind: 'limit', message: 'no' }
      ]
    }
  ])

  await withOrch(
    [claude, cursor],
    async (orch, store) => {
      orch.submit('planning', 'original')
      await orch.idle()
      expect(orch.list()[0]).toMatchObject({
        status: 'failed',
        error: 'all providers are at their limit',
        failedOver: ['claude', 'cursor']
      })
      expect(cursor.received[0].prompt.match(/\[Handoff:/g)).toHaveLength(1)
      expect(cursor.received[0].prompt).toContain('aaa')
      expect(store.runs().map((run) => run.outcome)).toEqual(['limit', 'limit'])
      expect(store.restingUntil('cursor')).toBe(now + 5 * 3600_000)
    },
    () => now
  )
})

test('stderr that is a limit error uses the default rest and fails over', async () => {
  const now = 5_000_000
  const claude = new FakeAdapter('claude', [
    { events: [{ kind: 'text', text: 'partial' }], exitCode: 1, stderr: 'usage limit reached' }
  ])
  const cursor = new FakeAdapter('cursor', [ok('recovered')])
  await withOrch(
    [claude, cursor],
    async (orch, store) => {
      orch.submit('planning', 'original')
      await orch.idle()
      expect(orch.list()[0]).toMatchObject({ status: 'done', failedOver: ['claude'] })
      expect(cursor.received[0].prompt).toContain(
        '[Handoff: a previous attempt on claude stopped at its usage limit. Its output so far:]\npartial'
      )
      expect(store.runs('claude')[0].outcome).toBe('limit')
      expect(store.restingUntil('claude')).toBe(now + 5 * 3600_000)
    },
    () => now
  )
})

test('a thrown adapter fails that job and the queue continues', async () => {
  const claude = new FakeAdapter('claude', [{ throwOnRun: true }, ok('second')])
  await withOrch([claude], async (orch, store) => {
    orch.submit('planning', 'first')
    orch.submit('planning', 'second')
    await orch.idle()
    expect(orch.list()[0]).toMatchObject({ status: 'failed', error: 'adapter exploded' })
    expect(orch.list()[1]).toMatchObject({ status: 'done', output: 'second' })
    expect(claude.received.map((job) => job.prompt)).toEqual(['first', 'second'])
    expect(store.restingUntil('claude')).toBeNull()
    expect(store.runs().map((run) => run.outcome)).toEqual(['error', 'ok'])
  })
})

test('onUpdate receives copies and unsubscribe stops them', async () => {
  const claude = new FakeAdapter('claude', [
    {
      events: [
        { kind: 'text', text: 'hello' },
        { kind: 'result', ok: true, text: 'ignored', costUsd: 1, tokens: 1 }
      ]
    },
    ok('next')
  ])
  await withOrch([claude], async (orch) => {
    const snaps: JobRecord[] = []
    const unsub = orch.onUpdate((job) => snaps.push(job))
    orch.submit('planning', 'prompt')
    await orch.idle()
    expect(snaps[0]).toMatchObject({ status: 'queued', output: '' })
    expect(snaps.some((snap) => snap.output === 'hello')).toBe(true)
    expect(snaps.some((snap) => snap.status === 'done' && snap.output === 'hello')).toBe(true)
    expect(snaps[0].status).toBe('queued')
    const n = snaps.length
    unsub()
    orch.submit('planning', 'after')
    await orch.idle()
    expect(snaps).toHaveLength(n)
    expect(orch.list()[1].status).toBe('done')
  })
})

test('failed runs prefer result text, else a trimmed stderr tail', async () => {
  const stderr = ` ${'a'.repeat(600)}END `
  const claude = new FakeAdapter('claude', [
    {
      events: [{ kind: 'result', ok: false, text: 'bad result', costUsd: 1, tokens: 1 }],
      exitCode: 1,
      stderr: 'stderr message'
    },
    {
      events: [{ kind: 'result', ok: false, text: '', costUsd: 1, tokens: 1 }],
      exitCode: 1,
      stderr
    }
  ])
  await withOrch([claude], async (orch, store) => {
    orch.submit('planning', 'one')
    orch.submit('planning', 'two')
    await orch.idle()
    expect(orch.list()[0].error).toBe('bad result')
    const trimmed = stderr.trim()
    expect(orch.list()[1].error).toBe(trimmed.slice(-500))
    expect(orch.list()[1].error?.endsWith('END')).toBe(true)
    expect(store.runs().map((run) => run.outcome)).toEqual(['error', 'error'])
    expect(store.restingUntil('claude')).toBeNull()
  })
})

test('no signed-in provider fails the job; init can be called again', async () => {
  const claude = new FakeAdapter('claude', [ok('a')])
  const cursor = new FakeAdapter('cursor', [ok('b')])
  claude.installed = false
  cursor.signedIn = false
  await withOrch([claude, cursor], async (orch) => {
    const missing = orch.submit('review', 'look')
    expect(missing).toMatchObject({
      status: 'failed',
      error: 'no provider available',
      provider: null
    })
    await orch.idle()

    claude.installed = true
    claude.signedIn = true
    cursor.signedIn = true
    await orch.init()
    orch.submit('planning', 'first')
    await orch.idle()
    expect(claude.received.map((job) => job.prompt)).toEqual(['first'])

    claude.signedIn = false
    await orch.init()
    orch.submit('planning', 'second')
    await orch.idle()
    expect(cursor.received.map((job) => job.prompt)).toEqual(['second'])
  })
})

test('workers reports availability, rest, headroom, busy work, and queue depth', async () => {
  const now = 1_700_000_000_000
  const held = gate()
  const claude = new FakeAdapter('claude', [{ gate: held.promise, ...ok('a') }, ok('b')])
  const cursor = new FakeAdapter('cursor', [])
  cursor.signedIn = false
  await withOrch(
    [claude, cursor],
    async (orch, store) => {
      store.setResting('claude', now - 1_000)
      store.setResting('cursor', now + 60_000)
      orch.submit('planning', 'one')
      orch.submit('planning', 'two')
      const rules = loadRules()
      const workers = orch.workers()
      expect(workers.map((worker) => worker.id)).toEqual(['claude', 'cursor'])
      expect(workers[0]).toMatchObject({
        available: true,
        restingUntil: null,
        busy: true,
        queued: 1
      })
      expect(workers[1]).toMatchObject({
        available: false,
        restingUntil: now + 60_000,
        busy: false,
        queued: 0
      })
      expect(workers[0].headroom).toBe(headroom('claude', store, rules, now))
      expect(workers[1].headroom).toBe(headroom('cursor', store, rules, now))
      expect(workers[1].headroom).toBe(0)
      held.open()
      await orch.idle()
      expect(orch.workers()[0]).toMatchObject({ busy: false, queued: 0 })
    },
    () => now
  )
})

test('modelFor is read when each run starts', async () => {
  let chosen = 'opus'
  const claude = new FakeAdapter('claude', [ok('a'), ok('b')])
  await withOrch(
    [claude],
    async (orch) => {
      orch.submit('planning', 'one')
      await orch.idle()
      expect(claude.options[0]?.model).toBe('opus')
      chosen = 'sonnet'
      orch.submit('planning', 'two')
      await orch.idle()
      expect(claude.options[1]?.model).toBe('sonnet')
    },
    undefined,
    () => chosen
  )
})

test('without modelFor the adapter sees no model', async () => {
  const claude = new FakeAdapter('claude', [ok('a')])
  await withOrch([claude], async (orch) => {
    orch.submit('planning', 'one')
    await orch.idle()
    expect(claude.options[0]?.model).toBeUndefined()
  })
})

test('an init model is stored, copied, and emitted', async () => {
  const claude = new FakeAdapter('claude', [
    {
      events: [
        { kind: 'init', sessionId: 's1', model: 'claude-opus-4-8' },
        { kind: 'result', ok: true, text: 'done', costUsd: 1, tokens: 1 }
      ]
    }
  ])
  await withOrch([claude], async (orch) => {
    const snaps: JobRecord[] = []
    orch.onUpdate((job) => snaps.push(job))
    const submitted = orch.submit('planning', 'prompt')
    await orch.idle()
    expect(orch.get(submitted.id)?.model).toBe('claude-opus-4-8')
    expect(orch.list()[0]?.model).toBe('claude-opus-4-8')
    expect(snaps.some((snap) => snap.model === 'claude-opus-4-8')).toBe(true)
    const listed = orch.list()[0]
    if (!listed) throw new Error('expected a job')
    listed.model = 'mutated'
    expect(orch.get(submitted.id)?.model).toBe('claude-opus-4-8')
  })
})

test('failover clears the model until the next provider reports one', async () => {
  const claude = new FakeAdapter('claude', [
    {
      events: [
        { kind: 'init', sessionId: 's1', model: 'opus' },
        { kind: 'text', text: 'partial' },
        { kind: 'limit', message: 'usage limit' }
      ]
    }
  ])
  const cursor = new FakeAdapter('cursor', [
    {
      events: [
        { kind: 'init', sessionId: 's2', model: 'composer' },
        { kind: 'result', ok: true, text: 'recovered', costUsd: 1, tokens: 1 }
      ]
    }
  ])
  await withOrch([claude, cursor], async (orch) => {
    const snaps: JobRecord[] = []
    orch.onUpdate((job) => snaps.push(job))
    orch.submit('planning', 'original')
    await orch.idle()
    const opusAt = snaps.findIndex((snap) => snap.model === 'opus')
    const handed = snaps.findIndex((snap) => snap.provider === 'cursor' && snap.status === 'queued')
    expect(opusAt).toBeGreaterThanOrEqual(0)
    expect(handed).toBeGreaterThan(opusAt)
    expect(snaps[handed]?.model).toBeUndefined()
    expect(orch.list()[0]?.model).toBe('composer')
  })
})

test('get returns a copy of the job, or null for an unknown id', async () => {
  const held = gate()
  const claude = new FakeAdapter('claude', [{ gate: held.promise, ...ok('secret') }])
  await withOrch([claude], async (orch) => {
    const submitted = orch.submit('review', 'read me')
    const first = orch.get(submitted.id)
    expect(first).toEqual(submitted)
    expect(first).not.toBe(submitted)
    if (!first) throw new Error('expected a job')
    first.output = 'mutated'
    first.prompt = 'changed'
    first.failedOver.push('cursor')
    expect(orch.get(submitted.id)).toEqual(submitted)
    expect(orch.get('missing')).toBeNull()
    held.open()
    await orch.idle()
    expect(orch.get(submitted.id)?.output).toBe('secret')
  })
})
