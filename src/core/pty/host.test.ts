import { afterEach, expect, test, vi } from 'vitest'
import { Store, TERMINAL_SCROLLBACK_CAP } from '../router/store'
import type { ProviderId, TerminalInfo } from '../types'
import {
  PtyHost,
  terminalEnv,
  type Launch,
  type LaunchOpts,
  type PtyProcess,
  type SpawnPty
} from './host'

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
  launch?: (provider: ProviderId, opts: LaunchOpts) => Launch
  createChat?: () => Promise<string>
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
    createChat: partial?.createChat,
    launch:
      partial?.launch ??
      ((provider, opts) => ({
        command: provider,
        args: [
          ...(opts.resume ? ['--resume', opts.resume] : []),
          ...(opts.sessionId ? ['--session-id', opts.sessionId] : []),
          ...(opts.model ? ['--model', opts.model] : [])
        ]
      })),
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

test('open returns running terminals, passes the model, and clamps the spawn', async () => {
  const cwd = 'work'
  const seen: Array<{ provider: ProviderId; model: string | undefined }> = []
  const { host, calls } = makeHost({
    cwd,
    now: () => 5_000,
    modelFor: (provider) => (provider === 'claude' ? 'opus' : 'fast'),
    launch: (provider, opts) => {
      seen.push({ provider, model: opts.model })
      return {
        command: provider === 'claude' ? 'claude.exe' : 'node.exe',
        args: [
          ...(provider === 'cursor' ? ['index.js'] : []),
          ...(opts.model ? ['--model', opts.model] : [])
        ],
        env: { PATH: 'bin', NO_COLOR: '1', FORCE_COLOR: '0', KEEP: 'yes' }
      }
    }
  })
  const updates: Array<{ title: string; removed: boolean }> = []
  host.onUpdate((info, removed) => updates.push({ title: info.title, removed }))

  const first = await host.open('claude', 80.9, 24.2)
  const second = await host.open('claude', 0, 9_000)
  const third = await host.open('cursor', Number.NaN, 10)

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

test('cwd is read when a terminal opens', async () => {
  let dir = 'one'
  const { host, calls } = makeHost({ cwd: () => dir })
  await host.open('claude', 80, 24)
  dir = 'two'
  await host.open('cursor', 80, 24)
  expect(calls.map((call) => call.opts.cwd)).toEqual(['one', 'two'])
})

test('open throws at the terminal limit', async () => {
  const { host, calls } = makeHost()
  for (let i = 0; i < 8; i++) await host.open(i % 2 === 0 ? 'claude' : 'cursor', 80, 24)
  expect(host.list()).toHaveLength(8)
  await expect(host.open('claude', 80, 24)).rejects.toThrow('too many terminals')
  expect(host.list()).toHaveLength(8)
  expect(calls).toHaveLength(8)
})

test('a throwing spawn names the provider and registers nothing', async () => {
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
  await expect(host.open('claude', 40, 12)).rejects.toThrow(
    'could not start claude: CreateProcess failed'
  )
  await expect(host.open('cursor', 40, 12)).rejects.toThrow(
    'could not start cursor: CreateProcess failed'
  )
  expect(host.list()).toEqual([])
  fail = false
  expect((await host.open('claude', 40, 12)).title).toBe('claude 1')
  expect(host.list()).toHaveLength(1)
  expect(ptys).toHaveLength(1)
})

test('pty data is coalesced into one flush and the snapshot keeps the tail', async () => {
  vi.useFakeTimers()
  const { host, ptys } = makeHost()
  const info = await host.open('claude', 80, 24)
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

  const capped = await host.open('cursor', 80, 24)
  ptys[1].push('a'.repeat(SNAPSHOT_MAX))
  ptys[1].push('xyz')
  expect(host.snapshot(capped.id)).toBe(`${'a'.repeat(SNAPSHOT_MAX - 3)}xyz`)
  expect(host.snapshot(capped.id)).toHaveLength(SNAPSHOT_MAX)
  expect(host.snapshot('missing')).toBe('')
})

test('write and resize reach a running pty and are ignored otherwise', async () => {
  const { host, ptys } = makeHost()
  const info = await host.open('claude', 80, 24)
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

test('exit flushes pending output and writes one terminal run', async () => {
  vi.useFakeTimers()
  const store = new Store(':memory:')
  try {
    let now = 10_000
    const { host, ptys } = makeHost({ store, now: () => now })
    const order: string[] = []
    host.onData(() => order.push('data'))
    host.onUpdate((info) => order.push(info.status))
    const ok = await host.open('claude', 80, 24)
    expect(order).toEqual(['running'])
    ptys[0].push('hi')
    expect(host.snapshot(ok.id)).toBe('hi')
    expect(order).toEqual(['running'])
    now = 12_500
    ptys[0].exit(0)
    expect(order).toEqual(['running', 'data', 'exited'])
    expect(host.list()[0]).toMatchObject({ status: 'exited', exitCode: 0, id: ok.id })

    now = 13_000
    const bad = await host.open('cursor', 80, 24)
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

test('close kills the tree, removes the terminal, and still logs one ok row', async () => {
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
    const info = await host.open('claude', 80, 24)
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

    await host.open('cursor', 80, 24)
    await host.open('claude', 80, 24)
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

test('a throwing listener does not stop the others', async () => {
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
    const info = await host.open('claude', 80, 24)
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
  const info = await host.open('claude', 80, 24)
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

const SESSION_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i
const RESTORE_MARK = '\r\n\x1b[2m— restored session —\x1b[0m\r\n'
const FRESH_MARK = '\r\n\x1b[2m— could not resume; started a new session —\x1b[0m\r\n'

function putTerminal(
  store: Store,
  partial?: {
    id?: string
    project?: string
    provider?: ProviderId
    model?: string | null
    session_id?: string | null
    title?: string
    scrollback?: string
  }
): string {
  const id = partial?.id ?? 'term-1'
  store.saveTerminal({
    id,
    project: partial?.project ?? 'work',
    provider: partial?.provider ?? 'claude',
    model: partial?.model === undefined ? 'opus' : partial.model,
    session_id: partial?.session_id === undefined ? 'old-session' : partial.session_id,
    title: partial?.title ?? 'claude 1',
    created_at: 1,
    updated_at: 1,
    scrollback: partial?.scrollback ?? 'old'
  })
  return id
}

async function flushMicrotasks(): Promise<void> {
  for (let i = 0; i < 8; i++) await Promise.resolve()
}

test('claude open passes --session-id', async () => {
  const { host, calls } = makeHost()
  await host.open('claude', 80, 24)
  expect(calls[0].args[0]).toBe('--session-id')
  expect(calls[0].args[1]).toMatch(SESSION_ID)
})

test('cursor open calls create-chat then passes --resume', async () => {
  let created = 0
  const { host, calls } = makeHost({
    createChat: async () => {
      created += 1
      return 'chat-abc-12345678'
    }
  })
  await host.open('cursor', 80, 24)
  expect(created).toBe(1)
  expect(calls[0].args).toEqual(['--resume', 'chat-abc-12345678'])
})

test('create-chat failure still opens a terminal', async () => {
  const { host, calls } = makeHost({
    createChat: async () => {
      throw new Error('offline')
    }
  })
  const info = await host.open('cursor', 80, 24)
  expect(info.status).toBe('running')
  expect(calls).toHaveLength(1)
  expect(calls[0].args).toEqual([])

  const empty = makeHost({ createChat: async () => 'not an id' })
  await empty.host.open('cursor', 80, 24)
  expect(empty.calls[0].args).toEqual([])
})

test('a row is written on open', async () => {
  const store = new Store(':memory:')
  try {
    const { host } = makeHost({
      store,
      cwd: '/proj',
      now: () => 50,
      modelFor: () => 'opus',
      createChat: async () => 'cursor-chat-0001'
    })
    const claude = await host.open('claude', 80, 24)
    const cursor = await host.open('cursor', 80, 24)
    const rows = store.terminals('/proj')
    expect(rows).toHaveLength(2)
    expect(rows[0]).toMatchObject({
      id: claude.id,
      project: '/proj',
      provider: 'claude',
      model: 'opus',
      title: 'claude 1',
      created_at: 50,
      scrollback: ''
    })
    expect(rows[0].session_id).toMatch(SESSION_ID)
    expect(rows[1]).toMatchObject({
      id: cursor.id,
      provider: 'cursor',
      session_id: 'cursor-chat-0001',
      title: 'cursor 1'
    })
  } finally {
    store.close()
  }
})

test('scrollback flush is throttled and happens on exit', async () => {
  vi.useFakeTimers()
  const store = new Store(':memory:')
  try {
    let now = 1_000
    const { host, ptys } = makeHost({ store, cwd: 'work', now: () => now })
    const info = await host.open('claude', 80, 24)
    ptys[0].push('one')
    expect(store.terminals('work')[0].scrollback).toBe('')
    vi.advanceTimersByTime(1_999)
    expect(store.terminals('work')[0].scrollback).toBe('')
    now = 3_000
    vi.advanceTimersByTime(1)
    expect(store.terminals('work')[0].scrollback).toBe('one')
    ptys[0].push('two')
    vi.advanceTimersByTime(1_999)
    expect(store.terminals('work')[0].scrollback).toBe('one')
    now = 5_000
    vi.advanceTimersByTime(1)
    expect(store.terminals('work')[0].scrollback).toBe('onetwo')
    ptys[0].push('three')
    now = 5_500
    ptys[0].exit(0)
    expect(store.terminals('work')[0].scrollback).toBe('onetwothree')
    expect(store.terminals('work')[0].id).toBe(info.id)
  } finally {
    store.close()
  }
})

test('close deletes the row and closeAll keeps it', async () => {
  const store = new Store(':memory:')
  try {
    const { host, ptys } = makeHost({ store, cwd: 'work', now: () => 1 })
    const closed = await host.open('claude', 80, 24)
    const kept = await host.open('cursor', 80, 24)
    expect(store.terminals('work')).toHaveLength(2)
    host.close(closed.id)
    expect(store.terminals('work').map((row) => row.id)).toEqual([kept.id])
    host.closeAll()
    expect(host.list()).toEqual([])
    expect(store.terminals('work').map((row) => row.id)).toEqual([kept.id])
    ptys[0].exit(0)
    ptys[1].exit(0)
    expect(store.terminals('work').map((row) => row.id)).toEqual([kept.id])
  } finally {
    store.close()
  }
})

test('restore relaunches with --resume, seeds the snapshot, keeps ids, and is idempotent', async () => {
  vi.useFakeTimers()
  const store = new Store(':memory:')
  try {
    const first = makeHost({
      store,
      cwd: 'alpha',
      now: () => 10,
      modelFor: (provider) => (provider === 'claude' ? 'opus' : 'fast'),
      createChat: async () => 'cursor-session-01'
    })
    const claude = await first.host.open('claude', 80, 24)
    const cursor = await first.host.open('cursor', 80, 24)
    first.ptys[0].push('hello claude')
    first.ptys[1].push('hello cursor')
    vi.advanceTimersByTime(2_000)
    first.host.closeAll()

    store.saveTerminal({
      id: 'other',
      project: 'beta',
      provider: 'claude',
      model: null,
      session_id: 'other-session',
      title: 'claude 9',
      created_at: 1,
      updated_at: 1,
      scrollback: 'nope'
    })

    const second = makeHost({
      store,
      cwd: 'alpha',
      now: () => 99,
      modelFor: () => 'ignored'
    })
    const updates: TerminalInfo[] = []
    second.host.onUpdate((info, removed) => {
      if (!removed) updates.push(info)
    })
    const restored = second.host.restore('alpha', 80, 24)
    expect(restored.map((info) => info.id)).toEqual([claude.id, cursor.id])
    expect(restored[0]).toMatchObject({
      id: claude.id,
      provider: 'claude',
      title: 'claude 1',
      model: 'opus',
      status: 'running',
      startedAt: 99
    })
    expect(restored[1]).toMatchObject({
      id: cursor.id,
      provider: 'cursor',
      model: 'fast',
      title: 'cursor 1'
    })
    expect(second.calls[0].args[0]).toBe('--resume')
    expect(second.calls[0].args[1]).toMatch(SESSION_ID)
    expect(second.calls[0].args.slice(2)).toEqual(['--model', 'opus'])
    expect(second.calls[1].args).toEqual(['--resume', 'cursor-session-01', '--model', 'fast'])
    expect(second.host.snapshot(claude.id)).toBe(`hello claude${RESTORE_MARK}`)
    expect(second.host.snapshot(cursor.id)).toBe(`hello cursor${RESTORE_MARK}`)
    expect(second.host.list().map((info) => info.id)).toEqual([claude.id, cursor.id])
    expect(updates.map((info) => info.id)).toEqual([claude.id, cursor.id])

    expect(second.host.restore('alpha', 80, 24)).toEqual([])
    expect(second.calls).toHaveLength(2)
    expect(second.host.list()).toHaveLength(2)

    const other = makeHost({ store, cwd: 'beta', now: () => 100 })
    const beta = other.host.restore('beta', 40, 12)
    expect(beta.map((info) => info.id)).toEqual(['other'])
    expect(other.calls[0].args).toEqual(['--resume', 'other-session'])
    expect(second.host.list()).toHaveLength(2)
  } finally {
    store.close()
  }
})

test('restored claude that dies on probation relaunches fresh in the same tab', async () => {
  const store = new Store(':memory:')
  try {
    let now = 1_000
    const id = putTerminal(store)
    const { host, ptys, calls } = makeHost({ store, now: () => now })
    const restored = host.restore('work', 80, 24)
    expect(restored[0]).toMatchObject({ id, title: 'claude 1', status: 'running' })
    expect(calls[0].args[0]).toBe('--resume')
    const updates: Array<{ status: string; removed: boolean }> = []
    host.onUpdate((info, removed) => updates.push({ status: info.status, removed }))
    now = 3_000
    ptys[0].exit(1)
    expect(calls).toHaveLength(2)
    expect(calls[1].args[0]).toBe('--session-id')
    expect(calls[1].args[1]).toMatch(SESSION_ID)
    expect(calls[1].args[1]).not.toBe('old-session')
    expect(calls[1].opts).toMatchObject({ cols: 80, rows: 24, cwd: 'work' })
    expect(host.list()).toEqual([
      expect.objectContaining({
        id,
        title: 'claude 1',
        model: 'opus',
        status: 'running',
        exitCode: null
      })
    ])
    expect(host.snapshot(id)).toBe(`old${RESTORE_MARK}${FRESH_MARK}`)
    expect(store.terminals('work')[0].session_id).toMatch(SESSION_ID)
    expect(store.terminals('work')[0].session_id).not.toBe('old-session')
    expect(store.terminals('work')[0].updated_at).toBe(3_000)
    expect(updates).toEqual([{ status: 'running', removed: false }])
    expect(store.runs()).toEqual([])
  } finally {
    store.close()
  }
})

test('restored terminal that exits 0 on probation stays exited', async () => {
  const store = new Store(':memory:')
  try {
    let now = 1_000
    const id = putTerminal(store)
    const { host, ptys, calls } = makeHost({ store, now: () => now })
    host.restore('work', 80, 24)
    now = 3_000
    ptys[0].exit(0)
    expect(calls).toHaveLength(1)
    expect(host.list()[0]).toMatchObject({ id, status: 'exited', exitCode: 0 })
    expect(store.terminals('work')[0].session_id).toBe('old-session')
  } finally {
    store.close()
  }
})

test('restored terminal that fails after probation stays exited', async () => {
  const store = new Store(':memory:')
  try {
    let now = 1_000
    const id = putTerminal(store)
    const { host, ptys, calls } = makeHost({ store, now: () => now })
    host.restore('work', 80, 24)
    now = 10_000
    ptys[0].exit(1)
    expect(calls).toHaveLength(1)
    expect(host.list()[0]).toMatchObject({ id, status: 'exited', exitCode: 1 })
  } finally {
    store.close()
  }
})

test('restored terminal that was typed into stays exited on failure', async () => {
  const store = new Store(':memory:')
  try {
    let now = 1_000
    const id = putTerminal(store)
    const { host, ptys, calls } = makeHost({ store, now: () => now })
    host.restore('work', 80, 24)
    host.write(id, 'hi')
    now = 3_000
    ptys[0].exit(1)
    expect(calls).toHaveLength(1)
    expect(ptys[0].written).toEqual(['hi'])
    expect(host.list()[0]).toMatchObject({ id, status: 'exited', exitCode: 1 })
  } finally {
    store.close()
  }
})

test('a fresh relaunch is not on probation', async () => {
  const store = new Store(':memory:')
  try {
    let now = 1_000
    const id = putTerminal(store)
    const { host, ptys, calls } = makeHost({ store, now: () => now })
    host.restore('work', 80, 24)
    now = 2_000
    ptys[0].exit(1)
    expect(calls).toHaveLength(2)
    now = 2_100
    ptys[1].exit(1)
    expect(calls).toHaveLength(2)
    expect(host.list()[0]).toMatchObject({ id, status: 'exited', exitCode: 1 })
    expect(store.runs()).toHaveLength(1)
  } finally {
    store.close()
  }
})

test('restored cursor that fails on probation gets a new chat id or none', async () => {
  const store = new Store(':memory:')
  try {
    let now = 1_000
    const id = putTerminal(store, {
      provider: 'cursor',
      title: 'cursor 1',
      model: 'fast',
      session_id: 'cursor-old-01'
    })
    let created = 0
    const { host, ptys, calls } = makeHost({
      store,
      now: () => now,
      createChat: async () => {
        created += 1
        return 'cursor-new-01'
      }
    })
    host.restore('work', 80, 24)
    expect(created).toBe(0)
    now = 2_000
    ptys[0].exit(1)
    await flushMicrotasks()
    expect(created).toBe(1)
    expect(calls).toHaveLength(2)
    expect(calls[1].args).toEqual(['--resume', 'cursor-new-01', '--model', 'fast'])
    expect(store.terminals('work')[0].session_id).toBe('cursor-new-01')
    expect(host.list()[0]).toMatchObject({ id, title: 'cursor 1', status: 'running' })
    expect(store.runs()).toEqual([])
  } finally {
    store.close()
  }

  const again = new Store(':memory:')
  try {
    let now = 1_000
    putTerminal(again, {
      provider: 'cursor',
      title: 'cursor 1',
      model: null,
      session_id: 'cursor-old-02'
    })
    const { host, ptys, calls } = makeHost({
      store: again,
      now: () => now,
      createChat: async () => {
        throw new Error('offline')
      }
    })
    host.restore('work', 80, 24)
    now = 2_000
    ptys[0].exit(1)
    await flushMicrotasks()
    expect(calls).toHaveLength(2)
    expect(calls[1].args).toEqual([])
    expect(again.terminals('work')[0].session_id).toBeNull()
    expect(host.list()[0].status).toBe('running')
  } finally {
    again.close()
  }
})

test('close during a pending cursor relaunch starts no process and deletes the row', async () => {
  const store = new Store(':memory:')
  try {
    let now = 1_000
    const id = putTerminal(store, {
      provider: 'cursor',
      title: 'cursor 1',
      session_id: 'cursor-old-03'
    })
    let finish: ((value: string) => void) | undefined
    const { host, ptys, calls } = makeHost({
      store,
      now: () => now,
      createChat: () =>
        new Promise((resolve) => {
          finish = resolve
        })
    })
    host.restore('work', 80, 24)
    now = 2_000
    ptys[0].exit(1)
    await flushMicrotasks()
    expect(calls).toHaveLength(1)
    expect(typeof finish).toBe('function')
    host.close(id)
    expect(store.terminals('work')).toEqual([])
    expect(host.list()).toEqual([])
    finish?.('cursor-late-01')
    await flushMicrotasks()
    expect(calls).toHaveLength(1)
    expect(store.terminals('work')).toEqual([])
  } finally {
    store.close()
  }
})

test('stored scrollback is capped at the last 200_000 characters', async () => {
  vi.useFakeTimers()
  const store = new Store(':memory:')
  try {
    const { host, ptys } = makeHost({ store, cwd: 'work', now: () => 1 })
    await host.open('claude', 80, 24)
    ptys[0].push(`ab${'x'.repeat(TERMINAL_SCROLLBACK_CAP)}`)
    vi.advanceTimersByTime(2_000)
    const saved = store.terminals('work')[0].scrollback
    expect(saved).toHaveLength(TERMINAL_SCROLLBACK_CAP)
    expect(saved).toBe(`${'x'.repeat(TERMINAL_SCROLLBACK_CAP)}`)
  } finally {
    store.close()
  }
})

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
