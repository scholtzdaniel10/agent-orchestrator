import { spawn } from 'node:child_process'
import type { AgentEvent, RunHandle } from '../types'

/** A parsed NDJSON line: an event, a known type the adapter skips, or unrecognised. */
export type LineParse = AgentEvent | 'ignored' | null

/** Executable to spawn. Extra args are placed before the per-call args. */
export interface Bin {
  command: string
  args?: string[]
}

const STDERR_CAP = 64 * 1024

export interface CapturedRun {
  code: number | null
  stdout: string
  stderr: string
}

/** Run a CLI to completion and collect its output. Spawn errors resolve; they do not reject. */
export function runCaptured(bin: Bin, args: string[]): Promise<CapturedRun> {
  return new Promise((resolve) => {
    let stdout = ''
    let stderr = ''
    let settled = false
    const finish = (code: number | null, errText?: string): void => {
      if (settled) return
      settled = true
      resolve({ code, stdout, stderr: errText ?? stderr })
    }

    const child = spawn(bin.command, [...(bin.args ?? []), ...args], {
      windowsHide: true,
      shell: false,
      stdio: ['ignore', 'pipe', 'pipe']
    })
    child.stdout?.setEncoding('utf8')
    child.stderr?.setEncoding('utf8')
    child.stdout?.on('data', (chunk: string | Buffer) => {
      stdout += asText(chunk)
    })
    child.stderr?.on('data', (chunk: string | Buffer) => {
      stderr += asText(chunk)
    })
    child.stdout?.on('error', () => {})
    child.stderr?.on('error', () => {})
    child.on('error', (err: Error) => {
      finish(null, err.message)
    })
    child.on('close', (code) => {
      finish(code)
    })
  })
}

/**
 * True when a CLI exited 0 without a final `result` event this app can read. Unknown lines
 * beside a readable result are fine: newer CLIs add event types.
 */
export function isUnreadableOutput(code: number | null, hadResult: boolean): boolean {
  if (code !== 0) return false
  return !hadResult
}

/**
 * Spawn a provider CLI, write the prompt on stdin, and parse stdout NDJSON.
 * `events` finishes after the process closes and stdout is drained.
 * `exit` never rejects. Spawn failure (ENOENT) resolves `{ code: null, stderr: message }`.
 */
export function spawnCli(
  bin: Bin,
  args: string[],
  prompt: string,
  cwd: string,
  parse: (line: string) => LineParse,
  env?: NodeJS.ProcessEnv
): RunHandle {
  const queue = createEventQueue()
  let stderr = ''
  let pending = ''
  let stdoutDone = false
  let closed = false
  let eventsEnded = false
  let spawnError: string | null = null
  let settled = false
  let hadResult = false
  let resolveExit: (value: {
    code: number | null
    stderr: string
    unreadable?: boolean
  }) => void = () => {}
  const exit = new Promise<{ code: number | null; stderr: string; unreadable?: boolean }>(
    (resolve) => {
      resolveExit = resolve
    }
  )

  const settle = (code: number | null, errText: string): void => {
    if (settled) return
    settled = true
    resolveExit({
      code,
      stderr: errText,
      unreadable: isUnreadableOutput(code, hadResult)
    })
  }

  const emitLine = (line: string): void => {
    const trimmed = line.trim()
    if (!trimmed) return
    const parsed = parse(trimmed)
    if (parsed === null || parsed === 'ignored') return
    if (parsed.kind === 'result') hadResult = true
    queue.push(parsed)
  }

  const flush = (): void => {
    if (!pending) return
    const line = pending
    pending = ''
    emitLine(line)
  }

  const finishEvents = (): void => {
    if (eventsEnded || !stdoutDone || !closed) return
    eventsEnded = true
    queue.end()
  }

  const child = spawn(bin.command, [...(bin.args ?? []), ...args], {
    cwd,
    env,
    windowsHide: true,
    shell: false,
    stdio: ['pipe', 'pipe', 'pipe'],
    detached: process.platform !== 'win32'
  })

  child.stdout?.setEncoding('utf8')
  child.stderr?.setEncoding('utf8')
  child.stdout?.on('data', (chunk: string | Buffer) => {
    pending += asText(chunk)
    for (;;) {
      const nl = pending.indexOf('\n')
      if (nl < 0) break
      const line = pending.slice(0, nl)
      pending = pending.slice(nl + 1)
      emitLine(line)
    }
  })
  child.stderr?.on('data', (chunk: string | Buffer) => {
    stderr += asText(chunk)
    if (stderr.length > STDERR_CAP) stderr = stderr.slice(-STDERR_CAP)
  })
  child.stdout?.on('error', () => {
    flush()
    stdoutDone = true
    finishEvents()
  })
  child.stderr?.on('error', () => {})
  child.stdin?.on('error', () => {})
  child.stdout?.on('end', () => {
    flush()
    stdoutDone = true
    finishEvents()
  })
  child.on('error', (err: Error) => {
    spawnError = err.message
    flush()
    stdoutDone = true
    closed = true
    settle(null, err.message)
    finishEvents()
  })
  child.on('close', (code) => {
    flush()
    stdoutDone = true
    closed = true
    settle(spawnError !== null ? null : code, spawnError ?? stderr)
    finishEvents()
  })

  try {
    child.stdin?.end(prompt)
  } catch {
    // Spawn can fail before stdin is writable.
  }

  const kill = (): void => {
    const pid = child.pid
    if (pid === undefined) return
    if (process.platform === 'win32') {
      const killer = spawn('taskkill', ['/pid', String(pid), '/T', '/F'], {
        windowsHide: true,
        shell: false,
        stdio: 'ignore'
      })
      killer.on('error', () => {})
      killer.unref()
    } else {
      try {
        process.kill(-pid, 'SIGKILL')
      } catch {
        // Already exited, or it was never a process-group leader.
      }
    }
  }

  return { events: queue.events, exit, kill }
}

function asText(chunk: string | Buffer): string {
  return typeof chunk === 'string' ? chunk : chunk.toString('utf8')
}

interface EventQueue {
  push(event: AgentEvent): void
  end(): void
  events: AsyncIterable<AgentEvent>
}

function createEventQueue(): EventQueue {
  const buffer: AgentEvent[] = []
  let ended = false
  let pending: ((result: IteratorResult<AgentEvent>) => void) | null = null

  const iterator: AsyncIterator<AgentEvent> = {
    next(): Promise<IteratorResult<AgentEvent>> {
      const event = buffer.shift()
      if (event !== undefined) return Promise.resolve({ value: event, done: false })
      if (ended) return Promise.resolve({ value: undefined, done: true })
      return new Promise((resolve) => {
        pending = resolve
      })
    }
  }

  return {
    push(event: AgentEvent): void {
      if (ended) return
      const resolve = pending
      if (resolve) {
        pending = null
        resolve({ value: event, done: false })
      } else {
        buffer.push(event)
      }
    },
    end(): void {
      if (ended) return
      ended = true
      if (pending) {
        const resolve = pending
        pending = null
        resolve({ value: undefined, done: true })
      }
    },
    events: {
      [Symbol.asyncIterator](): AsyncIterator<AgentEvent> {
        return iterator
      }
    }
  }
}
