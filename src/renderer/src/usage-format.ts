/** "updated just now", "updated 4 min ago", "updated 2 h ago". */
export function updatedPhrase(at: number, now: number): string {
  const minutes = Math.max(0, Math.floor((now - at) / 60_000))
  if (minutes < 1) return 'updated just now'
  if (minutes < 60) return `updated ${String(minutes)} min ago`
  return `updated ${String(Math.floor(minutes / 60))} h ago`
}
