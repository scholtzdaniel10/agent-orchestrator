/** "now", "5m", "3h", "2d", "4mo": how long ago a commit was made. */
export function ago(time: number, now: number): string {
  const minutes = Math.floor(Math.max(0, now - time) / 60_000)
  if (minutes < 1) return 'now'
  if (minutes < 60) return `${String(minutes)}m`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${String(hours)}h`
  const days = Math.floor(hours / 24)
  if (days < 60) return `${String(days)}d`
  return `${String(Math.floor(days / 30))}mo`
}

export interface RefBadge {
  name: string
  kind: 'head' | 'tag' | 'branch'
}

/** "HEAD -> main" → main (current), "tag: v1" → v1 (tag), "origin/HEAD" left out. */
export function refBadges(refs: readonly string[]): RefBadge[] {
  const out: RefBadge[] = []
  for (const ref of refs) {
    if (ref === 'HEAD') continue
    if (ref.startsWith('HEAD -> ')) out.push({ name: ref.slice(8), kind: 'head' })
    else if (ref.startsWith('tag: ')) out.push({ name: ref.slice(5), kind: 'tag' })
    else if (!ref.endsWith('/HEAD')) out.push({ name: ref, kind: 'branch' })
  }
  return out
}
