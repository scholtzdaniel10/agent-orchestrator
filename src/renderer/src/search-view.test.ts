import { expect, test } from 'vitest'
import type { SearchHit } from '../../core/search'
import { hitAge, hitMeta, moveActive, resultsLine, searchable, SEARCH_SHOWN } from './search-view'

const MIN = 60_000

function hit(partial: Partial<SearchHit>): SearchHit {
  return {
    kind: 'job',
    id: 'j1',
    title: 'Fix it',
    at: 0,
    field: 'prompt',
    snippet: 'fix it',
    marks: [],
    matches: 1,
    ...partial
  }
}

test('searchable needs at least one word', () => {
  expect(searchable('')).toBe(false)
  expect(searchable('   ')).toBe(false)
  expect(searchable('""')).toBe(false)
  expect(searchable('pager')).toBe(true)
})

test('moveActive wraps at both ends and starts from the top or bottom', () => {
  expect(moveActive(-1, 1, 3)).toBe(0)
  expect(moveActive(-1, -1, 3)).toBe(2)
  expect(moveActive(0, 1, 3)).toBe(1)
  expect(moveActive(2, 1, 3)).toBe(0)
  expect(moveActive(0, -1, 3)).toBe(2)
  expect(moveActive(1, 1, 0)).toBe(-1)
})

test('hitAge buckets like the sidebar, with "ago"', () => {
  expect(hitAge(1000, 1000)).toBe('just now')
  expect(hitAge(0, 5 * MIN)).toBe('5m ago')
  expect(hitAge(0, 125 * MIN)).toBe('2h ago')
  expect(hitAge(0, 3 * 24 * 60 * MIN)).toBe('3d ago')
  expect(hitAge(10 * MIN, 0)).toBe('just now')
})

test('hitMeta says what the hit is', () => {
  expect(hitMeta(hit({ status: 'done', provider: 'cursor', at: 0 }), 2 * 60 * MIN)).toBe(
    'Job · done · Cursor · 2h ago'
  )
  expect(hitMeta(hit({ status: 'queued', provider: null }), 0)).toBe('Job · queued · just now')
  expect(hitMeta(hit({ kind: 'chat' }), 5 * MIN)).toBe('Chat · 5m ago')
  expect(hitMeta(hit({ kind: 'chat', direct: 'grok' }), 5 * MIN)).toBe('Chat with Grok · 5m ago')
})

test('resultsLine explains empty, cut and normal results', () => {
  expect(resultsLine('', 0)).toBe('Type to search chats and jobs in this project.')
  expect(resultsLine('pager', 0)).toBe('No chats or jobs here contain every word.')
  expect(resultsLine('pager', 1)).toBe('1 result')
  expect(resultsLine('pager', 7)).toBe('7 results')
  expect(resultsLine('pager', SEARCH_SHOWN)).toBe(
    `Newest ${String(SEARCH_SHOWN)} results. Add a word to narrow them.`
  )
})
