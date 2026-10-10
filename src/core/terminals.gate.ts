import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { spawn as spawnPty } from 'node-pty'
import { expect, test } from 'vitest'
import { PtyHost } from './pty'
import { ClaudeAdapter } from './providers/claude'
import { CursorAdapter } from './providers/cursor'
import { Store } from './router'
import type { ProviderAdapter, ProviderId } from './types'

const QUESTION = 'What is 1234 + 4321? Reply with just the number.'

test('interactive claude and cursor sessions answer in a terminal', async () => {
  const claude = new ClaudeAdapter()
  const cursor = new CursorAdapter()
  await requireAvailable(claude)
  await requireAvailable(cursor)
  for (const provider of ['claude', 'cursor'] as const) {
    await runSession(provider, claude, cursor)
  }
}, 600_000)

async function runSession(
  provider: ProviderId,
  claude: ClaudeAdapter,
  cursor: CursorAdapter
): Promise<void> {
  const root = await mkdtemp(join(tmpdir(), 'orch-term-'))
  let store: Store | undefined
  let host: PtyHost | undefined
  let pid = 0
  const started = Date.now()
  let trustSeconds: number | null = null
  let answerSeconds: number | null = null
  let raw = ''
  let resizeThrew = false
  let removed = false
  let processGone = false
  try {
    const cwd = join(root, 'work')
    await mkdir(cwd)
    await writeFile(join(cwd, 'hello.txt'), 'hello\n')
    store = new Store(join(root, 'orchestrator.sqlite'))
    const ledger = store
    host = new PtyHost({
      launch: (id, model) =>
        id === 'claude' ? claude.interactive(model) : cursor.interactive(model),
      cwd,
      store: ledger,
      spawn: (command, args, opts) => {
        const proc = spawnPty(command, args, opts)
        pid = proc.pid
        return proc
      }
    })
    const terms = host
    let after = ''
    let collectAfter = false
    terms.onData((_id, data) => {
      raw += data
      if (collectAfter) after += data
    })
    const opened = await terms.open(provider, 110, 32)
    terms.onUpdate((info, gone) => {
      if (info.id === opened.id && gone) removed = true
    })

    const trustDeadline = Date.now() + 90_000
    while (Date.now() < trustDeadline) {
      if (/trust/i.test(stripAnsi(raw))) {
        trustSeconds = (Date.now() - started) / 1000
        break
      }
      await sleep(100)
    }
    if (trustSeconds !== null) {
      // The word shows up before the dialog takes input; keys sent too early are dropped,
      // and Enter alone then confirms Claude's default, "No, exit".
      await waitUntilQuiet(() => raw.length)
      if (provider === 'claude') {
        terms.write(opened.id, '\x1b[B')
        await sleep(600)
        terms.write(opened.id, '\r')
      } else {
        terms.write(opened.id, 'a')
      }
    }

    await waitUntilQuiet(() => raw.length)
    collectAfter = true
    const asked = Date.now()
    terms.write(opened.id, QUESTION)
    await sleep(600)
    terms.write(opened.id, '\r')
    const answerDeadline = Date.now() + 240_000
    while (Date.now() < answerDeadline) {
      if (stripAnsi(after).includes('5555')) {
        answerSeconds = (Date.now() - asked) / 1000
        break
      }
      await sleep(200)
    }

    try {
      terms.resize(opened.id, 90, 28)
    } catch {
      resizeThrew = true
    }
    terms.close(opened.id)
    const closeDeadline = Date.now() + 10_000
    while (Date.now() < closeDeadline) {
      processGone = pid !== 0 && !alive(pid)
      const rows = ledger
        .runs()
        .filter((row) => row.job_type === 'terminal' && row.provider === provider)
      if (removed && terms.list().length === 0 && processGone && rows.length === 1) break
      await sleep(100)
    }

    printEvidence(provider, trustSeconds, answerSeconds, stripAnsi(raw))
    expect(answerSeconds, `${provider} answer`).not.toBeNull()
    expect(resizeThrew, `${provider} resize`).toBe(false)
    expect(removed, `${provider} removed`).toBe(true)
    expect(terms.list(), `${provider} list`).toEqual([])
    expect(processGone, `${provider} process`).toBe(true)
    expect(
      ledger.runs().filter((row) => row.job_type === 'terminal' && row.provider === provider)
    ).toHaveLength(1)
  } finally {
    try {
      host?.closeAll()
    } catch (err) {
      console.error(err)
    }
    if (pid !== 0) {
      const deadline = Date.now() + 10_000
      while (Date.now() < deadline && alive(pid)) await sleep(100)
    }
    store?.close()
    await rm(root, { recursive: true, force: true })
  }
}

function printEvidence(
  provider: string,
  trustSeconds: number | null,
  answerSeconds: number | null,
  stripped: string
): void {
  const trust = trustSeconds === null ? 'none' : `${trustSeconds.toFixed(1)}s`
  const answer = answerSeconds === null ? 'none' : `${answerSeconds.toFixed(1)}s`
  console.log(`${provider} trust: ${trust}, answer: ${answer}`)
  const lines = stripped
    .split(/\r\n|\n|\r/)
    .map((line) => line.trim())
    .filter((line) => line !== '')
  for (const line of lines.slice(-12)) console.log(line)
}

async function waitUntilQuiet(lengthOf: () => number): Promise<void> {
  let last = lengthOf()
  let changedAt = Date.now()
  const deadline = Date.now() + 90_000
  while (Date.now() < deadline) {
    const next = lengthOf()
    if (next !== last) {
      last = next
      changedAt = Date.now()
    } else if (Date.now() - changedAt >= 3_000) {
      return
    }
    await sleep(100)
  }
}

function alive(pid: number): boolean {
  try {
    process.kill(pid, 0)
    return true
  } catch {
    return false
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms)
  })
}

async function requireAvailable(adapter: ProviderAdapter): Promise<void> {
  if (!(await adapter.isInstalled())) {
    throw new Error(`${adapter.id} is not available: not installed`)
  }
  if (!(await adapter.isSignedIn())) {
    throw new Error(`${adapter.id} is not available: not signed in`)
  }
}

/** CSI, OSC, and a lone ESC. Cursor-move sequences become a space so words stay apart. */
function stripAnsi(input: string): string {
  let out = ''
  for (let i = 0; i < input.length; i++) {
    if (input[i] !== '\x1b') {
      out += input[i]
      continue
    }
    const next = input[i + 1]
    if (next === '[') {
      const csi = parseCsi(input, i + 2)
      if (!csi) {
        out += '\x1b'
        continue
      }
      if (csi.move) out += ' '
      i = csi.end
      continue
    }
    if (next === ']') {
      const end = parseOsc(input, i + 2)
      if (end < 0) {
        out += '\x1b'
        continue
      }
      i = end
      continue
    }
    if (next !== undefined) i += 1
  }
  return out
}

function parseCsi(input: string, start: number): { end: number; move: boolean } | null {
  let j = start
  while (j < input.length) {
    const code = input.charCodeAt(j)
    if ((code >= 0x30 && code <= 0x3f) || (code >= 0x20 && code <= 0x2f)) {
      j++
      continue
    }
    if (code >= 0x40 && code <= 0x7e) {
      return { end: j, move: 'ABCDEFGHfd'.includes(input[j]) }
    }
    return null
  }
  return null
}

function parseOsc(input: string, start: number): number {
  for (let j = start; j < input.length; j++) {
    if (input[j] === '\x07') return j
    if (input[j] === '\x1b' && input[j + 1] === '\\') return j + 1
  }
  return -1
}
