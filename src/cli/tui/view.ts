import { formatDuration, stepLine } from '../format'
import type { Focus, JobItem, TuiState, WorktreeItem } from './state'

const FOOTER_KEYS = [
  'n new job',
  'Tab switch pane',
  'Enter open',
  'd diff',
  'm merge',
  'x discard',
  'w worktree',
  'c/u agent here',
  's stop'
]
const FOOTER_SHORT = '? help  q quit'

export function isNarrow(columns: number): boolean {
  return columns < 100
}

/** As many keys as fit, always ending with help and quit. */
export function footerText(columns: number): string {
  if (isNarrow(columns)) return FOOTER_SHORT
  const shown: string[] = []
  let width = FOOTER_SHORT.length
  for (const key of FOOTER_KEYS) {
    if (width + key.length + 2 > columns) break
    shown.push(key)
    width += key.length + 2
  }
  return [...shown, FOOTER_SHORT].join('  ')
}

export function leftColumn(columns: number): number {
  if (columns < 40) return Math.max(12, Math.floor(columns / 2))
  if (isNarrow(columns)) return 24
  return 36
}

export type LineTone = 'plain' | 'dim' | 'add' | 'remove' | 'error' | 'accent'

export interface LineModel {
  text: string
  tone: LineTone
}

export interface RowModel {
  text: string
  selected: boolean
  focused: boolean
}

export interface ScreenModel {
  top: string
  /** The top line in parts, so the screen can colour them. */
  where: string
  plans: string[]
  focus: Focus
  /** Heights of the three boxes, filling the terminal between the top line and the prompt. */
  heights: { worktrees: number; jobs: number; detail: number }
  worktreesTitle: string
  jobsTitle: string
  worktrees: RowModel[]
  jobs: RowModel[]
  title: string
  detail: LineModel[]
  editing: boolean
  placeholder: string
  confirm: string
  aside: string
  error: string | null
  footer: string
  left: number
  color: boolean
}

/** Rows between the top line and the prompt: top, prompt and footer, plus the error line. */
export function bodyRows(state: TuiState): number {
  const chrome = 3 + (state.error !== null ? 1 : 0)
  return Math.max(8, state.rows - chrome)
}

export function detailPage(state: TuiState): number {
  return Math.max(1, bodyRows(state) - 3)
}

export function detailBody(state: TuiState): LineModel[] {
  if (state.pane === 'help') return helpLines()
  if (state.pane === 'routines') return routineLines(state)
  if (state.pane === 'diff') return diffLines(state.diffText)
  if (state.pane === 'worktree' || (state.focus === 'worktrees' && state.pane !== 'job')) {
    return fileLines(selectedWorktree(state))
  }
  return jobLines(state)
}

export function screenModel(state: TuiState): ScreenModel {
  const left = leftColumn(state.columns)
  const inner = Math.max(8, left - 2)
  const body = bodyRows(state)
  const worktreesHeight = Math.max(4, Math.floor(body / 2))
  const jobsHeight = Math.max(4, body - worktreesHeight)
  const worktreeAt = indexOf(state.worktrees, (item) => item.id === state.worktreeId)
  const jobAt = indexOf(state.jobs, (item) => item.id === state.jobId)
  const workFocus = state.focus === 'worktrees'
  const jobFocus = state.focus === 'jobs'
  return {
    top: topLine(state),
    where: whereText(state),
    plans: state.plans.map((plan) => plan.text),
    focus: state.focus,
    heights: { worktrees: worktreesHeight, jobs: jobsHeight, detail: body },
    worktreesTitle: countTitle('Worktrees', state.worktrees.filter((item) => !item.project).length),
    jobsTitle: jobsTitle(state.jobs),
    worktrees: windowItems(state.worktrees, worktreeAt, worktreesHeight - 3).map(
      ({ item, selected }) => ({
        text: fit(mark(selected) + worktreeText(item), inner),
        selected,
        focused: workFocus && selected
      })
    ),
    jobs: windowItems(state.jobs, jobAt, jobsHeight - 3).map(({ item, selected }) => ({
      text: fit(mark(selected) + jobText(item), inner),
      selected,
      focused: jobFocus && selected
    })),
    title: fit(rightTitle(state), Math.max(8, state.columns - left - 2)),
    detail: visibleDetail(state),
    editing: state.mode === 'job' || state.mode === 'name',
    placeholder: state.mode === 'name' ? 'worktree name, optional' : 'type a job and press Enter',
    confirm: confirmText(state),
    aside:
      state.mode === 'idle' || state.mode === 'job'
        ? `type: ${state.jobType} · worker: ${state.worker} · access: ${state.access}`
        : '',
    error: state.error,
    footer: footerText(state.columns),
    left,
    color: state.color
  }
}

function visibleDetail(state: TuiState): LineModel[] {
  const lines = detailBody(state)
  const width = Math.max(8, state.columns - leftColumn(state.columns) - 2)
  const start = state.scroll
  return lines.slice(start, start + detailPage(state)).map((line) => ({
    text: fit(line.text, width),
    tone: line.tone
  }))
}

function whereText(state: TuiState): string {
  const branch = state.branch !== null && state.branch !== '' ? state.branch : 'no branch'
  return `${state.projectName} · ${branch}`
}

function countTitle(word: string, count: number): string {
  return count > 0 ? `${word}  ${String(count)}` : word
}

function jobsTitle(jobs: readonly JobItem[]): string {
  const running = jobs.filter((job) => job.status === 'running').length
  if (running > 0) return `Jobs  ${String(running)} running`
  return countTitle('Jobs', jobs.length)
}

function topLine(state: TuiState): string {
  const left = `Legate · ${whereText(state)}`
  const right = state.plans.map((plan) => plan.text).join('  ')
  if (right === '') return fit(left, state.columns)
  const room = state.columns - left.length - 2
  if (room < 8) return fit(left, state.columns)
  return `${left}  ${fit(right, room)}`
}

function rightTitle(state: TuiState): string {
  if (state.pane === 'help') return 'Keys'
  if (state.pane === 'routines') return 'Routines'
  if (state.pane === 'diff') {
    const row = selectedWorktree(state)
    return `Diff ${row?.label ?? ''}`
  }
  if (state.pane === 'worktree' || (state.focus === 'worktrees' && state.pane !== 'job')) {
    const row = selectedWorktree(state)
    if (row === undefined) return 'Worktree'
    if (row.project) return `${row.label} · ${row.branch ?? 'no branch'}`
    return row.label
  }
  const job = selectedJob(state)
  if (job === undefined) return 'Job'
  const parts = [`Job ${job.id.slice(0, 8)}`]
  if (job.provider !== null) parts.push(job.provider)
  if (job.model !== '') parts.push(job.model)
  parts.push(job.type, job.access)
  if (job.reason !== '') parts.push(job.reason)
  if (job.status === 'running') parts.push(`running ${formatDuration(state.now - job.createdAt)}`)
  else parts.push(job.status)
  return parts.join(' · ')
}

function jobLines(state: TuiState): LineModel[] {
  const job = selectedJob(state)
  if (job === undefined) return [{ text: 'No jobs yet', tone: 'plain' }]
  const lines: LineModel[] = []
  for (const step of job.steps) {
    const text = stepLine(step, true, false)
    if (text === null) continue
    const dim = step.tool === 'say' || step.tool === 'think'
    lines.push({ text, tone: dim ? 'dim' : step.status === 'failed' ? 'error' : 'plain' })
  }
  if (job.status === 'running') lines.push({ text: '…', tone: 'dim' })
  if (job.status === 'queued') lines.push({ text: 'queued', tone: 'dim' })
  if (job.output.trim() !== '') {
    for (const line of job.output.replace(/\r\n/g, '\n').split('\n')) {
      lines.push({ text: line, tone: 'plain' })
    }
  }
  if (job.status === 'failed') {
    lines.push({ text: `failed: ${job.error !== '' ? job.error : 'failed'}`, tone: 'error' })
  }
  if (job.files > 0 || job.added > 0 || job.removed > 0) {
    const noun = job.files === 1 ? '1 file' : `${String(job.files)} files`
    lines.push({
      text: `Edited ${noun} +${String(job.added)} −${String(job.removed)}`,
      tone: 'plain'
    })
  }
  return lines
}

function fileLines(row: WorktreeItem | undefined): LineModel[] {
  if (row === undefined) return [{ text: 'No folder selected', tone: 'plain' }]
  if (row.project) {
    return [
      { text: 'The project folder.', tone: 'plain' },
      { text: 'Pick a worktree to see its changes, w to make one, n to start a job.', tone: 'dim' }
    ]
  }
  if (row.files.length === 0) return [{ text: 'no changes', tone: 'plain' }]
  return row.files.map((file) => {
    if (file.added === null || file.removed === null)
      return { text: file.path, tone: 'plain' as const }
    return {
      text: `${file.path}  +${String(file.added)} −${String(file.removed)}`,
      tone: 'plain' as const
    }
  })
}

function diffLines(text: string): LineModel[] {
  if (text.trim() === '') return [{ text: 'No diff', tone: 'plain' }]
  return text
    .replace(/\r\n/g, '\n')
    .split('\n')
    .map((line) => ({ text: line, tone: diffTone(line) }))
}

function diffTone(line: string): LineTone {
  if (line.startsWith('+++') || line.startsWith('---')) return 'plain'
  if (line.startsWith('+')) return 'add'
  if (line.startsWith('-')) return 'remove'
  if (line.startsWith('@@')) return 'accent'
  return 'plain'
}

function routineLines(state: TuiState): LineModel[] {
  if (state.routines.length === 0) return [{ text: 'No routines.', tone: 'plain' }]
  return state.routines.map((routine, index) => ({
    text: `${index === state.routineIndex ? '▸ ' : '  '}${routine.name}  ${routine.type}  ${routine.worker}  ${routine.access}`,
    tone: 'plain' as const
  }))
}

function helpLines(): LineModel[] {
  const rows = [
    'n or i        new job',
    'Tab           switch pane',
    'Up Down j k   move',
    'Enter         open',
    'd             diff',
    'Esc           back',
    'm             merge',
    'x             discard',
    'w             new worktree',
    'c             claude in this folder',
    'u             cursor agent in this folder',
    's             stop the job',
    'r             routines',
    'g G           top and bottom',
    'PgUp PgDn     scroll',
    'Ctrl+T        job type',
    'Ctrl+W        worker',
    'Ctrl+A        access',
    'q or Ctrl+C   quit',
    '?             this help'
  ]
  return rows.map((text) => ({ text, tone: 'plain' as const }))
}

function confirmText(state: TuiState): string {
  if (state.mode === 'quit') return 'Jobs are running. Quit and stop them? y/n'
  if (state.mode === 'discard') {
    const row = selectedWorktree(state)
    return `Discard ${row?.label ?? 'worktree'}? y/n`
  }
  return ''
}

function worktreeText(item: WorktreeItem): string {
  if (item.project) return `${item.label}  ${item.branch ?? 'no branch'}`
  if (item.files.length === 0 && item.added === 0 && item.removed === 0) {
    return `${item.label}  no changes`
  }
  return `${item.label}  +${String(item.added)} −${String(item.removed)}`
}

function jobText(job: JobItem): string {
  const status = job.status.padEnd(7, ' ')
  const worker = (job.provider ?? '-').padEnd(6, ' ')
  const prompt = job.prompt.replace(/[\r\n]+/g, ' ')
  return `${status} ${worker} ${prompt}`
}

function selectedWorktree(state: TuiState): WorktreeItem | undefined {
  return state.worktrees.find((item) => item.id === state.worktreeId) ?? state.worktrees[0]
}

function selectedJob(state: TuiState): JobItem | undefined {
  return state.jobs.find((item) => item.id === state.jobId)
}

function windowItems<T>(
  items: T[],
  selected: number,
  slots: number
): { item: T; selected: boolean }[] {
  if (items.length === 0) return []
  const at = selected < 0 ? 0 : selected
  let start = 0
  if (items.length > slots) {
    start = Math.max(0, at - Math.floor(slots / 2))
    if (start + slots > items.length) start = items.length - slots
  }
  const end = Math.min(items.length, start + slots)
  const out: { item: T; selected: boolean }[] = []
  for (let index = start; index < end; index++) {
    const item = items[index]
    if (item === undefined) continue
    out.push({ item, selected: index === at })
  }
  return out
}

function indexOf<T>(items: T[], match: (item: T) => boolean): number {
  const index = items.findIndex(match)
  return index < 0 ? 0 : index
}

function mark(selected: boolean): string {
  return selected ? '▸ ' : '  '
}

function fit(text: string, width: number): string {
  if (width <= 0) return ''
  if (text.length <= width) return text
  if (width === 1) return '…'
  return `${text.slice(0, width - 1)}…`
}
