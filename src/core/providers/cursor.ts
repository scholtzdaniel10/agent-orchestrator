import { existsSync, mkdirSync, readdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import type {
  AgentEvent,
  BridgeInfo,
  Job,
  ModelOption,
  ProviderAdapter,
  RunHandle,
  RunOptions
} from '../types'
import { runCaptured, spawnCli, type Bin } from './process'

const LIMIT_RE = /usage limit|rate.?limit|limit reached|out of (usage|credits)|spend limit|quota/i

const ZERO_WIDTH_RE = /[\u200B-\u200D\uFEFF]/g

export class CursorAdapter implements ProviderAdapter {
  readonly id = 'cursor' as const
  private readonly bin: Bin
  private cachedModels: ModelOption[] | undefined

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

  async listModels(): Promise<ModelOption[]> {
    if (this.cachedModels !== undefined) return this.cachedModels
    const { code, stdout } = await runCaptured(this.bin, ['models'])
    if (code !== 0) return []
    const parsed = parseCursorModels(stdout)
    if (parsed.length === 0) return []
    this.cachedModels = parsed
    return parsed
  }

  /** Interactive session: the same node + index.js as `run`, with no `-p`. */
  interactive(model?: string): { command: string; args: string[]; env?: NodeJS.ProcessEnv } {
    return {
      command: this.bin.command,
      args: [...(this.bin.args ?? []), ...(model ? ['--model', model] : [])],
      env: cursorEnv(process.platform, process.env)
    }
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
    if (opts?.model) args.push('--model', opts.model)
    return spawnCli(
      this.bin,
      args,
      job.prompt,
      cwd,
      (line) => this.parseEvent(line),
      cursorEnv(process.platform, process.env)
    )
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

export function parseCursorModels(stdout: string): ModelOption[] {
  const models: ModelOption[] = []
  for (const raw of stdout.split(/\r?\n/)) {
    const line = raw.replace(ZERO_WIDTH_RE, '').trim()
    const sep = line.indexOf(' - ')
    if (sep <= 0) continue
    const id = line.slice(0, sep)
    const label = line.slice(sep + 3)
    if (!/^\S+$/.test(id) || label.length === 0) continue
    models.push({ id, label })
  }
  return models
}

/**
 * On Windows the Cursor CLI runs its hooks through bash when SHELL or MSYSTEM is set (the app
 * was started from Git Bash), but hooks installed there are PowerShell commands. They fail to
 * parse, and a failed beforeMCPExecution hook blocks every MCP tool call. Drop both variables.
 */
export function cursorEnv(
  platform: NodeJS.Platform,
  source: NodeJS.ProcessEnv
): NodeJS.ProcessEnv | undefined {
  if (platform !== 'win32') return undefined
  const env: NodeJS.ProcessEnv = {}
  for (const [key, value] of Object.entries(source)) {
    const upper = key.toUpperCase()
    if (upper !== 'SHELL' && upper !== 'MSYSTEM') env[key] = value
  }
  return env
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
