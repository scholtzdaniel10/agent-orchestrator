import { existsSync, mkdirSync, readdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import type { AgentEvent, BridgeInfo, Job, ProviderAdapter, RunHandle, RunOptions } from '../types'
import { runCaptured, spawnCli, type Bin } from './process'

const LIMIT_RE = /usage limit|rate.?limit|limit reached|out of (usage|credits)|spend limit|quota/i

export class CursorAdapter implements ProviderAdapter {
  readonly id = 'cursor' as const
  private readonly bin: Bin

  constructor(bin?: Bin) {
    this.bin = bin ?? resolveCursorBin()
  }

  async isInstalled(): Promise<boolean> {
    const { code } = await runCaptured(this.bin, ['--version'])
    return code === 0
  }

  async isSignedIn(): Promise<boolean> {
    const { code, stdout, stderr } = await runCaptured(this.bin, ['status'])
    return code === 0 && /logged in/i.test(stdout + stderr)
  }

  run(job: Pick<Job, 'id' | 'prompt'>, cwd: string, opts?: RunOptions): RunHandle {
    const args = ['-p', '--trust', '--output-format', 'stream-json']
    if (opts?.resume) args.push('--resume', opts.resume)
    if (opts?.bridge) {
      const cursorDir = join(cwd, '.cursor')
      mkdirSync(cursorDir, { recursive: true })
      writeFileSync(join(cursorDir, 'mcp.json'), cursorMcpConfig(opts.bridge))
      writeFileSync(join(cursorDir, 'cli.json'), cursorCliConfig())
      args.push('--approve-mcps')
    }
    return spawnCli(this.bin, args, job.prompt, cwd, (line) => this.parseEvent(line))
  }

  parseEvent(line: string): AgentEvent | null {
    try {
      return mapCursor(JSON.parse(line), (text) => this.isLimitError(text))
    } catch {
      return null
    }
  }

  isLimitError(e: AgentEvent | string): boolean {
    if (typeof e === 'string') return LIMIT_RE.test(e)
    return e.kind === 'limit'
  }
}

function cursorMcpConfig(bridge: BridgeInfo): string {
  return JSON.stringify({
    mcpServers: {
      orchestrator: {
        url: bridge.url,
        headers: { Authorization: `Bearer ${bridge.token}` }
      }
    }
  })
}

function cursorCliConfig(): string {
  return JSON.stringify({
    permissions: {
      allow: ['Mcp(orchestrator:*)'],
      deny: ['Shell(*)', 'Write(**)']
    }
  })
}

function resolveCursorBin(): Bin {
  if (process.platform !== 'win32') return { command: 'agent' }
  const localAppData = process.env.LOCALAPPDATA
  if (!localAppData) return { command: 'agent' }
  const versionsDir = join(localAppData, 'cursor-agent', 'versions')
  let names: string[]
  try {
    names = readdirSync(versionsDir, { withFileTypes: true })
      .filter((entry) => entry.isDirectory())
      .map((entry) => entry.name)
  } catch {
    return { command: 'agent' }
  }
  names.sort((a, b) => (a < b ? 1 : a > b ? -1 : 0))
  for (const name of names) {
    const dir = join(versionsDir, name)
    const node = join(dir, 'node.exe')
    const index = join(dir, 'index.js')
    if (existsSync(node) && existsSync(index)) return { command: node, args: [index] }
  }
  return { command: 'agent' }
}

function mapCursor(value: unknown, isLimitText: (text: string) => boolean): AgentEvent | null {
  if (!isRecord(value)) return null
  if (value.type === 'system' && value.subtype === 'init') return initEvent(value)
  if (value.type === 'assistant') {
    const text = assistantText(value)
    return text === null ? null : { kind: 'text', text }
  }
  if (value.type === 'result') return resultEvent(value, isLimitText)
  return null
}

function initEvent(value: Record<string, unknown>): AgentEvent | null {
  if (typeof value.session_id !== 'string') return null
  if (typeof value.model === 'string') {
    return { kind: 'init', sessionId: value.session_id, model: value.model }
  }
  return { kind: 'init', sessionId: value.session_id }
}

function assistantText(value: Record<string, unknown>): string | null {
  const message = value.message
  if (!isRecord(message) || !Array.isArray(message.content)) return null
  const parts: string[] = []
  for (const block of message.content) {
    if (isRecord(block) && block.type === 'text' && typeof block.text === 'string') {
      parts.push(block.text)
    }
  }
  return parts.length === 0 ? null : parts.join('')
}

function resultEvent(
  value: Record<string, unknown>,
  isLimitText: (text: string) => boolean
): AgentEvent {
  const text = typeof value.result === 'string' ? value.result : ''
  if (value.is_error === true && isLimitText(text)) return { kind: 'limit', message: text }
  const event: Extract<AgentEvent, { kind: 'result' }> = {
    kind: 'result',
    ok: value.is_error !== true,
    text
  }
  if (typeof value.session_id === 'string') event.sessionId = value.session_id
  if (typeof value.duration_ms === 'number') event.durationMs = value.duration_ms
  const usage = value.usage
  if (
    isRecord(usage) &&
    typeof usage.inputTokens === 'number' &&
    typeof usage.outputTokens === 'number'
  ) {
    event.tokens = usage.inputTokens + usage.outputTokens
  }
  return event
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
