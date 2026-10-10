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
