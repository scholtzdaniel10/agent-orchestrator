import type { RunMode } from '../../core/types'

/** What turning LEG IT! on does, in the order the confirm lists it. */
export const LEG_IT_EFFECTS: readonly string[] = [
  "The lead's plans start as soon as it proposes them. Nobody presses Run, up to five plans per message.",
  'Jobs may get full access: they edit files and run commands, each in its own worktree.',
  'Terminals you open skip every prompt (Claude with bypassPermissions, Cursor with --force), in the folder they open in.',
  'The lead reports once, when a check shows the work done, not while it works.'
]

/** What still holds while LEG IT! is on. */
export const LEG_IT_KEPT = 'Changes from jobs still wait in Changes until you merge them.'

/**
 * What pressing LEG IT! does: turn it off, ask first, or turn it on. The first switch on in a
 * window asks; once confirmed there, it switches straight away.
 */
export function legItPress(current: RunMode, confirmed: boolean): RunMode | 'confirm' {
  if (current === 'legit') return 'ask'
  return confirmed ? 'legit' : 'confirm'
}
