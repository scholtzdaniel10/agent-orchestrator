import { createReadStream } from 'node:fs'
import { open, readdir, stat } from 'node:fs/promises'
import { homedir } from 'node:os'
import { join } from 'node:path'
import { createInterface } from 'node:readline'
import { DatabaseSync } from 'node:sqlite'

/** One past agent session found on this machine, as the Sessions tab lists it. */
export interface AgentSession {
  /** The CLI's own session id; resuming it opens the same conversation. */
  id: string
  provider: 'claude' | 'cursor'
  title: string
  /** The newest message, for the preview line. */
  last: string
  lastRole: 'you' | 'agent'
  /** Messages in the session; null for a long log, which is only sampled. */
  messages: number | null
  model: string | null
  /** Folder the session ran in; null when the log does not say. */
  cwd: string | null
  updatedAt: number
}

export interface SessionScan {
  sessions: AgentSession[]
  files: number
  scannedAt: number
}

const PREVIEW = 160
/** Logs past this are skipped: a session that size is a runaway, and reading it stalls a scan. */
const MAX_LOG_BYTES = 200 * 1024 * 1024
/**
 * Logs up to this are read whole. A longer one (a long session can pass 40 MB) is sampled: its
 * first and last part only, enough for the title, folder and newest message, so a scan never
 * reads hundreds of megabytes (and never makes the virus scanner read them too).
 */
const FULL_READ_BYTES = 4 * 1024 * 1024
const SAMPLE_BYTES = 512 * 1024

interface Cached {
  key: string
  session: AgentSession | null
}

/**
 * Reads the session logs Claude Code and the Cursor CLI keep in the person's home folder. Read
 * only: nothing here writes to them. Unchanged files are served from memory between scans.
 */
export class Sessions {
  private readonly home: string
  private readonly cache = new Map<string, Cached>()

  constructor(home: string = homedir()) {
    this.home = home
  }

  async scan(now: number = Date.now()): Promise<SessionScan> {
    const found = [...(await this.claudeLogs()), ...(await this.cursorStores())]
    const sessions: AgentSession[] = []
    const seen = new Set<string>()
    for (const file of found) {
      seen.add(file.path)
      const key = `${String(file.size)}:${String(file.mtime)}`
      let cached = this.cache.get(file.path)
      if (cached?.key !== key) {
        let session: AgentSession | null = null
        try {
          session =
            file.kind === 'claude'
              ? await readClaudeLog(file.path, file.mtime, file.size)
              : readCursorStore(file.path, file.mtime)
        } catch {
          session = null
        }
        cached = { key, session }
        this.cache.set(file.path, cached)
      }
      if (cached.session !== null) sessions.push(cached.session)
    }
    for (const path of this.cache.keys()) if (!seen.has(path)) this.cache.delete(path)
    sessions.sort((a, b) => b.updatedAt - a.updatedAt)
    return { sessions, files: found.length, scannedAt: now }
  }

  private async claudeLogs(): Promise<Found[]> {
    const root = join(this.home, '.claude', 'projects')
    const out: Found[] = []
    for (const dir of await names(root)) {
      for (const name of await names(join(root, dir))) {
        if (!name.endsWith('.jsonl')) continue
        const found = await statFile(join(root, dir, name), 'claude')
        if (found !== null) out.push(found)
      }
    }
    return out
  }

  private async cursorStores(): Promise<Found[]> {
    const root = join(this.home, '.cursor', 'chats')
    const out: Found[] = []
    for (const workspace of await names(root)) {
      for (const chat of await names(join(root, workspace))) {
        const found = await statFile(join(root, workspace, chat, 'store.db'), 'cursor')
        if (found !== null) out.push(found)
      }
    }
    return out
  }
}

interface Found {
  path: string
  kind: 'claude' | 'cursor'
  size: number
  mtime: number
}

async function names(dir: string): Promise<string[]> {
  try {
    return await readdir(dir)
  } catch {
    return []
  }
}

async function statFile(path: string, kind: Found['kind']): Promise<Found | null> {
  try {
    const info = await stat(path)
    if (!info.isFile() || info.size > MAX_LOG_BYTES) return null
    return { path, kind, size: info.size, mtime: info.mtimeMs }
  } catch {
    return null
  }
}

/** One Claude Code session log (`~/.claude/projects/<folder>/<id>.jsonl`), read line by line. */
export async function readClaudeLog(
  path: string,
  mtime: number,
  size = 0
): Promise<AgentSession | null> {
  const sampled = size > FULL_READ_BYTES
  let id: string | null = null
  let title: string | null = null
  let firstPrompt: string | null = null
  let lastLine: string | null = null
  let messages = 0
  let model: string | null = null
  let cwd: string | null = null
  const lines = sampled
    ? await sampleLines(path, size)
    : createInterface({ input: createReadStream(path, 'utf8'), crlfDelay: Infinity })
  for await (const line of lines) {
    // Most lines are tool output and snapshots; only these few are parsed.
    if (line.startsWith('{"type":"ai-title"')) {
      const row = parse(line)
      if (typeof row?.aiTitle === 'string') title = row.aiTitle
      continue
    }
    const user = line.includes('"type":"user"')
    const agent = !user && line.includes('"type":"assistant"')
    if (!user && !agent) continue
    const row = parse(line)
    if (row === null || (row.type !== 'user' && row.type !== 'assistant')) continue
    if (row.isSidechain === true) continue
    const text = messageText(row.message)
    if (text === '') continue
    messages += 1
    lastLine = line
    if (id === null && typeof row.sessionId === 'string') id = row.sessionId
    if (cwd === null && typeof row.cwd === 'string') cwd = row.cwd
    if (row.type === 'user' && firstPrompt === null) firstPrompt = text
    const message = row.message as { model?: unknown } | undefined
    if (row.type === 'assistant' && typeof message?.model === 'string') model = message.model
  }
  if (id === null || lastLine === null) return null
  const last = parse(lastLine)
  return {
    id,
    provider: 'claude',
    title: oneLine(title ?? firstPrompt ?? 'Untitled session'),
    last: oneLine(messageText(last?.message)),
    lastRole: last?.type === 'user' ? 'you' : 'agent',
    messages: sampled ? null : messages,
    model,
    cwd,
    updatedAt: mtime
  }
}

/** Whole lines from the first and last SAMPLE_BYTES of a file; a line cut at either edge is dropped. */
async function sampleLines(path: string, size: number): Promise<string[]> {
  const file = await open(path, 'r')
  try {
    const read = async (from: number): Promise<string> => {
      const buf = Buffer.alloc(Math.min(SAMPLE_BYTES, size - from))
      await file.read(buf, 0, buf.length, from)
      return buf.toString('utf8')
    }
    const head = (await read(0)).split(/\r?\n/).slice(0, -1)
    const tail = (await read(size - SAMPLE_BYTES)).split(/\r?\n/).slice(1)
    return [...head, ...tail]
  } finally {
    await file.close()
  }
}

/** One Cursor CLI chat (`~/.cursor/chats/<workspace>/<id>/store.db`), opened read only. */
export function readCursorStore(path: string, mtime: number): AgentSession | null {
  const db = new DatabaseSync(path, { readOnly: true })
  try {
    const metaRow = db.prepare('SELECT value FROM meta WHERE key = ?').get('0') as
      { value?: unknown } | undefined
    if (typeof metaRow?.value !== 'string') return null
    const meta = parse(Buffer.from(metaRow.value, 'hex').toString('utf8'))
    const id = typeof meta?.agentId === 'string' ? meta.agentId : null
    if (id === null) return null
    let messages = 0
    let firstPrompt: string | null = null
    let last: { role: string; text: string } | null = null
    let cwd: string | null = null
    // Blobs are content-addressed, so their order is not the chat's; createdAt-free ordering by
    // rowid is the best the store offers for "newest".
    const rows = db.prepare('SELECT data FROM blobs ORDER BY rowid').all() as { data: unknown }[]
    for (const blob of rows) {
      if (!(blob.data instanceof Uint8Array) || blob.data[0] !== 0x7b) continue
      const row = parse(Buffer.from(blob.data).toString('utf8'))
      if (row === null || (row.role !== 'user' && row.role !== 'assistant')) continue
      const raw = messageText(row)
      if (cwd === null) cwd = /Workspace Path: (.+)/.exec(raw)?.[1]?.trim() ?? null
      const query = /<user_query>\s*([\s\S]*?)\s*<\/user_query>/.exec(raw)?.[1]
      const text = row.role === 'user' ? (query ?? '') : raw
      if (text === '') continue
      messages += 1
      last = { role: row.role, text }
      if (row.role === 'user' && firstPrompt === null) firstPrompt = text
    }
    if (last === null) return null
    const named = typeof meta?.name === 'string' && meta.name !== 'New Agent' ? meta.name : null
    return {
      id,
      provider: 'cursor',
      title: oneLine(named ?? firstPrompt ?? 'Untitled chat'),
      last: oneLine(last.text),
      lastRole: last.role === 'user' ? 'you' : 'agent',
      messages,
      model: null,
      cwd,
      updatedAt: mtime
    }
  } finally {
    db.close()
  }
}

function parse(line: string): Record<string, unknown> | null {
  try {
    const value: unknown = JSON.parse(line)
    return typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : null
  } catch {
    return null
  }
}

/** The plain text of a message whose content is a string or a list of parts. */
function messageText(message: unknown): string {
  if (typeof message !== 'object' || message === null) return ''
  const content = (message as { content?: unknown }).content
  if (typeof content === 'string') return content.trim()
  if (!Array.isArray(content)) return ''
  return content
    .map((part: unknown) => {
      if (typeof part !== 'object' || part === null) return ''
      const item = part as { type?: unknown; text?: unknown }
      return item.type === 'text' && typeof item.text === 'string' ? item.text : ''
    })
    .join(' ')
    .trim()
}

function oneLine(text: string): string {
  // Notes the CLIs wrap in tags (system reminders, command output) are not what anyone typed.
  const untagged = text.replace(/<([a-z][\w-]*)[^>]*>[\s\S]*?<\/\1>/g, ' ')
  const flat = (untagged.trim() === '' ? text : untagged).replace(/\s+/g, ' ').trim()
  return flat.length > PREVIEW ? `${flat.slice(0, PREVIEW - 1)}…` : flat
}
