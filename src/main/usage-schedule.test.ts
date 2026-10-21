import { expect, test } from 'vitest'
import { USAGE_GAP_MS, usageCheckDue } from './usage-schedule'

const NOW = 10_000_000
const focused = { focused: true, visible: true }
const visible = { focused: false, visible: true }
const hidden = { focused: false, visible: false }

test('coming back to the window checks at most every 30 seconds', () => {
  expect(usageCheckDue('focus', NOW, NOW - 29_000, focused)).toBe(false)
  expect(usageCheckDue('focus', NOW, NOW - USAGE_GAP_MS.focus, focused)).toBe(true)
})

test('Refresh keeps its two-minute gap', () => {
  expect(usageCheckDue('manual', NOW, NOW - 90_000, focused)).toBe(false)
  expect(usageCheckDue('manual', NOW, NOW - 2 * 60_000, focused)).toBe(true)
})

test('while a window has focus the timer checks every 5 minutes', () => {
  expect(usageCheckDue('tick', NOW, NOW - 4 * 60_000, focused)).toBe(false)
  expect(usageCheckDue('tick', NOW, NOW - 5 * 60_000, focused)).toBe(true)
})

test('on screen without focus it checks every 10 minutes, and not at all when hidden', () => {
  expect(usageCheckDue('tick', NOW, NOW - 6 * 60_000, visible)).toBe(false)
  expect(usageCheckDue('tick', NOW, NOW - 10 * 60_000, visible)).toBe(true)
  expect(usageCheckDue('tick', NOW, NOW - 60 * 60_000, hidden)).toBe(false)
})

test('with no reading yet any trigger checks, except a tick with nothing on screen', () => {
  expect(usageCheckDue('tick', NOW, null, visible)).toBe(true)
  expect(usageCheckDue('focus', NOW, null, hidden)).toBe(true)
  expect(usageCheckDue('tick', NOW, null, hidden)).toBe(false)
})
