import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'vitest'
import { isValidModel, Settings } from './settings'
import type { JobAccess } from './types'

function tempFile(): { root: string; path: string } {
  const root = mkdtempSync(join(tmpdir(), 'ao-settings-'))
  return { root, path: join(root, 'settings.json') }
}

test('missing file reads as empty and setModel round-trips through a new instance', () => {
  const { root, path } = tempFile()
  try {
    const settings = new Settings(path)
    expect(existsSync(path)).toBe(false)
    expect(settings.model('claude')).toBeUndefined()
    expect(settings.model('cursor')).toBeUndefined()

    settings.setModel('claude', '  opus  ')
    settings.setModel('cursor', 'auto')
    const again = new Settings(path)
    expect(again.model('claude')).toBe('opus')
    expect(again.model('cursor')).toBe('auto')

    again.setModel('claude', null)
    const cleared = new Settings(path)
    expect(cleared.model('claude')).toBeUndefined()
    expect(cleared.model('cursor')).toBe('auto')
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('corrupt JSON is empty and a later setModel still persists', () => {
  const { root, path } = tempFile()
  try {
    writeFileSync(path, '{')
    const settings = new Settings(path)
    expect(settings.model('claude')).toBeUndefined()
    settings.setModel('cursor', 'auto')
    expect(new Settings(path).model('cursor')).toBe('auto')
    expect(JSON.parse(readFileSync(path, 'utf8'))).toEqual({ models: { cursor: 'auto' } })
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('an invalid stored model is ignored and unknown keys survive a write', () => {
  const { root, path } = tempFile()
  try {
    writeFileSync(
      path,
      JSON.stringify({
        theme: 'dark',
        models: { claude: '--force', cursor: 'auto', extra: 1 }
      })
    )
    const settings = new Settings(path)
    expect(settings.model('claude')).toBeUndefined()
    expect(settings.model('cursor')).toBe('auto')
    settings.setModel('claude', 'opus')
    expect(JSON.parse(readFileSync(path, 'utf8'))).toEqual({
      theme: 'dark',
      models: { claude: 'opus', cursor: 'auto', extra: 1 }
    })
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('setModel rejects names that must not become command-line arguments', () => {
  const { root, path } = tempFile()
  try {
    const settings = new Settings(path)
    settings.setModel('claude', 'opus')
    const bad = ['', '   ', '--force', '-m', 'a b', 'a'.repeat(201), 'opus\n4']
    for (const name of bad) {
      expect(() => settings.setModel('claude', name)).toThrow('invalid model name')
    }
    expect(new Settings(path).model('claude')).toBe('opus')
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('isValidModel accepts CLI model ids, including bracketed options', () => {
  expect(isValidModel('opus')).toBe(true)
  expect(isValidModel('grok-4.7-xhigh')).toBe(true)
  expect(isValidModel('claude-opus-4-8[context=1m,effort=high,fast=false]')).toBe(true)
  expect(isValidModel('gpt-5.2')).toBe(true)
  expect(isValidModel('a'.repeat(200))).toBe(true)
  expect(isValidModel('a'.repeat(201))).toBe(false)
  expect(isValidModel('--force')).toBe(false)
  expect(isValidModel(null)).toBe(false)
})

test('leadPlan round-trips, null clears, and an invalid stored value is ignored', () => {
  const { root, path } = tempFile()
  try {
    writeFileSync(path, JSON.stringify({ lead: 'nope', models: { claude: 'opus' } }))
    const settings = new Settings(path)
    expect(settings.leadPlan()).toBeUndefined()
    expect(settings.model('claude')).toBe('opus')

    settings.setLeadPlan('claude')
    const saved = new Settings(path)
    expect(saved.leadPlan()).toBe('claude')
    expect(saved.model('claude')).toBe('opus')

    saved.setLeadPlan('cursor')
    expect(new Settings(path).leadPlan()).toBe('cursor')
    saved.setLeadPlan(null)
    const cleared = new Settings(path)
    expect(cleared.leadPlan()).toBeUndefined()
    expect(cleared.model('claude')).toBe('opus')
    expect(JSON.parse(readFileSync(path, 'utf8'))).toEqual({ models: { claude: 'opus' } })
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('leadAccess defaults to read, round-trips, and rejects a bad value', () => {
  const { root, path } = tempFile()
  try {
    writeFileSync(path, JSON.stringify({ leadAccess: 'nope', models: { claude: 'opus' } }))
    const settings = new Settings(path)
    expect(settings.leadAccess()).toBe('read')
    expect(settings.model('claude')).toBe('opus')

    settings.setLeadAccess('edit')
    expect(new Settings(path).leadAccess()).toBe('edit')
    settings.setLeadAccess('full')
    expect(new Settings(path).leadAccess()).toBe('full')
    settings.setLeadAccess('read')
    const again = new Settings(path)
    expect(again.leadAccess()).toBe('read')
    expect(again.model('claude')).toBe('opus')
    expect(JSON.parse(readFileSync(path, 'utf8'))).toEqual({
      models: { claude: 'opus' },
      leadAccess: 'read'
    })
    expect(() => settings.setLeadAccess('write' as JobAccess)).toThrow(
      'access must be read, edit, or full'
    )
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('project round-trips and ignores a stored value that is not a non-empty string', () => {
  const { root, path } = tempFile()
  try {
    writeFileSync(path, JSON.stringify({ project: '', models: { claude: 'opus' } }))
    expect(new Settings(path).project()).toBeUndefined()
    writeFileSync(path, JSON.stringify({ project: 4, models: { claude: 'opus' } }))
    const settings = new Settings(path)
    expect(settings.project()).toBeUndefined()
    expect(settings.model('claude')).toBe('opus')
    settings.setProject('repo')
    const again = new Settings(path)
    expect(again.project()).toBe('repo')
    expect(again.model('claude')).toBe('opus')
    expect(JSON.parse(readFileSync(path, 'utf8'))).toEqual({
      project: 'repo',
      models: { claude: 'opus' },
      projects: ['repo']
    })
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('projects round-trips through setProject and a new instance', () => {
  const { root, path } = tempFile()
  try {
    const settings = new Settings(path)
    expect(settings.projects()).toEqual([])
    settings.setProject('/a')
    settings.setProject('/b')
    expect(settings.projects()).toEqual(['/a', '/b'])
    expect(new Settings(path).projects()).toEqual(['/a', '/b'])
    expect(new Settings(path).project()).toBe('/b')
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('projects de-duplicates and keeps order', () => {
  const { root, path } = tempFile()
  try {
    writeFileSync(path, JSON.stringify({ projects: ['/a', '/b', '/a', '/c', '/b'] }))
    expect(new Settings(path).projects()).toEqual(['/a', '/b', '/c'])
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('projects ignores junk values', () => {
  const { root, path } = tempFile()
  try {
    writeFileSync(path, JSON.stringify({ projects: ['/ok', '', 4, null, '  ', '/also', { x: 1 }] }))
    expect(new Settings(path).projects()).toEqual(['/ok', '/also'])
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('projects includes the current project first when it is missing from the list', () => {
  const { root, path } = tempFile()
  try {
    writeFileSync(path, JSON.stringify({ project: '/current', projects: ['/a', '/b'] }))
    expect(new Settings(path).projects()).toEqual(['/current', '/a', '/b'])
    writeFileSync(path, JSON.stringify({ project: '/a', projects: ['/a', '/b'] }))
    expect(new Settings(path).projects()).toEqual(['/a', '/b'])
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('removeProject drops a folder from the list', () => {
  const { root, path } = tempFile()
  try {
    const settings = new Settings(path)
    settings.setProject('/a')
    settings.setProject('/b')
    settings.removeProject('/a')
    expect(settings.projects()).toEqual(['/b'])
    expect(new Settings(path).projects()).toEqual(['/b'])
    expect(settings.project()).toBe('/b')
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('removeProject throws when asked to remove the active project', () => {
  const { root, path } = tempFile()
  try {
    const settings = new Settings(path)
    settings.setProject('/a')
    settings.setProject('/b')
    expect(() => settings.removeProject('/b')).toThrow('cannot remove the active project')
    expect(settings.projects()).toEqual(['/a', '/b'])
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('setModel creates a missing parent folder', () => {
  const root = mkdtempSync(join(tmpdir(), 'ao-settings-'))
  try {
    const path = join(root, 'nested', 'dir', 'settings.json')
    expect(existsSync(join(root, 'nested'))).toBe(false)
    const settings = new Settings(path)
    settings.setModel('cursor', 'auto')
    expect(existsSync(path)).toBe(true)
    expect(existsSync(`${path}.tmp`)).toBe(false)
    expect(new Settings(path).model('cursor')).toBe('auto')
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})
