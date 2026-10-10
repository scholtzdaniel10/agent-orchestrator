export const THEMES = [
  { id: 'system', name: 'System' },
  { id: 'ember', name: 'Ember' },
  { id: 'paper', name: 'Paper' },
  { id: 'ultramarine', name: 'Ultramarine' },
  { id: 'phosphor', name: 'Phosphor' },
  { id: 'graphite', name: 'Graphite' },
  { id: 'dusk', name: 'Dusk' }
] as const

export type ThemeId = (typeof THEMES)[number]['id']

export interface ThemeStorage {
  getItem(key: string): string | null
  setItem(key: string, value: string): void
}

export interface ThemeRoot {
  setAttribute(name: string, value: string): void
  removeAttribute(name: string): void
}

const STORAGE_KEY = 'orch.theme'
const THEME_IDS: ReadonlySet<string> = new Set(THEMES.map((theme) => theme.id))

export function isThemeId(value: string): value is ThemeId {
  return THEME_IDS.has(value)
}

function liveStorage(): ThemeStorage {
  try {
    if (typeof localStorage === 'undefined') return noopStorage()
    return localStorage
  } catch {
    return noopStorage()
  }
}

function noopStorage(): ThemeStorage {
  return {
    getItem(): string | null {
      return null
    },
    setItem(): void {
      return
    }
  }
}

function liveRoot(): ThemeRoot {
  return document.documentElement
}

function liveTarget(): EventTarget {
  return window
}

export function loadTheme(storage: ThemeStorage = liveStorage()): ThemeId {
  try {
    const raw = storage.getItem(STORAGE_KEY)
    if (raw !== null && isThemeId(raw)) return raw
  } catch {
    return 'system'
  }
  return 'system'
}

export function applyTheme(
  id: ThemeId,
  storage: ThemeStorage = liveStorage(),
  root: ThemeRoot = liveRoot(),
  target: EventTarget = liveTarget()
): void {
  if (id === 'system') root.removeAttribute('data-theme')
  else root.setAttribute('data-theme', id)
  try {
    storage.setItem(STORAGE_KEY, id)
  } catch {
    // Private mode can block storage; the look still applies for this session.
  }
  target.dispatchEvent(new Event('themechange'))
}
