import { expect, test } from 'vitest'
import { updatedPhrase } from './usage-format'

test('how long ago the usage was read', () => {
  const now = 10 * 3_600_000
  expect(updatedPhrase(now - 20_000, now)).toBe('updated just now')
  expect(updatedPhrase(now - 4 * 60_000, now)).toBe('updated 4 min ago')
  expect(updatedPhrase(now - 2 * 3_600_000 - 60_000, now)).toBe('updated 2 h ago')
  expect(updatedPhrase(now + 60_000, now)).toBe('updated just now')
})
