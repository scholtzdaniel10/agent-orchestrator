import { expect, test } from 'vitest'
import {
  LIST_MAX,
  edgeEnabled,
  matchTyped,
  nextEnabled,
  placeList,
  type SelectOption
} from './select-logic'

const options: SelectOption[] = [
  { value: 'auto', label: 'auto' },
  { value: 'claude', label: 'claude', disabled: true },
  { value: 'cursor', label: 'cursor' },
  { value: 'grok', label: 'grok' },
  { value: 'both', label: 'both (compare)', disabled: true }
]

test('arrow keys skip disabled options and stop at the ends', () => {
  expect(nextEnabled(options, 0, 1)).toBe(2)
  expect(nextEnabled(options, 2, -1)).toBe(0)
  expect(nextEnabled(options, 3, 1)).toBe(3)
  expect(nextEnabled(options, 0, -1)).toBe(0)
  expect(nextEnabled(options, -1, 1)).toBe(0)
})

test('Home and End find the first and last option that can be picked', () => {
  expect(edgeEnabled(options, 'first')).toBe(0)
  expect(edgeEnabled(options, 'last')).toBe(3)
  expect(edgeEnabled([{ value: 'x', label: 'x', disabled: true }], 'first')).toBe(-1)
  expect(edgeEnabled([], 'last')).toBe(-1)
})

test('typing jumps to the next matching label, wrapping and skipping disabled ones', () => {
  expect(matchTyped(options, 0, 'c')).toBe(2)
  expect(matchTyped(options, 2, 'C')).toBe(2)
  expect(matchTyped(options, 3, 'a')).toBe(0)
  expect(matchTyped(options, 0, 'gr')).toBe(3)
  expect(matchTyped(options, 0, 'both')).toBe(-1)
  expect(matchTyped(options, 0, '')).toBe(-1)
})

test('the list opens below its button, as wide as it, when there is room', () => {
  const placed = placeList(
    { top: 100, bottom: 128, left: 40, width: 200 },
    { width: 1200, height: 800 },
    180,
    120
  )
  expect(placed).toEqual({ top: 132, left: 40, width: 200, maxHeight: LIST_MAX, above: false })
})

test('near the bottom of the window the list opens above', () => {
  const placed = placeList(
    { top: 700, bottom: 728, left: 40, width: 200 },
    { width: 1200, height: 800 },
    240,
    200
  )
  expect(placed.above).toBe(true)
  expect(placed.top).toBe(700 - 4 - 240)
  expect(placed.maxHeight).toBe(LIST_MAX)
})

test('a wide list is pulled back inside the right edge of the window', () => {
  const placed = placeList(
    { top: 100, bottom: 128, left: 1100, width: 80 },
    { width: 1200, height: 800 },
    100,
    360
  )
  expect(placed.width).toBe(360)
  expect(placed.left).toBe(1200 - 8 - 360)
})

test('with little room either way the list keeps to the larger side and scrolls', () => {
  const placed = placeList(
    { top: 150, bottom: 178, left: 0, width: 100 },
    { width: 400, height: 300 },
    400,
    100
  )
  expect(placed.above).toBe(true)
  expect(placed.maxHeight).toBe(150 - 4 - 8)
  expect(placed.top).toBe(150 - 4 - placed.maxHeight)
  expect(placed.left).toBe(8)
})
