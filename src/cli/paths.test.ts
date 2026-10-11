import { normalize, resolve } from 'node:path'
import { expect, test } from 'vitest'
import { dataDir, legacyDataDir, resolveProject } from './paths'

test('data folder follows LEGATE_DATA, then the platform app folder', () => {
  expect(dataDir({ LEGATE_DATA: 'D:\\custom' }, 'win32', 'C:\\Users\\a')).toBe('D:\\custom')
  expect(dataDir({ LEGATE_DATA: '' }, 'linux', '/home/a')).toBe('/home/a/.config/Legate')
  expect(dataDir({ APPDATA: 'C:\\Users\\a\\AppData\\Roaming' }, 'win32', 'C:\\Users\\a')).toBe(
    'C:\\Users\\a\\AppData\\Roaming\\Legate'
  )
  expect(dataDir({}, 'win32', 'C:\\Users\\a')).toBe('C:\\Users\\a\\AppData\\Roaming\\Legate')
  expect(dataDir({}, 'darwin', '/Users/a')).toBe('/Users/a/Library/Application Support/Legate')
  expect(dataDir({ XDG_CONFIG_HOME: '/xdg' }, 'linux', '/home/a')).toBe('/xdg/Legate')
  expect(dataDir({}, 'linux', '/home/a')).toBe('/home/a/.config/Legate')
})

test('legacy data folder is the pre-rename app folder', () => {
  expect(legacyDataDir({ APPDATA: 'C:\\Roaming' }, 'win32', 'C:\\Users\\a')).toBe(
    'C:\\Roaming\\agent-orchestrator'
  )
  expect(legacyDataDir({}, 'darwin', '/Users/a')).toBe(
    '/Users/a/Library/Application Support/agent-orchestrator'
  )
  expect(legacyDataDir({}, 'linux', '/home/a')).toBe('/home/a/.config/agent-orchestrator')
})

test('resolveProject prefers --project, then git top, then the directory', async () => {
  const cwd = resolve('work', 'sub')
  expect(await resolveProject(cwd, undefined, () => Promise.resolve(null))).toBe(normalize(cwd))
  expect(await resolveProject(cwd, undefined, () => Promise.resolve(resolve('work')))).toBe(
    normalize(resolve('work'))
  )
  expect(await resolveProject(cwd, 'other', () => Promise.resolve(resolve('work')))).toBe(
    normalize(resolve(cwd, 'other'))
  )
})
