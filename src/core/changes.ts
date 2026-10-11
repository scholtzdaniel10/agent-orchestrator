import type { ChangeSet } from './types'

export interface ListedName {
  id: string
  name: string
  created_at: number
}

/**
 * The Changes tab list: every orch worktree, including ones with no edits yet,
 * named from the store and ordered newest first.
 */
export function mergeListedChanges(input: {
  listed: readonly ChangeSet[]
  ids: readonly string[]
  names: readonly ListedName[]
  folder: (id: string) => string
  setup?: (id: string) => ChangeSet['setup']
}): ChangeSet[] {
  const byId = new Map(input.listed.map((change) => [change.id, change]))
  const nameById = new Map(input.names.map((row) => [row.id, row]))
  const merged: ChangeSet[] = []
  for (const id of input.ids) {
    const existing = byId.get(id)
    const row = nameById.get(id)
    const entry: ChangeSet =
      existing !== undefined
        ? { ...existing }
        : {
            id,
            branch: `orch/${id}`,
            path: input.folder(id),
            files: [],
            insertions: 0,
            deletions: 0
          }
    if (row !== undefined && row.name !== '') entry.name = row.name
    const setup = input.setup?.(id)
    if (setup !== undefined) entry.setup = setup
    merged.push(entry)
  }
  merged.sort((a, b) => {
    const ta = nameById.get(a.id)?.created_at ?? 0
    const tb = nameById.get(b.id)?.created_at ?? 0
    if (ta !== tb) return tb - ta
    return a.id < b.id ? -1 : a.id > b.id ? 1 : 0
  })
  return merged
}
