import { expect, test } from 'vitest'
import {
  CASCADE_OFFSET,
  MAX_WINDOWS,
  PoppedOutRegistry,
  WINDOW_CAP_MESSAGE,
  cascadeBounds,
  refuseWindow
} from './windows'

test('refuseWindow allows up to six and names the seventh', () => {
  expect(refuseWindow(0)).toBeNull()
  expect(refuseWindow(MAX_WINDOWS - 1)).toBeNull()
  expect(refuseWindow(MAX_WINDOWS)).toBe(WINDOW_CAP_MESSAGE)
  expect(refuseWindow(MAX_WINDOWS + 1)).toBe(WINDOW_CAP_MESSAGE)
})

test('cascadeBounds is 32px down-right at the asked size', () => {
  expect(
    cascadeBounds(
      { x: 40, y: 80, width: 1200, height: 700 },
      { width: 1200, height: 700 },
      { x: 0, y: 0, width: 1920, height: 1080 }
    )
  ).toEqual({
    x: 40 + CASCADE_OFFSET,
    y: 80 + CASCADE_OFFSET,
    width: 1200,
    height: 700
  })
})

test('cascadeBounds keeps the window inside the work area', () => {
  expect(
    cascadeBounds(
      { x: 100, y: 100, width: 1400, height: 820 },
      { width: 1400, height: 820 },
      { x: 0, y: 0, width: 1500, height: 900 }
    )
  ).toEqual({ x: 100, y: 80, width: 1400, height: 820 })
})

test('cascadeBounds shrinks to the work area and stays on it', () => {
  expect(
    cascadeBounds(
      { x: 120, y: 60, width: 900, height: 600 },
      { width: 900, height: 600 },
      { x: 100, y: 50, width: 800, height: 500 }
    )
  ).toEqual({ x: 100, y: 50, width: 800, height: 500 })
})

test('popped-out registry adds, lists, and refuses a second window for the same terminal', () => {
  const popped = new PoppedOutRegistry()
  expect(popped.add('a')).toBe(true)
  expect(popped.add('a')).toBe(false)
  expect(popped.has('a')).toBe(true)
  expect(popped.list()).toEqual(['a'])
  expect(popped.add('b')).toBe(true)
  expect(popped.list()).toEqual(['a', 'b'])
})

test('popped-out registry removes an id and clears it when that terminal is gone', () => {
  const popped = new PoppedOutRegistry()
  popped.add('a')
  popped.add('b')
  popped.remove('a')
  expect(popped.has('a')).toBe(false)
  expect(popped.list()).toEqual(['b'])
  popped.remove('missing')
  expect(popped.list()).toEqual(['b'])
  expect(popped.add('a')).toBe(true)
})
