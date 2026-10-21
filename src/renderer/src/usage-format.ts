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

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']
const MONTH_NAMES = [
  'January',
  'February',
  'March',
  'April',
  'May',
  'June',
  'July',
  'August',
  'September',
  'October',
  'November',
  'December'
]

/** How far ahead a monthly reset can be: a month of 31 days, and a day to spare. */
export const RESET_DAYS_AHEAD = 32

/** Whole calendar days from the day of `fromMs` to the day of `toMs`, in local time. */
export function daysBetween(fromMs: number, toMs: number): number {
  const from = new Date(fromMs)
  const to = new Date(toMs)
  const start = Date.UTC(from.getFullYear(), from.getMonth(), from.getDate())
  const end = Date.UTC(to.getFullYear(), to.getMonth(), to.getDate())
  return Math.round((end - start) / 86_400_000)
}

/** "Nov 4 (14d)": a reset day the way Cursor's dashboard shows it, with days left from today. */
export function resetLabel(ms: number, now: number): string {
  const date = new Date(ms)
  const left = daysBetween(now, ms)
  return `${MONTHS[date.getMonth()]} ${String(date.getDate())} (${String(left)}d)`
}

/** "November 2026" */
export function monthTitle(year: number, month: number): string {
  return `${MONTH_NAMES[month]} ${String(year)}`
}

/**
 * The weeks of a month for a calendar grid, Sunday first as Windows shows it. Each cell is a day
 * of the month, or null where the week runs into the month before or after.
 */
export function monthWeeks(year: number, month: number): (number | null)[][] {
  const lead = new Date(year, month, 1).getDay()
  const length = new Date(year, month + 1, 0).getDate()
  const cells: (number | null)[] = []
  for (let i = 0; i < lead; i++) cells.push(null)
  for (let day = 1; day <= length; day++) cells.push(day)
  while (cells.length % 7 !== 0) cells.push(null)
  const weeks: (number | null)[][] = []
  for (let i = 0; i < cells.length; i += 7) weeks.push(cells.slice(i, i + 7))
  return weeks
}

/**
 * The days a monthly usage reset can fall on, from tomorrow to `days` days ahead:
 * `{ value: '2026-11-04', label: 'Nov 4 (14d)' }`.
 */
export function resetChoices(
  now: number,
  days = RESET_DAYS_AHEAD
): { value: string; label: string }[] {
  const today = new Date(now)
  const choices: { value: string; label: string }[] = []
  for (let ahead = 1; ahead <= days; ahead++) {
    const date = new Date(today.getFullYear(), today.getMonth(), today.getDate() + ahead)
    choices.push({ value: dateValue(date.getTime()), label: resetLabel(date.getTime(), now) })
  }
  return choices
}
