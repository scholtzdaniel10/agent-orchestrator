/** "just now", "4 min ago", "2 h ago", "3 days ago": how old a reading the person typed in is. */
export function agoPhrase(at: number, now: number): string {
  const minutes = Math.max(0, Math.floor((now - at) / 60_000))
  if (minutes < 1) return 'just now'
  if (minutes < 60) return `${String(minutes)} min ago`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${String(hours)} h ago`
  const days = Math.floor(hours / 24)
  return days === 1 ? '1 day ago' : `${String(days)} days ago`
}

/** "updated just now", "updated 4 min ago", "updated 2 h ago". */
export function updatedPhrase(at: number, now: number): string {
  const minutes = Math.max(0, Math.floor((now - at) / 60_000))
  if (minutes < 1) return 'updated just now'
  if (minutes < 60) return `updated ${String(minutes)} min ago`
  return `updated ${String(Math.floor(minutes / 60))} h ago`
}

/** `yyyy-mm-dd` for a date input, in local time. */
export function dateValue(ms: number): string {
  const date = new Date(ms)
  const month = (date.getMonth() + 1).toString().padStart(2, '0')
  const day = date.getDate().toString().padStart(2, '0')
  return `${String(date.getFullYear())}-${month}-${day}`
}

/** Local midnight of a `yyyy-mm-dd` date input, or null when it is empty or malformed. */
export function dateFromValue(value: string): number | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value)
  if (match === null) return null
  const date = new Date(Number(match[1]), Number(match[2]) - 1, Number(match[3]))
  if (date.getDate() !== Number(match[3])) return null
  return date.getTime()
}

const WEEKDAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']

/** How far ahead a monthly reset can be: a month of 31 days, and a day to spare. */
export const RESET_DAYS_AHEAD = 32

/**
 * The days a monthly usage reset can fall on, from tomorrow to `days` days ahead:
 * `{ value: '2026-11-05', label: 'Thu 5 Nov · in 15 days' }`.
 */
export function resetChoices(
  now: number,
  days = RESET_DAYS_AHEAD
): { value: string; label: string }[] {
  const today = new Date(now)
  const choices: { value: string; label: string }[] = []
  for (let ahead = 1; ahead <= days; ahead++) {
    const date = new Date(today.getFullYear(), today.getMonth(), today.getDate() + ahead)
    const day = `${WEEKDAYS[date.getDay()]} ${String(date.getDate())} ${MONTHS[date.getMonth()]}`
    const when = ahead === 1 ? 'tomorrow' : `in ${String(ahead)} days`
    choices.push({ value: dateValue(date.getTime()), label: `${day} · ${when}` })
  }
  return choices
}
