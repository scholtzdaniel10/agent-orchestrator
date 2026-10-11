export interface IdItem {
  id: string
  name?: string
}

export type IdHit = { ok: true; id: string } | { ok: false; message: string }

/** Unique id prefix of at least 4 characters, or an exact name. */
export function resolveId(token: string, items: readonly IdItem[], noun: string): IdHit {
  const needle = token.toLowerCase()
  const exact = items.filter((item) => item.id.toLowerCase() === needle)
  if (exact.length === 1) {
    const only = exact[0]
    if (only !== undefined) return { ok: true, id: only.id }
  }

  const named = items.filter(
    (item) => item.name !== undefined && item.name !== '' && item.name === token
  )
  const prefixed =
    token.length >= 4 ? items.filter((item) => item.id.toLowerCase().startsWith(needle)) : []
  const matches = new Map<string, IdItem>()
  for (const item of exact) matches.set(item.id, item)
  for (const item of named) matches.set(item.id, item)
  for (const item of prefixed) matches.set(item.id, item)

  if (matches.size === 1) {
    const only = [...matches.values()][0]
    if (only !== undefined) return { ok: true, id: only.id }
  }
  if (matches.size > 1) {
    const listed = [...matches.values()].map((item) =>
      item.name !== undefined && item.name !== '' ? `${item.id} (${item.name})` : item.id
    )
    return {
      ok: false,
      message: `${noun} id "${token}" matches ${englishList(listed)}.`
    }
  }
  if (token.length < 4) return { ok: false, message: `${noun} id needs at least 4 characters.` }
  return { ok: false, message: `No ${noun.toLowerCase()} matches "${token}".` }
}

function englishList(parts: string[]): string {
  if (parts.length <= 1) return parts[0] ?? ''
  if (parts.length === 2) return `${parts[0]} and ${parts[1]}`
  return `${parts.slice(0, -1).join(', ')}, and ${parts[parts.length - 1]}`
}
