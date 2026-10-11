import type { StepView } from '../format'
import { detailBody, detailPage } from './view'

export { footerText, isNarrow, leftColumn } from './view'

export const JOB_TYPES = ['planning', 'debugging', 'review', 'refactor', 'boilerplate'] as const
export const WORKERS = ['auto', 'claude', 'cursor'] as const
export const ACCESSES = ['read', 'edit', 'full'] as const

export type JobTypeName = (typeof JOB_TYPES)[number]
export type WorkerChoice = (typeof WORKERS)[number]
export type AccessChoice = (typeof ACCESSES)[number]
export type Focus = 'worktrees' | 'jobs' | 'detail'
export type Pane = 'job' | 'worktree' | 'diff' | 'routines' | 'help'
export type PromptMode = 'idle' | 'job' | 'name' | 'discard' | 'quit'

export interface FileRow {
  path: string
  added: number | null
  removed: number | null
}

export interface WorktreeItem {
  id: string
  project: boolean
  label: string
  branch: string | null
  added: number
  removed: number
  files: FileRow[]
  path: string
}

export interface JobItem {
  id: string
  status: 'queued' | 'running' | 'done' | 'failed'
  provider: string | null
  type: string
  access: string
  model: string
  reason: string
  prompt: string
  output: string
  error: string
  steps: StepView[]
  createdAt: number
  startedHere: boolean
  change: string | null
  files: number
  added: number
  removed: number
}

export interface PlanItem {
  id: string
  text: string
}

export interface RoutineItem {
  id: string
  name: string
  type: string
  worker: string
  access: string
}

export interface TuiState {
  projectName: string
  projectPath: string
  branch: string | null
  plans: PlanItem[]
  worktrees: WorktreeItem[]
  jobs: JobItem[]
  routines: RoutineItem[]
  focus: Focus
  pane: Pane
  mode: PromptMode
  worktreeId: string
  jobId: string | null
  routineIndex: number
  draft: string
  jobType: JobTypeName
  worker: WorkerChoice
  access: AccessChoice
  diffId: string
  diffText: string
  scroll: number
  follow: boolean
  error: string | null
  columns: number
  rows: number
  now: number
  color: boolean
}

export interface KeyPress {
  input: string
  ctrl: boolean
  shift: boolean
  return: boolean
  escape: boolean
  tab: boolean
  backspace: boolean
  up: boolean
  down: boolean
  pageUp: boolean
  pageDown: boolean
}

export type Effect =
  | {
      type: 'submit'
      jobType: JobTypeName
      prompt: string
      worker: WorkerChoice
      access: AccessChoice
    }
  | { type: 'cancel'; id: string }
  | { type: 'merge'; id: string }
  | { type: 'discard'; id: string }
  | { type: 'create-worktree'; name: string }
  | { type: 'open-agent'; provider: 'claude' | 'cursor'; cwd: string }
  | { type: 'load-diff'; id: string }
  | { type: 'load-changes' }
  | { type: 'run-routine'; id: string }
  | { type: 'quit'; stop: string[] }

export type Action =
  | { type: 'key'; key: KeyPress; draft?: string }
  | { type: 'draft'; text: string }
  | { type: 'resize'; columns: number; rows: number }
  | { type: 'tick'; now: number }
  | { type: 'plans'; plans: PlanItem[] }
  | { type: 'error'; message: string }
  | {
      type: 'worktrees'
      project: WorktreeItem
      items: WorktreeItem[]
      branch: string | null
    }
  | { type: 'jobs'; jobs: JobItem[] }
  | { type: 'job'; job: JobItem }
  | { type: 'submitted'; job: JobItem }
  | { type: 'routines'; routines: RoutineItem[] }
  | { type: 'diff'; id: string; text: string }
  | { type: 'select-worktree'; id: string }

export interface ReduceResult {
  state: TuiState
  effects: Effect[]
}

const PROMPT_MAX = 8_000

export function initialState(input: {
  projectName: string
  columns: number
  rows: number
  color: boolean
  now: number
}): TuiState {
  return {
    projectName: input.projectName,
    projectPath: '',
    branch: null,
    plans: [],
    worktrees: [projectRow(input.projectName, '', null)],
    jobs: [],
    routines: [],
    focus: 'worktrees',
    pane: 'worktree',
    mode: 'idle',
    worktreeId: '',
    jobId: null,
    routineIndex: 0,
    draft: '',
    jobType: 'planning',
    worker: 'auto',
    access: 'read',
    diffId: '',
    diffText: '',
    scroll: 0,
    follow: true,
    error: null,
    columns: input.columns,
    rows: input.rows,
    now: input.now,
    color: input.color
  }
}

export function keyPress(partial: Partial<KeyPress>): KeyPress {
  return {
    input: '',
    ctrl: false,
    shift: false,
    return: false,
    escape: false,
    tab: false,
    backspace: false,
    up: false,
    down: false,
    pageUp: false,
    pageDown: false,
    ...partial
  }
}

export function toKey(
  input: string,
  key: {
    ctrl?: boolean
    shift?: boolean
    return?: boolean
    escape?: boolean
    tab?: boolean
    backspace?: boolean
    upArrow?: boolean
    downArrow?: boolean
    pageUp?: boolean
    pageDown?: boolean
  }
): KeyPress {
  return keyPress({
    input,
    ctrl: key.ctrl === true,
    shift: key.shift === true,
    return: key.return === true,
    escape: key.escape === true,
    tab: key.tab === true,
    backspace: key.backspace === true,
    up: key.upArrow === true,
    down: key.downArrow === true,
    pageUp: key.pageUp === true,
    pageDown: key.pageDown === true
  })
}

export function reduce(state: TuiState, action: Action): ReduceResult {
  switch (action.type) {
    case 'key':
      return handleKey(clearError(state, action.draft), action.key)
    case 'draft':
      return done({ ...state, draft: action.text.slice(0, PROMPT_MAX) }, [])
    case 'resize':
      if (state.columns === action.columns && state.rows === action.rows) return done(state, [])
      return done({ ...state, columns: action.columns, rows: action.rows }, [])
    case 'tick':
      return done({ ...state, now: action.now }, [])
    case 'plans':
      return done({ ...state, plans: action.plans }, [])
    case 'error':
      return done({ ...state, error: action.message }, [])
    case 'worktrees':
      return applyWorktrees(state, action)
    case 'jobs':
      return applyJobs(state, action.jobs)
    case 'job':
      return applyJob(state, action.job, false)
    case 'submitted':
      return applyJob(state, action.job, true)
    case 'routines':
      return done(
        {
          ...state,
          routines: action.routines,
          routineIndex: Math.min(state.routineIndex, Math.max(0, action.routines.length - 1))
        },
        []
      )
    case 'diff':
      if (state.pane !== 'diff' || state.diffId !== action.id) return done(state, [])
      return done({ ...state, diffText: action.text }, [])
    case 'select-worktree':
      return done({ ...state, worktreeId: action.id, pane: 'worktree', focus: 'worktrees' }, [])
    default:
      return done(state, [])
  }
}

export function selectedWorktree(state: TuiState): WorktreeItem | undefined {
  return state.worktrees.find((item) => item.id === state.worktreeId) ?? state.worktrees[0]
}

export function selectedJob(state: TuiState): JobItem | undefined {
  return state.jobs.find((item) => item.id === state.jobId)
}

function projectRow(label: string, path: string, branch: string | null): WorktreeItem {
  return {
    id: '',
    project: true,
    label,
    branch,
    added: 0,
    removed: 0,
    files: [],
    path
  }
}

function clearError(state: TuiState, draft: string | undefined): TuiState {
  let next = state.error !== null ? { ...state, error: null } : state
  if (draft !== undefined) next = { ...next, draft }
  return next
}

function done(state: TuiState, effects: Effect[]): ReduceResult {
  return { state: clampScroll(state), effects }
}

function clampScroll(state: TuiState): TuiState {
  const max = Math.max(0, detailBody(state).length - detailPage(state))
  const scroll = state.follow ? max : Math.min(state.scroll, max)
  if (scroll === state.scroll) return state
  return { ...state, scroll }
}

function handleKey(state: TuiState, key: KeyPress): ReduceResult {
  if (state.mode === 'discard' || state.mode === 'quit') return confirmKey(state, key)
  if (state.mode === 'job' || state.mode === 'name') return promptKey(state, key)
  return browseKey(state, key)
}

function confirmKey(state: TuiState, key: KeyPress): ReduceResult {
  if (key.input === 'y') {
    if (state.mode === 'discard') {
      const row = selectedWorktree(state)
      if (row === undefined || row.project) return done({ ...state, mode: 'idle' }, [])
      return done({ ...state, mode: 'idle' }, [{ type: 'discard', id: row.id }])
    }
    return done(state, [{ type: 'quit', stop: runningHere(state) }])
  }
  if (key.input === 'n' || key.escape) return done({ ...state, mode: 'idle' }, [])
  if (key.ctrl && key.input === 'c')
    return done(state, [{ type: 'quit', stop: runningHere(state) }])
  return done(state, [])
}

function promptKey(state: TuiState, key: KeyPress): ReduceResult {
  if (key.escape) return done({ ...state, mode: 'idle' }, [])
  if (key.ctrl && key.input === 'c') return requestQuit(state)
  if (state.mode === 'job' && key.ctrl && key.input === 't') {
    return done({ ...state, jobType: cycle(JOB_TYPES, state.jobType) }, [])
  }
  if (state.mode === 'job' && key.ctrl && key.input === 'w') {
    return done({ ...state, worker: cycle(WORKERS, state.worker) }, [])
  }
  if (state.mode === 'job' && key.ctrl && key.input === 'a') {
    return done({ ...state, access: cycle(ACCESSES, state.access) }, [])
  }
  if (key.return) return state.mode === 'job' ? submitDraft(state) : nameDraft(state)
  if (key.backspace) return done({ ...state, draft: state.draft.slice(0, -1) }, [])
  if (key.ctrl || key.tab || key.up || key.down || key.pageUp || key.pageDown)
    return done(state, [])
  if (key.input === '') return done(state, [])
  return done({ ...state, draft: (state.draft + key.input).slice(0, PROMPT_MAX) }, [])
}

function browseKey(state: TuiState, key: KeyPress): ReduceResult {
  if (key.ctrl && key.input === 'c') return requestQuit(state)
  if (key.input === 'q') return requestQuit(state)
  if (key.escape && state.pane === 'diff') {
    return done({ ...state, pane: 'worktree', follow: false, scroll: 0 }, [])
  }
  if (key.escape && (state.pane === 'help' || state.pane === 'routines')) {
    return done({ ...state, pane: state.focus === 'worktrees' ? 'worktree' : 'job' }, [])
  }
  if (state.pane === 'help' && key.input === '?') {
    return done({ ...state, pane: state.focus === 'worktrees' ? 'worktree' : 'job' }, [])
  }
  if (key.tab) return tabFocus(state)
  if (key.up || key.input === 'k') return move(state, -1)
  if (key.down || key.input === 'j') return move(state, 1)
  if (key.pageUp) return scrollBy(state, -detailPage(state))
  if (key.pageDown) return scrollBy(state, detailPage(state))
  if (key.input === 'g' && !key.shift) return jump(state, false)
  if (key.input === 'G' || (key.input === 'g' && key.shift)) return jump(state, true)
  if (key.input === 'n' || key.input === 'i') return done({ ...state, mode: 'job' }, [])
  if (key.input === '?') {
    return done({ ...state, pane: 'help', focus: 'detail', follow: false, scroll: 0 }, [])
  }
  if (key.input === 'r') {
    return done({ ...state, pane: 'routines', focus: 'detail', follow: false, scroll: 0 }, [])
  }
  if (key.return) return openSelection(state)
  if (key.input === 'd') return openDiff(state)
  if (key.input === 'm') return mergeSelected(state)
  if (key.input === 'x') return discardSelected(state)
  if (key.input === 'w') return done({ ...state, mode: 'name', draft: '' }, [])
  if (key.input === 's') return stopSelected(state)
  if (key.input === 'c' || key.input === 'u')
    return openAgent(state, key.input === 'c' ? 'claude' : 'cursor')
  return done(state, [])
}

function tabFocus(state: TuiState): ReduceResult {
  const order: Focus[] = ['worktrees', 'jobs', 'detail']
  const index = order.indexOf(state.focus)
  const focus = order[(index + 1) % order.length] ?? 'worktrees'
  let pane = state.pane
  if (pane === 'job' || pane === 'worktree') {
    if (focus === 'worktrees') pane = 'worktree'
    else if (focus === 'jobs') pane = 'job'
  }
  return done({ ...state, focus, pane }, [])
}

function move(state: TuiState, delta: number): ReduceResult {
  if (state.pane === 'routines') {
    const max = Math.max(0, state.routines.length - 1)
    return done({ ...state, routineIndex: clamp(state.routineIndex + delta, 0, max) }, [])
  }
  if (state.pane === 'diff' || state.pane === 'help' || state.focus === 'detail') {
    return scrollBy(state, delta)
  }
  if (state.focus === 'worktrees') {
    const max = Math.max(0, state.worktrees.length - 1)
    const index = clamp(worktreeIndex(state) + delta, 0, max)
    const row = state.worktrees[index]
    return done({ ...state, worktreeId: row?.id ?? '' }, [])
  }
  const max = Math.max(0, state.jobs.length - 1)
  const index = clamp(jobIndex(state) + delta, 0, max)
  const row = state.jobs[index]
  if (row === undefined) return done(state, [])
  return done({ ...state, jobId: row.id, pane: 'job', follow: true }, [])
}

function scrollBy(state: TuiState, delta: number): ReduceResult {
  if (state.pane === 'routines') return move(state, delta)
  const max = Math.max(0, detailBody(state).length - detailPage(state))
  const scroll = clamp(state.scroll + delta, 0, max)
  const follow = delta > 0 && scroll >= max
  return done({ ...state, scroll, follow: delta < 0 ? false : follow }, [])
}

function jump(state: TuiState, end: boolean): ReduceResult {
  if (state.pane === 'routines') {
    const routineIndex = end ? Math.max(0, state.routines.length - 1) : 0
    return done({ ...state, routineIndex }, [])
  }
  if (end) return done({ ...state, follow: true }, [])
  return done({ ...state, scroll: 0, follow: false }, [])
}

function openSelection(state: TuiState): ReduceResult {
  if (state.pane === 'routines') {
    const routine = state.routines[state.routineIndex]
    if (routine === undefined) return done(state, [])
    return done({ ...state, pane: 'job', focus: 'detail', follow: true }, [
      { type: 'run-routine', id: routine.id }
    ])
  }
  if (state.pane === 'help') {
    return done({ ...state, pane: state.focus === 'worktrees' ? 'worktree' : 'job' }, [])
  }
  if (state.focus === 'worktrees' || state.pane === 'worktree') return openDiff(state)
  return done({ ...state, focus: 'detail', pane: 'job' }, [])
}

function openDiff(state: TuiState): ReduceResult {
  const row = selectedWorktree(state)
  if (row === undefined || row.project) {
    return done({ ...state, error: 'Select a worktree to see its diff.' }, [])
  }
  return done(
    {
      ...state,
      pane: 'diff',
      diffId: row.id,
      diffText: '',
      follow: false,
      scroll: 0,
      focus: 'detail'
    },
    [{ type: 'load-diff', id: row.id }]
  )
}

function mergeSelected(state: TuiState): ReduceResult {
  const row = selectedWorktree(state)
  if (row === undefined || row.project) {
    return done({ ...state, error: 'Select a worktree to merge.' }, [])
  }
  return done(state, [{ type: 'merge', id: row.id }])
}

function discardSelected(state: TuiState): ReduceResult {
  const row = selectedWorktree(state)
  if (row === undefined || row.project) {
    return done({ ...state, error: 'Select a worktree to discard.' }, [])
  }
  return done({ ...state, mode: 'discard' }, [])
}

function stopSelected(state: TuiState): ReduceResult {
  const job = selectedJob(state)
  if (job === undefined || (job.status !== 'running' && job.status !== 'queued')) {
    return done({ ...state, error: 'That job is not running.' }, [])
  }
  return done(state, [{ type: 'cancel', id: job.id }])
}

function openAgent(state: TuiState, provider: 'claude' | 'cursor'): ReduceResult {
  const row = selectedWorktree(state)
  const cwd = row?.path !== undefined && row.path !== '' ? row.path : state.projectPath
  if (cwd === '') return done({ ...state, error: 'Select a folder first.' }, [])
  return done(state, [{ type: 'open-agent', provider, cwd }])
}

function submitDraft(state: TuiState): ReduceResult {
  const prompt = state.draft.trim()
  if (prompt === '') return done(state, [])
  return done({ ...state, mode: 'idle', draft: '', pane: 'job', focus: 'detail', follow: true }, [
    {
      type: 'submit',
      jobType: state.jobType,
      prompt,
      worker: state.worker,
      access: state.access
    }
  ])
}

function nameDraft(state: TuiState): ReduceResult {
  if (/[\r\n]/.test(state.draft) || state.draft.trim().length > 40) {
    return done({ ...state, error: 'Name must be 1 to 40 characters on one line.' }, [])
  }
  return done({ ...state, mode: 'idle', draft: '' }, [
    { type: 'create-worktree', name: state.draft.trim() }
  ])
}

function requestQuit(state: TuiState): ReduceResult {
  const stop = runningHere(state)
  if (stop.length > 0) return done({ ...state, mode: 'quit' }, [])
  return done(state, [{ type: 'quit', stop: [] }])
}

function runningHere(state: TuiState): string[] {
  return state.jobs
    .filter((job) => job.startedHere && (job.status === 'running' || job.status === 'queued'))
    .map((job) => job.id)
}

function applyWorktrees(
  state: TuiState,
  action: Extract<Action, { type: 'worktrees' }>
): ReduceResult {
  const worktrees = [action.project, ...action.items]
  const worktreeId = worktrees.some((item) => item.id === state.worktreeId)
    ? state.worktreeId
    : (worktrees[0]?.id ?? '')
  const jobs = state.jobs.map((job) => applyTreeCounts(job, action.items))
  return done(
    {
      ...state,
      worktrees,
      worktreeId,
      branch: action.branch,
      projectPath: action.project.path,
      projectName: action.project.label,
      jobs
    },
    []
  )
}

function applyJobs(state: TuiState, incoming: JobItem[]): ReduceResult {
  const jobs = incoming
    .map((job) => {
      const prev = state.jobs.find((item) => item.id === job.id)
      if (prev === undefined) return job
      return {
        ...job,
        startedHere: prev.startedHere || job.startedHere,
        createdAt: prev.createdAt
      }
    })
    .sort(byNewest)
  const jobId =
    state.jobId !== null && jobs.some((job) => job.id === state.jobId)
      ? state.jobId
      : (jobs[0]?.id ?? null)
  return done({ ...state, jobs, jobId }, [])
}

function applyJob(state: TuiState, job: JobItem, select: boolean): ReduceResult {
  const prev = state.jobs.find((item) => item.id === job.id)
  const merged = mergeJob(prev, job)
  const jobs = (
    prev === undefined
      ? [merged, ...state.jobs]
      : state.jobs.map((item) => (item.id === job.id ? merged : item))
  ).sort(byNewest)
  const ended =
    prev !== undefined &&
    prev.status !== job.status &&
    (job.status === 'done' || job.status === 'failed')
  const effects: Effect[] = ended ? [{ type: 'load-changes' }] : []
  if (!select) return done({ ...state, jobs }, effects)
  return done(
    { ...state, jobs, jobId: job.id, pane: 'job', focus: 'detail', follow: true, mode: 'idle' },
    effects
  )
}

function mergeJob(prev: JobItem | undefined, job: JobItem): JobItem {
  if (prev === undefined) return job
  const keepCounts = job.files === 0 && job.added === 0 && job.removed === 0
  return {
    ...job,
    createdAt: prev.createdAt,
    startedHere: prev.startedHere || job.startedHere,
    files: keepCounts ? prev.files : job.files,
    added: keepCounts ? prev.added : job.added,
    removed: keepCounts ? prev.removed : job.removed
  }
}

function applyTreeCounts(job: JobItem, items: WorktreeItem[]): JobItem {
  if (job.change === null) return job
  if (job.status === 'running' && (job.added > 0 || job.removed > 0)) return job
  const match = items.find((item) => item.id === job.change)
  if (match === undefined) return job
  return { ...job, files: match.files.length, added: match.added, removed: match.removed }
}

function byNewest(a: JobItem, b: JobItem): number {
  if (a.createdAt !== b.createdAt) return b.createdAt - a.createdAt
  return a.id < b.id ? -1 : a.id > b.id ? 1 : 0
}

function worktreeIndex(state: TuiState): number {
  const index = state.worktrees.findIndex((item) => item.id === state.worktreeId)
  return index < 0 ? 0 : index
}

function jobIndex(state: TuiState): number {
  const index = state.jobs.findIndex((item) => item.id === state.jobId)
  return index < 0 ? 0 : index
}

function cycle<T>(list: readonly T[], current: T): T {
  const index = list.indexOf(current)
  return list[(index + 1) % list.length] ?? current
}

function clamp(value: number, min: number, max: number): number {
  if (value < min) return min
  if (value > max) return max
  return value
}
