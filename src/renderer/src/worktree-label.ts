/** Display name of a worktree: the saved name when present, otherwise its id. */
export function worktreeLabel(change: { id: string; name?: string }): string {
  const name = change.name
  if (typeof name === 'string' && name !== '') return name
  return change.id
}

/** Tab and pane title with the worktree name in place of the id. */
export function terminalDisplayTitle(
  info: { title: string; change?: string },
  changes: readonly { id: string; name?: string }[]
): string {
  if (info.change === undefined) return info.title
  const found = changes.find((item) => item.id === info.change)
  const label = worktreeLabel(found ?? { id: info.change })
  const suffix = ` · ${info.change}`
  if (info.title.endsWith(suffix)) {
    return `${info.title.slice(0, -info.change.length)}${label}`
  }
  return info.title
}

/** Project-folder terminals first, then sidebar worktree order, then original order. */
export function terminalsInSidebarOrder<T extends { change?: string }>(
  terminals: readonly T[],
  changes: readonly { id: string }[]
): T[] {
  const order = new Map<string | undefined, number>()
  order.set(undefined, 0)
  for (let i = 0; i < changes.length; i += 1) {
    order.set(changes[i].id, i + 1)
  }
  return terminals
    .map((info, index) => ({ info, index }))
    .sort((a, b) => {
      const rankA = order.get(a.info.change) ?? Number.MAX_SAFE_INTEGER
      const rankB = order.get(b.info.change) ?? Number.MAX_SAFE_INTEGER
      if (rankA !== rankB) return rankA - rankB
      return a.index - b.index
    })
    .map((item) => item.info)
}
