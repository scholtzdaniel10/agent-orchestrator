import type { StepTool } from '../types'

/** Number of lines in a string. A trailing newline does not add an extra line. */
export function lineCount(text: string): number {
  if (text === '') return 0
  const parts = text.split('\n')
  if (parts[parts.length - 1] === '') parts.pop()
  return parts.length
}

/** Edit: lines of `newText` vs `oldText`. Write: `writeSize`. MultiEdit: sum these. */
export function editSize(oldText: string, newText: string): { added: number; removed: number } {
  return { added: lineCount(newText), removed: lineCount(oldText) }
}

export function writeSize(content: string): { added: number; removed: number } {
  return { added: lineCount(content), removed: 0 }
}

export function sumEditSize(parts: ReadonlyArray<{ added: number; removed: number }>): {
  added: number
  removed: number
} {
  let added = 0
  let removed = 0
  for (const part of parts) {
    added += part.added
    removed += part.removed
  }
  return { added, removed }
}

/**
 * Counts a completed Cursor edit/write reports, when present.
 * Looks at common field names on the result object (or its `success` wrapper).
 */
export function reportedEditSize(value: unknown): { added: number; removed: number } | null {
  if (!isRecord(value)) return null
  const source = isRecord(value.success) ? value.success : value
  const added = pickCount(source, ['added', 'addedLines', 'insertions', 'linesAdded'])
  const removed = pickCount(source, ['removed', 'removedLines', 'deletions', 'linesRemoved'])
  if (added === null || removed === null) return null
  return { added, removed }
}

/** Path relative to cwd when it is inside it, with forward slashes. */
export function relativeToCwd(filePath: string, cwd: string | undefined): string {
  const file = filePath.replace(/\\/g, '/')
  if (cwd === undefined || cwd === '') return file
  let root = cwd.replace(/\\/g, '/')
  if (root.endsWith('/')) root = root.slice(0, -1)
  if (file === root) return '.'
  const prefix = `${root}/`
  if (file.startsWith(prefix)) return file.slice(prefix.length)
  if (file.toLowerCase().startsWith(prefix.toLowerCase())) return file.slice(prefix.length)
  return file
}

export function capDetail(text: string): string {
  if (text.length <= 4000) return text
  return text.slice(-4000)
}

export function firstLine(text: string, max = 120): string {
  const line = (text.split(/\r?\n/, 1)[0] ?? '').trimEnd()
  if (line.length <= max) return line
  return line.slice(0, max)
}

function pickCount(value: Record<string, unknown>, keys: readonly string[]): number | null {
  for (const key of keys) {
    const n = value[key]
    if (typeof n === 'number' && Number.isFinite(n) && n >= 0) return n
  }
  return null
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

export interface PendingStep {
  tool: StepTool
  title: string
  edit?: { path: string; added: number; removed: number }
}

/** Per-adapter memory so start/end pair and paths can be relative to the session cwd. */
export class StepParseState {
  private readonly cwdBySession = new Map<string, string>()
  private readonly pending = new Map<string, PendingStep>()
  private seq = 0

  cwd(sessionId: string | undefined): string | undefined {
    if (sessionId === undefined) return undefined
    return this.cwdBySession.get(sessionId)
  }

  rememberCwd(sessionId: string, cwd: string): void {
    this.cwdBySession.set(sessionId, cwd)
  }

  rememberTool(id: string, info: PendingStep): void {
    this.pending.set(id, info)
  }

  tool(id: string): PendingStep | undefined {
    return this.pending.get(id)
  }

  nextId(prefix: string): string {
    this.seq += 1
    return `${prefix}-${String(this.seq)}`
  }
}
