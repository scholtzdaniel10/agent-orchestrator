import { expect, test } from 'vitest'
import { applyTheme, loadTheme, type ThemeRoot, type ThemeStorage } from './theme'

function fakeStorage(initial: Record<string, string> = {}): ThemeStorage & {
  data: Record<string, string>
} {
  const data = { ...initial }
  return {
    data,
    getItem(key: string): string | null {
      return Object.prototype.hasOwnProperty.call(data, key) ? data[key]! : null
    },
    setItem(key: string, value: string): void {
      data[key] = value
    }
  }
}

function fakeRoot(): ThemeRoot & { attrs: Record<string, string> } {
  const attrs: Record<string, string> = {}
  return {
    attrs,
    setAttribute(name: string, value: string): void {
      attrs[name] = value
    },
    removeAttribute(name: string): void {
      delete attrs[name]
    }
  }
}

test('loadTheme defaults to system when storage is empty', () => {
  expect(loadTheme(fakeStorage())).toBe('system')
})

test('loadTheme returns a saved look', () => {
  expect(loadTheme(fakeStorage({ 'orch.theme': 'phosphor' }))).toBe('phosphor')
})

test('loadTheme falls back to system for an unknown value', () => {
  expect(loadTheme(fakeStorage({ 'orch.theme': 'sunset' }))).toBe('system')
})

test('loadTheme falls back to system when storage throws', () => {
  const storage: ThemeStorage = {
    getItem(): string | null {
      throw new Error('blocked')
    },
    setItem(): void {
      return
    }
  }
  expect(loadTheme(storage)).toBe('system')
})

test('applyTheme sets data-theme, saves, and dispatches themechange', () => {
  const storage = fakeStorage()
  const root = fakeRoot()
  const target = new EventTarget()
  let n = 0
  target.addEventListener('themechange', () => {
    n += 1
  })
  applyTheme('ember', storage, root, target)
  expect(root.attrs['data-theme']).toBe('ember')
  expect(storage.data['orch.theme']).toBe('ember')
  expect(n).toBe(1)
})

test('applyTheme removes data-theme for system and still saves', () => {
  const storage = fakeStorage({ 'orch.theme': 'dusk' })
  const root = fakeRoot()
  root.setAttribute('data-theme', 'dusk')
  const target = new EventTarget()
  let n = 0
  target.addEventListener('themechange', () => {
    n += 1
  })
  applyTheme('system', storage, root, target)
  expect(root.attrs['data-theme']).toBeUndefined()
  expect(storage.data['orch.theme']).toBe('system')
  expect(n).toBe(1)
})
