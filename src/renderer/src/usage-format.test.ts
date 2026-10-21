import { expect, test } from 'vitest'
import { agoPhrase, dateFromValue, dateValue, resetChoices, updatedPhrase } from './usage-format'

test('how long ago the usage was read', () => {
  const now = 10 * 3_600_000
  expect(updatedPhrase(now - 20_000, now)).toBe('updated just now')
  expect(updatedPhrase(now - 4 * 60_000, now)).toBe('updated 4 min ago')
  expect(updatedPhrase(now - 2 * 3_600_000 - 60_000, now)).toBe('updated 2 h ago')
  expect(updatedPhrase(now + 60_000, now)).toBe('updated just now')
})

test('how old a dashboard reading is, in days once it is a day old', () => {
  const now = 10 * 24 * 3_600_000
  expect(agoPhrase(now - 20_000, now)).toBe('just now')
  expect(agoPhrase(now - 4 * 60_000, now)).toBe('4 min ago')
  expect(agoPhrase(now - 23 * 3_600_000, now)).toBe('23 h ago')
  expect(agoPhrase(now - 25 * 3_600_000, now)).toBe('1 day ago')
  expect(agoPhrase(now - 3 * 24 * 3_600_000, now)).toBe('3 days ago')
})

test('date input values round-trip through local midnight', () => {
  const nov5 = new Date(2026, 10, 5).getTime()
  expect(dateValue(nov5)).toBe('2026-11-05')
  expect(dateValue(new Date(2026, 10, 5, 23, 59).getTime())).toBe('2026-11-05')
  expect(dateFromValue('2026-11-05')).toBe(nov5)
  expect(dateFromValue('')).toBeNull()
  expect(dateFromValue('5 Nov')).toBeNull()
  expect(dateFromValue('2026-02-30')).toBeNull()
})

test('reset choices run from tomorrow to 32 days ahead', () => {
  const now = new Date(2026, 9, 21, 15, 30).getTime()
  const choices = resetChoices(now)
  expect(choices).toHaveLength(32)
  expect(choices[0]).toEqual({ value: '2026-10-22', label: 'Thu 22 Oct · tomorrow' })
  expect(choices[1]).toEqual({ value: '2026-10-23', label: 'Fri 23 Oct · in 2 days' })
  expect(choices[14]).toEqual({ value: '2026-11-05', label: 'Thu 5 Nov · in 15 days' })
  expect(choices[31]).toEqual({ value: '2026-11-22', label: 'Sun 22 Nov · in 32 days' })
  // Across a year end.
  expect(resetChoices(new Date(2026, 11, 31).getTime(), 1)).toEqual([
    { value: '2027-01-01', label: 'Fri 1 Jan · tomorrow' }
  ])
})
