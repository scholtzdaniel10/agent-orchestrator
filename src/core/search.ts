import type { JobRecord, JobStatus } from './router/orchestrator'
import type { ChatRow } from './router/store'
import { isProviderId, type LeadMessage, type ProviderId } from './types'

/** Hits returned when the caller names no limit. */
export const SEARCH_LIMIT = 50
export const SEARCH_LIMIT_MAX = 200
/** Longest query read; the rest is ignored. */
export const QUERY_MAX = 200
const MAX_TERMS = 8
/** Characters of context shown before the first match. */
const SNIPPET_BEFORE = 40
const SNIPPET_LENGTH = 160
const TITLE_MAX = 80

/** Where in a chat or job the shown snippet came from. */
export type SearchField = 'title' | 'message' | 'plan' | 'prompt' | 'output' | 'error' | 'step'

export interface SearchHit {
  kind: 'chat' | 'job'
  /** Chat id or job id. */
  id: string
  title: string
  /** Epoch ms: a chat's last message, or when a job was created. */
  at: number
  field: SearchField
  /** Text around the first match, whitespace collapsed, with … where it was cut. */
  snippet: string
  /** [start, end) ranges in `snippet` to highlight, sorted and not overlapping. */
  marks: [number, number][]
  /** Times the search terms appear in the whole chat or job. */
  matches: number
  /** Chat hits: the message the snippet came from. */
  messageId?: string
  role?: LeadMessage['role']
  /** Chat hits: the plan a direct chat talks to. */
  direct?: ProviderId
  /** Job hits. */
  status?: JobStatus
  provider?: ProviderId | null
  /** Job hits: the step the snippet came from. */
  stepId?: string
}

/** What a search reads. `word` narrows the rows cheaply; null means every row of the project. */
export interface SearchSource {
  chatsMatching(project: string, word: string | null): { chat: ChatRow; messages: LeadMessage[] }[]
  jobsMatching(project: string, word: string | null): { createdAt: number; job: JobRecord }[]
}

interface Field {
  field: SearchField
  text: string
  messageId?: string
  role?: LeadMessage['role']
  stepId?: string
}

interface Prepared {
  field: Field
  /** Whitespace collapsed, original case. */
  text: string
  /** Same length as `text`, lower case. */
  lower: string
}

/**
 * Lower-case search terms. Words split on whitespace; "a quoted phrase" stays one term.
 * Every term must appear somewhere in a chat or job for it to match. Empty means no search.
 */
export function parseQuery(query: string): string[] {
  const out: string[] = []
  const source = query.slice(0, QUERY_MAX)
  for (const match of source.matchAll(/"([^"]*)"|(\S+)/g)) {
    const raw = match[1] ?? (match[2] ?? '').replace(/^"+|"+$/g, '')
    const term = collapse(raw).toLowerCase()
    if (term === '' || out.includes(term)) continue
    out.push(term)
    if (out.length >= MAX_TERMS) break
  }
  return out
}

/**
 * The longest single word of the terms that a SQL `LIKE` can find in stored JSON as is:
 * printable ASCII with no quote or backslash (JSON escapes those, and `LIKE` only folds ASCII case).
 * Null when no word qualifies; then every row is read.
 */
export function prefilterWord(terms: readonly string[]): string | null {
  let best: string | null = null
  for (const term of terms) {
    for (const word of term.split(' ')) {
      if (word === '' || !/^[\x21-\x7e]+$/.test(word) || /["\\]/.test(word)) continue
      if (best === null || word.length > best.length) best = word
    }
  }
  return best
}

/** `%word%` with `\` escaping `%`, `_` and `\`, for `LIKE ? ESCAPE '\'`. */
export function likePattern(word: string): string {
  return `%${word.replace(/[\\%_]/g, '\\$&')}%`
}

export function clampLimit(limit: number | undefined): number {
  if (limit === undefined || !Number.isFinite(limit)) return SEARCH_LIMIT
  return Math.min(SEARCH_LIMIT_MAX, Math.max(1, Math.floor(limit)))
}

/** Chats and jobs of one project that contain every term, newest first. */
export function searchHistory(
  source: SearchSource,
  project: string,
  query: string,
  limit?: number
): SearchHit[] {
  const terms = parseQuery(query)
  if (terms.length === 0) return []
  const word = prefilterWord(terms)
  const hits: SearchHit[] = []
  for (const { chat, messages } of source.chatsMatching(project, word)) {
    const hit = chatHit(chat, messages, terms)
    if (hit !== null) hits.push(hit)
  }
  for (const { createdAt, job } of source.jobsMatching(project, word)) {
    const hit = jobHit(job, createdAt, terms)
    if (hit !== null) hits.push(hit)
  }
  hits.sort((a, b) => b.at - a.at || a.kind.localeCompare(b.kind) || a.id.localeCompare(b.id))
  return hits.slice(0, clampLimit(limit))
}

function chatHit(chat: ChatRow, messages: LeadMessage[], terms: string[]): SearchHit | null {
  const fields: Field[] = [{ field: 'title', text: chat.title }]
  for (const message of messages) {
    fields.push({ field: 'message', text: message.text, messageId: message.id, role: message.role })
    const proposal = message.proposal
    if (proposal !== undefined) {
      const parts = [proposal.summary]
      for (const job of proposal.jobs) parts.push(job.title, job.prompt)
      fields.push({
        field: 'plan',
        text: parts.join('\n'),
        messageId: message.id,
        role: message.role
      })
    }
  }
  const found = match(fields, terms, 'title')
  if (found === null) return null
  const best = found.best.field
  const direct = chat.direct ?? null
  return {
    kind: 'chat',
    id: chat.id,
    title: shortTitle(chat.title),
    at: chat.updated_at,
    field: best.field,
    ...snippetOf(found.best, terms),
    matches: found.matches,
    ...(best.messageId !== undefined ? { messageId: best.messageId } : {}),
    ...(best.role !== undefined ? { role: best.role } : {}),
    ...(isProviderId(direct) ? { direct } : {})
  }
}

function jobHit(job: JobRecord, createdAt: number, terms: string[]): SearchHit | null {
  const fields: Field[] = []
  if (typeof job.title === 'string' && job.title.trim() !== '') {
    fields.push({ field: 'title', text: job.title })
  }
  fields.push({ field: 'prompt', text: job.prompt }, { field: 'output', text: job.output })
  if (typeof job.error === 'string') fields.push({ field: 'error', text: job.error })
  for (const step of job.steps ?? []) {
    const parts = [step.title, step.detail ?? '', step.edit?.path ?? '']
    fields.push({ field: 'step', text: parts.join(' '), stepId: step.id })
  }
  // A job with no title of its own is listed under its prompt, so the prompt is the shown line.
  const shown = fields[0]?.field === 'title' ? 'title' : 'prompt'
  const found = match(fields, terms, shown)
  if (found === null) return null
  const best = found.best.field
  return {
    kind: 'job',
    id: job.id,
    title: jobTitle(job),
    at: createdAt,
    field: best.field,
    ...snippetOf(found.best, terms),
    matches: found.matches,
    status: job.status,
    provider: job.provider,
    ...(best.stepId !== undefined ? { stepId: best.stepId } : {})
  }
}

/**
 * Null unless every term is in some field. Picks the field holding the most distinct terms; on a
 * tie, one other than `shown`, the field the result is already titled with.
 */
function match(
  fields: Field[],
  terms: string[],
  shown: SearchField
): { best: Prepared; matches: number } | null {
  const seen = new Set<string>()
  let matches = 0
  let best: { prepared: Prepared; distinct: number } | null = null
  for (const field of fields) {
    if (typeof field.text !== 'string' || field.text === '') continue
    const prepared = prepare(field)
    let distinct = 0
    for (const term of terms) {
      const count = occurrences(prepared.lower, term)
      if (count === 0) continue
      distinct += 1
      matches += count
      seen.add(term)
    }
    if (distinct === 0) continue
    const better =
      best === null ||
      distinct > best.distinct ||
      (distinct === best.distinct && best.prepared.field.field === shown && field.field !== shown)
    if (better) best = { prepared, distinct }
  }
  if (best === null || seen.size < terms.length) return null
  return { best: best.prepared, matches }
}

function prepare(field: Field): Prepared {
  const text = collapse(field.text)
  return { field, text, lower: lowerSameLength(text) }
}

/** Text around the earliest match, cut at word boundaries, with the matches marked. */
export function snippetOf(
  prepared: { text: string; lower: string },
  terms: readonly string[]
): { snippet: string; marks: [number, number][] } {
  const { text, lower } = prepared
  let at = -1
  let length = 0
  for (const term of terms) {
    const index = lower.indexOf(term)
    if (index !== -1 && (at === -1 || index < at)) {
      at = index
      length = term.length
    }
  }
  if (at === -1) at = 0
  let start = Math.max(0, at - SNIPPET_BEFORE)
  if (start > 0) {
    const space = text.indexOf(' ', start)
    if (space !== -1 && space < at) start = space + 1
  }
  let end = Math.min(text.length, Math.max(start + SNIPPET_LENGTH, at + length))
  if (end < text.length) {
    const space = text.lastIndexOf(' ', end)
    if (space > at + length) end = space
  }
  const body = text.slice(start, end).trim()
  const snippet = `${start > 0 ? '…' : ''}${body}${end < text.length ? '…' : ''}`
  return { snippet, marks: markRanges(snippet, terms) }
}

/** Every place a term appears in `text`, merged where they touch or overlap. */
export function markRanges(text: string, terms: readonly string[]): [number, number][] {
  const lower = lowerSameLength(text)
  const ranges: [number, number][] = []
  for (const term of terms) {
    if (term === '') continue
    let from = 0
    for (;;) {
      const index = lower.indexOf(term, from)
      if (index === -1) break
      ranges.push([index, index + term.length])
      from = index + term.length
    }
  }
  ranges.sort((a, b) => a[0] - b[0] || a[1] - b[1])
  const merged: [number, number][] = []
  for (const range of ranges) {
    const last = merged[merged.length - 1]
    if (last !== undefined && range[0] <= last[1]) last[1] = Math.max(last[1], range[1])
    else merged.push([range[0], range[1]])
  }
  return merged
}

/** `text` cut into plain and marked pieces, in order. Marks out of range are clipped. */
export function highlightSegments(
  text: string,
  marks: readonly (readonly [number, number])[]
): { text: string; mark: boolean }[] {
  const out: { text: string; mark: boolean }[] = []
  let at = 0
  for (const [rawStart, rawEnd] of marks) {
    const start = Math.max(at, Math.min(text.length, rawStart))
    const end = Math.max(start, Math.min(text.length, rawEnd))
    if (start > at) out.push({ text: text.slice(at, start), mark: false })
    if (end > start) out.push({ text: text.slice(start, end), mark: true })
    at = end
  }
  if (at < text.length) out.push({ text: text.slice(at), mark: false })
  return out
}

/** Where the snippet came from, in a few words. */
export function fieldLabel(hit: Pick<SearchHit, 'field' | 'role'>): string {
  switch (hit.field) {
    case 'title':
      return 'title'
    case 'message':
      return hit.role === 'lead' ? 'lead' : 'you'
    case 'plan':
      return 'plan'
    case 'prompt':
      return 'prompt'
    case 'output':
      return 'output'
    case 'error':
      return 'error'
    case 'step':
      return 'step'
  }
}

function occurrences(lower: string, term: string): number {
  let count = 0
  let from = 0
  for (;;) {
    const index = lower.indexOf(term, from)
    if (index === -1) return count
    count += 1
    from = index + term.length
  }
}

function collapse(text: string): string {
  return text.replace(/\s+/g, ' ').trim()
}

/** Lower case that keeps every index: a character whose lower case is longer stays as it is. */
function lowerSameLength(text: string): string {
  const lower = text.toLowerCase()
  if (lower.length === text.length) return lower
  let out = ''
  for (const char of text) {
    const small = char.toLowerCase()
    out += small.length === char.length ? small : char
  }
  return out
}

function shortTitle(text: string): string {
  const line = collapse(text)
  if (line.length <= TITLE_MAX) return line
  return `${line.slice(0, TITLE_MAX - 1).trimEnd()}…`
}

/** A job's own title, else the first line of its prompt. */
export function jobTitle(job: Pick<JobRecord, 'title' | 'prompt'>): string {
  const own = typeof job.title === 'string' ? job.title.trim() : ''
  if (own !== '') return shortTitle(own)
  const first = job.prompt.split('\n').find((line) => line.trim() !== '') ?? ''
  return shortTitle(first)
}
