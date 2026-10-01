import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { expect, test } from 'vitest'
import type { AgentEvent, Job, RunHandle } from '../types'
import { ClaudeAdapter } from './claude'

const fixtureDir = join(dirname(fileURLToPath(import.meta.url)), 'fixtures')
const prompt = 'say "hello"\n100% |& done >'

function eventsFromFixture(adapter: ClaudeAdapter, name: string): AgentEvent[] {
  const raw = readFileSync(join(fixtureDir, name), 'utf8')
  const events: AgentEvent[] = []
  for (const line of raw.split('\n')) {
    const event = adapter.parseEvent(line.trim())
    if (event) events.push(event)
  }
  return events
}

function writeScript(dir: string, name: string, source: string): string {
  const script = join(dir, name)
  writeFileSync(script, source)
  return script
}

function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`timed out after ${ms}ms`)), ms)
    promise.then(
      (value) => {
        clearTimeout(timer)
        resolve(value)
      },
      (error: unknown) => {
        clearTimeout(timer)
        reject(error instanceof Error ? error : new Error(String(error)))
      }
    )
  })
}

async function collect(
  handle: RunHandle
): Promise<{ events: AgentEvent[]; exit: { code: number | null; stderr: string } }> {
  const events: AgentEvent[] = []
  for await (const event of handle.events) events.push(event)
  return { events, exit: await handle.exit }
}

test('claude-plain parses init, text, usage, and result', () => {
  const events = eventsFromFixture(new ClaudeAdapter(), 'claude-plain.ndjson')
  expect(events).toEqual([
    {
      kind: 'init',
      sessionId: '622ae2ea-0710-4c62-b456-dbab99f57d85',
      model: 'claude-opus-5-5'
    },
    { kind: 'text', text: 'ok' },
    { kind: 'usage', utilization: 0.41, resetsAt: 1791118800 },
    {
      kind: 'result',
      ok: true,
      text: 'ok',
      sessionId: '622ae2ea-0710-4c62-b456-dbab99f57d85',
      durationMs: 6515,
      costUsd: 0.1355346,
      tokens: 6
    }
  ])
})

test('claude-tool skips tool-only assistant output', () => {
  const adapter = new ClaudeAdapter()
  const raw = readFileSync(join(fixtureDir, 'claude-tool.ndjson'), 'utf8')
  const toolLine = raw.split('\n').find((line) => line.includes('"tool_use"'))
  expect(toolLine).toBeTruthy()
  expect(adapter.parseEvent(toolLine ?? '')).toBeNull()
  expect(eventsFromFixture(adapter, 'claude-tool.ndjson')).toEqual([
    {
      kind: 'init',
      sessionId: '3c3cd804-ea6e-4395-9d68-7031b3e29180',
      model: 'claude-opus-5-5'
    },
    { kind: 'usage', utilization: 0.41, resetsAt: 1791118800 },
    { kind: 'text', text: 'It says "hello fixture".' },
    {
      kind: 'result',
      ok: true,
      text: 'It says "hello fixture".',
      sessionId: '3c3cd804-ea6e-4395-9d68-7031b3e29180',
      durationMs: 5780,
      costUsd: 0.16763240000000001,
      tokens: 137
    }
  ])
})

test('claude-limit.synthetic yields a limit event', () => {
  const adapter = new ClaudeAdapter()
  const events = eventsFromFixture(adapter, 'claude-limit.synthetic.ndjson')
  expect(events).toEqual([
    {
      kind: 'init',
      sessionId: '00000000-0000-4000-8000-000000000001',
      model: 'claude-opus-5-5'
    },
    { kind: 'limit', resetsAt: 1790848800, message: 'rate limit rejected' },
    { kind: 'limit', message: 'rate_limit' },
    { kind: 'limit', message: '5-hour limit reached - resets 2pm' }
  ])
  expect(adapter.isLimitError(events[1])).toBe(true)
})

test('parseEvent returns null for garbage without throwing', () => {
  const adapter = new ClaudeAdapter()
  for (const line of ['', '   ', 'not-json', '{', 'null', '[]', '{"type":"commands_changed"}']) {
    expect(adapter.parseEvent(line)).toBeNull()
  }
  expect(
    adapter.parseEvent(
      JSON.stringify({ type: 'rate_limit_event', rate_limit_info: { status: 'allowed' } })
    )
  ).toBeNull()
})

test('isLimitError matches limit text and ignores unrelated errors', () => {
  const adapter = new ClaudeAdapter()
  expect(adapter.isLimitError('Claude AI usage limit reached')).toBe(true)
  expect(adapter.isLimitError('network timeout')).toBe(false)
})

test('run replays a fixture and passes the prompt on stdin', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'ao-claude-'))
  const fixturePath = join(fixtureDir, 'claude-plain.ndjson')
  const script = writeScript(
    dir,
    'replay.mjs',
    `import { readFileSync } from 'node:fs'
const chunks = []
for await (const chunk of process.stdin) chunks.push(chunk)
process.stderr.write(Buffer.concat(chunks).toString('utf8'))
process.stdout.write(readFileSync(${JSON.stringify(fixturePath)}, 'utf8'))
`
  )
  const adapter = new ClaudeAdapter({ command: process.execPath, args: [script] })
  const job: Job = { id: 'job-1', type: 'planning', prompt }
  try {
    const { events, exit } = await collect(adapter.run(job, dir))
    expect(events).toEqual(eventsFromFixture(adapter, 'claude-plain.ndjson'))
    expect(exit.code).toBe(0)
    expect(exit.stderr).toBe(prompt)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('kill stops a hung CLI', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'ao-claude-kill-'))
  const script = writeScript(
    dir,
    'hang.mjs',
    `import { writeSync } from 'node:fs'
writeSync(1, ${JSON.stringify('{"type":"system","subtype":"init","session_id":"kill-test","model":"m"}\n')})
setInterval(() => {}, 1000000)
`
  )
  const adapter = new ClaudeAdapter({ command: process.execPath, args: [script] })
  const handle = adapter.run({ id: 'k', type: 'debugging', prompt: 'x' }, dir)
  try {
    const iter = handle.events[Symbol.asyncIterator]()
    const first = await withTimeout(iter.next(), 3000)
    expect(first.done).toBe(false)
    expect(first.value).toMatchObject({ kind: 'init' })
    handle.kill()
    const exit = await withTimeout(handle.exit, 5000)
    expect(exit.code).not.toBe(0)
    expect(() => handle.kill()).not.toThrow()
  } finally {
    handle.kill()
    rmSync(dir, { recursive: true, force: true })
  }
}, 15_000)

test('missing CLI resolves exit and ends events', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'ao-claude-missing-'))
  const adapter = new ClaudeAdapter({ command: join(dir, 'missing-cli.exe') })
  try {
    const { events, exit } = await collect(
      adapter.run({ id: 'm', type: 'review', prompt: 'hi' }, dir)
    )
    expect(events).toEqual([])
    expect(exit.code).toBeNull()
    expect(exit.stderr.length).toBeGreaterThan(0)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})
