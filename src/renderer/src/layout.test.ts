import { expect, test } from 'vitest'
import {
  DEFAULT_LAYOUT,
  DEFAULT_SHOWN,
  DEFAULT_SIZES,
  MIN_FLOW,
  MIN_LEAD,
  MIN_SIDEBAR,
  MIN_USAGE,
  MIN_WORKERS,
  assignTabToPane,
  clampSizes,
  dropMissingTerminals,
  emptyPaneForTerminal,
  emptySlots,
  fillPanes,
  loadLayout,
  maxSize,
  paneCount,
  parseLayout,
  placeNewTerminal,
  placeTerminalInPane,
  saveLayout,
  type LayoutStorage
} from './layout'

function fakeStorage(initial: Record<string, string> = {}): LayoutStorage & {
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

const ALL = DEFAULT_SHOWN

test('parseLayout returns defaults for junk', () => {
  expect(parseLayout(null)).toEqual(DEFAULT_LAYOUT)
  expect(parseLayout('old')).toEqual(DEFAULT_LAYOUT)
  expect(parseLayout(1)).toEqual(DEFAULT_LAYOUT)
})

test('parseLayout validates field by field and keeps good values', () => {
  const parsed = parseLayout({
    sizes: { sidebar: 240, lead: 'wide', usage: 210, flow: 120, extra: 1 },
    shown: { projects: false, flow: 1, lead: true, usage: false },
    paneLayout: '4-grid',
    other: true
  })
  expect(parsed.sizes.sidebar).toBe(240)
  expect(parsed.sizes.lead).toBe(DEFAULT_SIZES.lead)
  expect(parsed.sizes.usage).toBe(210)
  expect(parsed.sizes.flow).toBe(120)
  expect(parsed.shown.projects).toBe(false)
  expect(parsed.shown.flow).toBe(true)
  expect(parsed.shown.lead).toBe(true)
  expect(parsed.shown.usage).toBe(false)
  expect(parsed.paneLayout).toBe('4-grid')
})

test('parseLayout falls back for an unknown pane layout', () => {
  expect(parseLayout({ paneLayout: 'split' }).paneLayout).toBe('1')
})

test('parseLayout raises sizes below the minimums', () => {
  const parsed = parseLayout({
    sizes: { sidebar: 10, lead: 10, usage: 10, flow: 10 }
  })
  expect(parsed.sizes.sidebar).toBe(MIN_SIDEBAR)
  expect(parsed.sizes.lead).toBe(MIN_LEAD)
  expect(parsed.sizes.usage).toBe(MIN_USAGE)
  expect(parsed.sizes.flow).toBe(MIN_FLOW)
})

test('loadLayout reads JSON and falls back when storage throws', () => {
  const storage = fakeStorage({
    'orch.layout': JSON.stringify({
      sizes: { sidebar: 180, lead: 300, usage: 220, flow: 100 },
      shown: { projects: true, flow: false, lead: true, usage: true },
      paneLayout: '2-side'
    })
  })
  expect(loadLayout(storage).paneLayout).toBe('2-side')
  expect(loadLayout(storage).shown.flow).toBe(false)
  const blocked: LayoutStorage = {
    getItem(): string | null {
      throw new Error('blocked')
    },
    setItem(): void {
      return
    }
  }
  expect(loadLayout(blocked)).toEqual(DEFAULT_LAYOUT)
  expect(loadLayout(fakeStorage({ 'orch.layout': '{bad' }))).toEqual(DEFAULT_LAYOUT)
})

test('saveLayout writes one JSON object', () => {
  const storage = fakeStorage()
  saveLayout(
    {
      sizes: { sidebar: 180, lead: 300, usage: 220, flow: 110 },
      shown: { projects: false, flow: true, lead: true, usage: true },
      paneLayout: '6-grid'
    },
    storage
  )
  expect(JSON.parse(storage.data['orch.layout']!)).toEqual({
    sizes: { sidebar: 180, lead: 300, usage: 220, flow: 110 },
    shown: { projects: false, flow: true, lead: true, usage: true },
    paneLayout: '6-grid'
  })
})

test('clampSizes keeps defaults on a wide window', () => {
  const next = clampSizes(DEFAULT_SIZES, { width: 1600, height: 900 }, ALL)
  expect(next).toEqual(DEFAULT_SIZES)
})

test('clampSizes at 900px keeps workers at least 360 and does not overflow', () => {
  const next = clampSizes(DEFAULT_SIZES, { width: 900, height: 800 }, ALL)
  expect(next.sidebar + next.lead).toBeLessThanOrEqual(900 - MIN_WORKERS)
  expect(next.sidebar).toBeGreaterThanOrEqual(MIN_SIDEBAR)
  expect(next.lead).toBeGreaterThanOrEqual(MIN_LEAD)
  expect(900 - next.sidebar - next.lead).toBeGreaterThanOrEqual(MIN_WORKERS)
})

test('clampSizes on a wide short window shrinks the row so workers stay 360', () => {
  const next = clampSizes(DEFAULT_SIZES, { width: 1100, height: 800 }, ALL)
  expect(next.sidebar + next.lead + next.usage).toBeLessThanOrEqual(1100 - MIN_WORKERS)
  expect(1100 - next.sidebar - next.lead - next.usage).toBeGreaterThanOrEqual(MIN_WORKERS)
})

test('clampSizes ignores hidden panels when fitting the row', () => {
  const next = clampSizes(
    DEFAULT_SIZES,
    { width: 700, height: 800 },
    {
      projects: false,
      flow: true,
      lead: false,
      usage: false
    }
  )
  expect(next.sidebar).toBe(DEFAULT_SIZES.sidebar)
  expect(next.lead).toBe(DEFAULT_SIZES.lead)
  expect(next.usage).toBe(DEFAULT_SIZES.usage)
})

test('clampSizes caps flow to the window', () => {
  const next = clampSizes({ ...DEFAULT_SIZES, flow: 2000 }, { width: 1200, height: 400 }, ALL)
  expect(next.flow).toBeLessThan(2000)
  expect(next.flow).toBeGreaterThanOrEqual(MIN_FLOW)
})

test('maxSize is the largest value clampSizes would allow', () => {
  const sizes = DEFAULT_SIZES
  const viewport = { width: 1200, height: 800 }
  const max = maxSize('sidebar', sizes, viewport, ALL)
  const fitted = clampSizes({ ...sizes, sidebar: 10_000 }, viewport, ALL)
  expect(max).toBe(fitted.sidebar)
  expect(max).toBeGreaterThanOrEqual(MIN_SIDEBAR)
})

test('fillPanes starts at the active terminal and never duplicates', () => {
  const slots = fillPanes('4-grid', ['a', 'b', 'c', 'd', 'e'], 'c')
  expect(slots).toEqual(['c', 'd', 'e', 'a'])
  expect(new Set(slots.filter((id) => id !== null)).size).toBe(4)
})

test('fillPanes leaves extra panes empty', () => {
  expect(fillPanes('6-grid', ['a', 'b'], 'b')).toEqual(['b', 'a', null, null, null, null])
})

test('fillPanes with no terminals is all empty', () => {
  expect(fillPanes('2-side', [], null)).toEqual([null, null])
  expect(emptySlots('1')).toEqual([null])
  expect(paneCount('6-grid')).toBe(6)
})

test('assignTabToPane swaps when the terminal is already shown', () => {
  expect(assignTabToPane(['a', 'b', null], 0, 'b')).toEqual(['b', 'a', null])
})

test('assignTabToPane puts an unseen terminal in the focused pane', () => {
  expect(assignTabToPane(['a', null], 1, 'b')).toEqual(['a', 'b'])
  expect(assignTabToPane(['a', 'c'], 1, 'b')).toEqual(['a', 'b'])
})

test('closing a terminal empties only its pane', () => {
  expect(emptyPaneForTerminal(['a', 'b', 'a'], 'a')).toEqual([null, 'b', null])
})

test('a new terminal goes to the first empty pane else the focused one', () => {
  expect(placeNewTerminal(['a', null, null], 2, 'b')).toEqual(['a', 'b', null])
  expect(placeNewTerminal(['a', 'c'], 0, 'b')).toEqual(['b', 'c'])
})

test('empty-pane open places the terminal in that pane', () => {
  expect(placeTerminalInPane([null, null], 1, 'a')).toEqual([null, 'a'])
  expect(placeTerminalInPane(['a', null], 1, 'a')).toEqual([null, 'a'])
})

test('dropMissingTerminals empties closed ids', () => {
  expect(dropMissingTerminals(['a', 'b'], new Set(['b']))).toEqual([null, 'b'])
})
