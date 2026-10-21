import { expect, test } from 'vitest'
import {
  agoPhrase,
  dateFromValue,
  dateValue,
  daysBetween,
  monthTitle,
  monthWeeks,
  resetChoices,
  updatedPhrase,
  usageFigure
} from './usage-format'

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

test('reset choices run from tomorrow to 32 days ahead, labelled as Cursor does', () => {
  const now = new Date(2026, 9, 21, 15, 30).getTime()
  const choices = resetChoices(now)
  expect(choices).toHaveLength(32)
  expect(choices[0]).toEqual({ value: '2026-10-22', label: 'Oct 22 (1d)' })
  expect(choices[13]).toEqual({ value: '2026-11-04', label: 'Nov 4 (14d)' })
  expect(choices[31]).toEqual({ value: '2026-11-22', label: 'Nov 22 (32d)' })
  // Across a year end.
  expect(resetChoices(new Date(2026, 11, 31).getTime(), 1)).toEqual([
    { value: '2027-01-01', label: 'Jan 1 (1d)' }
  ])
})

test('days between counts calendar days, whatever the time of day', () => {
  const late = new Date(2026, 9, 21, 23, 59).getTime()
  expect(daysBetween(late, new Date(2026, 10, 4, 0, 1).getTime())).toBe(14)
  expect(daysBetween(late, late)).toBe(0)
  // Across the clocks going back, still whole days.
  expect(daysBetween(new Date(2026, 2, 1).getTime(), new Date(2026, 3, 1).getTime())).toBe(31)
})

test('a month as calendar weeks, Sunday first', () => {
  // 1 November 2026 is a Sunday; 30 days.
  const nov = monthWeeks(2026, 10)
  expect(nov[0]).toEqual([1, 2, 3, 4, 5, 6, 7])
  expect(nov.at(-1)).toEqual([29, 30, null, null, null, null, null])
  // 1 October 2026 is a Thursday.
  expect(monthWeeks(2026, 9)[0]).toEqual([null, null, null, null, 1, 2, 3])
  expect(
    monthWeeks(2026, 9)
      .flat()
      .filter((day) => day !== null)
  ).toHaveLength(31)
  expect(monthTitle(2026, 10)).toBe('November 2026')
})

test('a plan reads as what is left or what is used, always adding up to 100', () => {
  expect(usageFigure(0.18, 'left')).toBe(82)
  expect(usageFigure(0.18, 'used')).toBe(18)
  expect(usageFigure(0.775, 'left') + usageFigure(0.775, 'used')).toBe(100)
  expect(usageFigure(1.2, 'left')).toBe(0)
  expect(usageFigure(1.2, 'used')).toBe(100)
  expect(usageFigure(-0.1, 'used')).toBe(0)
  expect(usageFigure(Number.NaN, 'left')).toBe(0)
})
