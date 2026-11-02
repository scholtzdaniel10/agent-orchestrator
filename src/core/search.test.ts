import { expect, test } from 'vitest'
import type { JobRecord } from './router/orchestrator'
import { Store } from './router/store'
import {
  clampLimit,
  fieldLabel,
  highlightSegments,
  jobTitle,
  likePattern,
  markRanges,
  parseQuery,
  prefilterWord,
  SEARCH_LIMIT,
  SEARCH_LIMIT_MAX,
  snippetOf
} from './search'
import type { LeadMessage } from './types'

function job(partial: Partial<JobRecord> & Pick<JobRecord, 'id'>): JobRecord {
  return {
    type: 'planning',
    prompt: 'go',
    provider: 'claude',
    status: 'done',
    output: '',
    failedOver: [],
    ...partial
  }
}

function message(partial: Partial<LeadMessage> & Pick<LeadMessage, 'id' | 'text'>): LeadMessage {
  return { role: 'user', status: 'done', ...partial }
}

function withStore(fn: (store: Store) => void): void {
  const store = new Store(':memory:')
  try {
    fn(store)
  } finally {
    store.close()
  }
}

function chat(
  store: Store,
  id: string,
  title: string,
  updatedAt: number,
  messages: LeadMessage[],
  project = '/p'
): void {
  store.saveChat({
    id,
    project,
    title,
    created_at: updatedAt,
    updated_at: updatedAt,
    session_id: null,
    session_provider: null
  })
  messages.forEach((item, seq) => {
    store.saveMessage(id, seq, item)
  })
}

test('parseQuery splits words, keeps quoted phrases and drops repeats', () => {
  expect(parseQuery('  Login   BUG  ')).toEqual(['login', 'bug'])
  expect(parseQuery('"fix the  pager" test')).toEqual(['fix the pager', 'test'])
  expect(parseQuery('bug Bug BUG')).toEqual(['bug'])
  expect(parseQuery('say "hi')).toEqual(['say', 'hi'])
  expect(parseQuery('""   ')).toEqual([])
  expect(parseQuery('a b c d e f g h i j')).toHaveLength(8)
})

test('prefilterWord picks the longest word LIKE can find in stored JSON', () => {
  expect(prefilterWord(['auth', 'refresh token'])).toBe('refresh')
  expect(prefilterWord(['café'])).toBe(null)
  expect(prefilterWord(['café', 'db'])).toBe('db')
  expect(prefilterWord(['c:\\src'])).toBe(null)
  expect(prefilterWord([])).toBe(null)
})

test('likePattern escapes LIKE wildcards and the escape character', () => {
  expect(likePattern('100%')).toBe('%100\\%%')
  expect(likePattern('a_b')).toBe('%a\\_b%')
  expect(likePattern('a\\b')).toBe('%a\\\\b%')
})

test('clampLimit keeps the limit between 1 and the maximum', () => {
  expect(clampLimit(undefined)).toBe(SEARCH_LIMIT)
  expect(clampLimit(0)).toBe(1)
  expect(clampLimit(5.7)).toBe(5)
  expect(clampLimit(10_000)).toBe(SEARCH_LIMIT_MAX)
  expect(clampLimit(Number.NaN)).toBe(SEARCH_LIMIT)
})

test('finds a chat by a message and points at that message', () => {
  withStore((store) => {
    chat(store, 'c1', 'Pager work', 100, [
      message({ id: 'm1', text: 'Hello there' }),
      message({
        id: 'm2',
        role: 'lead',
        text: 'The pager skips the last row when the count is even.'
      })
    ])
    chat(store, 'c2', 'Other', 200, [message({ id: 'm3', text: 'nothing here' })])

    const hits = store.search('/p', 'LAST row')
    expect(hits).toHaveLength(1)
    expect(hits[0]).toMatchObject({
      kind: 'chat',
      id: 'c1',
      title: 'Pager work',
      at: 100,
      field: 'message',
      messageId: 'm2',
      role: 'lead',
      matches: 2
    })
    const hit = hits[0]
    if (hit === undefined) throw new Error('no hit')
    const marked = hit.marks.map(([start, end]) => hit.snippet.slice(start, end))
    expect(marked).toEqual(['last', 'row'])
  })
})

test('every term must appear, but they may be in different places', () => {
  withStore((store) => {
    chat(store, 'c1', 'Login flow', 100, [message({ id: 'm1', text: 'the refresh token expires' })])
    expect(store.search('/p', 'login token').map((hit) => hit.id)).toEqual(['c1'])
    expect(store.search('/p', 'login cookie')).toEqual([])
  })
})

test('a snippet prefers a message over the title when both hold the terms', () => {
  withStore((store) => {
    chat(store, 'c1', 'Fix pager', 100, [message({ id: 'm1', text: 'please fix the pager today' })])
    const hit = store.search('/p', 'pager')[0]
    expect(hit?.field).toBe('message')
    expect(hit?.messageId).toBe('m1')
  })
})

test('an untitled job shows a field other than its prompt when as many words match there', () => {
  withStore((store) => {
    store.saveJob(
      '/p',
      10,
      job({
        id: 'j1',
        prompt: 'Find why the pager skips rows',
        output: 'The off-by-one is in paginate().'
      })
    )
    store.saveJob(
      '/p',
      20,
      job({ id: 'j2', title: 'Rows', prompt: 'the pager', output: 'off-by-one' })
    )
    const [titled, untitled] = store.search('/p', 'off-by-one pager')
    expect(untitled).toMatchObject({
      id: 'j1',
      field: 'output',
      title: 'Find why the pager skips rows'
    })
    expect(titled).toMatchObject({ id: 'j2', field: 'prompt' })
  })
})

test('finds text in a lead plan', () => {
  withStore((store) => {
    chat(store, 'c1', 'Plan', 100, [
      message({
        id: 'm1',
        role: 'lead',
        text: 'Here is a plan.',
        proposal: {
          summary: 'Two jobs',
          status: 'waiting',
          jobs: [
            {
              title: 'Add tests',
              type: 'review',
              access: 'read',
              prompt: 'Cover the invoice parser'
            }
          ]
        }
      })
    ])
    expect(store.search('/p', 'invoice parser')[0]).toMatchObject({
      field: 'plan',
      messageId: 'm1'
    })
  })
})

test('finds jobs by prompt, output, error and steps', () => {
  withStore((store) => {
    store.saveJob(
      '/p',
      10,
      job({
        id: 'j1',
        title: 'Pager',
        prompt: 'look at the pager',
        provider: 'cursor',
        status: 'failed'
      })
    )
    store.saveJob(
      '/p',
      20,
      job({ id: 'j2', prompt: 'review', output: 'Found a race in the queue.' })
    )
    store.saveJob('/p', 30, job({ id: 'j3', prompt: 'build', error: 'ENOSPC: disk full' }))
    store.saveJob(
      '/p',
      40,
      job({
        id: 'j4',
        prompt: 'edit things',
        steps: [
          {
            id: 's1',
            tool: 'read',
            title: 'Read',
            detail: 'src/a.ts',
            status: 'done',
            startedAt: 1
          },
          {
            id: 's2',
            tool: 'edit',
            title: 'Edit',
            status: 'done',
            startedAt: 2,
            edit: { path: 'src/core/invoice.ts', added: 3, removed: 1 }
          }
        ]
      })
    )

    expect(store.search('/p', 'pager')[0]).toMatchObject({
      kind: 'job',
      id: 'j1',
      title: 'Pager',
      status: 'failed',
      provider: 'cursor',
      at: 10
    })
    expect(store.search('/p', 'race queue')[0]).toMatchObject({ id: 'j2', field: 'output' })
    expect(store.search('/p', 'enospc')[0]).toMatchObject({ id: 'j3', field: 'error' })
    expect(store.search('/p', 'invoice.ts')[0]).toMatchObject({
      id: 'j4',
      field: 'step',
      stepId: 's2'
    })
  })
})

test('JSON keys and other projects do not match', () => {
  withStore((store) => {
    store.saveJob('/p', 10, job({ id: 'j1', prompt: 'hello' }))
    store.saveJob('/other', 20, job({ id: 'j2', prompt: 'hello' }))
    chat(store, 'c9', 'hello', 30, [], '/other')
    // "failedOver" and "provider" are keys of every saved job, never its text.
    expect(store.search('/p', 'failedOver')).toEqual([])
    expect(store.search('/p', 'provider')).toEqual([])
    expect(store.search('/p', 'hello').map((hit) => hit.id)).toEqual(['j1'])
  })
})

test('LIKE wildcards in a query are literal', () => {
  withStore((store) => {
    store.saveJob('/p', 10, job({ id: 'j1', prompt: 'cut 1000 rows from a x b' }))
    store.saveJob('/p', 20, job({ id: 'j2', prompt: 'cut 100% of a_b' }))
    expect(store.search('/p', '100%').map((hit) => hit.id)).toEqual(['j2'])
    expect(store.search('/p', 'a_b').map((hit) => hit.id)).toEqual(['j2'])
    // The SQL narrowing itself treats them literally, not only the final check.
    expect(store.jobsMatching('/p', '100%').map((row) => row.job.id)).toEqual(['j2'])
    expect(store.jobsMatching('/p', 'a_b').map((row) => row.job.id)).toEqual(['j2'])
    expect(store.jobsMatching('/p', null).map((row) => row.job.id)).toEqual(['j2', 'j1'])
  })
})

test('chatsMatching narrows by title or any message and returns every message', () => {
  withStore((store) => {
    chat(store, 'c1', 'Billing', 10, [
      message({ id: 'm1', text: 'a' }),
      message({ id: 'm2', text: 'b' })
    ])
    chat(store, 'c2', 'Other', 20, [message({ id: 'm3', text: 'about BILLING runs' })])
    chat(store, 'c3', 'Nothing', 30, [message({ id: 'm4', text: 'c' })])
    const rows = store.chatsMatching('/p', 'billing')
    expect(rows.map((row) => row.chat.id)).toEqual(['c2', 'c1'])
    expect(rows[1]?.messages.map((item) => item.id)).toEqual(['m1', 'm2'])
    expect(store.chatsMatching('/p', null)).toHaveLength(3)
  })
})

test('non-ASCII text, quotes and backslashes are found case-insensitively', () => {
  withStore((store) => {
    store.saveJob('/p', 10, job({ id: 'j1', prompt: 'Menu du CAFÉ' }))
    store.saveJob('/p', 20, job({ id: 'j2', prompt: 'open C:\\Src\\App and say "hi"' }))
    chat(store, 'c1', 'Ünïcode', 30, [message({ id: 'm1', text: 'plain' })])
    expect(store.search('/p', 'café').map((hit) => hit.id)).toEqual(['j1'])
    expect(store.search('/p', 'c:\\src\\app').map((hit) => hit.id)).toEqual(['j2'])
    expect(store.search('/p', 'say "hi"').map((hit) => hit.id)).toEqual(['j2'])
    expect(store.search('/p', 'ünïcode').map((hit) => hit.id)).toEqual(['c1'])
  })
})

test('newest first across chats and jobs, cut to the limit', () => {
  withStore((store) => {
    store.saveJob('/p', 10, job({ id: 'j-old', prompt: 'deploy' }))
    chat(store, 'c-mid', 'deploy', 20, [])
    store.saveJob('/p', 30, job({ id: 'j-new', prompt: 'deploy again' }))
    expect(store.search('/p', 'deploy').map((hit) => hit.id)).toEqual(['j-new', 'c-mid', 'j-old'])
    expect(store.search('/p', 'deploy', 2).map((hit) => hit.id)).toEqual(['j-new', 'c-mid'])
    expect(store.search('/p', '   ')).toEqual([])
  })
})

test('job looks one saved job up by id', () => {
  withStore((store) => {
    store.saveJob('/p', 42, job({ id: 'j1', prompt: 'x' }))
    expect(store.job('j1')).toMatchObject({ project: '/p', createdAt: 42, job: { id: 'j1' } })
    expect(store.job('nope')).toBe(null)
  })
})

test('snippetOf cuts long text at words and marks every term', () => {
  const words = Array.from({ length: 60 }, (_, i) => `word${String(i)}`)
  const text = words.join(' ')
  const lower = text.toLowerCase()
  const { snippet, marks } = snippetOf({ text, lower }, ['word30'])
  expect(snippet.startsWith('…word')).toBe(true)
  expect(snippet.endsWith('…')).toBe(true)
  expect(snippet.length).toBeLessThanOrEqual(162)
  const shown = marks.map(([start, end]) => snippet.slice(start, end))
  expect(shown).toEqual(['word30'])

  const short = snippetOf({ text: 'Fix it', lower: 'fix it' }, ['fix'])
  expect(short).toEqual({ snippet: 'Fix it', marks: [[0, 3]] })
})

test('markRanges merges overlapping terms', () => {
  expect(markRanges('foobar foo', ['foo', 'oba'])).toEqual([
    [0, 5],
    [7, 10]
  ])
})

test('jobTitle uses the title, else the first line of the prompt', () => {
  expect(jobTitle({ title: ' Pager ', prompt: 'x' })).toBe('Pager')
  expect(jobTitle({ prompt: '\n\nFix the pager\nand more' })).toBe('Fix the pager')
  expect(jobTitle({ prompt: 'a'.repeat(200) })).toHaveLength(80)
})

test('highlightSegments splits text around marks and clips bad ranges', () => {
  expect(highlightSegments('fix the pager', [[4, 7]])).toEqual([
    { text: 'fix ', mark: false },
    { text: 'the', mark: true },
    { text: ' pager', mark: false }
  ])
  expect(highlightSegments('abc', [[0, 3]])).toEqual([{ text: 'abc', mark: true }])
  expect(
    highlightSegments('abc', [
      [2, 99],
      [1, 2]
    ])
  ).toEqual([
    { text: 'ab', mark: false },
    { text: 'c', mark: true }
  ])
  expect(highlightSegments('', [])).toEqual([])
})

test('fieldLabel names where the snippet came from', () => {
  expect(fieldLabel({ field: 'message', role: 'lead' })).toBe('lead')
  expect(fieldLabel({ field: 'message', role: 'user' })).toBe('you')
  expect(fieldLabel({ field: 'step' })).toBe('step')
})
