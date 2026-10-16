import { expect, test } from 'vitest'
import { ago, refBadges } from './history-format'

test('ago counts minutes, hours, days and months', () => {
  const now = 100 * 86_400_000
  expect(ago(now - 10_000, now)).toBe('now')
  expect(ago(now - 5 * 60_000, now)).toBe('5m')
  expect(ago(now - 3 * 3_600_000, now)).toBe('3h')
  expect(ago(now - 2 * 86_400_000, now)).toBe('2d')
  expect(ago(now - 90 * 86_400_000, now)).toBe('3mo')
})

test('refBadges marks the current branch and tags, and drops HEAD pointers', () => {
  expect(refBadges(['HEAD -> main', 'origin/main', 'origin/HEAD', 'tag: v1.1.1', 'HEAD'])).toEqual([
    { name: 'main', kind: 'head' },
    { name: 'origin/main', kind: 'branch' },
    { name: 'v1.1.1', kind: 'tag' }
  ])
})
