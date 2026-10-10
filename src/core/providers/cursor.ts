import { existsSync, mkdirSync, readdirSync, rmdirSync, unlinkSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import {
  type StepTool,
  accessOf,
  type AgentEvent,
  type BridgeInfo,
  type Job,
  type ModelOption,
  type ProviderAdapter,
  type RunHandle,
  type RunOptions
} from '../types'
import {
  capDetail,
  editSize,
  firstLine,
  relativeToCwd,
  reportedEditSize,
  StepParseState,
  writeSize
} from './edit-size'
import { runCaptured, spawnCli, type Bin, type CapturedRun, type LineParse } from './process'
import { parseCliVersion } from './versions'

const LIMIT_RE = /usage limit|rate.?limit|limit reached|out of (usage|credits)|spend limit|quota/i

const ZERO_WIDTH_RE = /[\u200B-\u200D\uFEFF]/g

export class CursorAdapter implements ProviderAdapter {
  readonly id = 'cursor' as const
  private readonly bin: Bin
  private cachedModels: ModelOption[] | undefined
  private versionCapture: Promise<CapturedRun> | null = null
  private readonly steps = new StepParseState()

  constructor(bin?: Bin) {
    this.bin = bin ?? resolveCursorBin()
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
  interactive(modelOrOpts?: string | { model?: string; sessionId?: string; resume?: string }): {
    command: string
    args: string[]
    env?: NodeJS.ProcessEnv
  } {
    const opts = typeof modelOrOpts === 'string' ? { model: modelOrOpts } : (modelOrOpts ?? {})
    const args = [...(this.bin.args ?? [])]
    const resume = opts.resume ?? opts.sessionId
    if (resume) args.push('--resume', resume)
    if (opts.model) args.push('--model', opts.model)
    return {
      command: this.bin.command,
      args,
      env: cursorEnv(process.platform, process.env)
    }
  }

  run(job: Pick<Job, 'id' | 'prompt'>, cwd: string, opts?: RunOptions): RunHandle {
    const args = ['-p', '--trust', '--output-format', 'stream-json']
    if (opts?.resume) args.push('--resume', opts.resume)
    const access = accessOf(opts ?? {})
    // bridge wins: its own permission file stays, and edit/full are ignored.
    if (access === 'full' && !opts?.bridge) args.push('--force')
    const cleanupEdit = access === 'edit' && !opts?.bridge ? allowEdits(cwd) : undefined
    if (opts?.bridge) {
      const cursorDir = join(cwd, '.cursor')
      mkdirSync(cursorDir, { recursive: true })
      writeFileSync(join(cursorDir, 'mcp.json'), cursorMcpConfig(opts.bridge))
      writeFileSync(join(cursorDir, 'cli.json'), cursorCliConfig())
      args.push('--approve-mcps')
    }
    if (opts?.model) args.push('--model', opts.model)
    const handle = spawnCli(
      this.bin,
      args,
      job.prompt,
      cwd,
      (line) => this.parseLine(line),
      cursorEnv(process.platform, process.env)
    )
    if (!cleanupEdit) return handle
    return {
      events: handle.events,
      kill: (): void => {
        handle.kill()
      },
      exit: handle.exit.then((result) => {
        cleanupEdit()
        return result
      })
    }
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
      const events = mapCursor(value, (text) => this.isLimitError(text), this.steps)
      if (events.length > 0) return events.length === 1 ? events[0] : events
      return knownCursorType(value) ? 'ignored' : null
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

const EDIT_CLI = '{"permissions":{"allow":["Read(**)","Write(**)"],"deny":["Shell(*)"]}}'

/** Write a headless edit permission file when the project has none, and remove only what we added. */
function allowEdits(cwd: string): (() => void) | undefined {
  const cursorDir = join(cwd, '.cursor')
  const cliPath = join(cursorDir, 'cli.json')
  if (existsSync(cliPath)) return undefined
  const createdDir = !existsSync(cursorDir)
  mkdirSync(cursorDir, { recursive: true })
  writeFileSync(cliPath, EDIT_CLI)
  return (): void => {
    try {
      unlinkSync(cliPath)
    } catch {
      // Already gone.
    }
    if (!createdDir) return
    try {
      if (readdirSync(cursorDir).length === 0) rmdirSync(cursorDir)
    } catch {
      // Not empty, or already gone.
    }
  }
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

function mapCursor(
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
  if (value.type === 'thinking') return thinkingEvents(value, state)
  if (value.type === 'assistant') return assistantEvents(value)
  if (value.type === 'tool_call') return toolCallEvents(value, state)
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

function thinkingEvents(value: Record<string, unknown>, state: StepParseState): AgentEvent[] {
  const raw = typeof value.text === 'string' ? value.text : ''
  const text = raw.trim()
  if (text === '') return []
  const id =
    typeof value.timestamp_ms === 'number'
      ? `think-${String(value.timestamp_ms)}`
      : state.nextId('think')
  return [
    {
      kind: 'step',
      id,
      phase: 'end',
      tool: 'think',
      title: firstLine(text),
      detail: capDetail(raw)
    }
  ]
}

function assistantEvents(value: Record<string, unknown>): AgentEvent[] {
  const message = value.message
  if (!isRecord(message) || !Array.isArray(message.content)) return []
  const parts: string[] = []
  for (const block of message.content) {
    if (isRecord(block) && block.type === 'text' && typeof block.text === 'string') {
      parts.push(block.text)
    }
  }
  const text = parts.join('')
  if (text === '') return []
  const sessionId = typeof value.session_id === 'string' ? value.session_id : 'x'
  const sayId =
    typeof message.id === 'string'
      ? message.id
      : typeof value.model_call_id === 'string'
        ? `say-${value.model_call_id}`
        : `say-${sessionId}-${firstLine(text)}`
  return [
    { kind: 'text', text },
    {
      kind: 'step',
      id: sayId,
      phase: 'end',
      tool: 'say',
      title: firstLine(text),
      detail: capDetail(text)
    }
  ]
}

function toolCallEvents(value: Record<string, unknown>, state: StepParseState): AgentEvent[] {
  const toolCall = isRecord(value.tool_call) ? value.tool_call : {}
  const entry = cursorToolEntry(toolCall)
  if (entry === null && typeof value.call_id !== 'string') return []
  const id = typeof value.call_id === 'string' ? value.call_id : state.nextId('tool')
  const sessionId = typeof value.session_id === 'string' ? value.session_id : undefined
  const cwd = state.cwd(sessionId)
  const phase: 'start' | 'end' =
    value.subtype === 'completed' || value.subtype === 'error' ? 'end' : 'start'
  if (entry === null) {
    const step: Extract<AgentEvent, { kind: 'step' }> = {
      kind: 'step',
      id,
      phase,
      tool: 'other',
      title: 'tool'
    }
    if (phase === 'end') step.ok = value.subtype !== 'error'
    return [step]
  }
  const args = isRecord(entry.body.args) ? entry.body.args : {}
  const tool = cursorToolKind(entry.key)
  const title = cursorTitle(entry.key, tool, args, cwd)
  const result = entry.body.result
  const edit = cursorEdit(tool, args, result, cwd)
  const step: Extract<AgentEvent, { kind: 'step' }> = {
    kind: 'step',
    id,
    phase,
    tool,
    title
  }
  if (phase === 'start') {
    const detail = cursorStartDetail(tool, args)
    if (detail !== undefined) step.detail = detail
    if (edit !== undefined) step.edit = edit
    state.rememberTool(id, { tool, title, ...(edit !== undefined ? { edit } : {}) })
  } else {
    step.ok = cursorResultOk(result) && value.subtype !== 'error'
    const detail = cursorResultDetail(result)
    if (detail !== undefined) step.detail = detail
    const pending = state.tool(id)
    if (edit !== undefined) step.edit = edit
    else if (pending?.edit !== undefined) step.edit = pending.edit
  }
  return [step]
}

function cursorToolEntry(
  toolCall: Record<string, unknown>
): { key: string; body: Record<string, unknown> } | null {
  for (const [key, val] of Object.entries(toolCall)) {
    if (key.endsWith('ToolCall') && isRecord(val)) return { key, body: val }
  }
  return null
}

function cursorToolKind(key: string): StepTool {
  const name = key.replace(/ToolCall$/i, '').toLowerCase()
  if (name === 'read') return 'read'
  if (
    name === 'write' ||
    name === 'edit' ||
    name === 'applypatch' ||
    name === 'strreplace' ||
    name === 'delete'
  ) {
    return 'edit'
  }
  if (name === 'shell' || name === 'awaitshell' || name === 'bash') return 'shell'
  if (
    name === 'glob' ||
    name === 'grep' ||
    name === 'ls' ||
    name === 'semsearch' ||
    name === 'search'
  ) {
    return 'search'
  }
  if (name === 'websearch' || name === 'webfetch' || name === 'web') return 'web'
  if (name === 'mcp' || name.startsWith('mcp')) return 'mcp'
  return 'other'
}

function cursorTitle(
  key: string,
  tool: StepTool,
  args: Record<string, unknown>,
  cwd: string | undefined
): string {
  const pathRaw = stringArg(args, ['path', 'filePath'])
  const path = pathRaw !== undefined ? relativeToCwd(pathRaw, cwd) : undefined
  if (tool === 'read') return path !== undefined ? `Read ${path}` : 'Read'
  if (tool === 'edit') return path !== undefined ? `Edited ${path}` : 'Edited'
  if (tool === 'shell') {
    const cmd = stringArg(args, ['command', 'cmd'])
    return cmd !== undefined ? `Ran ${firstLine(cmd)}` : 'Ran'
  }
  if (tool === 'search') {
    const query = stringArg(args, ['globPattern', 'pattern', 'query', 'glob_pattern'])
    return query !== undefined ? `Searched for ${firstLine(query)}` : 'Searched'
  }
  if (tool === 'web') {
    const query = stringArg(args, ['searchTerm', 'query', 'url'])
    return query !== undefined ? firstLine(query) : 'Web'
  }
  if (tool === 'mcp') {
    return stringArg(args, ['name', 'toolName']) ?? key.replace(/ToolCall$/i, '')
  }
  return key.replace(/ToolCall$/i, '')
}

function cursorStartDetail(tool: StepTool, args: Record<string, unknown>): string | undefined {
  if (tool !== 'shell') return undefined
  const cmd = stringArg(args, ['command', 'cmd'])
  return cmd !== undefined ? capDetail(cmd) : undefined
}

function cursorEdit(
  tool: StepTool,
  args: Record<string, unknown>,
  result: unknown,
  cwd: string | undefined
): { path: string; added: number; removed: number } | undefined {
  if (tool !== 'edit') return undefined
  const pathRaw = stringArg(args, ['path', 'filePath'])
  if (pathRaw === undefined) return undefined
  const path = relativeToCwd(pathRaw, cwd)
  const reported =
    reportedEditSize(result) ?? (isRecord(result) ? reportedEditSize(result.success) : null)
  if (reported !== null) return { path, ...reported }
  const oldText = stringArg(args, ['old_string', 'oldString']) ?? ''
  const newText = stringArg(args, ['new_string', 'newString']) ?? ''
  if (oldText !== '' || newText !== '') return { path, ...editSize(oldText, newText) }
  const content = stringArg(args, ['contents', 'fileContent', 'streamContent', 'content']) ?? ''
  if (content !== '') return { path, ...writeSize(content) }
  return { path, added: 0, removed: 0 }
}

function cursorResultOk(result: unknown): boolean {
  if (result === undefined || !isRecord(result)) return true
  if (result.error !== undefined || result.failure !== undefined || result.rejected !== undefined) {
    return false
  }
  if (result.success === false) return false
  return true
}

function cursorResultDetail(result: unknown): string | undefined {
  if (result === undefined) return undefined
  if (isRecord(result) && isRecord(result.success)) {
    const success = result.success
    if (typeof success.content === 'string') return capDetail(success.content)
    if (typeof success.stdout === 'string') return capDetail(success.stdout)
    if (typeof success.output === 'string') return capDetail(success.output)
    if (Array.isArray(success.files)) return capDetail(success.files.map(String).join('\n'))
  }
  if (isRecord(result) && isRecord(result.error)) {
    const message =
      typeof result.error.message === 'string' ? result.error.message : JSON.stringify(result.error)
    return capDetail(message)
  }
  try {
    return capDetail(JSON.stringify(result))
  } catch {
    return undefined
  }
}

function stringArg(input: Record<string, unknown>, keys: readonly string[]): string | undefined {
  for (const key of keys) {
    const value = input[key]
    if (typeof value === 'string' && value !== '') return value
  }
  return undefined
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

function knownCursorType(value: unknown): boolean {
  if (!isRecord(value)) return false
  return (
    value.type === 'system' ||
    value.type === 'assistant' ||
    value.type === 'result' ||
    value.type === 'thinking' ||
    value.type === 'tool_call' ||
    value.type === 'user'
  )
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
