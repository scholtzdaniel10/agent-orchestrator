import { parseQuery, type SearchHit } from '../../core/search'
import type { ProviderId } from '../../core/types'

/** Wait this long after the last key before searching. */
export const SEARCH_DEBOUNCE_MS = 150
/** Hits asked for per search. */
export const SEARCH_SHOWN = 50

const NAMES: Record<ProviderId, string> = { claude: 'Claude', cursor: 'Cursor', grok: 'Grok' }

/** True when the query holds at least one word to look for. */
export function searchable(query: string): boolean {
  return parseQuery(query).length > 0
}

/** The next highlighted row after an arrow key, wrapping at both ends; -1 with no rows. */
export function moveActive(current: number, delta: number, count: number): number {
  if (count <= 0) return -1
  if (current < 0) return delta < 0 ? count - 1 : 0
  return (((current + delta) % count) + count) % count
}

export function hitAge(at: number, now: number): string {
  const minutes = Math.floor(Math.max(0, now - at) / 60_000)
  if (minutes < 1) return 'just now'
  if (minutes < 60) return `${String(minutes)}m ago`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${String(hours)}h ago`
  return `${String(Math.floor(hours / 24))}d ago`
}

/** "Job · done · Cursor · 2h ago", "Chat · 3d ago", "Chat with Claude · 5m ago". */
export function hitMeta(hit: SearchHit, now: number): string {
  const parts: string[] = []
  if (hit.kind === 'job') {
    parts.push('Job')
    if (hit.status !== undefined) parts.push(hit.status)
    if (hit.provider !== undefined && hit.provider !== null) parts.push(NAMES[hit.provider])
  } else {
    parts.push(hit.direct !== undefined ? `Chat with ${NAMES[hit.direct]}` : 'Chat')
  }
  parts.push(hitAge(hit.at, now))
  return parts.join(' · ')
}

/** The line under the results: how many, and whether more were cut. */
export function resultsLine(query: string, count: number, limit = SEARCH_SHOWN): string {
  if (!searchable(query)) return 'Type to search chats and jobs in this project.'
  if (count === 0) return 'No chats or jobs here contain every word.'
  if (count >= limit) return `Newest ${String(limit)} results. Add a word to narrow them.`
  return count === 1 ? '1 result' : `${String(count)} results`
}
