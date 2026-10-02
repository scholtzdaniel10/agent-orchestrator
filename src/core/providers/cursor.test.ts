import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { expect, test } from 'vitest'
import type { AgentEvent, Job, RunHandle } from '../types'
import { CursorAdapter, cursorEnv, parseCursorModels } from './cursor'

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

    const modelDir = join(root, 'model')
    mkdirSync(modelDir)
    expect(await cliArgs(adapter.run(job, modelDir, { model: 'composer-2.5' }))).toEqual([
      ...base,
      '--model',
      'composer-2.5'
    ])
    expect(await cliArgs(adapter.run(job, plain, { model: '' }))).toEqual(base)

    const all = join(root, 'all')
    expect(
      await cliArgs(adapter.run(job, all, { resume: 'sess-3', bridge, model: 'composer-2.5' }))
    ).toEqual([...base, '--resume', 'sess-3', '--approve-mcps', '--model', 'composer-2.5'])
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

test('parseCursorModels keeps id - label lines and drops the rest', () => {
  const zw = '\u200B\u200C\u200D\uFEFF'
  const stdout = [
    'Available models',
    '',
    `${zw}auto${zw} - Auto${zw}`,
    'composer-2.5 - Composer 2.5 - fast',
    'not a model line'
  ].join('\r\n')
  expect(parseCursorModels(stdout)).toEqual([
    { id: 'auto', label: 'Auto' },
    { id: 'composer-2.5', label: 'Composer 2.5 - fast' }
  ])
  expect(parseCursorModels('')).toEqual([])
  expect(parseCursorModels('\n\n')).toEqual([])
})

test('listModels parses the CLI, caches a success, and does not cache a failure', async () => {
  const root = mkdtempSync(join(tmpdir(), 'ao-cursor-models-'))
  try {
    const counter = join(root, 'count.txt')
    const okScript = writeScript(
      root,
      'ok.mjs',
      `import { existsSync, readFileSync, writeFileSync } from 'node:fs'
const file = ${JSON.stringify(counter)}
const n = existsSync(file) ? Number(readFileSync(file, 'utf8')) : 0
writeFileSync(file, String(n + 1))
if (n > 0) process.exit(1)
process.stdout.write('Available models\\n\\nauto - Auto\\ncomposer-2.5 - Composer 2.5\\n')
`
    )
    const ok = new CursorAdapter({ command: process.execPath, args: [okScript] })
    const first = await ok.listModels()
    const second = await ok.listModels()
    expect(first).toEqual([
      { id: 'auto', label: 'Auto' },
      { id: 'composer-2.5', label: 'Composer 2.5' }
    ])
    expect(second).toEqual(first)
    expect(readFileSync(counter, 'utf8')).toBe('1')

    const failCount = join(root, 'fail.txt')
    const failScript = writeScript(
      root,
      'fail.mjs',
      `import { existsSync, readFileSync, writeFileSync } from 'node:fs'
const file = ${JSON.stringify(failCount)}
const n = existsSync(file) ? Number(readFileSync(file, 'utf8')) : 0
writeFileSync(file, String(n + 1))
if (n === 0) process.exit(2)
process.stdout.write('auto - Auto\\n')
`
    )
    const failing = new CursorAdapter({ command: process.execPath, args: [failScript] })
    expect(await failing.listModels()).toEqual([])
    expect(await failing.listModels()).toEqual([{ id: 'auto', label: 'Auto' }])
    expect(readFileSync(failCount, 'utf8')).toBe('2')

    const emptyCount = join(root, 'empty.txt')
    const emptyScript = writeScript(
      root,
      'empty.mjs',
      `import { existsSync, readFileSync, writeFileSync } from 'node:fs'
const file = ${JSON.stringify(emptyCount)}
const n = existsSync(file) ? Number(readFileSync(file, 'utf8')) : 0
writeFileSync(file, String(n + 1))
if (n === 0) {
  process.stdout.write('Available models\\n\\n')
  process.exit(0)
}
process.stdout.write('haiku - Haiku\\n')
`
    )
    const empty = new CursorAdapter({ command: process.execPath, args: [emptyScript] })
    expect(await empty.listModels()).toEqual([])
    expect(await empty.listModels()).toEqual([{ id: 'haiku', label: 'Haiku' }])
    expect(readFileSync(emptyCount, 'utf8')).toBe('2')
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('cursorEnv drops the Git Bash markers on Windows only', () => {
  const source = { PATH: 'p', SHELL: '/bin/bash.exe', MSYSTEM: 'MINGW64', Shell: 'x', TERM: 't' }
  expect(cursorEnv('win32', source)).toEqual({ PATH: 'p', TERM: 't' })
  expect(cursorEnv('linux', source)).toBeUndefined()
  expect(cursorEnv('darwin', source)).toBeUndefined()
})

test.runIf(process.platform === 'win32')(
  'run does not pass SHELL or MSYSTEM to the CLI on Windows',
  async () => {
    const dir = mkdtempSync(join(tmpdir(), 'ao-cursor-env-'))
    const script = writeScript(
      dir,
      'env.mjs',
      `process.stderr.write(JSON.stringify({ shell: process.env.SHELL ?? null, msystem: process.env.MSYSTEM ?? null, path: typeof process.env.PATH }))\n`
    )
    const saved = { SHELL: process.env.SHELL, MSYSTEM: process.env.MSYSTEM }
    process.env.SHELL = '/bin/bash.exe'
    process.env.MSYSTEM = 'MINGW64'
    try {
      const adapter = new CursorAdapter({ command: process.execPath, args: [script] })
      const { exit } = await collect(adapter.run({ id: 'e', prompt: 'x' }, dir))
      expect(JSON.parse(exit.stderr)).toEqual({ shell: null, msystem: null, path: 'string' })
    } finally {
      if (saved.SHELL === undefined) delete process.env.SHELL
      else process.env.SHELL = saved.SHELL
      if (saved.MSYSTEM === undefined) delete process.env.MSYSTEM
      else process.env.MSYSTEM = saved.MSYSTEM
      rmSync(dir, { recursive: true, force: true })
    }
  }
)
