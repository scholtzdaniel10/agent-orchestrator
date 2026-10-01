import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { expect, test } from 'vitest'
import type { AgentEvent, Job, RunHandle } from '../types'
import { CursorAdapter } from './cursor'

const fixtureDir = join(dirname(fileURLToPath(import.meta.url)), 'fixtures')
const prompt = 'say "hello"\n100% |& done >'

function eventsFromFixture(adapter: CursorAdapter, name: string): AgentEvent[] {
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

async function cliArgs(handle: RunHandle): Promise<string[]> {
  const { exit } = await collect(handle)
  expect(exit.code).toBe(0)
  return JSON.parse(exit.stderr) as string[]
}

async function collect(
  handle: RunHandle
): Promise<{ events: AgentEvent[]; exit: { code: number | null; stderr: string } }> {
  const events: AgentEvent[] = []
  for await (const event of handle.events) events.push(event)
  return { events, exit: await handle.exit }
}

test('cursor-plain parses init, text, and result', () => {
  const events = eventsFromFixture(new CursorAdapter(), 'cursor-plain.ndjson')
  expect(events).toEqual([
    {
      kind: 'init',
      sessionId: 'f7be1458-1eb0-461c-a896-c2dca9092f79',
      model: 'Grok 4.7 256K Extra High'
    },
    { kind: 'text', text: 'ok' },
    {
      kind: 'result',
      ok: true,
      text: 'ok',
      sessionId: 'f7be1458-1eb0-461c-a896-c2dca9092f79',
      durationMs: 14224,
      tokens: 10406
    }
  ])
})

test('cursor-tool result contains the fixture text', () => {
  const events = eventsFromFixture(new CursorAdapter(), 'cursor-tool.ndjson')
  expect(events.map((event) => event.kind)).toEqual(['init', 'text', 'text', 'result'])
  const last = events[events.length - 1]
  expect(last).toMatchObject({ kind: 'result', ok: true })
  if (last.kind === 'result') expect(last.text).toContain('hello fixture')
})

test('cursor-limit.synthetic yields a limit event', () => {
  const adapter = new CursorAdapter()
  const events = eventsFromFixture(adapter, 'cursor-limit.synthetic.ndjson')
  expect(events).toEqual([
    {
      kind: 'init',
      sessionId: '00000000-0000-4000-8000-000000000002',
      model: 'Grok 4.7 256K Extra High'
    },
    { kind: 'limit', message: "You've hit your usage limit for this plan." }
  ])
  expect(adapter.isLimitError(events[1])).toBe(true)
})

test('parseEvent returns null for garbage without throwing', () => {
  const adapter = new CursorAdapter()
  for (const line of [
    '',
    '   ',
    'not-json',
    '{',
    'null',
    '[]',
    '{"type":"thinking"}',
    '{"type":"tool_call"}',
    '{"type":"user"}'
  ]) {
    expect(adapter.parseEvent(line)).toBeNull()
  }
})

test('isLimitError matches limit text and ignores unrelated errors', () => {
  const adapter = new CursorAdapter()
  expect(adapter.isLimitError('Claude AI usage limit reached')).toBe(true)
  expect(adapter.isLimitError('network timeout')).toBe(false)
})

test('run replays a fixture and passes the prompt on stdin', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'ao-cursor-'))
  const fixturePath = join(fixtureDir, 'cursor-plain.ndjson')
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
  const adapter = new CursorAdapter({ command: process.execPath, args: [script] })
  const job: Job = { id: 'job-1', type: 'planning', prompt }
  try {
    const { events, exit } = await collect(adapter.run(job, dir))
    expect(events).toEqual(eventsFromFixture(adapter, 'cursor-plain.ndjson'))
    expect(exit.code).toBe(0)
    expect(exit.stderr).toBe(prompt)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('kill stops a hung CLI', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'ao-cursor-kill-'))
  const script = writeScript(
    dir,
    'hang.mjs',
    `import { writeSync } from 'node:fs'
writeSync(1, ${JSON.stringify('{"type":"system","subtype":"init","session_id":"kill-test","model":"m"}\n')})
setInterval(() => {}, 1000000)
`
  )
  const adapter = new CursorAdapter({ command: process.execPath, args: [script] })
  const handle = adapter.run({ id: 'k', prompt: 'x' }, dir)
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

test('run args cover no opts, resume, bridge, and both', async () => {
  const root = mkdtempSync(join(tmpdir(), 'ao-cursor-opts-'))
  const script = writeScript(
    root,
    'argv.mjs',
    `import { writeSync } from 'node:fs'
writeSync(2, JSON.stringify(process.argv.slice(2)))
process.exit(0)
`
  )
  const adapter = new CursorAdapter({ command: process.execPath, args: [script] })
  const url = 'http://127.0.0.1:9/mcp'
  const token = 'secret-token'
  const bridge = { url, token, tools: ['send_job', 'get_result'] }
  const job = { id: 'job-1', prompt: 'hello' }
  const base = ['-p', '--trust', '--output-format', 'stream-json']
  const mcpJson = `{"mcpServers":{"orchestrator":{"url":"${url}","headers":{"Authorization":"Bearer ${token}"}}}}`
  const cliJson =
    '{"permissions":{"allow":["Mcp(orchestrator:*)"],"deny":["Shell(*)","Write(**)"]}}'
  try {
    const plain = join(root, 'plain')
    const resumeDir = join(root, 'resume')
    const bridged = join(root, 'bridged')
    const both = join(root, 'both')
    mkdirSync(plain)
    mkdirSync(resumeDir)

    expect(await cliArgs(adapter.run(job, plain))).toEqual(base)
    expect(existsSync(join(plain, '.cursor'))).toBe(false)

    expect(await cliArgs(adapter.run(job, resumeDir, { resume: 'sess-1' }))).toEqual([
      ...base,
      '--resume',
      'sess-1'
    ])
    expect(existsSync(join(resumeDir, '.cursor'))).toBe(false)

    expect(await cliArgs(adapter.run(job, bridged, { bridge }))).toEqual([
      ...base,
      '--approve-mcps'
    ])
    expect(readFileSync(join(bridged, '.cursor', 'mcp.json'), 'utf8')).toBe(mcpJson)
    expect(readFileSync(join(bridged, '.cursor', 'cli.json'), 'utf8')).toBe(cliJson)

    expect(await cliArgs(adapter.run(job, both, { resume: 'sess-2', bridge }))).toEqual([
      ...base,
      '--resume',
      'sess-2',
      '--approve-mcps'
    ])
    expect(readFileSync(join(both, '.cursor', 'mcp.json'), 'utf8')).toBe(mcpJson)
    expect(readFileSync(join(both, '.cursor', 'cli.json'), 'utf8')).toBe(cliJson)
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('missing CLI resolves exit and ends events', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'ao-cursor-missing-'))
  const adapter = new CursorAdapter({ command: join(dir, 'missing-cli.exe') })
  try {
    const { events, exit } = await collect(adapter.run({ id: 'm', prompt: 'hi' }, dir))
    expect(events).toEqual([])
    expect(exit.code).toBeNull()
    expect(exit.stderr.length).toBeGreaterThan(0)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})
