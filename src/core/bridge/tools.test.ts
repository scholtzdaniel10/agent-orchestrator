import { expect, test } from 'vitest'
import type { AgentEvent, ProviderAdapter, ProviderId, RunHandle } from '../types'
import type { Worktrees } from '../worktrees'
import { loadRules, Orchestrator, Store } from '../router'
import { createOrchestratorTools, startBridge, type BridgeTool } from './index'

interface Script {
  events?: AgentEvent[]
  gate?: Promise<void>
}

class FakeAdapter implements ProviderAdapter {
  readonly id: ProviderId
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

  run(): RunHandle {
    const script = this.scripts.shift()
    if (!script) throw new Error(`no script left for ${this.id}`)
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
      exit: Promise.resolve({ code: 0, stderr: '' }),
      kill(): void {
        killed = true
      }
    }
  }

  parseEvent(): AgentEvent | null {
    return null
  }

  isLimitError(): boolean {
    return false
  }
}

function ok(text: string): Script {
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

async function withOrch(
  adapters: ProviderAdapter[],
  fn: (orch: Orchestrator, store: Store) => Promise<void>,
  now?: () => number
): Promise<void> {
  const store = new Store(':memory:')
  const orch = new Orchestrator({
    adapters,
    store,
    rules: loadRules(),
    cwd: '.',
    now
  })
  try {
    await orch.init()
    await fn(orch, store)
  } finally {
    store.close()
  }
}

function tool(tools: BridgeTool[], name: string): BridgeTool {
  const found = tools.find((item) => item.name === name)
  if (!found) throw new Error(`missing tool ${name}`)
  return found
}

test('send_job returns the job id and provider and rejects bad input', async () => {
  const held = gate()
  const claude = new FakeAdapter('claude', [{ gate: held.promise, ...ok('done') }, ok('next')])
  await withOrch([claude], async (orch) => {
    const send = tool(createOrchestratorTools(orch), 'send_job')
    await expect(send.handler({ type: 'ship', prompt: 'x' })).rejects.toThrow(/type/)
    await expect(send.handler({ type: 'planning', prompt: '   ' })).rejects.toThrow(/prompt/)
    await expect(send.handler({ type: 'planning', prompt: '' })).rejects.toThrow(/prompt/)
    await expect(send.handler({ type: 'planning', prompt: 'x'.repeat(20_001) })).rejects.toThrow(
      /20000/
    )

    try {
      const first = (await send.handler({ type: 'planning', prompt: 'hello' })) as {
        id: string
        provider: string
        status: string
      }
      expect(first).toMatchObject({ provider: 'claude', status: 'running' })
      expect(first.id).toEqual(expect.any(String))
      expect(Object.keys(first).sort()).toEqual(['id', 'provider', 'status'])
      expect(orch.get(first.id)?.prompt).toBe('hello')

      const second = (await send.handler({ type: 'planning', prompt: 'queued-job' })) as {
        status: string
        provider: string
      }
      expect(second).toMatchObject({ provider: 'claude', status: 'queued' })
    } finally {
      held.open()
      await orch.idle()
    }
  })
})

test('get_status lists jobs oldest first without output, and throws for an unknown id', async () => {
  const claude = new FakeAdapter('claude', [ok('a'), ok('b')])
  await withOrch([claude], async (orch) => {
    const tools = createOrchestratorTools(orch)
    const send = tool(tools, 'send_job')
    const long = `${'p'.repeat(120)}MORE`
    const first = (await send.handler({ type: 'planning', prompt: long })) as { id: string }
    await send.handler({ type: 'debugging', prompt: 'short' })
    await orch.idle()

    const status = tool(tools, 'get_status')
    const all = (await status.handler({})) as Record<string, unknown>[]
    expect(all.map((job) => job.prompt)).toEqual(['p'.repeat(120), 'short'])
    expect(all[0]).not.toHaveProperty('output')
    expect(all[1]).not.toHaveProperty('output')
    expect(Object.keys(all[0]).sort()).toEqual([
      'failed_over',
      'id',
      'prompt',
      'provider',
      'status',
      'type'
    ])
    expect(all[0]).toMatchObject({
      id: first.id,
      type: 'planning',
      provider: 'claude',
      failed_over: []
    })

    const one = await status.handler({ id: first.id })
    expect(one).toMatchObject({ id: first.id, prompt: 'p'.repeat(120) })
    expect(one).not.toHaveProperty('output')
    await expect(status.handler({ id: 'missing' })).rejects.toThrow('Unknown job id: missing')
  })
})

test('get_result waits until the job finishes and returns its output', async () => {
  const held = gate()
  const claude = new FakeAdapter('claude', [{ gate: held.promise, ...ok('finished-text') }])
  await withOrch([claude], async (orch) => {
    const tools = createOrchestratorTools(orch)
    const sent = (await tool(tools, 'send_job').handler({
      type: 'review',
      prompt: 'look'
    })) as { id: string }
    let returned = false
    const pending = Promise.resolve(
      tool(tools, 'get_result').handler({ id: sent.id, wait_seconds: 2 })
    ).then((value) => {
      returned = true
      return value
    })
    await new Promise((resolve) => setTimeout(resolve, 30))
    expect(returned).toBe(false)
    held.open()
    await expect(pending).resolves.toEqual({
      id: sent.id,
      status: 'done',
      provider: 'claude',
      failed_over: [],
      output: 'finished-text',
      truncated: false,
      edit: false,
      change: null
    })
    await orch.idle()
  })
})

test('get_result returns running when the wait is zero or ends first', async () => {
  const held = gate()
  const claude = new FakeAdapter('claude', [{ gate: held.promise, ...ok('later') }])
  await withOrch([claude], async (orch) => {
    const tools = createOrchestratorTools(orch)
    const sent = (await tool(tools, 'send_job').handler({
      type: 'planning',
      prompt: 'hold'
    })) as { id: string }
    const getResult = tool(tools, 'get_result')
    try {
      const immediate = await getResult.handler({ id: sent.id, wait_seconds: 0 })
      expect(immediate).toMatchObject({
        id: sent.id,
        status: 'running',
        provider: 'claude',
        failed_over: [],
        output: '',
        truncated: false
      })
      expect(immediate).not.toHaveProperty('error')

      const early = await getResult.handler({ id: sent.id, wait_seconds: 0.05 })
      expect(early).toMatchObject({ status: 'running', output: '' })
    } finally {
      held.open()
      await orch.idle()
    }
  })
})

test('get_result keeps the last 6000 characters and reports a failed job', async () => {
  const text = `${'a'.repeat(7000)}END`
  const claude = new FakeAdapter('claude', [
    ok(text),
    {
      events: [{ kind: 'result', ok: false, text: 'nope', costUsd: 1, tokens: 1 }]
    }
  ])
  await withOrch([claude], async (orch) => {
    const tools = createOrchestratorTools(orch)
    const send = tool(tools, 'send_job')
    const getResult = tool(tools, 'get_result')
    const big = (await send.handler({ type: 'planning', prompt: 'big' })) as { id: string }
    const cut = (await getResult.handler({ id: big.id, wait_seconds: 2 })) as {
      output: string
      truncated: boolean
      status: string
    }
    expect(cut.status).toBe('done')
    expect(cut.truncated).toBe(true)
    expect(cut.output).toBe(text.slice(-6000))
    expect(cut.output.endsWith('END')).toBe(true)
    expect(cut.output.startsWith('a'.repeat(7000))).toBe(false)

    const failed = (await send.handler({ type: 'planning', prompt: 'bad' })) as { id: string }
    const result = await getResult.handler({ id: failed.id, wait_seconds: 2 })
    expect(result).toMatchObject({ status: 'failed', error: 'nope', provider: 'claude' })

    await expect(getResult.handler({ id: 'missing' })).rejects.toThrow('Unknown job id: missing')
  })
})

test('list_workers reports availability, headroom, rest, busy, and queued', async () => {
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
      const tools = createOrchestratorTools(orch)
      try {
        await tool(tools, 'send_job').handler({ type: 'planning', prompt: 'one' })
        await tool(tools, 'send_job').handler({ type: 'planning', prompt: 'two' })
        const listed = await tool(tools, 'list_workers').handler({})
        expect(listed).toEqual([
          {
            provider: 'claude',
            available: true,
            headroom: 1,
            resting_until: null,
            busy: true,
            queued: 1
          },
          {
            provider: 'cursor',
            available: false,
            headroom: 0,
            resting_until: new Date(now + 60_000).toISOString(),
            busy: false,
            queued: 0
          }
        ])
      } finally {
        held.open()
        await orch.idle()
      }
    },
    () => now
  )
})

test('send_job and get_result work over the bridge', async () => {
  const claude = new FakeAdapter('claude', [ok('bridge-output')])
  await withOrch([claude], async (orch) => {
    const bridge = await startBridge(createOrchestratorTools(orch))
    try {
      const sent = await callTool(bridge.info.url, bridge.info.token, 'send_job', {
        type: 'planning',
        prompt: 'over http'
      })
      const result = await callTool(bridge.info.url, bridge.info.token, 'get_result', {
        id: sent.id,
        wait_seconds: 2
      })
      expect(result).toMatchObject({
        id: sent.id,
        status: 'done',
        provider: 'claude',
        output: 'bridge-output',
        truncated: false
      })
    } finally {
      await bridge.close()
      await orch.idle()
    }
  })
})

async function callTool(
  url: string,
  token: string,
  name: string,
  args: Record<string, unknown>
): Promise<Record<string, unknown>> {
  const res = await fetch(url, {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${token}`,
      'Content-Type': 'application/json'
    },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id: 1,
      method: 'tools/call',
      params: { name, arguments: args }
    })
  })
  expect(res.status).toBe(200)
  const body = (await res.json()) as {
    result?: { content: { text: string }[]; isError?: boolean }
    error?: { message: string }
  }
  expect(body.error).toBeUndefined()
  expect(body.result?.isError).toBeUndefined()
  return JSON.parse(body.result?.content[0]?.text ?? 'null') as Record<string, unknown>
}

test('send_job passes edit through and get_result reports the change', async () => {
  const claude = new FakeAdapter('claude', [ok('edited')])
  const store = new Store(':memory:')
  const worktrees = {
    async create(_project: string, jobId: string) {
      const id = jobId.slice(0, 8)
      return { id, path: 'wt', branch: `orch/${id}` }
    }
  } as unknown as Worktrees
  const orch = new Orchestrator({
    adapters: [claude],
    store,
    rules: loadRules(),
    cwd: '.',
    worktrees
  })
  try {
    await orch.init()
    const tools = createOrchestratorTools(orch)
    const send = tool(tools, 'send_job')
    expect(send.description).toContain(
      'Set edit to true only when the job has to change files; its changes go to a separate worktree and wait for the user to review and merge them.'
    )
    await expect(send.handler({ type: 'planning', prompt: 'x', edit: 'yes' })).rejects.toThrow(
      'edit must be a boolean'
    )
    const sent = (await send.handler({
      type: 'boilerplate',
      prompt: 'change it',
      edit: true
    })) as { id: string }
    await orch.idle()
    const result = await tool(tools, 'get_result').handler({ id: sent.id, wait_seconds: 1 })
    expect(result).toMatchObject({
      status: 'done',
      edit: true,
      change: sent.id.slice(0, 8)
    })
  } finally {
    store.close()
  }
})
