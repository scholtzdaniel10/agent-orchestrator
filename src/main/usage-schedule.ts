/**
 * When to ask Claude for its usage. Each check is one tiny turn on the smallest model (about 600
 * tokens), so it may run often, but use outside Legate only shows up at the next one.
 */
export const USAGE_GAP_MS = {
  /** Refresh in the Usage tab: a double click must not run two checks. */
  manual: 2 * 60_000,
  /** Coming back to a Legate window, often after using Claude somewhere else. */
  focus: 30_000,
  /** While a Legate window has focus. */
  focused: 5 * 60_000,
  /** While a window is on screen but another app has focus. */
  visible: 10 * 60_000
} as const

/** How often the timer looks at whether a check is due. */
export const USAGE_TICK_MS = 60_000

export type UsageTrigger = 'manual' | 'focus' | 'tick'

/**
 * True when a check should run now. `last` is the newest reading, from a check or from a job's
 * own report, or null when there has been none. A tick checks only while a window is on screen.
 */
export function usageCheckDue(
  trigger: UsageTrigger,
  now: number,
  last: number | null,
  windows: { focused: boolean; visible: boolean }
): boolean {
  if (last === null) return trigger !== 'tick' || windows.visible
  const age = now - last
  if (trigger === 'manual') return age >= USAGE_GAP_MS.manual
  if (trigger === 'focus') return age >= USAGE_GAP_MS.focus
  if (windows.focused) return age >= USAGE_GAP_MS.focused
  if (windows.visible) return age >= USAGE_GAP_MS.visible
  return false
}
