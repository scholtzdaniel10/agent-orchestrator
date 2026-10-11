import { expect, test } from 'vitest'
import { terminalDisplayTitle, terminalsInSidebarOrder, worktreeLabel } from './worktree-label'

test('worktreeLabel uses the name when present, else the id', () => {
  expect(worktreeLabel({ id: '3fa9c1d2' })).toBe('3fa9c1d2')
  expect(worktreeLabel({ id: '3fa9c1d2', name: undefined })).toBe('3fa9c1d2')
  expect(worktreeLabel({ id: '3fa9c1d2', name: '' })).toBe('3fa9c1d2')
  expect(worktreeLabel({ id: '3fa9c1d2', name: 'my-feature' })).toBe('my-feature')
})

test('terminalDisplayTitle swaps the id suffix for the worktree name', () => {
  const named = [{ id: '3fa9c1d2', name: 'my-feature' }]
  expect(terminalDisplayTitle({ title: 'claude 2 · 3fa9c1d2', change: '3fa9c1d2' }, named)).toBe(
    'claude 2 · my-feature'
  )
  expect(terminalDisplayTitle({ title: 'claude 2 · 3fa9c1d2', change: '3fa9c1d2' }, [])).toBe(
    'claude 2 · 3fa9c1d2'
  )
  expect(terminalDisplayTitle({ title: 'claude 1' }, named)).toBe('claude 1')
})

test('terminalsInSidebarOrder puts the project folder first, then sidebar order', () => {
  const changes = [{ id: 'wt-a' }, { id: 'wt-b' }]
  const terminals = [
    { id: '1', change: 'wt-b' },
    { id: '2' },
    { id: '3', change: 'wt-a' },
    { id: '4', change: 'wt-b' },
    { id: '5', change: 'gone' }
  ]
  expect(terminalsInSidebarOrder(terminals, changes).map((item) => item.id)).toEqual([
    '2',
    '3',
    '1',
    '4',
    '5'
  ])
})
