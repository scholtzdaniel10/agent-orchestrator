import { expect, test } from 'vitest'
import { markSetupSeen, setupSeen } from './setup-seen'

function memory(): {
  getItem: (key: string) => string | null
  setItem: (key: string, value: string) => void
} {
  const data = new Map<string, string>()
  return {
    getItem: (key) => data.get(key) ?? null,
    setItem: (key, value) => {
      data.set(key, value)
    }
  }
}

test('a new install has not seen Setup; closing it once is remembered', () => {
  const storage = memory()
  expect(setupSeen(storage)).toBe(false)
  markSetupSeen(storage)
  expect(setupSeen(storage)).toBe(true)
})

test('without storage, or with storage that throws, Setup counts as seen', () => {
  expect(setupSeen(null)).toBe(true)
  const broken = {
    getItem: (): string | null => {
      throw new Error('blocked')
    },
    setItem: (): void => {
      throw new Error('blocked')
    }
  }
  expect(setupSeen(broken)).toBe(true)
  expect(() => {
    markSetupSeen(broken)
  }).not.toThrow()
  expect(() => {
    markSetupSeen(null)
  }).not.toThrow()
})
