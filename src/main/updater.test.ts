import { afterEach, expect, test, vi } from 'vitest'
import { startAutoUpdate, type AutoUpdaterSubset } from './updater'

afterEach(() => {
  vi.useRealTimers()
  delete process.env.ORCH_NO_UPDATE
})

function fakeUpdater(
  checkForUpdates: AutoUpdaterSubset['checkForUpdates'] = vi.fn(() => Promise.resolve())
): AutoUpdaterSubset {
  return {
    autoDownload: false,
    autoInstallOnAppQuit: false,
    checkForUpdates
  }
}

test('does not touch the updater when not packaged', () => {
  vi.useFakeTimers()
  const updater = fakeUpdater()
  startAutoUpdate({ isPackaged: false, log: () => {} }, updater)
  expect(updater.autoDownload).toBe(false)
  expect(updater.autoInstallOnAppQuit).toBe(false)
  expect(updater.checkForUpdates).not.toHaveBeenCalled()
})

test('does not touch the updater when ORCH_NO_UPDATE is 1', () => {
  vi.useFakeTimers()
  process.env.ORCH_NO_UPDATE = '1'
  const updater = fakeUpdater()
  startAutoUpdate({ isPackaged: true, log: () => {} }, updater)
  expect(updater.autoDownload).toBe(false)
  expect(updater.autoInstallOnAppQuit).toBe(false)
  expect(updater.checkForUpdates).not.toHaveBeenCalled()
})

test('checks for updates once after 10 seconds when packaged', async () => {
  vi.useFakeTimers()
  delete process.env.ORCH_NO_UPDATE
  const updater = fakeUpdater()
  startAutoUpdate({ isPackaged: true, log: () => {} }, updater)
  expect(updater.autoDownload).toBe(true)
  expect(updater.autoInstallOnAppQuit).toBe(true)
  expect(updater.checkForUpdates).not.toHaveBeenCalled()
  await vi.advanceTimersByTimeAsync(10_000)
  expect(updater.checkForUpdates).toHaveBeenCalledTimes(1)
})

test('logs a rejected check and does not throw', async () => {
  vi.useFakeTimers()
  delete process.env.ORCH_NO_UPDATE
  const lines: string[] = []
  const updater = fakeUpdater(() => Promise.reject(new Error('update failed')))
  expect(() => {
    startAutoUpdate({ isPackaged: true, log: (line) => lines.push(line) }, updater)
  }).not.toThrow()
  await vi.advanceTimersByTimeAsync(10_000)
  expect(lines).toEqual(['update failed'])
})
