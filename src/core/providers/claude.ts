import { existsSync, mkdirSync, writeFileSync } from 'node:fs'
import { delimiter, join } from 'node:path'
import type {
  AgentEvent,
  BridgeInfo,
  Job,
  ModelOption,
  ProviderAdapter,
  RunHandle,
  RunOptions,
  UsageWindow
} from '../types'
import { runCaptured, spawnCli, type Bin, type CapturedRun, type LineParse } from './process'
import { parseCliVersion } from './versions'

const LIMIT_RE =
  /rate.?limit|usage limit|limit reached|hit your (usage )?limit|out of (usage|credits)/i

export class ClaudeAdapter implements ProviderAdapter {
  readonly id = 'claude' as const
  private readonly bin: Bin
  private versionCapture: Promise<CapturedRun> | null = null

  constructor(bin?: Bin) {
    this.bin = bin ?? resolveClaudeBin()
  }

  async isInstalled(): Promise<boolean> {
    this.versionCapture = runCaptured(this.bin, ['--version'])
    const { code } = await this.versionCapture
    return code === 0
  }

  async version(): Promise<string | null> {
    const captured =
      this.versionCapture ?? (this.versionCapture = runCaptured(this.bin, ['--version']))
    const { stdout } = await captured
    return parseCliVersion(this.id, stdout)
  }

  async isSignedIn(): Promise<boolean> {
    const { stdout } = await runCaptured(this.bin, ['auth', 'status'])
    try {
      const parsed: unknown = JSON.parse(stdout.trim())
      return isRecord(parsed) && parsed.loggedIn === true
    } catch {
      return false
    }
  }

  // Static: the Claude CLI has no command that lists models; help names these aliases.
  async listModels(): Promise<ModelOption[]> {
    return [
      { id: 'fable', label: 'Fable (latest)' },
      { id: 'opus', label: 'Opus (latest)' },
      { id: 'sonnet', label: 'Sonnet (latest)' },
      { id: 'haiku', label: 'Haiku (latest)' }
    ]
  }

  /** Interactive session: the same binary as `run`, with no `-p`. */
  interactive(modelOrOpts?: string | { model?: string; sessionId?: string; resume?: string }): {
    command: string
    args: string[]
    env?: NodeJS.ProcessEnv
  } {
    const opts = typeof modelOrOpts === 'string' ? { model: modelOrOpts } : (modelOrOpts ?? {})
    const args = [...(this.bin.args ?? [])]
    if (opts.resume) args.push('--resume', opts.resume)
    else if (opts.sessionId) args.push('--session-id', opts.sessionId)
    if (opts.model) args.push('--model', opts.model)
    return { command: this.bin.command, args }
  }

  run(job: Pick<Job, 'id' | 'prompt'>, cwd: string, opts?: RunOptions): RunHandle {
    const args = ['-p', '--output-format', 'stream-json', '--verbose']
    if (!opts?.bridge) args.push(...workerLimits(opts?.edit === true))
    if (opts?.edit) args.push('--permission-mode', 'acceptEdits')
    if (opts?.resume) args.push('--resume', opts.resume)
    if (opts?.bridge) {
      mkdirSync(cwd, { recursive: true })
      const configPath = join(cwd, 'orchestrator-mcp.json')
      writeFileSync(configPath, claudeMcpConfig(opts.bridge))
      const allowed = opts.bridge.tools.map((tool) => `mcp__orchestrator__${tool}`).join(',')
      args.push(
        '--mcp-config',
        configPath,
        '--strict-mcp-config',
        '--allowedTools',
        allowed,
        '--setting-sources',
        'project'
      )
    }
    if (opts?.model) args.push('--model', opts.model)
    return spawnCli(this.bin, args, job.prompt, cwd, (line) => this.parseLine(line))
  }

  parseEvent(line: string): AgentEvent | null {
    const parsed = this.parseLine(line)
    return parsed === null || parsed === 'ignored' ? null : parsed
  }

  private parseLine(line: string): LineParse {
    try {
      const value: unknown = JSON.parse(line)
      const event = mapClaude(value, (text) => this.isLimitError(text))
      if (event) return event
      return knownClaudeType(value) ? 'ignored' : null
    } catch {
      return null
    }
  }

  isLimitError(e: AgentEvent | string): boolean {
    if (typeof e === 'string') return LIMIT_RE.test(e)
    return e.kind === 'limit'
  }
}

const SHELL_TOOLS = ['Bash', 'PowerShell']
const WRITE_TOOLS = ['Edit', 'Write', 'NotebookEdit']

/**
 * The person's own Claude settings may allow a shell, edits or their MCP servers everywhere.
 * A worker never gets a shell or those servers, and writes only when it was allowed to edit.
 * Deny rules win over any allow rule or permission mode.
 */
function workerLimits(edit: boolean): string[] {
  const denied = edit ? SHELL_TOOLS : [...SHELL_TOOLS, ...WRITE_TOOLS]
  return ['--disallowedTools', denied.join(','), '--strict-mcp-config']
}

function claudeMcpConfig(bridge: BridgeInfo): string {
  return JSON.stringify({
    mcpServers: {
      orchestrator: {
        type: 'http',
        url: bridge.url,
        headers: { Authorization: `Bearer ${bridge.token}` }
      }
    }
  })
}

function resolveClaudeBin(): Bin {
  const file = process.platform === 'win32' ? 'claude.exe' : 'claude'
  for (const dir of (process.env.PATH ?? '').split(delimiter)) {
    if (!dir) continue
    const candidate = join(dir, file)
    if (existsSync(candidate)) return { command: candidate }
  }
  if (process.platform === 'win32' && process.env.APPDATA) {
    const fallback = join(
      process.env.APPDATA,
      'npm',
      'node_modules',
      '@anthropic-ai',
      'claude-code',
      'bin',
      'claude.exe'
    )
    if (existsSync(fallback)) return { command: fallback }
  }
  return { command: 'claude' }
}

function mapClaude(value: unknown, isLimitText: (text: string) => boolean): AgentEvent | null {
  if (!isRecord(value)) return null
  if (value.type === 'system' && value.subtype === 'init') return initEvent(value)
  if (value.type === 'system' && value.subtype === 'api_retry' && value.error === 'rate_limit') {
    return { kind: 'limit', message: 'rate_limit' }
  }
  if (value.type === 'assistant') {
    const text = assistantText(value)
    return text === null ? null : { kind: 'text', text }
  }
  if (value.type === 'rate_limit_event') return rateLimitEvent(value)
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

function rateLimitEvent(value: Record<string, unknown>): AgentEvent | null {
  const info = value.rate_limit_info
  if (!isRecord(info)) return null
  if (info.status === 'rejected') {
    const resetsAt = finiteNumber(info.resetsAt)
    return resetsAt === undefined
      ? { kind: 'limit', message: 'rate limit rejected' }
      : { kind: 'limit', message: 'rate limit rejected', resetsAt }
  }
  if (!isRecord(info.unifiedWindows)) return null
  const windows: UsageWindow[] = []
  let best: { utilization: number; resetsAt?: number } | null = null
  for (const [name, window] of Object.entries(info.unifiedWindows)) {
    if (!isRecord(window)) continue
    const utilization = finiteNumber(window.utilization)
    if (utilization === undefined) continue
    const resetsAt = finiteNumber(window.resetsAt)
    windows.push(resetsAt === undefined ? { name, utilization } : { name, utilization, resetsAt })
    if (!best || utilization > best.utilization) {
      best = resetsAt === undefined ? { utilization } : { utilization, resetsAt }
    }
  }
  if (!best) return null
  return best.resetsAt === undefined
    ? { kind: 'usage', utilization: best.utilization, windows }
    : { kind: 'usage', utilization: best.utilization, resetsAt: best.resetsAt, windows }
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
  if (typeof value.total_cost_usd === 'number') event.costUsd = value.total_cost_usd
  const usage = value.usage
  if (
    isRecord(usage) &&
    typeof usage.input_tokens === 'number' &&
    typeof usage.output_tokens === 'number'
  ) {
    event.tokens = usage.input_tokens + usage.output_tokens
  }
  return event
}

function finiteNumber(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined
}

function knownClaudeType(value: unknown): boolean {
  if (!isRecord(value)) return false
  return (
    value.type === 'system' ||
    value.type === 'assistant' ||
    value.type === 'rate_limit_event' ||
    value.type === 'result' ||
    value.type === 'user'
  )
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
