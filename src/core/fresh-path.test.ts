import { expect, test } from 'vitest'
import { mergedPath, refreshPath, registryPath } from './fresh-path'

const USER_OUTPUT = [
  '',
  'HKEY_CURRENT_USER\\Environment',
  '    Path    REG_EXPAND_SZ    %USERPROFILE%\\AppData\\Local\\Microsoft\\WinGet\\Links;C:\\Tools\\',
  ''
].join('\r\n')

test('reads Path from reg query output', () => {
  expect(registryPath(USER_OUTPUT)).toBe(
    '%USERPROFILE%\\AppData\\Local\\Microsoft\\WinGet\\Links;C:\\Tools\\'
  )
  expect(registryPath('    Path    REG_SZ    C:\\A;C:\\B')).toBe('C:\\A;C:\\B')
  expect(registryPath('ERROR: The system was unable to find the specified registry key')).toBe('')
})

test('adds new registry entries after the current ones, expanded and without repeats', () => {
  const env = { UserProfile: 'C:\\Users\\new' }
  const merged = mergedPath(
    'C:\\Windows;C:\\Tools',
    [registryPath(USER_OUTPUT), 'C:\\WINDOWS\\;%MISSING%\\bin;;'],
    env
  )
  expect(merged).toBe(
    'C:\\Windows;C:\\Tools;C:\\Users\\new\\AppData\\Local\\Microsoft\\WinGet\\Links;%MISSING%\\bin'
  )
})

test('refreshPath only runs on Windows and keeps PATH when reg fails', async () => {
  const calls: string[][] = []
  const env: NodeJS.ProcessEnv = { PATH: 'C:\\Windows' }
  await refreshPath(
    (command, args) => {
      calls.push([command, ...args])
      return Promise.resolve({ code: 1, stdout: '', stderr: 'denied' })
    },
    env,
    'win32'
  )
  expect(calls).toHaveLength(2)
  expect(env.PATH).toBe('C:\\Windows')

  await refreshPath(() => Promise.reject(new Error('should not run')), env, 'linux')
  expect(env.PATH).toBe('C:\\Windows')
})

test('refreshPath picks up a tool installed after the app started', async () => {
  const env: NodeJS.ProcessEnv = { PATH: 'C:\\Windows', USERPROFILE: 'C:\\Users\\new' }
  await refreshPath(
    (_command, args) =>
      Promise.resolve(
        args[1]?.startsWith('HKCU')
          ? { code: 0, stdout: USER_OUTPUT, stderr: '' }
          : {
              code: 0,
              stdout: '    Path    REG_EXPAND_SZ    C:\\Windows;C:\\Program Files\\GitHub CLI\\',
              stderr: ''
            }
      ),
    env,
    'win32'
  )
  expect(env.PATH).toBe(
    'C:\\Windows;C:\\Program Files\\GitHub CLI\\;C:\\Users\\new\\AppData\\Local\\Microsoft\\WinGet\\Links;C:\\Tools\\'
  )
})
