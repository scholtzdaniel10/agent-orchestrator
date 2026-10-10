export const LAYOUT_KEY = 'orch.layout'
export const LAYOUT_FIT_EVENT = 'layoutfit'

export const MIN_SIDEBAR = 160
export const MIN_LEAD = 280
export const MIN_USAGE = 200
export const MIN_FLOW = 96
export const MIN_WORKERS = 360
export const HEADER_HEIGHT = 40
export const FLOW_REST = 80

export const PANE_LAYOUT_IDS = ['1', '2-side', '2-stack', '3-side', '4-grid', '6-grid'] as const

export type PaneLayoutId = (typeof PANE_LAYOUT_IDS)[number]

export const PANE_LAYOUT_LABELS: Record<PaneLayoutId, string> = {
  '1': '1',
  '2-side': '2 side by side',
  '2-stack': '2 stacked',
  '3-side': '3 side by side',
  '4-grid': '4 grid',
  '6-grid': '6 grid'
}

export const PANE_COUNTS: Record<PaneLayoutId, number> = {
  '1': 1,
  '2-side': 2,
  '2-stack': 2,
  '3-side': 3,
  '4-grid': 4,
  '6-grid': 6
}

export interface LayoutSizes {
  sidebar: number
  lead: number
  usage: number
  flow: number
}

export interface PanelVisibility {
  projects: boolean
  flow: boolean
  lead: boolean
  usage: boolean
}

export interface LayoutState {
  sizes: LayoutSizes
  shown: PanelVisibility
  paneLayout: PaneLayoutId
}

export interface LayoutStorage {
  getItem(key: string): string | null
  setItem(key: string, value: string): void
}

export type PaneSlots = (string | null)[]

export type SizeKey = keyof LayoutSizes
export type PanelKey = keyof PanelVisibility
export type RowMode = 'wide' | 'medium' | 'narrow'

export const DEFAULT_SIZES: LayoutSizes = {
  sidebar: 200,
  lead: 360,
  usage: 230,
  flow: 190
}

export const DEFAULT_SHOWN: PanelVisibility = {
  projects: true,
  flow: true,
  lead: true,
  usage: true
}

export const DEFAULT_LAYOUT: LayoutState = {
  sizes: { ...DEFAULT_SIZES },
  shown: { ...DEFAULT_SHOWN },
  paneLayout: '1'
}

const PANE_LAYOUT_SET: ReadonlySet<string> = new Set(PANE_LAYOUT_IDS)

export function isPaneLayout(value: string): value is PaneLayoutId {
  return PANE_LAYOUT_SET.has(value)
}

export function paneCount(layout: PaneLayoutId): number {
  return PANE_COUNTS[layout]
}

export function rowMode(width: number): RowMode {
  if (width < 700) return 'narrow'
  if (width < 900) return 'medium'
  return 'wide'
}

function asNumber(value: unknown, fallback: number, min: number): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) return fallback
  return Math.max(min, Math.round(value))
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
      shown: { ...DEFAULT_SHOWN },
      paneLayout: DEFAULT_LAYOUT.paneLayout
    }
  }

  const sizesRaw = asRecord(root.sizes)
  const sizes: LayoutSizes = {
    sidebar: asNumber(sizesRaw?.sidebar, DEFAULT_SIZES.sidebar, MIN_SIDEBAR),
    lead: asNumber(sizesRaw?.lead, DEFAULT_SIZES.lead, MIN_LEAD),
    usage: asNumber(sizesRaw?.usage, DEFAULT_SIZES.usage, MIN_USAGE),
    flow: asNumber(sizesRaw?.flow, DEFAULT_SIZES.flow, MIN_FLOW)
  }

  const shownRaw = asRecord(root.shown)
  const shown: PanelVisibility = {
    projects: asBool(shownRaw?.projects, DEFAULT_SHOWN.projects),
    flow: asBool(shownRaw?.flow, DEFAULT_SHOWN.flow),
    lead: asBool(shownRaw?.lead, DEFAULT_SHOWN.lead),
    usage: asBool(shownRaw?.usage, DEFAULT_SHOWN.usage)
  }

  const paneLayout =
    typeof root.paneLayout === 'string' && isPaneLayout(root.paneLayout)
      ? root.paneLayout
      : DEFAULT_LAYOUT.paneLayout

  return { sizes, shown, paneLayout }
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
        shown: { ...DEFAULT_SHOWN },
        paneLayout: DEFAULT_LAYOUT.paneLayout
      }
    }
    return parseLayout(JSON.parse(raw) as unknown)
  } catch {
    return {
      sizes: { ...DEFAULT_SIZES },
      shown: { ...DEFAULT_SHOWN },
      paneLayout: DEFAULT_LAYOUT.paneLayout
    }
  }
}

export function saveLayout(state: LayoutState, storage: LayoutStorage = liveStorage()): void {
  try {
    storage.setItem(
      LAYOUT_KEY,
      JSON.stringify({
        sizes: state.sizes,
        shown: state.shown,
        paneLayout: state.paneLayout
      })
    )
  } catch {
    // Private mode or a full quota must not block the layout.
  }
}

function takeFromStart(parts: number[], mins: number[], need: number): number[] {
  const next = parts.slice()
  let rest = need
  for (let i = 0; i < next.length && rest > 0; i += 1) {
    const extra = next[i] - mins[i]
    if (extra <= 0) continue
    const take = Math.min(extra, rest)
    next[i] -= take
    rest -= take
  }
  return next
}

export function clampSizes(
  sizes: LayoutSizes,
  viewport: { width: number; height: number },
  shown: PanelVisibility
): LayoutSizes {
  const width = Math.max(1, Math.round(viewport.width))
  const height = Math.max(1, Math.round(viewport.height))
  const maxFlow = Math.max(MIN_FLOW, height - HEADER_HEIGHT - FLOW_REST)
  const flow = shown.flow
    ? Math.min(maxFlow, Math.max(MIN_FLOW, Math.round(sizes.flow)))
    : Math.max(MIN_FLOW, Math.round(sizes.flow))

  let sidebar = Math.max(MIN_SIDEBAR, Math.round(sizes.sidebar))
  let lead = Math.max(MIN_LEAD, Math.round(sizes.lead))
  let usage = Math.max(MIN_USAGE, Math.round(sizes.usage))

  const mode = rowMode(width)
  const parts: number[] = []
  const mins: number[] = []
  if (shown.projects) {
    parts.push(sidebar)
    mins.push(MIN_SIDEBAR)
  }
  if (mode !== 'narrow' && shown.lead) {
    parts.push(lead)
    mins.push(MIN_LEAD)
  }
  if (mode === 'wide' && shown.usage) {
    parts.push(usage)
    mins.push(MIN_USAGE)
  }

  const used = parts.reduce((sum, value) => sum + value, 0)
  const leftover = width - used
  if (leftover < MIN_WORKERS && parts.length > 0) {
    const fitted = takeFromStart(parts, mins, MIN_WORKERS - leftover)
    let index = 0
    if (shown.projects) {
      sidebar = fitted[index]
      index += 1
    }
    if (mode !== 'narrow' && shown.lead) {
      lead = fitted[index]
      index += 1
    }
    if (mode === 'wide' && shown.usage) {
      usage = fitted[index]
    }
  }

  if (mode === 'narrow' && shown.lead) {
    const cap = Math.max(MIN_LEAD, width - (shown.projects ? sidebar : 0))
    lead = Math.min(lead, cap)
  }
  if (mode !== 'wide' && shown.usage) {
    const cap = Math.max(MIN_USAGE, width - (shown.projects ? sidebar : 0))
    usage = Math.min(usage, cap)
  }

  return { sidebar, lead, usage, flow }
}

export function maxSize(
  key: SizeKey,
  sizes: LayoutSizes,
  viewport: { width: number; height: number },
  shown: PanelVisibility
): number {
  if (key === 'flow') {
    return Math.max(MIN_FLOW, Math.round(viewport.height) - HEADER_HEIGHT - FLOW_REST)
  }
  return clampSizes({ ...sizes, [key]: 10_000 }, viewport, shown)[key]
}

export function emptySlots(layout: PaneLayoutId): PaneSlots {
  return Array.from({ length: paneCount(layout) }, () => null)
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
  const slots = emptySlots(layout)
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
