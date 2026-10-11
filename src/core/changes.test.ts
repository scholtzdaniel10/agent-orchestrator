import { expect, test } from 'vitest'
import { mergeListedChanges } from './changes'
import type { ChangeSet } from './types'

function change(id: string, files = 1): ChangeSet {
  return {
    id,
    branch: `orch/${id}`,
    path: `/wt/${id}`,
    files: files === 0 ? [] : [{ path: 'a.ts', insertions: 2, deletions: 1 }],
    insertions: files === 0 ? 0 : 2,
    deletions: files === 0 ? 0 : 1
  }
}

test('merge keeps worktrees with no edits, names, setup, and newest first', () => {
  const merged = mergeListedChanges({
    listed: [change('bbbbbbbb')],
    ids: ['bbbbbbbb', 'aaaaaaaa'],
    names: [
      { id: 'aaaaaaaa', name: 'login', created_at: 20 },
      { id: 'bbbbbbbb', name: '', created_at: 5 }
    ],
    folder: (id) => `/root/${id}`,
    setup: (id) => (id === 'aaaaaaaa' ? 'running' : undefined)
  })
  expect(merged.map((item) => item.id)).toEqual(['aaaaaaaa', 'bbbbbbbb'])
  expect(merged[0]).toMatchObject({
    name: 'login',
    path: '/root/aaaaaaaa',
    files: [],
    setup: 'running'
  })
  expect(merged[1]?.name).toBeUndefined()
  expect(merged[1]?.files).toHaveLength(1)
})
