import { existsSync, mkdirSync, writeFileSync } from 'node:fs'
import { delimiter, join } from 'node:path'
import {
  type StepTool,
  accessOf,
  type AgentEvent,
  type BridgeInfo,
  type Job,
  type JobAccess,
  type ModelOption,
  type ProviderAdapter,
  type RunHandle,
  type RunOptions,
  type UsageWindow
} from '../types'
import {
  capDetail,
  editSize,
  firstLine,
  relativeToCwd,
  StepParseState,
  sumEditSize,
  writeSize
} from './edit-size'
import { runCaptured, spawnCli, type Bin, type CapturedRun, type LineParse } from './process'
import { parseCliVersion } from './versions'

const LIMIT_RE =
  /rate.?limit|usage limit|limit reached|hit your (usage )?limit|out of (usage|credits)/i

export class ClaudeAdapter implements ProviderAdapter {
  readonly id = 'claude' as const
  private readonly bin: Bin
  private versionCapture: Promise<CapturedRun> | null = null
  private readonly steps = new StepParseState()

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
    // The CLI has no command that lists models. A short name means the newest one the installed
    // CLI knows, which lags behind: 2.1.258 turns `opus` into Opus 5 and rejects the 5.5 names.
    return [
      { id: 'opus', label: 'Opus, newest this CLI knows' },
      { id: 'claude-opus-5-5', label: 'Opus 5.5' },
      { id: 'claude-opus-5', label: 'Opus 5' },
      { id: 'sonnet', label: 'Sonnet, newest this CLI knows' },
      { id: 'claude-sonnet-5-5', label: 'Sonnet 5.5' },
      { id: 'claude-sonnet-5', label: 'Sonnet 5' },
      { id: 'haiku', label: 'Haiku, newest this CLI knows' },
      { id: 'claude-haiku-5-5', label: 'Haiku 5.5' },
      { id: 'fable', label: 'Fable, needs usage credits on Pro' },
      { id: 'claude-fable-5-1', label: 'Fable 5.1, needs usage credits on Pro' }
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
    if (!opts?.bridge) args.push(...workerArgs(accessOf(opts ?? {})))
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
    return this.parseEvents(line)[0] ?? null
  }

  parseEvents(line: string): AgentEvent[] {
    const parsed = this.parseLine(line)
    if (parsed === null || parsed === 'ignored') return []
    return Array.isArray(parsed) ? parsed : [parsed]
  }

  private parseLine(line: string): LineParse {
    try {
      const value: unknown = JSON.parse(line)
      const events = mapClaude(value, (text) => this.isLimitError(text), this.steps)
      if (events.length > 0) return events.length === 1 ? events[0] : events
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
 * A worker never gets those servers. Read and edit jobs never get a shell, and write only
 * when allowed to edit. A full-access job may run commands. Deny rules win over any allow
 * rule or permission mode.
 */
function workerArgs(access: JobAccess): string[] {
  if (access === 'full') {
    return ['--strict-mcp-config', '--permission-mode', 'bypassPermissions']
  }
  const limits = workerLimits(access === 'edit')
  if (access === 'edit') return [...limits, '--permission-mode', 'acceptEdits']
  return limits
}

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

function mapClaude(
  value: unknown,
  isLimitText: (text: string) => boolean,
  state: StepParseState
): AgentEvent[] {
  if (!isRecord(value)) return []
  if (value.type === 'system' && value.subtype === 'init') {
    if (typeof value.session_id === 'string' && typeof value.cwd === 'string') {
      state.rememberCwd(value.session_id, value.cwd)
    }
    const event = initEvent(value)
    return event ? [event] : []
  }
  if (value.type === 'system' && value.subtype === 'api_retry' && value.error === 'rate_limit') {
    return [{ kind: 'limit', message: 'rate_limit' }]
  }
  if (value.type === 'assistant') return assistantEvents(value, state)
  if (value.type === 'user') return userEvents(value, state)
  if (value.type === 'rate_limit_event') {
    const event = rateLimitEvent(value)
    return event ? [event] : []
  }
  if (value.type === 'result') return [resultEvent(value, isLimitText)]
  return []
}

function initEvent(value: Record<string, unknown>): AgentEvent | null {
  if (typeof value.session_id !== 'string') return null
  if (typeof value.model === 'string') {
    return { kind: 'init', sessionId: value.session_id, model: value.model }
  }
  return { kind: 'init', sessionId: value.session_id }
}

function assistantEvents(value: Record<string, unknown>, state: StepParseState): AgentEvent[] {
  const message = value.message
  if (!isRecord(message) || !Array.isArray(message.content)) return []
  const sessionId = typeof value.session_id === 'string' ? value.session_id : undefined
  const cwd = state.cwd(sessionId)
  const events: AgentEvent[] = []
  const textParts: string[] = []
  const starts: AgentEvent[] = []
  for (const block of message.content) {
    if (!isRecord(block)) continue
    if (block.type === 'thinking') {
      const thinking = readableThinking(block)
      if (thinking === null) continue
      events.push({
        kind: 'step',
        id: state.nextId('think'),
        phase: 'end',
        tool: 'think',
        title: firstLine(thinking),
        detail: capDetail(thinking)
      })
      continue
    }
    if (block.type === 'text' && typeof block.text === 'string') {
      textParts.push(block.text)
      continue
    }
    if (block.type !== 'tool_use') continue
    const id = typeof block.id === 'string' ? block.id : state.nextId('tool')
    const name = typeof block.name === 'string' ? block.name : 'tool'
    const input = isRecord(block.input) ? block.input : {}
    const tool = claudeTool(name)
    const title = claudeTitle(name, tool, input, cwd)
    const edit = claudeEdit(name, input, cwd)
    const detail = claudeStartDetail(tool, input)
    const start: Extract<AgentEvent, { kind: 'step' }> = {
      kind: 'step',
      id,
      phase: 'start',
      tool,
      title
    }
    if (detail !== undefined) start.detail = detail
    if (edit !== undefined) start.edit = edit
    state.rememberTool(id, { tool, title, ...(edit !== undefined ? { edit } : {}) })
    starts.push(start)
  }
  const text = textParts.join('')
  if (text !== '') {
    events.push({ kind: 'text', text })
    const sayId = typeof message.id === 'string' ? message.id : state.nextId('say')
    events.push({
      kind: 'step',
      id: sayId,
      phase: 'end',
      tool: 'say',
      title: firstLine(text),
      detail: capDetail(text)
    })
  }
  for (const start of starts) events.push(start)
  return events
}

function userEvents(value: Record<string, unknown>, state: StepParseState): AgentEvent[] {
  const message = value.message
  if (!isRecord(message) || !Array.isArray(message.content)) return []
  const events: AgentEvent[] = []
  for (const block of message.content) {
    if (!isRecord(block) || block.type !== 'tool_result') continue
    const id = typeof block.tool_use_id === 'string' ? block.tool_use_id : state.nextId('tool')
    const pending = state.tool(id)
    const text = toolResultText(block)
    const step: Extract<AgentEvent, { kind: 'step' }> = {
      kind: 'step',
      id,
      phase: 'end',
      tool: pending?.tool ?? 'other',
      title: pending?.title ?? 'tool',
      ok: block.is_error !== true
    }
    if (text !== '') step.detail = capDetail(text)
    if (pending?.edit !== undefined) step.edit = pending.edit
    events.push(step)
  }
  return events
}

function claudeTool(name: string): StepTool {
  const lower = name.toLowerCase()
  if (lower === 'read') return 'read'
  if (lower === 'edit' || lower === 'write' || lower === 'multiedit' || lower === 'notebookedit') {
    return 'edit'
  }
  if (lower === 'bash' || lower === 'powershell' || lower === 'bashoutput') return 'shell'
  if (lower === 'grep' || lower === 'glob' || lower === 'ls') return 'search'
  if (lower === 'websearch' || lower === 'webfetch') return 'web'
  if (lower.startsWith('mcp__') || lower.startsWith('mcp_')) return 'mcp'
  return 'other'
}

function claudeTitle(
  name: string,
  tool: StepTool,
  input: Record<string, unknown>,
  cwd: string | undefined
): string {
  const path = pathFromInput(input, cwd)
  if (tool === 'read') return path !== undefined ? `Read ${path}` : 'Read'
  if (tool === 'edit') return path !== undefined ? `Edited ${path}` : 'Edited'
  if (tool === 'shell') {
    const cmd = stringArg(input, ['command', 'cmd'])
    return cmd !== undefined ? `Ran ${firstLine(cmd)}` : name
  }
  if (tool === 'search') {
    const query = stringArg(input, ['pattern', 'query', 'glob_pattern', 'globPattern'])
    return query !== undefined ? `Searched for ${firstLine(query)}` : name
  }
  if (tool === 'web') {
    const query = stringArg(input, ['query', 'url'])
    return query !== undefined ? firstLine(query) : name
  }
  if (tool === 'mcp') {
    const parts = name.split('__')
    return parts[parts.length - 1] ?? name
  }
  return name
}

function claudeStartDetail(tool: StepTool, input: Record<string, unknown>): string | undefined {
  if (tool !== 'shell') return undefined
  const cmd = stringArg(input, ['command', 'cmd'])
  return cmd !== undefined ? capDetail(cmd) : undefined
}

function claudeEdit(
  name: string,
  input: Record<string, unknown>,
  cwd: string | undefined
): { path: string; added: number; removed: number } | undefined {
  if (claudeTool(name) !== 'edit') return undefined
  const path = pathFromInput(input, cwd)
  if (path === undefined) return undefined
  const lower = name.toLowerCase()
  if (lower === 'write') {
    const content = typeof input.content === 'string' ? input.content : ''
    return { path, ...writeSize(content) }
  }
  if (lower === 'multiedit' && Array.isArray(input.edits)) {
    const parts = input.edits.map((item) => {
      if (!isRecord(item)) return { added: 0, removed: 0 }
      const oldText = typeof item.old_string === 'string' ? item.old_string : ''
      const newText = typeof item.new_string === 'string' ? item.new_string : ''
      return editSize(oldText, newText)
    })
    return { path, ...sumEditSize(parts) }
  }
  if (lower === 'notebookedit') {
    const content =
      typeof input.new_source === 'string'
        ? input.new_source
        : typeof input.content === 'string'
          ? input.content
          : ''
    if (content !== '') return { path, ...writeSize(content) }
  }
  const oldText = typeof input.old_string === 'string' ? input.old_string : ''
  const newText = typeof input.new_string === 'string' ? input.new_string : ''
  return { path, ...editSize(oldText, newText) }
}

function pathFromInput(
  input: Record<string, unknown>,
  cwd: string | undefined
): string | undefined {
  const raw = stringArg(input, ['file_path', 'path', 'filePath'])
  if (raw === undefined) return undefined
  return relativeToCwd(raw, cwd)
}

function stringArg(input: Record<string, unknown>, keys: readonly string[]): string | undefined {
  for (const key of keys) {
    const value = input[key]
    if (typeof value === 'string' && value !== '') return value
  }
  return undefined
}

function readableThinking(block: Record<string, unknown>): string | null {
  const text =
    typeof block.thinking === 'string'
      ? block.thinking
      : typeof block.text === 'string'
        ? block.text
        : ''
  const trimmed = text.trim()
  if (trimmed === '') return null
  return text
}

function toolResultText(block: Record<string, unknown>): string {
  const content = block.content
  if (typeof content === 'string') return content
  if (Array.isArray(content)) {
    const parts: string[] = []
    for (const item of content) {
      if (typeof item === 'string') parts.push(item)
      else if (isRecord(item) && typeof item.text === 'string') parts.push(item.text)
    }
    return parts.join('')
  }
  return ''
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
