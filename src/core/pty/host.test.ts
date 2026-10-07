import { afterEach, expect, test, vi } from 'vitest'
import { Store } from '../router/store'
import type { ProviderId, TerminalInfo } from '../types'
import { PtyHost, terminalEnv, type Launch, type PtyProcess, type SpawnPty } from './host'

const SNAPSHOT_MAX = 256_000

afterEach(() => {
  vi.useRealTimers()
})

class FakePty implements PtyProcess {
  pid: number
  readonly written: string[] = []
  readonly resized: Array<{ cols: number; rows: number }> = []
  resizeError = false
  private readonly dataListeners: Array<(data: string) => void> = []
  private readonly exitListeners: Array<(e: { exitCode: number }) => void> = []

  constructor(pid: number) {
    this.pid = pid
  }

  onData(cb: (data: string) => void): unknown {
    this.dataListeners.push(cb)
    return undefined
  }

  onExit(cb: (e: { exitCode: number }) => void): unknown {
    this.exitListeners.push(cb)
    return undefined
  }

  write(data: string): void {
    this.written.push(data)
  }

  resize(cols: number, rows: number): void {
    if (this.resizeError) throw new Error('resize failed')
    this.resized.push({ cols, rows })
  }

  kill(): void {
    return undefined
  }

  push(data: string): void {
    for (const cb of [...this.dataListeners]) cb(data)
  }

  exit(code: number): void {
    for (const cb of [...this.exitListeners]) cb({ exitCode: code })
  }
}

interface SpawnCall {
  command: string
  args: string[]
  opts: { name: string; cols: number; rows: number; cwd: string; env: Record<string, string> }
}

function makeHost(partial?: {
  cwd?: string | (() => string)
  maxTerminals?: number
  modelFor?: (provider: ProviderId) => string | undefined
  store?: Store
  now?: () => number
  launch?: (provider: ProviderId, model: string | undefined) => Launch
  killTree?: (proc: PtyProcess) => void
  spawn?: SpawnPty
}): { host: PtyHost; ptys: FakePty[]; calls: SpawnCall[] } {
  const ptys: FakePty[] = []
  const calls: SpawnCall[] = []
  const host = new PtyHost({
    cwd: partial?.cwd ?? 'work',
    maxTerminals: partial?.maxTerminals,
    modelFor: partial?.modelFor,
    store: partial?.store,
    now: partial?.now,
    launch:
      partial?.launch ?? ((provider, model) => ({ command: provider, args: model ? [model] : [] })),
    killTree: partial?.killTree ?? ((proc) => proc.kill()),
    spawn:
      partial?.spawn ??
      ((command, args, opts) => {
        calls.push({ command, args, opts })
        const pty = new FakePty(1000 + ptys.length)
        ptys.push(pty)
        return pty
      })
  })
  return { host, ptys, calls }
}

test('open returns running terminals, passes the model, and clamps the spawn', () => {
  const cwd = 'work'
  const seen: Array<{ provider: ProviderId; model: string | undefined }> = []
  const { host, calls } = makeHost({
    cwd,
    now: () => 5_000,
    modelFor: (provider) => (provider === 'claude' ? 'opus' : 'fast'),
    launch: (provider, model) => {
      seen.push({ provider, model })
      return {
        command: provider === 'claude' ? 'claude.exe' : 'node.exe',
        args: [
          ...(provider === 'cursor' ? ['index.js'] : []),
          ...(model ? ['--model', model] : [])
        ],
        env: { PATH: 'bin', NO_COLOR: '1', FORCE_COLOR: '0', KEEP: 'yes' }
      }
    }
  })
  const updates: Array<{ title: string; removed: boolean }> = []
  host.onUpdate((info, removed) => updates.push({ title: info.title, removed }))

  const first = host.open('claude', 80.9, 24.2)
  const second = host.open('claude', 0, 9_000)
  const third = host.open('cursor', Number.NaN, 10)

  expect(first).toMatchObject({
    provider: 'claude',
    title: 'claude 1',
    status: 'running',
    exitCode: null,
    model: 'opus',
    startedAt: 5_000
  })
  expect(first.id).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i)
  expect(second).toMatchObject({ provider: 'claude', title: 'claude 2', model: 'opus' })
  expect(third).toMatchObject({ provider: 'cursor', title: 'cursor 1', model: 'fast' })
  expect(seen).toEqual([
    { provider: 'claude', model: 'opus' },
    { provider: 'claude', model: 'opus' },
    { provider: 'cursor', model: 'fast' }
  ])
  expect(calls.map((call) => ({ command: call.command, args: call.args, ...call.opts }))).toEqual([
    {
      command: 'claude.exe',
      args: ['--model', 'opus'],
      name: 'xterm-256color',
      cols: 80,
      rows: 24,
      cwd,
      env: { PATH: 'bin', KEEP: 'yes', COLORTERM: 'truecolor' }
    },
    {
      command: 'claude.exe',
      args: ['--model', 'opus'],
      name: 'xterm-256color',
      cols: 2,
      rows: 300,
      cwd,
      env: { PATH: 'bin', KEEP: 'yes', COLORTERM: 'truecolor' }
    },
    {
      command: 'node.exe',
      args: ['index.js', '--model', 'fast'],
      name: 'xterm-256color',
      cols: 100,
      rows: 30,
      cwd,
      env: { PATH: 'bin', KEEP: 'yes', COLORTERM: 'truecolor' }
    }
  ])
  const listed = host.list()
  expect(listed.map((info) => info.title)).toEqual(['claude 1', 'claude 2', 'cursor 1'])
  listed[0].title = 'mutated'
  expect(host.list()[0].title).toBe('claude 1')
  expect(host.list()[0]).not.toBe(listed[0])
  expect(updates).toEqual([
    { title: 'claude 1', removed: false },
    { title: 'claude 2', removed: false },
    { title: 'cursor 1', removed: false }
  ])
})

test('cwd is read when a terminal opens', () => {
  let dir = 'one'
  const { host, calls } = makeHost({ cwd: () => dir })
  host.open('claude', 80, 24)
  dir = 'two'
  host.open('cursor', 80, 24)
  expect(calls.map((call) => call.opts.cwd)).toEqual(['one', 'two'])
})

test('open throws at the terminal limit', () => {
  const { host, calls } = makeHost()
  for (let i = 0; i < 8; i++) host.open(i % 2 === 0 ? 'claude' : 'cursor', 80, 24)
  expect(host.list()).toHaveLength(8)
  expect(() => host.open('claude', 80, 24)).toThrow('too many terminals')
  expect(host.list()).toHaveLength(8)
  expect(calls).toHaveLength(8)
})

test('a throwing spawn names the provider and registers nothing', () => {
  let fail = true
  const ptys: FakePty[] = []
  const host = new PtyHost({
    cwd: 'work',
    launch: (provider) => ({ command: provider, args: [] }),
    spawn: () => {
      if (fail) throw new Error('CreateProcess failed')
      const pty = new FakePty(7)
      ptys.push(pty)
      return pty
    }
  })
  expect(() => host.open('claude', 40, 12)).toThrow('could not start claude: CreateProcess failed')
  expect(() => host.open('cursor', 40, 12)).toThrow('could not start cursor: CreateProcess failed')
  expect(host.list()).toEqual([])
  fail = false
  expect(host.open('claude', 40, 12).title).toBe('claude 1')
  expect(host.list()).toHaveLength(1)
  expect(ptys).toHaveLength(1)
})

test('pty data is coalesced into one flush and the snapshot keeps the tail', () => {
  vi.useFakeTimers()
  const { host, ptys } = makeHost()
  const info = host.open('claude', 80, 24)
  const chunks: string[] = []
  host.onData((id, data) => chunks.push(`${id}:${data}`))
  const pty = ptys[0]
  pty.push('a')
  pty.push('b')
  expect(chunks).toEqual([])
  expect(host.snapshot(info.id)).toBe('ab')
  vi.advanceTimersByTime(11)
  expect(chunks).toEqual([])
  vi.advanceTimersByTime(1)
  expect(chunks).toEqual([`${info.id}:ab`])
  pty.push('c')
  vi.advanceTimersByTime(12)
  expect(chunks).toEqual([`${info.id}:ab`, `${info.id}:c`])

  const capped = host.open('cursor', 80, 24)
  ptys[1].push('a'.repeat(SNAPSHOT_MAX))
  ptys[1].push('xyz')
  expect(host.snapshot(capped.id)).toBe(`${'a'.repeat(SNAPSHOT_MAX - 3)}xyz`)
  expect(host.snapshot(capped.id)).toHaveLength(SNAPSHOT_MAX)
  expect(host.snapshot('missing')).toBe('')
})

test('write and resize reach a running pty and are ignored otherwise', () => {
  const { host, ptys } = makeHost()
  const info = host.open('claude', 80, 24)
  const pty = ptys[0]
  host.write(info.id, 'hi')
  host.resize(info.id, 40.8, 12)
  const max = 'x'.repeat(65_536)
  host.write(info.id, max)
  host.write(info.id, 'y'.repeat(65_537))
  host.write(info.id, '')
  host.write(info.id, 12 as unknown as string)
  host.write('missing', 'nope')
  host.resize('missing', 10, 10)
  host.resize(info.id, 1, 9_999)
  host.resize(info.id, Number.POSITIVE_INFINITY, 4)
  pty.resizeError = true
  expect(() => host.resize(info.id, 20, 10)).not.toThrow()
  expect(pty.written).toEqual(['hi', max])
  expect(pty.resized).toEqual([
    { cols: 40, rows: 12 },
    { cols: 2, rows: 300 },
    { cols: 100, rows: 30 }
  ])
  pty.exit(0)
  host.write(info.id, 'late')
  host.resize(info.id, 20, 10)
  expect(pty.written).toEqual(['hi', max])
  expect(pty.resized).toHaveLength(3)
})

test('exit flushes pending output and writes one terminal run', () => {
  vi.useFakeTimers()
  const store = new Store(':memory:')
  try {
    let now = 10_000
    const { host, ptys } = makeHost({ store, now: () => now })
    const order: string[] = []
    host.onData(() => order.push('data'))
    host.onUpdate((info) => order.push(info.status))
    const ok = host.open('claude', 80, 24)
    expect(order).toEqual(['running'])
    ptys[0].push('hi')
    expect(host.snapshot(ok.id)).toBe('hi')
    expect(order).toEqual(['running'])
    now = 12_500
    ptys[0].exit(0)
    expect(order).toEqual(['running', 'data', 'exited'])
    expect(host.list()[0]).toMatchObject({ status: 'exited', exitCode: 0, id: ok.id })

    now = 13_000
    const bad = host.open('cursor', 80, 24)
    now = 13_400
    ptys[1].exit(3)
    const runs = store.runs()
    expect(runs).toHaveLength(2)
    expect(runs[0]).toMatchObject({
      job_id: ok.id,
      provider: 'claude',
      job_type: 'terminal',
      started_at: 10_000,
      duration_ms: 2_500,
      cost_usd: null,
      tokens: null,
      utilization: null,
      outcome: 'ok',
      session_id: null
    })
    expect(runs[1]).toMatchObject({
      job_id: bad.id,
      provider: 'cursor',
      job_type: 'terminal',
      started_at: 13_000,
      duration_ms: 400,
      cost_usd: null,
      tokens: null,
      utilization: null,
      outcome: 'error',
      session_id: null
    })
    ptys[0].exit(0)
    ptys[1].exit(1)
    expect(store.runs()).toHaveLength(2)
  } finally {
    store.close()
  }
})

test('close kills the tree, removes the terminal, and still logs one ok row', () => {
  const store = new Store(':memory:')
  try {
    const killed: PtyProcess[] = []
    const { host, ptys } = makeHost({
      store,
      now: () => 100,
      killTree: (proc) => {
        killed.push(proc)
      }
    })
    const removed: TerminalInfo[] = []
    const info = host.open('claude', 80, 24)
    host.onUpdate((next, gone) => {
      if (gone) removed.push(next)
    })
    host.close(info.id)
    expect(killed).toEqual([ptys[0]])
    expect(host.list()).toEqual([])
    expect(removed.map((next) => next.id)).toEqual([info.id])
    expect(removed[0].status).toBe('running')
    ptys[0].exit(1)
    expect(store.runs()).toHaveLength(1)
    expect(store.runs()[0]).toMatchObject({
      job_id: info.id,
      provider: 'claude',
      job_type: 'terminal',
      outcome: 'ok',
      cost_usd: null,
      tokens: null,
      utilization: null,
      session_id: null
    })
    host.close(info.id)
    host.close('missing')
    expect(killed).toHaveLength(1)
    expect(store.runs()).toHaveLength(1)

    host.open('cursor', 80, 24)
    host.open('claude', 80, 24)
    host.closeAll()
    expect(host.list()).toEqual([])
    expect(killed).toHaveLength(3)
    ptys[1].exit(9)
    ptys[2].exit(0)
    const runs = store.runs()
    expect(runs).toHaveLength(3)
    expect(runs.every((row) => row.outcome === 'ok' && row.job_type === 'terminal')).toBe(true)
  } finally {
    store.close()
  }
})

test('terminalEnv drops no-colour variables and sets COLORTERM', () => {
  expect(
    terminalEnv({
      PATH: 'bin',
      NO_COLOR: '1',
      no_color: '1',
      FORCE_COLOR: '0',
      CLICOLOR: '0',
      SKIP: undefined,
      TERM: 'dumb'
    })
  ).toEqual({ PATH: 'bin', TERM: 'dumb', COLORTERM: 'truecolor' })
  expect(terminalEnv({ FORCE_COLOR: '1', CLICOLOR: '1' })).toEqual({
    FORCE_COLOR: '1',
    CLICOLOR: '1',
    COLORTERM: 'truecolor'
  })
  expect(terminalEnv({ COLORTERM: '8bit' }).COLORTERM).toBe('truecolor')
})

test('a throwing listener does not stop the others', () => {
  vi.useFakeTimers()
  const error = vi.spyOn(console, 'error').mockImplementation(() => {})
  try {
    const { host, ptys } = makeHost()
    const seen: string[] = []
    host.onData(() => {
      throw new Error('data listener')
    })
    host.onData((_id, data) => seen.push(data))
    const titles: string[] = []
    host.onUpdate(() => {
      throw new Error('update listener')
    })
    host.onUpdate((info) => titles.push(info.title))
    const info = host.open('claude', 80, 24)
    ptys[0].push('q')
    vi.advanceTimersByTime(12)
    expect(seen).toEqual(['q'])
    expect(titles).toEqual([info.title])
    expect(error).toHaveBeenCalled()
  } finally {
    error.mockRestore()
  }
})

test('a real pty echoes a line and exits', async () => {
  const host = new PtyHost({
    cwd: process.cwd(),
    launch: () => ({
      command: process.execPath,
      args: [
        '-e',
        "process.stdin.once('data', d => { console.log('got:' + String(d).trim()); process.exit(0) }); console.log('ready')"
      ]
    })
  })
  const exits: TerminalInfo[] = []
  host.onUpdate((info, removed) => {
    if (!removed && info.status === 'exited') exits.push({ ...info })
  })
  const info = host.open('claude', 80, 24)
  try {
    const ready = await waitUntil(() => host.snapshot(info.id).includes('ready'), 15_000)
    expect(ready, host.snapshot(info.id).slice(-800)).toBe(true)
    host.write(info.id, 'ping\r')
    const done = await waitUntil(
      () => host.snapshot(info.id).includes('got:ping') && exits.some((row) => row.exitCode === 0),
      15_000
    )
    expect(done, host.snapshot(info.id).slice(-800)).toBe(true)
    expect(
      exits.some((row) => row.id === info.id && row.status === 'exited' && row.exitCode === 0)
    ).toBe(true)
  } finally {
    host.closeAll()
  }
}, 20_000)

function waitUntil(pred: () => boolean, ms: number): Promise<boolean> {
  const start = Date.now()
  return new Promise((resolve) => {
    const tick = (): void => {
      if (pred()) {
        resolve(true)
        return
      }
      if (Date.now() - start >= ms) {
        resolve(false)
        return
      }
      setTimeout(tick, 30)
    }
    tick()
  })
}
