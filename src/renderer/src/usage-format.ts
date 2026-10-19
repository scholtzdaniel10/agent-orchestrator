const MINUTE_MS = 60_000

/** Time until a reset as "4h 8m", or null once it has passed. */
export function untilReset(resetsAt: number, now: number): string | null {
  const remaining = resetsAt - now
  if (remaining <= 0) return null
  if (remaining < MINUTE_MS) return 'under 1m'
  const days = Math.floor(remaining / (24 * 60 * MINUTE_MS))
  const hours = Math.floor(remaining / (60 * MINUTE_MS)) % 24
  const minutes = Math.floor(remaining / MINUTE_MS) % 60
  const parts: string[] = []
  if (days > 0) parts.push(`${days}d`)
  if (hours > 0) parts.push(`${hours}h`)
  if (minutes > 0) parts.push(`${minutes}m`)
  return parts.slice(0, 2).join(' ')
}

export function meterFillClass(id: string, left: number): string {
  if (left < 15) return 'meter-fill meter-bad'
  if (left < 30) return 'meter-fill meter-warn'
  return `meter-fill meter-${id}`
}
