export const LAYOUT_KEY = 'orch.layout'
export const LAYOUT_FIT_EVENT = 'layoutfit'
export const LAYOUT_SHORTCUT_EVENT = 'layoutshortcut'

export const MIN_SIDEBAR = 160
export const MAX_SIDEBAR = 480
export const MIN_DRAWER = 320
export const MAX_DRAWER = 720
export const MIN_WORKERS = 360
export const HEADER_HEIGHT = 40
export const STATUS_HEIGHT = 28

export const PANE_LAYOUT_IDS = [
  'auto',
  '1',
  '2-side',
  '2-stack',
  '3-side',
  '4-grid',
  '6-grid'
] as const

export type PaneLayoutId = (typeof PANE_LAYOUT_IDS)[number]

export type ConcretePaneLayoutId = Exclude<PaneLayoutId, 'auto'>

export const AUTO_PANE_LAYOUTS = ['1', '2-side', '3-side', '4-grid', '6-grid'] as const

export const PANE_LAYOUT_LABELS: Record<PaneLayoutId, string> = {
  auto: 'Auto',
  '1': '1',
  '2-side': '2 side by side',
  '2-stack': '2 stacked',
  '3-side': '3 side by side',
  '4-grid': '4 grid',
  '6-grid': '6 grid'
}

export const PANE_COUNTS: Record<ConcretePaneLayoutId, number> = {
  '1': 1,
  '2-side': 2,
  '2-stack': 2,
  '3-side': 3,
  '4-grid': 4,
  '6-grid': 6
}

export const DRAWER_TABS = [
  'files',
  'lead',
  'jobs',
  'changes',
  'routines',
  'usage',
  'activity',
  'history'
] as const

export type DrawerTab = (typeof DRAWER_TABS)[number]

export const DRAWER_TAB_LABELS: Record<DrawerTab, string> = {
  files: 'Files',
  lead: 'Lead',
  jobs: 'Jobs',
  changes: 'Changes',
  routines: 'Routines',
  usage: 'Usage',
  activity: 'Activity',
  history: 'History'
}

export type LayoutShortcut = 'drawer' | 'sidebar' | DrawerTab

export interface LayoutSizes {
  sidebar: number
  drawer: number
}

export interface LayoutFlags {
  sidebarShown: boolean
  drawerOpen: boolean
}

export interface LayoutState {
  sizes: LayoutSizes
  sidebarShown: boolean
  drawerOpen: boolean
  drawerTab: DrawerTab
  paneLayout: PaneLayoutId
  /** The panel's tabs in the order the person dragged them into. */
  tabOrder: DrawerTab[]
}

export interface LayoutStorage {
  getItem(key: string): string | null
  setItem(key: string, value: string): void
}

export type PaneSlots = (string | null)[]

export type SizeKey = keyof LayoutSizes

export const DEFAULT_SIZES: LayoutSizes = {
  sidebar: 240,
  drawer: 420
}

export const DEFAULT_LAYOUT: LayoutState = {
  sizes: { ...DEFAULT_SIZES },
  sidebarShown: true,
  drawerOpen: true,
  drawerTab: 'files',
  paneLayout: 'auto',
  tabOrder: [...DRAWER_TABS]
}

const PANE_LAYOUT_SET: ReadonlySet<string> = new Set(PANE_LAYOUT_IDS)
const DRAWER_TAB_SET: ReadonlySet<string> = new Set(DRAWER_TABS)

export function isPaneLayout(value: string): value is PaneLayoutId {
  return PANE_LAYOUT_SET.has(value)
}

export function isDrawerTab(value: string): value is DrawerTab {
  return DRAWER_TAB_SET.has(value)
}

export function paneCount(layout: PaneLayoutId, terminalCount = 0): number {
  return PANE_COUNTS[resolvedPaneLayout(layout, terminalCount)]
}

/** Smallest of 1, 2-side, 3-side, 4-grid, 6-grid that fits `terminalCount` (capped at 6). */
export function autoPaneLayout(terminalCount: number): (typeof AUTO_PANE_LAYOUTS)[number] {
  const n = Math.max(0, Math.floor(terminalCount))
  if (n <= 1) return '1'
  if (n === 2) return '2-side'
  if (n === 3) return '3-side'
  if (n === 4) return '4-grid'
  return '6-grid'
}

export function resolvedPaneLayout(
  layout: PaneLayoutId,
  terminalCount: number
): ConcretePaneLayoutId {
  if (layout === 'auto') return autoPaneLayout(terminalCount)
  return layout
}

function asNumber(value: unknown, fallback: number, min: number, max: number): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) return fallback
  return Math.min(max, Math.max(min, Math.round(value)))
}

function asBool(value: unknown, fallback: boolean): boolean {
  return typeof value === 'boolean' ? value : fallback
}

function asRecord(value: unknown): Record<string, unknown> | null {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) return null
  return value as Record<string, unknown>
}

export function parseLayout(raw: unknown): LayoutState {
  const root = asRecord(raw)
  if (root === null) {
    return {
      sizes: { ...DEFAULT_SIZES },
      sidebarShown: DEFAULT_LAYOUT.sidebarShown,
      drawerOpen: DEFAULT_LAYOUT.drawerOpen,
      drawerTab: DEFAULT_LAYOUT.drawerTab,
      paneLayout: DEFAULT_LAYOUT.paneLayout,
      tabOrder: [...DRAWER_TABS]
    }
  }

  const sizesRaw = asRecord(root.sizes)
  const sizes: LayoutSizes = {
    sidebar: asNumber(sizesRaw?.sidebar, DEFAULT_SIZES.sidebar, MIN_SIDEBAR, MAX_SIDEBAR),
    drawer: asNumber(sizesRaw?.drawer, DEFAULT_SIZES.drawer, MIN_DRAWER, MAX_DRAWER)
  }

  const shownRaw = asRecord(root.shown)
  const sidebarShown = asBool(
    root.sidebarShown,
    asBool(shownRaw?.projects, DEFAULT_LAYOUT.sidebarShown)
  )
  const drawerOpen = asBool(root.drawerOpen, DEFAULT_LAYOUT.drawerOpen)
  const drawerTab =
    typeof root.drawerTab === 'string' && isDrawerTab(root.drawerTab)
      ? root.drawerTab
      : DEFAULT_LAYOUT.drawerTab

  const paneLayout =
    typeof root.paneLayout === 'string' && isPaneLayout(root.paneLayout)
      ? root.paneLayout
      : DEFAULT_LAYOUT.paneLayout

  return {
    sizes,
    sidebarShown,
    drawerOpen,
    drawerTab,
    paneLayout,
    tabOrder: tabOrderFrom(root.tabOrder)
  }
}

/** A saved order, keeping known tabs once each; tabs it lacks (new in an update) go at the end. */
export function tabOrderFrom(raw: unknown): DrawerTab[] {
  const out: DrawerTab[] = []
  if (Array.isArray(raw)) {
    for (const entry of raw) {
      if (typeof entry === 'string' && isDrawerTab(entry) && !out.includes(entry)) out.push(entry)
    }
  }
  for (const tab of DRAWER_TABS) if (!out.includes(tab)) out.push(tab)
  return out
}

/** The order with `tab` moved to sit at `index` (counted in the order without it). */
export function moveTab(order: readonly DrawerTab[], tab: DrawerTab, index: number): DrawerTab[] {
  const rest = order.filter((entry) => entry !== tab)
  const at = Math.max(0, Math.min(rest.length, Math.round(index)))
  return [...rest.slice(0, at), tab, ...rest.slice(at)]
}

/** True when the order is the built-in one. */
export function isDefaultTabOrder(order: readonly DrawerTab[]): boolean {
  return (
    order.length === DRAWER_TABS.length && order.every((tab, index) => tab === DRAWER_TABS[index])
  )
}

function liveStorage(): LayoutStorage {
  try {
    if (typeof localStorage === 'undefined') return noopStorage()
    return localStorage
  } catch {
    return noopStorage()
  }
}

function noopStorage(): LayoutStorage {
  return {
    getItem(): string | null {
      return null
    },
    setItem(): void {
      return
    }
  }
}

export function loadLayout(storage: LayoutStorage = liveStorage()): LayoutState {
  try {
    const raw = storage.getItem(LAYOUT_KEY)
    if (raw === null) {
      return {
        sizes: { ...DEFAULT_SIZES },
        sidebarShown: DEFAULT_LAYOUT.sidebarShown,
        drawerOpen: DEFAULT_LAYOUT.drawerOpen,
        drawerTab: DEFAULT_LAYOUT.drawerTab,
        paneLayout: DEFAULT_LAYOUT.paneLayout,
        tabOrder: [...DRAWER_TABS]
      }
    }
    return parseLayout(JSON.parse(raw) as unknown)
  } catch {
    return {
      sizes: { ...DEFAULT_SIZES },
      sidebarShown: DEFAULT_LAYOUT.sidebarShown,
      drawerOpen: DEFAULT_LAYOUT.drawerOpen,
      drawerTab: DEFAULT_LAYOUT.drawerTab,
      paneLayout: DEFAULT_LAYOUT.paneLayout,
      tabOrder: [...DRAWER_TABS]
    }
  }
}

export function saveLayout(state: LayoutState, storage: LayoutStorage = liveStorage()): void {
  try {
    storage.setItem(
      LAYOUT_KEY,
      JSON.stringify({
        sizes: state.sizes,
        sidebarShown: state.sidebarShown,
        drawerOpen: state.drawerOpen,
        drawerTab: state.drawerTab,
        paneLayout: state.paneLayout,
        tabOrder: state.tabOrder
      })
    )
  } catch {
    // Private mode or a full quota must not block the layout.
  }
}

export function drawerOverlays(
  sizes: LayoutSizes,
  viewport: { width: number },
  flags: LayoutFlags
): boolean {
  if (!flags.drawerOpen) return false
  const width = Math.max(1, Math.round(viewport.width))
  const sidebar = flags.sidebarShown ? sizes.sidebar : 0
  return sidebar + MIN_WORKERS + sizes.drawer > width
}

export function clampSizes(
  sizes: LayoutSizes,
  viewport: { width: number; height: number },
  flags: LayoutFlags
): LayoutSizes {
  const width = Math.max(1, Math.round(viewport.width))
  let sidebar = asNumber(sizes.sidebar, DEFAULT_SIZES.sidebar, MIN_SIDEBAR, MAX_SIDEBAR)
  const drawer = asNumber(sizes.drawer, DEFAULT_SIZES.drawer, MIN_DRAWER, MAX_DRAWER)

  if (flags.sidebarShown) {
    const overlay = drawerOverlays({ sidebar, drawer }, { width }, flags)
    const reserved = overlay || !flags.drawerOpen ? 0 : drawer
    const leftover = width - sidebar - reserved
    if (leftover < MIN_WORKERS) {
      sidebar = Math.max(MIN_SIDEBAR, sidebar - (MIN_WORKERS - leftover))
    }
  }

  return { sidebar, drawer }
}

export function maxSize(
  key: SizeKey,
  sizes: LayoutSizes,
  viewport: { width: number; height: number },
  flags: LayoutFlags
): number {
  return clampSizes({ ...sizes, [key]: 10_000 }, viewport, flags)[key]
}

export function emptySlots(layout: PaneLayoutId, terminalCount = 0): PaneSlots {
  return Array.from({ length: paneCount(layout, terminalCount) }, () => null)
}

function clampIndex(index: number, length: number): number {
  if (length <= 0) return 0
  return Math.min(length - 1, Math.max(0, index))
}

export function fillPanes(
  layout: PaneLayoutId,
  terminalIds: readonly string[],
  activeId: string | null
): PaneSlots {
  const slots = emptySlots(layout, terminalIds.length)
  if (terminalIds.length === 0) return slots
  let start = activeId === null ? 0 : terminalIds.indexOf(activeId)
  if (start < 0) start = 0
  const n = Math.min(slots.length, terminalIds.length)
  for (let i = 0; i < n; i += 1) {
    slots[i] = terminalIds[(start + i) % terminalIds.length]
  }
  return slots
}

export function assignTabToPane(
  slots: readonly (string | null)[],
  focusedIndex: number,
  terminalId: string
): PaneSlots {
  if (slots.length === 0) return []
  const next = slots.slice()
  const focused = clampIndex(focusedIndex, next.length)
  const existing = next.indexOf(terminalId)
  if (existing === focused) return next
  if (existing >= 0) {
    const swapped = next[focused]
    next[focused] = terminalId
    next[existing] = swapped
    return next
  }
  next[focused] = terminalId
  return next
}

export function emptyPaneForTerminal(
  slots: readonly (string | null)[],
  terminalId: string
): PaneSlots {
  return slots.map((id) => (id === terminalId ? null : id))
}

export function placeNewTerminal(
  slots: readonly (string | null)[],
  focusedIndex: number,
  terminalId: string
): PaneSlots {
  if (slots.length === 0) return []
  const next = slots.slice()
  const empty = next.indexOf(null)
  if (empty >= 0) {
    next[empty] = terminalId
    return next
  }
  next[clampIndex(focusedIndex, next.length)] = terminalId
  return next
}

export function placeTerminalInPane(
  slots: readonly (string | null)[],
  paneIndex: number,
  terminalId: string
): PaneSlots {
  if (slots.length === 0) return []
  const next = slots.slice()
  const existing = next.indexOf(terminalId)
  if (existing >= 0) next[existing] = null
  next[clampIndex(paneIndex, next.length)] = terminalId
  return next
}

export function dropMissingTerminals(
  slots: readonly (string | null)[],
  terminalIds: ReadonlySet<string>
): PaneSlots {
  let changed = false
  const next = slots.map((id) => {
    if (id !== null && !terminalIds.has(id)) {
      changed = true
      return null
    }
    return id
  })
  return changed ? next : (slots as PaneSlots)
}

export function requestLayoutFit(): void {
  if (typeof window === 'undefined') return
  window.dispatchEvent(new Event(LAYOUT_FIT_EVENT))
}

export function dispatchLayoutShortcut(kind: LayoutShortcut): void {
  if (typeof window === 'undefined') return
  window.dispatchEvent(new CustomEvent<LayoutShortcut>(LAYOUT_SHORTCUT_EVENT, { detail: kind }))
}

function modifierPlain(event: {
  ctrlKey: boolean
  shiftKey: boolean
  altKey: boolean
  metaKey: boolean
}): boolean {
  return event.ctrlKey && !event.shiftKey && !event.altKey && !event.metaKey
}

export function layoutShortcut(event: {
  key: string
  code?: string
  ctrlKey: boolean
  shiftKey: boolean
  altKey: boolean
  metaKey: boolean
}): LayoutShortcut | null {
  if (!modifierPlain(event)) return null
  if (event.key === 'b' || event.key === 'B') return 'drawer'
  if (event.key === '\\' || event.key === 'Backslash' || event.code === 'Backslash')
    return 'sidebar'
  if (event.key === '1') return 'files'
  if (event.key === '2') return 'lead'
  if (event.key === '3') return 'jobs'
  if (event.key === '4') return 'changes'
  if (event.key === '5') return 'routines'
  if (event.key === '6') return 'usage'
  if (event.key === '7') return 'activity'
  if (event.key === '8') return 'history'
  return null
}

/**
 * Terminals that came back after a restart, or were opened from Changes, arrive with no pane.
 * Each takes the first empty pane, in tab order; none is moved and a full grid is left alone.
 */
export function seatUnplaced(slots: PaneSlots, terminalIds: readonly string[]): PaneSlots {
  let next = slots
  for (const id of terminalIds) {
    if (next.includes(id)) continue
    const empty = next.indexOf(null)
    if (empty < 0) break
    next = placeTerminalInPane(next, empty, id)
  }
  return next
}
