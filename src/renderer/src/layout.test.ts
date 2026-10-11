import { expect, test } from 'vitest'
import {
  DEFAULT_LAYOUT,
  DEFAULT_SIZES,
  MIN_DRAWER,
  MIN_SIDEBAR,
  MIN_WORKERS,
  assignTabToPane,
  autoPaneLayout,
  clampSizes,
  drawerOverlays,
  dropMissingTerminals,
  emptyPaneForTerminal,
  emptySlots,
  fillPanes,
  layoutShortcut,
  loadLayout,
  maxSize,
  paneCount,
  parseLayout,
  placeNewTerminal,
  placeTerminalInPane,
  resolvedPaneLayout,
  seatUnplaced,
  type PaneSlots,
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

const OPEN = { sidebarShown: true, drawerOpen: true }
const WIDE = { width: 1600, height: 900 }

test('parseLayout returns defaults for junk', () => {
  expect(parseLayout(null)).toEqual(DEFAULT_LAYOUT)
  expect(parseLayout('old')).toEqual(DEFAULT_LAYOUT)
  expect(parseLayout(1)).toEqual(DEFAULT_LAYOUT)
})

test('parseLayout validates field by field and keeps good values', () => {
  const parsed = parseLayout({
    sizes: { sidebar: 260, drawer: 500, lead: 300, extra: 1 },
    sidebarShown: false,
    drawerOpen: false,
    drawerTab: 'jobs',
    paneLayout: '4-grid',
    other: true
  })
  expect(parsed.sizes.sidebar).toBe(260)
  expect(parsed.sizes.drawer).toBe(500)
  expect(parsed.sidebarShown).toBe(false)
  expect(parsed.drawerOpen).toBe(false)
  expect(parsed.drawerTab).toBe('jobs')
  expect(parsed.paneLayout).toBe('4-grid')
})

test('parseLayout falls back for an unknown pane layout and drawer tab', () => {
  expect(parseLayout({ paneLayout: 'split' }).paneLayout).toBe('auto')
  expect(parseLayout({ drawerTab: 'flow' }).drawerTab).toBe('files')
})

test('parseLayout keeps auto and a saved concrete pane layout', () => {
  expect(parseLayout({ paneLayout: 'auto' }).paneLayout).toBe('auto')
  expect(parseLayout({ paneLayout: '1' }).paneLayout).toBe('1')
})

test('parseLayout reads old saves without error and fills new defaults', () => {
  const parsed = parseLayout({
    sizes: { sidebar: 180, lead: 300, usage: 220, flow: 100 },
    shown: { projects: false, flow: true, lead: true, usage: true },
    paneLayout: '2-side',
    terminalsMode: true
  })
  expect(parsed.sizes.sidebar).toBe(180)
  expect(parsed.sizes.drawer).toBe(DEFAULT_SIZES.drawer)
  expect(parsed.sidebarShown).toBe(false)
  expect(parsed.drawerOpen).toBe(true)
  expect(parsed.drawerTab).toBe('files')
  expect(parsed.paneLayout).toBe('2-side')
})

test('parseLayout raises sizes below the minimums and caps the maximums', () => {
  const parsed = parseLayout({
    sizes: { sidebar: 10, drawer: 10 }
  })
  expect(parsed.sizes.sidebar).toBe(MIN_SIDEBAR)
  expect(parsed.sizes.drawer).toBe(MIN_DRAWER)
  const wide = parseLayout({
    sizes: { sidebar: 900, drawer: 2000 }
  })
  expect(wide.sizes.sidebar).toBe(480)
  expect(wide.sizes.drawer).toBe(720)
})

test('loadLayout reads JSON and falls back when storage throws', () => {
  const storage = fakeStorage({
    'orch.layout': JSON.stringify({
      sizes: { sidebar: 180, drawer: 400 },
      sidebarShown: true,
      drawerOpen: false,
      drawerTab: 'usage',
      paneLayout: '2-side'
    })
  })
  expect(loadLayout(storage).paneLayout).toBe('2-side')
  expect(loadLayout(storage).drawerOpen).toBe(false)
  expect(loadLayout(storage).drawerTab).toBe('usage')
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
      sizes: { sidebar: 180, drawer: 400 },
      sidebarShown: false,
      drawerOpen: true,
      drawerTab: 'activity',
      paneLayout: '6-grid'
    },
    storage
  )
  expect(JSON.parse(storage.data['orch.layout']!)).toEqual({
    sizes: { sidebar: 180, drawer: 400 },
    sidebarShown: false,
    drawerOpen: true,
    drawerTab: 'activity',
    paneLayout: '6-grid'
  })
})

test('clampSizes keeps defaults on a wide window', () => {
  const next = clampSizes(DEFAULT_SIZES, WIDE, OPEN)
  expect(next).toEqual(DEFAULT_SIZES)
})

test('clampSizes at 900px overlays instead of squeezing the workspace below 360', () => {
  const next = clampSizes(DEFAULT_SIZES, { width: 900, height: 800 }, OPEN)
  expect(drawerOverlays(next, { width: 900 }, OPEN)).toBe(true)
  expect(next.sidebar).toBeGreaterThanOrEqual(MIN_SIDEBAR)
  expect(900 - next.sidebar).toBeGreaterThanOrEqual(MIN_WORKERS)
})

test('drawerOverlays is false when sidebar plus drawer plus 360 fit', () => {
  expect(drawerOverlays(DEFAULT_SIZES, { width: 1600 }, OPEN)).toBe(false)
  expect(drawerOverlays(DEFAULT_SIZES, { width: 900 }, OPEN)).toBe(true)
  expect(
    drawerOverlays(DEFAULT_SIZES, { width: 900 }, { sidebarShown: true, drawerOpen: false })
  ).toBe(false)
})

test('clampSizes ignores a hidden sidebar when fitting the row', () => {
  const next = clampSizes(
    DEFAULT_SIZES,
    { width: 700, height: 800 },
    { sidebarShown: false, drawerOpen: false }
  )
  expect(next.sidebar).toBe(DEFAULT_SIZES.sidebar)
  expect(next.drawer).toBe(DEFAULT_SIZES.drawer)
})

test('maxSize is the largest value clampSizes would allow', () => {
  const sizes = DEFAULT_SIZES
  const viewport = { width: 1600, height: 800 }
  const max = maxSize('sidebar', sizes, viewport, OPEN)
  const fitted = clampSizes({ ...sizes, sidebar: 10_000 }, viewport, OPEN)
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

test('autoPaneLayout picks the smallest fitting grid and caps at six', () => {
  expect(autoPaneLayout(0)).toBe('1')
  expect(autoPaneLayout(1)).toBe('1')
  expect(autoPaneLayout(2)).toBe('2-side')
  expect(autoPaneLayout(3)).toBe('3-side')
  expect(autoPaneLayout(4)).toBe('4-grid')
  expect(autoPaneLayout(5)).toBe('6-grid')
  expect(autoPaneLayout(6)).toBe('6-grid')
  expect(autoPaneLayout(9)).toBe('6-grid')
  expect(resolvedPaneLayout('2-stack', 5)).toBe('2-stack')
  expect(resolvedPaneLayout('auto', 3)).toBe('3-side')
})

test('layoutShortcut maps Ctrl+B, Ctrl+\\, and Ctrl+1 through Ctrl+7', () => {
  const base = { ctrlKey: true, shiftKey: false, altKey: false, metaKey: false }
  expect(layoutShortcut({ ...base, key: 'b' })).toBe('drawer')
  expect(layoutShortcut({ ...base, key: 'B' })).toBe('drawer')
  expect(layoutShortcut({ ...base, key: '\\' })).toBe('sidebar')
  expect(layoutShortcut({ ...base, key: '1' })).toBe('files')
  expect(layoutShortcut({ ...base, key: '2' })).toBe('lead')
  expect(layoutShortcut({ ...base, key: '7' })).toBe('activity')
  expect(layoutShortcut({ ...base, key: 'b', shiftKey: true })).toBe(null)
  expect(layoutShortcut({ ...base, key: 't' })).toBe(null)
  expect(layoutShortcut({ ...base, key: '0' })).toBe(null)
})

test('fillPanes auto uses the smallest fitting layout', () => {
  expect(fillPanes('auto', ['a', 'b'], 'a')).toEqual(['a', 'b'])
  expect(fillPanes('auto', ['a', 'b', 'c', 'd', 'e'], 'a')).toEqual(['a', 'b', 'c', 'd', 'e', null])
  expect(emptySlots('auto')).toEqual([null])
  expect(paneCount('auto', 5)).toBe(6)
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

test('seatUnplaced gives unplaced terminals the empty panes, in order, and moves nothing', () => {
  expect(seatUnplaced(['a', null, null, null], ['a', 'b', 'c'])).toEqual(['a', 'b', 'c', null])
  expect(seatUnplaced([null, 'b'], ['a', 'b', 'c'])).toEqual(['a', 'b'])
  const full: PaneSlots = ['a']
  expect(seatUnplaced(full, ['a', 'b'])).toBe(full)
  const settled: PaneSlots = ['a', null]
  expect(seatUnplaced(settled, ['a'])).toBe(settled)
})
