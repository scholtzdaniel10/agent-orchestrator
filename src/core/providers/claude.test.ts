import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
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

async function cliArgs(handle: RunHandle): Promise<string[]> {
  const { exit } = await collect(handle)
  expect(exit.code).toBe(0)
  return JSON.parse(exit.stderr) as string[]
}

async function collect(handle: RunHandle): Promise<{
  events: AgentEvent[]
  exit: { code: number | null; stderr: string; unreadable?: boolean }
}> {
  const events: AgentEvent[] = []
  for await (const event of handle.events) events.push(event)
  return { events, exit: await handle.exit }
}

test('claude-plain parses init, text, usage, and result', () => {
  const events = eventsFromFixture(new ClaudeAdapter(), 'claude-plain.ndjson')
  expect(events).toEqual([
    {
      kind: 'init',
      sessionId: '00000000-0000-4000-8000-000000000103',
      model: 'claude-opus-5-5'
    },
    { kind: 'text', text: 'ok' },
    {
      kind: 'usage',
      utilization: 0.41,
      resetsAt: 1791118800,
      windows: [
        { name: 'five_hour', utilization: 0.24, resetsAt: 1790848800 },
        { name: 'seven_day', utilization: 0.41, resetsAt: 1791118800 }
      ]
    },
    {
      kind: 'result',
      ok: true,
      text: 'ok',
      sessionId: '00000000-0000-4000-8000-000000000103',
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
      sessionId: '00000000-0000-4000-8000-00000000010d',
      model: 'claude-opus-5-5'
    },
    {
      kind: 'usage',
      utilization: 0.41,
      resetsAt: 1791118800,
      windows: [
        { name: 'five_hour', utilization: 0.25, resetsAt: 1790848800 },
        { name: 'seven_day', utilization: 0.41, resetsAt: 1791118800 }
      ]
    },
    { kind: 'text', text: 'It says "hello fixture".' },
    {
      kind: 'result',
      ok: true,
      text: 'It says "hello fixture".',
      sessionId: '00000000-0000-4000-8000-00000000010d',
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
    expect(exit.unreadable).toBe(false)
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
  const root = mkdtempSync(join(tmpdir(), 'ao-claude-opts-'))
  const script = writeScript(
    root,
    'argv.mjs',
    `import { writeSync } from 'node:fs'
writeSync(2, JSON.stringify(process.argv.slice(2)))
process.exit(0)
`
  )
  const adapter = new ClaudeAdapter({ command: process.execPath, args: [script] })
  const url = 'http://127.0.0.1:9/mcp'
  const token = 'secret-token'
  const bridge = { url, token, tools: ['send_job', 'get_result'] }
  const job = { id: 'job-1', prompt: 'hello' }
  const start = ['-p', '--output-format', 'stream-json', '--verbose']
  const strict = '--strict-mcp-config'
  const base = [...start, '--disallowedTools', 'Bash,PowerShell,Edit,Write,NotebookEdit', strict]
  const editing = [...start, '--disallowedTools', 'Bash,PowerShell', strict]
  const mcpArgs = (cwd: string): string[] => [
    '--mcp-config',
    join(cwd, 'orchestrator-mcp.json'),
    '--strict-mcp-config',
    '--allowedTools',
    'mcp__orchestrator__send_job,mcp__orchestrator__get_result',
    '--setting-sources',
    'project'
  ]
  const expectedConfig = `{"mcpServers":{"orchestrator":{"type":"http","url":"${url}","headers":{"Authorization":"Bearer ${token}"}}}}`
  try {
    const plain = join(root, 'plain')
    const resumeDir = join(root, 'resume')
    const bridged = join(root, 'bridged')
    const both = join(root, 'both')
    mkdirSync(plain)
    mkdirSync(resumeDir)

    expect(await cliArgs(adapter.run(job, plain))).toEqual(base)
    expect(existsSync(join(plain, 'orchestrator-mcp.json'))).toBe(false)

    expect(await cliArgs(adapter.run(job, resumeDir, { resume: 'sess-1' }))).toEqual([
      ...base,
      '--resume',
      'sess-1'
    ])
    expect(existsSync(join(resumeDir, 'orchestrator-mcp.json'))).toBe(false)

    expect(await cliArgs(adapter.run(job, bridged, { bridge }))).toEqual([
      ...start,
      ...mcpArgs(bridged)
    ])
    expect(readFileSync(join(bridged, 'orchestrator-mcp.json'), 'utf8')).toBe(expectedConfig)

    expect(await cliArgs(adapter.run(job, both, { resume: 'sess-2', bridge }))).toEqual([
      ...start,
      '--resume',
      'sess-2',
      ...mcpArgs(both)
    ])
    expect(readFileSync(join(both, 'orchestrator-mcp.json'), 'utf8')).toBe(expectedConfig)

    const modelDir = join(root, 'model')
    mkdirSync(modelDir)
    expect(await cliArgs(adapter.run(job, modelDir, { model: 'opus' }))).toEqual([
      ...base,
      '--model',
      'opus'
    ])
    expect(await cliArgs(adapter.run(job, plain, { model: '' }))).toEqual(base)

    const all = join(root, 'all')
    expect(
      await cliArgs(adapter.run(job, all, { resume: 'sess-3', bridge, model: 'opus' }))
    ).toEqual([...start, '--resume', 'sess-3', ...mcpArgs(all), '--model', 'opus'])

    const editDir = join(root, 'edit')
    mkdirSync(editDir)
    expect(await cliArgs(adapter.run(job, editDir, { edit: true }))).toEqual([
      ...editing,
      '--permission-mode',
      'acceptEdits'
    ])
    expect(
      await cliArgs(adapter.run(job, editDir, { edit: true, resume: 'sess-e', model: 'opus' }))
    ).toEqual([
      ...editing,
      '--permission-mode',
      'acceptEdits',
      '--resume',
      'sess-e',
      '--model',
      'opus'
    ])
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('listModels returns the four help-text aliases', async () => {
  const adapter = new ClaudeAdapter({ command: 'claude' })
  await expect(adapter.listModels()).resolves.toEqual([
    { id: 'fable', label: 'Fable (latest)' },
    { id: 'opus', label: 'Opus (latest)' },
    { id: 'sonnet', label: 'Sonnet (latest)' },
    { id: 'haiku', label: 'Haiku (latest)' }
  ])
})

test('interactive is the resolved binary with no -p', () => {
  const plain = new ClaudeAdapter({ command: 'claude.exe' })
  expect(plain.interactive()).toEqual({ command: 'claude.exe', args: [] })
  expect(plain.interactive('opus')).toEqual({
    command: 'claude.exe',
    args: ['--model', 'opus']
  })
  const wrapped = new ClaudeAdapter({ command: 'node.exe', args: ['cli.js'] })
  expect(wrapped.interactive()).toEqual({ command: 'node.exe', args: ['cli.js'] })
  expect(wrapped.interactive('sonnet')).toEqual({
    command: 'node.exe',
    args: ['cli.js', '--model', 'sonnet']
  })
})

test('unknown-format fixture is unreadable and has no result event', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'ao-claude-unknown-'))
  const fixturePath = join(fixtureDir, 'claude-unknown-format.synthetic.ndjson')
  const script = writeScript(
    dir,
    'replay.mjs',
    `import { readFileSync } from 'node:fs'
process.stdout.write(readFileSync(${JSON.stringify(fixturePath)}, 'utf8'))
`
  )
  const adapter = new ClaudeAdapter({ command: process.execPath, args: [script] })
  try {
    const raw = readFileSync(fixturePath, 'utf8')
    for (const line of raw.split('\n')) {
      if (line.trim()) expect(adapter.parseEvent(line.trim())).toBeNull()
    }
    const { events, exit } = await collect(adapter.run({ id: 'u', prompt: 'hi' }, dir))
    expect(events.some((event) => event.kind === 'result')).toBe(false)
    expect(exit.code).toBe(0)
    expect(exit.unreadable).toBe(true)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('isInstalled and version share one --version call', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'ao-claude-ver-'))
  const counter = join(dir, 'count.txt')
  const script = writeScript(
    dir,
    'version.mjs',
    `import { existsSync, readFileSync, writeFileSync } from 'node:fs'
const file = ${JSON.stringify(counter)}
const n = existsSync(file) ? Number(readFileSync(file, 'utf8')) : 0
writeFileSync(file, String(n + 1))
process.stdout.write('2.1.258 (Claude Code)\\n')
`
  )
  const adapter = new ClaudeAdapter({ command: process.execPath, args: [script] })
  try {
    expect(await adapter.isInstalled()).toBe(true)
    expect(await adapter.version()).toBe('2.1.258')
    expect(readFileSync(counter, 'utf8')).toBe('1')
    expect(await adapter.isInstalled()).toBe(true)
    expect(await adapter.version()).toBe('2.1.258')
    expect(readFileSync(counter, 'utf8')).toBe('2')
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('missing CLI resolves exit and ends events', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'ao-claude-missing-'))
  const adapter = new ClaudeAdapter({ command: join(dir, 'missing-cli.exe') })
  try {
    const { events, exit } = await collect(adapter.run({ id: 'm', prompt: 'hi' }, dir))
    expect(events).toEqual([])
    expect(exit.code).toBeNull()
    expect(exit.stderr.length).toBeGreaterThan(0)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})
