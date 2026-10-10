import { expect, test } from 'vitest'
import { TESTED_VERSIONS, unknownFormatMessage } from '../providers/versions'
import type {
  AgentEvent,
  Job,
  ProviderAdapter,
  ProviderId,
  RouterRules,
  RunHandle,
  RunOptions
} from '../types'
import type { Worktrees } from '../worktrees'
import { Orchestrator, type JobRecord } from './orchestrator'
import { headroom, loadRules } from './router'
import { Store } from './store'

const CWD = 'C:\\work'

interface Script {
  events?: AgentEvent[]
  gate?: Promise<void>
  /** Awaited once, after the first event, so a test can look mid-run. */
  pause?: Promise<void>
  exitCode?: number | null
  stderr?: string
  unreadable?: boolean
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
  cliVersion: string | null
  private readonly scripts: Script[]

  constructor(id: ProviderId, scripts: Script[]) {
    this.id = id
    this.scripts = [...scripts]
    this.cliVersion = TESTED_VERSIONS[id]
  }

  async isInstalled(): Promise<boolean> {
    return this.installed
  }

  async version(): Promise<string | null> {
    return this.cliVersion
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
    let wake: (() => void) | undefined
    const killedWait = new Promise<void>((resolve) => {
      wake = resolve
    })
    const events = (async function* (): AsyncGenerator<AgentEvent> {
      if (script.gate) await Promise.race([script.gate, killedWait])
      if (killed) return
      let paused = false
      for (const event of script.events ?? []) {
        if (killed) return
        yield event
        if (!paused && script.pause) {
          paused = true
          await Promise.race([script.pause, killedWait])
          if (killed) return
        }
      }
    })()
    return {
      events,
      exit: Promise.resolve({
        code: script.exitCode === undefined ? 0 : script.exitCode,
        stderr: script.stderr ?? '',
        ...(script.unreadable === true ? { unreadable: true } : {})
      }),
      kill(): void {
        killed = true
        wake?.()
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
  modelFor?: (provider: ProviderId) => string | undefined,
  extra?: {
    cwd?: string | (() => string)
    worktrees?: Worktrees
    onChanges?: () => void
    rules?: RouterRules
  }
): Promise<void> {
  const store = new Store(':memory:')
  try {
    const orch = new Orchestrator({
      adapters,
      store,
      rules: extra?.rules ?? loadRules(),
      cwd: extra?.cwd ?? CWD,
      now,
      modelFor,
      worktrees: extra?.worktrees,
      onChanges: extra?.onChanges
    })
    await orch.init()
    await fn(orch, store)
  } finally {
    store.close()
  }
}

function rulesWith(maxParallel: number): RouterRules {
  return { ...loadRules(), maxParallel }
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

test('runs up to maxParallel jobs per provider and lets providers run concurrently', async () => {
  const claudeHold = gate()
  const cursorHold = gate()
  const claude = new FakeAdapter('claude', [
    { gate: claudeHold.promise, ...ok('a') },
    { gate: claudeHold.promise, ...ok('b') }
  ])
  const cursor = new FakeAdapter('cursor', [{ gate: cursorHold.promise, ...ok('c') }])

  await withOrch([claude, cursor], async (orch) => {
    orch.submit('planning', 'c1')
    orch.submit('refactor', 'r1')
    orch.submit('planning', 'c2')

    expect(claude.received.map((job) => job.prompt)).toEqual(['c1', 'c2'])
    expect(cursor.received.map((job) => job.prompt)).toEqual(['r1'])
    expect(orch.list()[2]).toMatchObject({ prompt: 'c2', status: 'running', provider: 'claude' })
    expect(orch.workers()[0]).toMatchObject({ busy: true, running: 2, queued: 0 })

    claudeHold.open()
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
  cursor.signedIn = false

  await withOrch(
    [claude, cursor],
    async (orch) => {
      orch.submit('planning', 'alpha')
      expect(claude.received).toHaveLength(1)
      const second = orch.submit('planning', 'beta')
      expect(second).toMatchObject({ status: 'queued', provider: 'claude' })
      expect(cursor.received).toHaveLength(0)

      cursor.signedIn = true
      await orch.init()
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
    },
    undefined,
    undefined,
    { rules: rulesWith(1) }
  )
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
  const claude = new FakeAdapter('claude', [
    { gate: held.promise, ...ok('a') },
    { gate: held.promise, ...ok('b') }
  ])
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
        running: 2,
        queued: 0,
        resetsAt: null,
        atRisk: false,
        windows: []
      })
      expect(workers[1]).toMatchObject({
        available: false,
        restingUntil: now + 60_000,
        busy: false,
        running: 0,
        queued: 0
      })
      expect(workers[0].headroom).toBe(headroom('claude', store, rules, now))
      expect(workers[1].headroom).toBe(headroom('cursor', store, rules, now))
      expect(workers[1].headroom).toBe(0)
      held.open()
      await orch.idle()
      expect(orch.workers()[0]).toMatchObject({ busy: false, running: 0, queued: 0 })
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

test('a usage event with windows is stored before the run ends and changes the next pick', async () => {
  const now = 1_800_000_000_000
  const pause = gate()
  const claude = new FakeAdapter('claude', [
    {
      pause: pause.promise,
      events: [
        {
          kind: 'usage',
          utilization: 0.9,
          windows: [{ name: 'reported', utilization: 0.9, resetsAt: 1_800_003_600 }]
        },
        { kind: 'result', ok: true, text: 'done', costUsd: 1, tokens: 1, sessionId: 's' }
      ]
    }
  ])
  const cursor = new FakeAdapter('cursor', [ok('next')])
  await withOrch(
    [claude, cursor],
    async (orch, store) => {
      const first = orch.submit('planning', 'first')
      await waitFor(() => store.windows('claude').length === 1)
      expect(orch.get(first.id)?.status).toBe('running')
      expect(store.windows('claude')).toEqual([
        {
          name: 'reported',
          utilization: 0.9,
          resetsAt: 1_800_003_600_000,
          observedAt: now
        }
      ])
      const second = orch.submit('planning', 'second')
      expect(second).toMatchObject({ provider: 'cursor', reason: 'more headroom' })
      pause.open()
      await orch.idle()
      expect(cursor.received.map((job) => job.prompt)).toEqual(['second'])
      expect(claude.received).toHaveLength(1)
      expect(store.runs('claude')[0]).toMatchObject({ utilization: 0.9, outcome: 'ok' })
    },
    () => now
  )
})

test('reason is set and copied on submit, list, get, and updates', async () => {
  const claude = new FakeAdapter('claude', [ok('a')])
  const cursor = new FakeAdapter('cursor', [ok('b')])
  await withOrch([claude, cursor], async (orch) => {
    const snaps: JobRecord[] = []
    orch.onUpdate((job) => snaps.push(job))
    const submitted = orch.submit('planning', 'plan')
    expect(submitted.reason).toBe('first choice')
    expect(orch.list()[0]?.reason).toBe('first choice')
    expect(orch.get(submitted.id)?.reason).toBe('first choice')
    const queued = snaps.find((snap) => snap.status === 'queued')
    expect(queued?.reason).toBe('first choice')
    if (queued) queued.reason = 'mutated'
    expect(orch.get(submitted.id)?.reason).toBe('first choice')
    await orch.idle()
    expect(orch.list()[0]?.reason).toBe('first choice')
    expect(snaps.some((snap) => snap.status === 'done' && snap.reason === 'first choice')).toBe(
      true
    )
  })
})

test('submit with a provider runs on that plan even when the router would not', async () => {
  const claude = new FakeAdapter('claude', [ok('plan')])
  const cursor = new FakeAdapter('cursor', [ok('files')])
  await withOrch([claude, cursor], async (orch) => {
    const job = orch.submit('planning', 'on cursor', 'cursor')
    expect(job).toMatchObject({ provider: 'cursor', reason: 'chosen' })
    await orch.idle()
    expect(cursor.received.map((received) => received.prompt)).toEqual(['on cursor'])
    expect(claude.received).toHaveLength(0)
    expect(orch.list()[0]).toMatchObject({ status: 'done', provider: 'cursor', reason: 'chosen' })
  })
})

test('jobs submitted with the same group keep it on submit and list', async () => {
  const claude = new FakeAdapter('claude', [ok('a')])
  const cursor = new FakeAdapter('cursor', [ok('b')])
  await withOrch([claude, cursor], async (orch) => {
    const a = orch.submit('planning', 'same', 'claude', undefined, { group: 'cmp-1' })
    const b = orch.submit('planning', 'same', 'cursor', undefined, { group: 'cmp-1' })
    const plain = orch.submit('planning', 'alone', 'claude')
    expect(a.group).toBe('cmp-1')
    expect(b.group).toBe('cmp-1')
    expect(plain).not.toHaveProperty('group')
    const listed = orch.list()
    expect(listed.find((job) => job.id === a.id)?.group).toBe('cmp-1')
    expect(listed.find((job) => job.id === b.id)?.group).toBe('cmp-1')
    expect(listed.find((job) => job.id === plain.id)).not.toHaveProperty('group')
    await orch.idle()
  })
})

test('jobs submitted with leadMessage keep it on submit and list', async () => {
  const claude = new FakeAdapter('claude', [ok('a')])
  await withOrch([claude], async (orch) => {
    const linked = orch.submit('planning', 'from lead', undefined, undefined, {
      leadMessage: 'm1'
    })
    const plain = orch.submit('planning', 'alone')
    expect(linked.leadMessage).toBe('m1')
    expect(plain).not.toHaveProperty('leadMessage')
    const listed = orch.list()
    expect(listed.find((job) => job.id === linked.id)?.leadMessage).toBe('m1')
    expect(listed.find((job) => job.id === plain.id)).not.toHaveProperty('leadMessage')
    await orch.idle()
  })
})

test('a chosen plan that is not signed in or is resting fails the job', async () => {
  const now = 1_800_000_000_000
  const claude = new FakeAdapter('claude', [ok('no')])
  const cursor = new FakeAdapter('cursor', [ok('no')])
  cursor.signedIn = false
  await withOrch(
    [claude, cursor],
    async (orch, store) => {
      const unsigned = orch.submit('planning', 'cursor please', 'cursor')
      expect(unsigned).toMatchObject({ status: 'failed', error: 'cursor is not signed in' })
      expect(cursor.received).toHaveLength(0)

      store.setResting('claude', now + 1000)
      const resting = orch.submit('boilerplate', 'claude please', 'claude')
      expect(resting).toMatchObject({ status: 'failed', error: 'claude is resting' })
      expect(claude.received).toHaveLength(0)
      await orch.idle()
    },
    () => now
  )
})

test('a chosen job that hits its limit fails, and an unchosen sibling fails over', async () => {
  const now = 1_800_000_000_000
  const held = gate()
  const stuck = gate()
  const claude = new FakeAdapter('claude', [
    {
      gate: held.promise,
      events: [
        { kind: 'text', text: 'partial' },
        { kind: 'limit', message: 'spent', resetsAt: 1_900_000_000 }
      ]
    },
    {
      gate: held.promise,
      pause: stuck.promise,
      events: [{ kind: 'text', text: 'sibling-partial' }]
    }
  ])
  const cursor = new FakeAdapter('cursor', [ok('moved')])
  await withOrch(
    [claude, cursor],
    async (orch, store) => {
      const chosen = orch.submit('planning', 'pinned', 'claude')
      expect(chosen.reason).toBe('chosen')
      const sibling = orch.submit('planning', 'sibling')
      expect(sibling).toMatchObject({
        status: 'running',
        provider: 'claude',
        reason: 'first choice'
      })
      held.open()
      await orch.idle()
      expect(orch.get(chosen.id)).toMatchObject({
        status: 'failed',
        error: 'claude hit its usage limit',
        provider: 'claude',
        reason: 'chosen'
      })
      expect(orch.get(sibling.id)).toMatchObject({
        status: 'done',
        provider: 'cursor',
        reason: 'failover',
        output: 'moved',
        failedOver: ['claude']
      })
      expect(cursor.received).toHaveLength(1)
      expect(cursor.received[0]?.prompt).toContain('sibling')
      expect(claude.received).toHaveLength(2)
      expect(store.restingUntil('claude')).toBe(1_900_000_000_000)
      expect(store.runs('claude')[0]).toMatchObject({ outcome: 'limit' })
    },
    () => now
  )
})

test('a chosen job on a plan that hits its limit fails instead of moving', async () => {
  const now = 1_800_000_000_000
  const held = gate()
  const stuck = gate()
  const claude = new FakeAdapter('claude', [
    {
      gate: held.promise,
      events: [
        { kind: 'text', text: 'partial' },
        { kind: 'limit', message: 'spent', resetsAt: 1_900_000_000 }
      ]
    },
    {
      gate: held.promise,
      pause: stuck.promise,
      events: [{ kind: 'text', text: 'chosen-partial' }]
    }
  ])
  const cursor = new FakeAdapter('cursor', [ok('moved')])
  await withOrch(
    [claude, cursor],
    async (orch) => {
      const runner = orch.submit('planning', 'runner')
      const chosen = orch.submit('planning', 'pinned', 'claude')
      expect(chosen).toMatchObject({ status: 'running', provider: 'claude', reason: 'chosen' })
      held.open()
      await orch.idle()
      expect(orch.get(chosen.id)).toMatchObject({
        status: 'failed',
        error: 'claude hit its usage limit',
        provider: 'claude',
        reason: 'chosen'
      })
      expect(orch.get(runner.id)).toMatchObject({
        status: 'done',
        provider: 'cursor',
        reason: 'failover'
      })
      expect(cursor.received).toHaveLength(1)
      expect(cursor.received[0]?.prompt).toContain('runner')
      expect(cursor.received[0]?.prompt).not.toContain('pinned')
    },
    () => now
  )
})

test('workers reports resetsAt, atRisk, and windows', async () => {
  const now = 1_800_000_000_000
  const week = 168 * 3600_000
  const claude = new FakeAdapter('claude', [])
  await withOrch(
    [claude],
    async (orch, store) => {
      store.saveWindows(
        'claude',
        [
          { name: 'five_hour', utilization: 0.2, resetsAt: now + 3600_000 },
          { name: 'seven_day', utilization: 0.5, resetsAt: now + week / 10 }
        ],
        now
      )
      expect(orch.workers()[0]).toMatchObject({
        resetsAt: now + week / 10,
        atRisk: true,
        windows: [
          { name: 'five_hour', used: 0.2, resetsAt: now + 3600_000 },
          { name: 'seven_day', used: 0.5, resetsAt: now + week / 10 }
        ]
      })
    },
    () => now
  )
})

function fakeWorktrees(
  create: (project: string, jobId: string) => Promise<{ id: string; path: string; branch: string }>
): Worktrees {
  return { create } as unknown as Worktrees
}

test('an editing job runs in its worktree and reports the change', async () => {
  const claude = new FakeAdapter('claude', [ok('edited')])
  const projects: string[] = []
  const worktrees = fakeWorktrees(async (project, jobId) => {
    projects.push(project)
    const id = jobId.slice(0, 8)
    return { id, path: `wt/${id}`, branch: `orch/${id}` }
  })
  await withOrch(
    [claude],
    async (orch) => {
      const submitted = orch.submit('boilerplate', 'change it', undefined, true)
      expect(submitted.edit).toBe(true)
      await orch.idle()
      const id = submitted.id.slice(0, 8)
      expect(orch.get(submitted.id)).toMatchObject({ status: 'done', edit: true, change: id })
      expect(claude.cwds).toEqual([`wt/${id}`])
      expect(claude.options[0]).toMatchObject({ edit: true })
      expect(projects).toEqual([CWD])
    },
    undefined,
    undefined,
    { worktrees }
  )
})

test('a failed worktree create fails the job without spawning', async () => {
  const claude = new FakeAdapter('claude', [ok('nope')])
  const worktrees = fakeWorktrees(async () => {
    throw new Error('not a git repository')
  })
  await withOrch(
    [claude],
    async (orch, store) => {
      orch.submit('boilerplate', 'change it', undefined, true)
      await orch.idle()
      expect(orch.list()[0]).toMatchObject({
        status: 'failed',
        error: 'cannot edit here: not a git repository'
      })
      expect(claude.received).toHaveLength(0)
      expect(store.runs()).toHaveLength(0)
    },
    undefined,
    undefined,
    { worktrees }
  )
})

test('editing without worktrees fails the job without spawning', async () => {
  const claude = new FakeAdapter('claude', [ok('nope')])
  await withOrch([claude], async (orch, store) => {
    orch.submit('boilerplate', 'change it', undefined, true)
    await orch.idle()
    expect(orch.list()[0]).toMatchObject({
      status: 'failed',
      error: 'cannot edit here: no worktrees'
    })
    expect(claude.received).toHaveLength(0)
    expect(store.runs()).toHaveLength(0)
  })
})

test('a non-editing job is unchanged and does not touch worktrees', async () => {
  const claude = new FakeAdapter('claude', [ok('read')])
  let creates = 0
  let changes = 0
  const worktrees = fakeWorktrees(async () => {
    creates += 1
    throw new Error('should not create')
  })
  await withOrch(
    [claude],
    async (orch) => {
      orch.submit('planning', 'look')
      await orch.idle()
      expect(creates).toBe(0)
      expect(changes).toBe(0)
      expect(claude.cwds).toEqual([CWD])
      expect(claude.options[0]?.edit).toBeUndefined()
      expect(orch.list()[0]?.edit).toBeUndefined()
      expect(orch.list()[0]?.change).toBeUndefined()
    },
    undefined,
    undefined,
    {
      worktrees,
      onChanges: () => {
        changes += 1
      }
    }
  )
})

test('failover of an editing job keeps the folder and adds the handoff line', async () => {
  const claude = new FakeAdapter('claude', [
    {
      events: [
        { kind: 'text', text: 'partial' },
        { kind: 'limit', message: 'usage limit' }
      ]
    }
  ])
  const cursor = new FakeAdapter('cursor', [ok('done')])
  const worktrees = fakeWorktrees(async (_project, jobId) => {
    const id = jobId.slice(0, 8)
    return { id, path: `wt/${id}`, branch: `orch/${id}` }
  })
  await withOrch(
    [claude, cursor],
    async (orch) => {
      orch.submit('planning', 'change it', undefined, true)
      await orch.idle()
      expect(claude.cwds).toEqual(cursor.cwds)
      expect(claude.cwds[0]).toMatch(/^wt\//)
      expect(cursor.received[0]?.prompt).toContain(
        'The files you need are already changed in this folder; continue from them.'
      )
      expect(cursor.options[0]?.edit).toBe(true)
      expect(orch.list()[0]).toMatchObject({ status: 'done', edit: true })
    },
    undefined,
    undefined,
    { worktrees }
  )
})

test('onChanges fires after an editing run of any outcome', async () => {
  const claude = new FakeAdapter('claude', [
    ok('edited'),
    { events: [{ kind: 'result', ok: false, text: 'nope' }] }
  ])
  let calls = 0
  const worktrees = fakeWorktrees(async (_project, jobId) => {
    const id = jobId.slice(0, 8)
    return { id, path: `wt/${id}`, branch: `orch/${id}` }
  })
  await withOrch(
    [claude],
    async (orch) => {
      orch.submit('boilerplate', 'change it', 'claude', true)
      await orch.idle()
      expect(calls).toBe(1)
      orch.submit('boilerplate', 'fail it', 'claude', true)
      await orch.idle()
      expect(calls).toBe(2)
      expect(orch.list()[1]).toMatchObject({ status: 'failed', edit: true })
    },
    undefined,
    undefined,
    {
      worktrees,
      onChanges: () => {
        calls += 1
      }
    }
  )
})

test('cwd as a function is read when each job starts', async () => {
  let folder = 'alpha'
  const claude = new FakeAdapter('claude', [ok('a'), ok('b')])
  await withOrch(
    [claude],
    async (orch) => {
      orch.submit('planning', 'one')
      await orch.idle()
      folder = 'beta'
      orch.submit('planning', 'two')
      await orch.idle()
      expect(claude.cwds).toEqual(['alpha', 'beta'])
    },
    undefined,
    undefined,
    { cwd: () => folder }
  )
})

test('busy is true while any job is queued or running', async () => {
  const held = gate()
  const claude = new FakeAdapter('claude', [{ gate: held.promise, ...ok('a') }, ok('b')])
  await withOrch([claude], async (orch) => {
    expect(orch.busy()).toBe(false)
    orch.submit('planning', 'one')
    orch.submit('planning', 'two')
    expect(orch.busy()).toBe(true)
    held.open()
    await orch.idle()
    expect(orch.busy()).toBe(false)
  })
})

test('a finished job is saved and listed after restore on a new Orchestrator', async () => {
  const store = new Store(':memory:')
  try {
    const claude = new FakeAdapter('claude', [ok('saved output')])
    const orch = new Orchestrator({
      adapters: [claude],
      store,
      rules: loadRules(),
      cwd: '/proj'
    })
    await orch.init()
    orch.submit('planning', 'remember me')
    await orch.idle()
    const saved = store.jobs('/proj', 10)
    expect(saved).toHaveLength(1)
    expect(saved[0]).toMatchObject({
      status: 'done',
      output: 'saved output',
      prompt: 'remember me'
    })

    const again = new Orchestrator({
      adapters: [new FakeAdapter('claude', [])],
      store,
      rules: loadRules(),
      cwd: '/proj'
    })
    await again.init()
    again.restore('/proj')
    expect(again.list()).toHaveLength(1)
    expect(again.list()[0]).toMatchObject({
      status: 'done',
      output: 'saved output',
      prompt: 'remember me'
    })
  } finally {
    store.close()
  }
})

test('a job stored as running comes back failed after restore', async () => {
  await withOrch([new FakeAdapter('claude', [])], async (orch, store) => {
    store.saveJob('/proj', 100, {
      id: 'cut-off',
      type: 'planning',
      prompt: 'halfway',
      provider: 'claude',
      status: 'running',
      output: 'partial',
      failedOver: []
    })
    orch.restore('/proj')
    expect(orch.list()).toEqual([
      expect.objectContaining({
        id: 'cut-off',
        status: 'failed',
        error: 'interrupted when the app closed',
        output: 'partial'
      })
    ])
    expect(store.jobs('/proj', 10)[0]).toMatchObject({
      id: 'cut-off',
      status: 'failed',
      error: 'interrupted when the app closed'
    })
  })
})

test('restore for another project does not show its jobs', async () => {
  await withOrch([new FakeAdapter('claude', [])], async (orch, store) => {
    store.saveJob('/a', 100, {
      id: 'only-a',
      type: 'planning',
      prompt: 'a',
      provider: 'claude',
      status: 'done',
      output: 'ok',
      failedOver: []
    })
    orch.restore('/b')
    expect(orch.list()).toEqual([])
    orch.restore('/a')
    expect(orch.list().map((job) => job.id)).toEqual(['only-a'])
  })
})

test('restore while busy throws', async () => {
  const held = gate()
  const claude = new FakeAdapter('claude', [{ gate: held.promise, ...ok('a') }])
  await withOrch([claude], async (orch) => {
    orch.submit('planning', 'one')
    expect(orch.busy()).toBe(true)
    expect(() => orch.restore(CWD)).toThrow('orchestrator is busy')
    held.open()
    await orch.idle()
  })
})

test('maxParallel 2 runs two jobs and queues the third until a slot frees', async () => {
  const hold = gate()
  const thirdStarted = gate()
  const claude = new FakeAdapter('claude', [
    { gate: hold.promise, ...ok('a') },
    { gate: hold.promise, ...ok('b') },
    { onStart: () => thirdStarted.open(), ...ok('c') }
  ])
  await withOrch(
    [claude],
    async (orch) => {
      orch.submit('planning', 'first')
      orch.submit('planning', 'second')
      orch.submit('planning', 'third')
      expect(claude.received.map((job) => job.prompt)).toEqual(['first', 'second'])
      expect(orch.list().map((job) => job.status)).toEqual(['running', 'running', 'queued'])
      expect(orch.workers()[0]).toMatchObject({ running: 2, queued: 1, busy: true })
      hold.open()
      await thirdStarted.promise
      expect(claude.received.map((job) => job.prompt)).toEqual(['first', 'second', 'third'])
      await orch.idle()
      expect(orch.list().map((job) => job.status)).toEqual(['done', 'done', 'done'])
    },
    undefined,
    undefined,
    { rules: rulesWith(2) }
  )
})

test('jobs start in submission order when a parallel slot frees', async () => {
  const hold = gate()
  const started: string[] = []
  const claude = new FakeAdapter('claude', [
    {
      gate: hold.promise,
      onStart: () => started.push('a'),
      ...ok('a')
    },
    {
      gate: hold.promise,
      onStart: () => started.push('b'),
      ...ok('b')
    },
    { onStart: () => started.push('c'), ...ok('c') },
    { onStart: () => started.push('d'), ...ok('d') }
  ])
  await withOrch(
    [claude],
    async (orch) => {
      orch.submit('planning', 'a')
      orch.submit('planning', 'b')
      orch.submit('planning', 'c')
      orch.submit('planning', 'd')
      expect(started).toEqual(['a', 'b'])
      hold.open()
      await orch.idle()
      expect(started).toEqual(['a', 'b', 'c', 'd'])
    },
    undefined,
    undefined,
    { rules: rulesWith(2) }
  )
})

test('a limit with two running jobs fails over both, once each', async () => {
  const now = 1_800_000_000_000
  const held = gate()
  const stuck = gate()
  const claude = new FakeAdapter('claude', [
    {
      gate: held.promise,
      events: [
        { kind: 'text', text: 'one' },
        { kind: 'limit', message: 'spent', resetsAt: 1_900_000_000 }
      ]
    },
    {
      gate: held.promise,
      pause: stuck.promise,
      events: [{ kind: 'text', text: 'two' }]
    }
  ])
  const cursor = new FakeAdapter('cursor', [ok('moved-a'), ok('moved-b')])
  await withOrch(
    [claude, cursor],
    async (orch) => {
      const first = orch.submit('planning', 'alpha')
      const second = orch.submit('planning', 'beta')
      expect(orch.workers()[0]?.running).toBe(2)
      held.open()
      await orch.idle()
      expect(orch.get(first.id)).toMatchObject({
        status: 'done',
        provider: 'cursor',
        reason: 'failover',
        failedOver: ['claude']
      })
      expect(orch.get(second.id)).toMatchObject({
        status: 'done',
        provider: 'cursor',
        reason: 'failover',
        failedOver: ['claude']
      })
      expect(cursor.received).toHaveLength(2)
    },
    () => now,
    undefined,
    { rules: rulesWith(2) }
  )
})

test('maxParallel 1 queues a second job on the same plan', async () => {
  const held = gate()
  const secondStarted = gate()
  const claude = new FakeAdapter('claude', [
    { gate: held.promise, ...ok('a') },
    { onStart: () => secondStarted.open(), ...ok('b') }
  ])
  await withOrch(
    [claude],
    async (orch) => {
      orch.submit('planning', 'one')
      orch.submit('planning', 'two')
      expect(claude.received.map((job) => job.prompt)).toEqual(['one'])
      expect(orch.list()[1]).toMatchObject({ prompt: 'two', status: 'queued', provider: 'claude' })
      expect(orch.workers()[0]).toMatchObject({ running: 1, queued: 1 })
      held.open()
      await secondStarted.promise
      expect(claude.received.map((job) => job.prompt)).toEqual(['one', 'two'])
      await orch.idle()
    },
    undefined,
    undefined,
    { rules: rulesWith(1) }
  )
})

test('auto routing skips a plan at its parallel limit when the other has a free slot', async () => {
  const held = gate()
  const claude = new FakeAdapter('claude', [{ gate: held.promise, ...ok('a') }])
  const cursor = new FakeAdapter('cursor', [ok('b'), ok('c')])
  await withOrch(
    [claude, cursor],
    async (orch) => {
      orch.submit('planning', 'first')
      expect(orch.list()[0]).toMatchObject({ provider: 'claude', status: 'running' })
      const second = orch.submit('planning', 'second')
      expect(second).toMatchObject({ provider: 'cursor', reason: 'more headroom' })
      held.open()
      await orch.idle()
      expect(cursor.received.map((job) => job.prompt)).toEqual(['second'])
    },
    undefined,
    undefined,
    { rules: rulesWith(1) }
  )
})

test('workers report not-installed and signed-out problems', async () => {
  const claude = new FakeAdapter('claude', [])
  const cursor = new FakeAdapter('cursor', [])
  claude.installed = false
  cursor.signedIn = false
  await withOrch([claude, cursor], async (orch) => {
    expect(orch.workers()[0]).toMatchObject({
      id: 'claude',
      available: false,
      problem: 'not-installed'
    })
    expect(orch.workers()[1]).toMatchObject({
      id: 'cursor',
      available: false,
      problem: 'signed-out'
    })
  })
})

test('recheck picks up and drops plans without disturbing a running job', async () => {
  const held = gate()
  const claude = new FakeAdapter('claude', [{ gate: held.promise, ...ok('a') }])
  const cursor = new FakeAdapter('cursor', [ok('b')])
  cursor.signedIn = false
  await withOrch([claude, cursor], async (orch) => {
    const running = orch.submit('planning', 'keep going')
    expect(running).toMatchObject({ status: 'running', provider: 'claude' })
    expect(orch.workers().map((worker) => [worker.id, worker.available, worker.problem])).toEqual([
      ['claude', true, undefined],
      ['cursor', false, 'signed-out']
    ])

    claude.signedIn = false
    cursor.signedIn = true
    await orch.recheck()
    expect(orch.get(running.id)).toMatchObject({ status: 'running', provider: 'claude' })
    expect(claude.received.map((job) => job.prompt)).toEqual(['keep going'])
    expect(orch.workers().map((worker) => [worker.id, worker.available, worker.problem])).toEqual([
      ['claude', false, 'signed-out'],
      ['cursor', true, undefined]
    ])

    const rejected = orch.submit('planning', 'needs claude', 'claude')
    expect(rejected).toMatchObject({ status: 'failed', error: 'claude is not signed in' })

    const onCursor = orch.submit('planning', 'now cursor', 'cursor')
    expect(onCursor).toMatchObject({ provider: 'cursor' })
    expect(['queued', 'running']).toContain(onCursor.status)

    held.open()
    await orch.idle()
    expect(orch.get(running.id)).toMatchObject({ status: 'done', provider: 'claude' })
    expect(orch.get(onCursor.id)).toMatchObject({ status: 'done', provider: 'cursor' })
  })
})

test('submit to a plan with a problem fails with a readable message', async () => {
  const claude = new FakeAdapter('claude', [ok('a')])
  const cursor = new FakeAdapter('cursor', [])
  cursor.installed = false
  await withOrch([claude, cursor], async (orch) => {
    const missing = orch.submit('planning', 'cursor please', 'cursor')
    expect(missing).toMatchObject({
      status: 'failed',
      error: 'cursor is not signed in',
      provider: 'cursor'
    })
    expect(cursor.received).toHaveLength(0)
    await orch.idle()
  })
})

test('cancel removes a queued job and marks it stopped', async () => {
  const held = gate()
  const claude = new FakeAdapter('claude', [{ gate: held.promise, ...ok('a') }])
  await withOrch(
    [claude],
    async (orch) => {
      orch.submit('planning', 'first')
      const queued = orch.submit('planning', 'second')
      expect(queued).toMatchObject({ status: 'queued', provider: 'claude' })
      const stopped = orch.cancel(queued.id)
      expect(stopped).toMatchObject({ status: 'failed', error: 'stopped', id: queued.id })
      expect(orch.get(queued.id)).toMatchObject({ status: 'failed', error: 'stopped' })
      expect(orch.workers()[0]).toMatchObject({ running: 1, queued: 0 })
      held.open()
      await orch.idle()
      expect(claude.received.map((job) => job.prompt)).toEqual(['first'])
    },
    undefined,
    undefined,
    { rules: rulesWith(1) }
  )
})

test('cancel kills a running job without failover or resting the plan', async () => {
  const held = gate()
  const claude = new FakeAdapter('claude', [
    {
      gate: held.promise,
      events: [
        { kind: 'text', text: 'partial' },
        { kind: 'limit', message: 'should not apply' }
      ]
    }
  ])
  const cursor = new FakeAdapter('cursor', [ok('failover')])
  await withOrch([claude, cursor], async (orch, store) => {
    const job = orch.submit('planning', 'stop me')
    expect(job).toMatchObject({ status: 'running', provider: 'claude' })
    orch.cancel(job.id)
    // Kill must wake the hung run; the gate is never opened.
    await waitFor(() => orch.get(job.id)?.status === 'failed')
    expect(orch.get(job.id)).toMatchObject({
      status: 'failed',
      error: 'stopped',
      provider: 'claude',
      failedOver: []
    })
    expect(store.restingUntil('claude')).toBeNull()
    expect(store.runs().map((run) => run.outcome)).toEqual(['error'])
    expect(cursor.received).toHaveLength(0)
    await orch.idle()
  })
})

test('cancel of one running job with maxParallel 2 starts the next queued job', async () => {
  const hold = gate()
  const thirdStarted = gate()
  const claude = new FakeAdapter('claude', [
    { gate: hold.promise, ...ok('a') },
    { gate: hold.promise, ...ok('b') },
    { onStart: () => thirdStarted.open(), ...ok('c') }
  ])
  await withOrch(
    [claude],
    async (orch) => {
      const first = orch.submit('planning', 'one')
      const second = orch.submit('planning', 'two')
      const queued = orch.submit('planning', 'three')
      expect(first).toMatchObject({ status: 'running' })
      expect(second).toMatchObject({ status: 'running' })
      expect(queued).toMatchObject({ status: 'queued' })
      orch.cancel(first.id)
      await thirdStarted.promise
      expect(orch.get(queued.id)).toMatchObject({ status: 'running', prompt: 'three' })
      await waitFor(() => orch.get(first.id)?.status === 'failed')
      expect(orch.get(first.id)).toMatchObject({ error: 'stopped' })
      hold.open()
      await orch.idle()
      expect(claude.received.map((job) => job.prompt)).toEqual(['one', 'two', 'three'])
    },
    undefined,
    undefined,
    { rules: rulesWith(2) }
  )
})

test('a limit event keeps its message even when the stream is also unreadable', async () => {
  const now = 1_700_000_000_000
  const claude = new FakeAdapter('claude', [
    {
      events: [{ kind: 'limit', message: 'spent', resetsAt: 1_900_000_000 }],
      unreadable: true
    }
  ])
  const cursor = new FakeAdapter('cursor', [ok('moved')])
  await withOrch(
    [claude, cursor],
    async (orch, store) => {
      orch.submit('planning', 'go')
      await orch.idle()
      expect(orch.list()[0]).toMatchObject({
        status: 'done',
        provider: 'cursor',
        failedOver: ['claude']
      })
      expect(store.restingUntil('claude')).toBe(1_900_000_000_000)
      expect(store.runs('claude')[0].outcome).toBe('limit')
    },
    () => now
  )
})

test('unreadable CLI output fails the job with a readable message and does not rest the plan', async () => {
  const claude = new FakeAdapter('claude', [
    { events: [{ kind: 'text', text: '??' }], unreadable: true }
  ])
  const cursor = new FakeAdapter('cursor', [ok('should-not-run')])
  await withOrch([claude, cursor], async (orch, store) => {
    orch.submit('planning', 'go')
    await orch.idle()
    expect(orch.list()[0]).toMatchObject({
      status: 'failed',
      error: unknownFormatMessage('claude', TESTED_VERSIONS.claude),
      provider: 'claude',
      failedOver: []
    })
    expect(store.restingUntil('claude')).toBeNull()
    expect(store.runs().map((run) => run.outcome)).toEqual(['error'])
    expect(cursor.received).toHaveLength(0)
  })
})

test('workers report CLI version and tested status', async () => {
  const claude = new FakeAdapter('claude', [])
  const cursor = new FakeAdapter('cursor', [])
  claude.cliVersion = '2.2.0'
  cursor.cliVersion = null
  await withOrch([claude, cursor], async (orch) => {
    expect(orch.workers()[0]).toMatchObject({
      id: 'claude',
      version: '2.2.0',
      versionStatus: 'newer'
    })
    expect(orch.workers()[1]).toMatchObject({
      id: 'cursor',
      version: null,
      versionStatus: 'unknown'
    })
  })
})

test('cancel of a finished or unknown job throws', async () => {
  const claude = new FakeAdapter('claude', [ok('a')])
  await withOrch([claude], async (orch) => {
    const job = orch.submit('planning', 'done soon')
    await orch.idle()
    expect(orch.get(job.id)?.status).toBe('done')
    expect(() => orch.cancel(job.id)).toThrow('job is not running')
    expect(() => orch.cancel('missing-id')).toThrow('job is not running')
  })
})
