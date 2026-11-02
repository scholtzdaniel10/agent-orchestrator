import { expect, test } from 'vitest'
import {
  belowCritical,
  formatGb,
  memoryLimits,
  memoryStopMessage,
  START_SPACING_MS,
  startHold
} from './memory-guard'

const GIB = 1024 ** 3

test('limits scale with the computer and never drop below a floor', () => {
  expect(memoryLimits(8 * GIB)).toEqual({ reserve: 2 * GIB, perJob: 1.5 * GIB, critical: 1 * GIB })
  const big = memoryLimits(32 * GIB)
  expect(big.reserve).toBeCloseTo(4.8 * GIB)
  expect(big.critical).toBeCloseTo(2.56 * GIB)
})

test('a job starts only with room for it on top of the reserve, and not right after another', () => {
  const total = 16 * GIB
  // Reserve 2.4 GB + 1.5 GB for the job.
  expect(startHold({ free: 3.8 * GIB, total }, -Infinity, 0)).toBe('memory')
  expect(startHold({ free: 4 * GIB, total }, -Infinity, 0)).toBe(null)
  expect(startHold({ free: 12 * GIB, total }, 1_000, 1_000 + START_SPACING_MS - 1)).toBe('spacing')
  expect(startHold({ free: 12 * GIB, total }, 1_000, 1_000 + START_SPACING_MS)).toBe(null)
  // Short on memory wins over spacing, so the job says why it waits.
  expect(startHold({ free: 1 * GIB, total }, 1_000, 1_001)).toBe('memory')
})

test('critical is below the reserve', () => {
  const total = 16 * GIB
  expect(belowCritical({ free: 1.2 * GIB, total })).toBe(true)
  expect(belowCritical({ free: 1.4 * GIB, total })).toBe(false)
})

test('messages give sizes in GB with one decimal', () => {
  expect(formatGb(1.25 * GIB)).toBe('1.3 GB')
  expect(formatGb(-5)).toBe('0.0 GB')
  expect(memoryStopMessage({ free: 0.9 * GIB, total: 16 * GIB })).toBe(
    'Stopped to free memory: only 0.9 GB of 16.0 GB was left.'
  )
})
