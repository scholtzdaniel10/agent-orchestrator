import { expect, test } from 'vitest'
import { terminalDisplayTitle, worktreeLabel } from './worktree-label'

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
