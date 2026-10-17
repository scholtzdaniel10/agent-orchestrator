/** Sidebar worktree cards: status, filtering, sorting and grouping. Pure, so it is tested alone. */

export type CardStatus = 'working' | 'unread' | 'idle'

export interface SidebarOptions {
  groupBy: 'none' | 'status'
  sortBy: 'created' | 'recent' | 'name'
  layout: 'detailed' | 'compact'
  /** Hide worktrees with no changes, no terminals and nothing happening. */
  hideClean: boolean
}

export const DEFAULT_OPTIONS: SidebarOptions = {
  groupBy: 'status',
  sortBy: 'created',
  layout: 'detailed',
  hideClean: false
}

/** Terminal output this recent counts as an agent working. */
// ponytail: output-based guess (typing and redraws count too); use CLI hooks if it misleads.
export const WORKING_MS = 4000

export interface CardInput {
  /** Worktree id, or null for the project folder. */
  key: string | null
  title: string
  branch: string | null
  terminalIds: readonly string[]
  jobRunning: boolean
  hasChanges: boolean
  /** Position in the list as the app has it (project folder first). */
  createdIndex: number
  pinned: boolean
  /** Epoch ms the person last looked at this worktree. */
  seenAt: number
  selected: boolean
}

export interface Card extends CardInput {
  status: CardStatus
  /** Epoch ms of the latest terminal output, or 0. */
  lastActive: number
}

export interface CardGroup {
  /** Heading, or null when the list has a single unlabelled group. */
  label: string | null
  cards: Card[]
}

export function cardStatus(
  input: CardInput,
  activity: Readonly<Record<string, number>>,
  now: number
): Card {
  let lastActive = 0
  for (const id of input.terminalIds) lastActive = Math.max(lastActive, activity[id] ?? 0)
  let status: CardStatus = 'idle'
  if (input.jobRunning || (lastActive > 0 && now - lastActive < WORKING_MS)) status = 'working'
  else if (!input.selected && lastActive > input.seenAt) status = 'unread'
  return { ...input, status, lastActive }
}

export function arrangeCards(
  cards: readonly Card[],
  options: SidebarOptions,
  filter: string
): CardGroup[] {
  const needle = filter.trim().toLowerCase()
  const shown = cards.filter((card) => {
    if (needle !== '') {
      const hay = `${card.title} ${card.branch ?? ''}`.toLowerCase()
      if (!hay.includes(needle)) return false
    }
    if (!options.hideClean || card.key === null || card.selected || card.pinned) return true
    return card.hasChanges || card.terminalIds.length > 0 || card.status !== 'idle'
  })
  shown.sort((a, b) => {
    if (options.sortBy === 'name') return a.title.localeCompare(b.title)
    if (options.sortBy === 'recent' && a.lastActive !== b.lastActive) {
      return b.lastActive - a.lastActive
    }
    return a.createdIndex - b.createdIndex
  })
  // The project folder is the primary card, so it always leads the pinned group.
  const pinned = shown.filter((card) => card.pinned || card.key === null)
  const rest = shown.filter((card) => !card.pinned && card.key !== null)
  const groups: CardGroup[] = []
  if (pinned.length > 0) groups.push({ label: 'Pinned', cards: pinned })
  if (options.groupBy === 'status') {
    const buckets: [string, (card: Card) => boolean][] = [
      ['In progress', (card) => card.status === 'working'],
      ['Needs you', (card) => card.status === 'unread'],
      ['Ready to review', (card) => card.status === 'idle' && card.hasChanges],
      ['Quiet', (card) => card.status === 'idle' && !card.hasChanges]
    ]
    for (const [label, test] of buckets) {
      const inGroup = rest.filter(test)
      if (inGroup.length > 0) groups.push({ label, cards: inGroup })
    }
  } else if (rest.length > 0) {
    groups.push({ label: pinned.length > 0 ? 'Worktrees' : null, cards: rest })
  }
  return groups
}

export function readOptions(raw: unknown): SidebarOptions {
  if (typeof raw !== 'object' || raw === null) return DEFAULT_OPTIONS
  const value = raw as Record<string, unknown>
  return {
    groupBy: value.groupBy === 'status' ? 'status' : 'none',
    sortBy: value.sortBy === 'recent' || value.sortBy === 'name' ? value.sortBy : 'created',
    layout: value.layout === 'compact' ? 'compact' : 'detailed',
    hideClean: value.hideClean === true
  }
}
