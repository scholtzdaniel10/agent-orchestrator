/** One row of a Select, read from its `<option>` children. */
export interface SelectOption {
  value: string
  label: string
  /** Shown after the label in the code face, e.g. a model id. */
  detail?: string
  disabled?: boolean
  title?: string
}

/**
 * The next option that can be picked, `step` rows from `from` (1 down, -1 up), stopping at the
 * ends rather than wrapping. `from` -1 with a step of 1 finds the first. Returns `from` when
 * nothing further can be picked.
 */
export function nextEnabled(options: readonly SelectOption[], from: number, step: 1 | -1): number {
  for (let i = from + step; i >= 0 && i < options.length; i += step) {
    if (options[i].disabled !== true) return i
  }
  return from
}

/** The first (`end` 'first') or last pickable option, or -1 when none can be picked. */
export function edgeEnabled(options: readonly SelectOption[], end: 'first' | 'last'): number {
  const found =
    end === 'first' ? nextEnabled(options, -1, 1) : nextEnabled(options, options.length, -1)
  return found === -1 || found === options.length ? -1 : found
}

/**
 * Type to jump: the next pickable option after `from` whose label starts with `typed`
 * (ignoring case), wrapping round once. -1 when none matches.
 */
export function matchTyped(options: readonly SelectOption[], from: number, typed: string): number {
  const wanted = typed.toLowerCase()
  if (wanted === '') return -1
  for (let k = 1; k <= options.length; k++) {
    const i = (from + k + options.length) % options.length
    const option = options[i]
    if (option.disabled !== true && option.label.toLowerCase().startsWith(wanted)) return i
  }
  return -1
}

export interface Placement {
  top: number
  left: number
  width: number
  maxHeight: number
  /** True when the list opens above its button because there is more room there. */
  above: boolean
}

const GAP = 4
const EDGE = 8
/** Taller than this and the list scrolls. */
export const LIST_MAX = 320

/**
 * Where the list goes for a button at `rect` in a window `viewport` wide and high: under the
 * button and at least as wide as it, or above it when there is more room above and the list
 * would not fit below. Kept `EDGE` px inside the window on every side.
 */
export function placeList(
  rect: { top: number; bottom: number; left: number; width: number },
  viewport: { width: number; height: number },
  listHeight: number,
  listWidth: number
): Placement {
  const width = Math.min(Math.max(rect.width, listWidth), viewport.width - 2 * EDGE)
  const left = Math.min(Math.max(rect.left, EDGE), viewport.width - EDGE - width)
  const below = viewport.height - rect.bottom - GAP - EDGE
  const above = rect.top - GAP - EDGE
  const wanted = Math.min(listHeight, LIST_MAX)
  const goUp = wanted > below && above > below
  const maxHeight = Math.max(0, Math.min(LIST_MAX, goUp ? above : below))
  const height = Math.min(wanted, maxHeight)
  const top = goUp ? rect.top - GAP - height : rect.bottom + GAP
  return { top, left, width, maxHeight, above: goUp }
}
