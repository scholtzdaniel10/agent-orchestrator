import { expect, test } from 'vitest'
import { detailBody, detailPage } from './view'
import {
  footerText,
  initialState,
  isNarrow,
  keyPress,
  reduce,
  type JobItem,
  type TuiState,
  type WorktreeItem
} from './state'

function job(partial: Partial<JobItem> & Pick<JobItem, 'id'>): JobItem {
  return {
    status: 'done',
    provider: 'cursor',
    type: 'planning',
    access: 'read',
    model: '',
    reason: '',
    prompt: 'do the thing',
    output: '',
    error: '',
    steps: [],
    createdAt: 1,
    startedHere: false,
    change: null,
    files: 0,
    added: 0,
    removed: 0,
    ...partial
  }
}

function tree(partial: Partial<WorktreeItem> & Pick<WorktreeItem, 'id' | 'label'>): WorktreeItem {
  return {
    project: false,
    branch: `orch/${partial.id}`,
    added: 0,
    removed: 0,
    files: [],
    path: `/wt/${partial.id}`,
    ...partial
  }
}

function base(): TuiState {
  return {
    ...initialState({
      projectName: 'agent-orchestrator',
      columns: 120,
      rows: 30,
      color: true,
      now: 10_000
    }),
    projectPath: '/proj',
    branch: 'v1',
    worktrees: [
      tree({ id: '', label: 'agent-orchestrator', project: true, branch: 'v1', path: '/proj' }),
      tree({
        id: 'login123',
        label: 'login fix',
        added: 10,
        removed: 3,
        files: [{ path: 'a.ts', added: 10, removed: 3 }]
      }),
      tree({ id: '3687475d', label: '3687475d' })
    ],
    worktreeId: 'login123',
    jobs: [
      job({
        id: 'job-run',
        status: 'running',
        prompt: 'Add a button',
        createdAt: 9_000,
        startedHere: true
      }),
      job({
        id: 'job-done',
        status: 'done',
        provider: 'claude',
        prompt: 'Review the diff',
        createdAt: 8_000
      }),
      job({
        id: 'job-bad',
        status: 'failed',
        prompt: 'Fix the crash',
        createdAt: 7_000,
        error: 'tests failed'
      })
    ],
    jobId: 'job-run',
    focus: 'jobs',
    pane: 'job'
  }
}

test('j and k move the focused list and stop at the ends', () => {
  const start = {
    ...base(),
    focus: 'worktrees' as const,
    pane: 'worktree' as const,
    worktreeId: 'login123'
  }
  const down = reduce(start, { type: 'key', key: keyPress({ input: 'j' }) }).state
  expect(down.worktreeId).toBe('3687475d')
  const stuck = reduce(down, { type: 'key', key: keyPress({ down: true }) }).state
  expect(stuck.worktreeId).toBe('3687475d')
  const up = reduce(stuck, { type: 'key', key: keyPress({ input: 'k' }) }).state
  expect(up.worktreeId).toBe('login123')
  const top = reduce({ ...up, worktreeId: '' }, { type: 'key', key: keyPress({ up: true }) }).state
  expect(top.worktreeId).toBe('')
})

test('tab cycles focus between worktrees, jobs, and the detail', () => {
  let state: TuiState = { ...base(), focus: 'worktrees', pane: 'worktree' }
  state = reduce(state, { type: 'key', key: keyPress({ tab: true }) }).state
  expect(state.focus).toBe('jobs')
  expect(state.pane).toBe('job')
  state = reduce(state, { type: 'key', key: keyPress({ tab: true }) }).state
  expect(state.focus).toBe('detail')
  expect(state.pane).toBe('job')
  state = reduce(state, { type: 'key', key: keyPress({ tab: true }) }).state
  expect(state.focus).toBe('worktrees')
  expect(state.pane).toBe('worktree')
})

test('the prompt edits, cycles type worker and access, and ignores an empty submit', () => {
  let state: TuiState = reduce(base(), { type: 'key', key: keyPress({ input: 'n' }) }).state
  expect(state.mode).toBe('job')
  state = reduce(state, { type: 'key', key: keyPress({ input: 'hi' }) }).state
  state = reduce(state, { type: 'key', key: keyPress({ backspace: true }) }).state
  expect(state.draft).toBe('h')
  state = reduce(state, { type: 'key', key: keyPress({ ctrl: true, input: 't' }) }).state
  expect(state.jobType).toBe('debugging')
  state = reduce(state, { type: 'key', key: keyPress({ ctrl: true, input: 'w' }) }).state
  expect(state.worker).toBe('claude')
  state = reduce(state, { type: 'key', key: keyPress({ ctrl: true, input: 'w' }) }).state
  expect(state.worker).toBe('cursor')
  state = reduce(state, { type: 'key', key: keyPress({ ctrl: true, input: 'w' }) }).state
  expect(state.worker).toBe('auto')
  state = reduce(state, { type: 'key', key: keyPress({ ctrl: true, input: 'a' }) }).state
  expect(state.access).toBe('edit')
  state = reduce(state, { type: 'key', key: keyPress({ ctrl: true, input: 'a' }) }).state
  expect(state.access).toBe('full')
  state = reduce(state, { type: 'key', key: keyPress({ ctrl: true, input: 'a' }) }).state
  expect(state.access).toBe('read')
  const empty = reduce({ ...state, draft: '   ' }, { type: 'key', key: keyPress({ return: true }) })
  expect(empty.effects).toEqual([])
  expect(empty.state.mode).toBe('job')
  const left = reduce(state, { type: 'key', key: keyPress({ escape: true }) }).state
  expect(left.mode).toBe('idle')
  expect(left.draft).toBe('h')
})

test('enter submits the draft and the new job becomes the selection', () => {
  let state: TuiState = reduce(base(), { type: 'key', key: keyPress({ input: 'i' }) }).state
  state = { ...state, draft: 'ship the ui', jobType: 'refactor', worker: 'cursor', access: 'edit' }
  const submitted = reduce(state, { type: 'key', key: keyPress({ return: true }) })
  expect(submitted.effects).toEqual([
    { type: 'submit', jobType: 'refactor', prompt: 'ship the ui', worker: 'cursor', access: 'edit' }
  ])
  expect(submitted.state.mode).toBe('idle')
  expect(submitted.state.draft).toBe('')
  const arrived = reduce(submitted.state, {
    type: 'submitted',
    job: job({
      id: 'job-new',
      status: 'queued',
      prompt: 'ship the ui',
      createdAt: 11_000,
      startedHere: true
    })
  }).state
  expect(arrived.jobId).toBe('job-new')
  expect(arrived.pane).toBe('job')
  expect(arrived.focus).toBe('detail')
  expect(arrived.jobs[0]?.id).toBe('job-new')
})

test('follow stays at the bottom until the reader scrolls up', () => {
  const steps = Array.from({ length: 8 }, (_, index) => ({
    id: `s${String(index)}`,
    tool: 'read' as const,
    title: `src/file-${String(index)}.ts`,
    status: 'done' as const
  }))
  const running = job({
    id: 'job-run',
    status: 'running',
    steps,
    createdAt: 9_000,
    startedHere: true
  })
  let state: TuiState = reduce(
    { ...base(), rows: 10, jobs: [running], jobId: 'job-run', follow: true, scroll: 0 },
    { type: 'tick', now: 10_000 }
  ).state
  const max = Math.max(0, detailBody(state).length - detailPage(state))
  expect(max).toBeGreaterThan(0)
  expect(state.follow).toBe(true)
  expect(state.scroll).toBe(max)

  state = reduce(state, { type: 'key', key: keyPress({ pageUp: true }) }).state
  expect(state.follow).toBe(false)
  const held = state.scroll
  expect(held).toBeLessThan(max)

  const more = {
    ...running,
    steps: [
      ...steps,
      { id: 's-extra', tool: 'shell' as const, title: 'pnpm test', status: 'done' as const }
    ]
  }
  state = reduce(state, { type: 'job', job: more }).state
  expect(state.follow).toBe(false)
  expect(state.scroll).toBe(held)

  state = reduce(state, { type: 'key', key: keyPress({ input: 'G' }) }).state
  expect(state.follow).toBe(true)
  expect(state.scroll).toBe(Math.max(0, detailBody(state).length - detailPage(state)))
})

test('discard and quit wait for y, and n leaves them alone', () => {
  const discard = reduce(base(), { type: 'key', key: keyPress({ input: 'x' }) })
  expect(discard.state.mode).toBe('discard')
  expect(discard.effects).toEqual([])
  const kept = reduce(discard.state, { type: 'key', key: keyPress({ input: 'n' }) })
  expect(kept.state.mode).toBe('idle')
  expect(kept.effects).toEqual([])
  const armed = reduce(base(), { type: 'key', key: keyPress({ input: 'x' }) }).state
  const gone = reduce(armed, { type: 'key', key: keyPress({ input: 'y' }) })
  expect(gone.effects).toEqual([{ type: 'discard', id: 'login123' }])

  const quit = reduce(base(), { type: 'key', key: keyPress({ input: 'q' }) })
  expect(quit.state.mode).toBe('quit')
  expect(quit.effects).toEqual([])
  const stopped = reduce(quit.state, { type: 'key', key: keyPress({ input: 'y' }) })
  expect(stopped.effects).toEqual([{ type: 'quit', stop: ['job-run'] }])

  const idle = base()
  idle.jobs = idle.jobs.map((item) => ({ ...item, startedHere: false }))
  const free = reduce(idle, { type: 'key', key: keyPress({ input: 'q' }) })
  expect(free.effects).toEqual([{ type: 'quit', stop: [] }])
})

test('narrow widths use the short footer', () => {
  expect(isNarrow(80)).toBe(true)
  expect(isNarrow(99)).toBe(true)
  expect(isNarrow(100)).toBe(false)
  expect(footerText(80)).toBe('? help  q quit')
  expect(footerText(120)).toContain('n new job')
  const narrow = reduce(base(), { type: 'resize', columns: 80, rows: 24 }).state
  expect(footerText(narrow.columns)).toBe('? help  q quit')
  expect(narrow.rows).toBe(24)
})

test('an error line clears on the next key, which still counts', () => {
  const state = { ...base(), error: 'does not apply cleanly', focus: 'jobs' as const }
  const next = reduce(state, { type: 'key', key: keyPress({ input: 'j' }) }).state
  expect(next.error).toBeNull()
  expect(next.jobId).toBe('job-done')
})

test('merge, diff, and stop describe what went wrong', () => {
  const project = { ...base(), worktreeId: '' }
  expect(reduce(project, { type: 'key', key: keyPress({ input: 'm' }) }).state.error).toBe(
    'Select a worktree to merge.'
  )
  expect(reduce(project, { type: 'key', key: keyPress({ input: 'd' }) }).state.error).toBe(
    'Select a worktree to see its diff.'
  )
  const stopped = reduce(
    { ...base(), jobId: 'job-done' },
    { type: 'key', key: keyPress({ input: 's' }) }
  )
  expect(stopped.effects).toEqual([])
  expect(stopped.state.error).toBe('That job is not running.')
  const diff = reduce(base(), { type: 'key', key: keyPress({ input: 'd' }) })
  expect(diff.state.pane).toBe('diff')
  expect(diff.effects).toEqual([{ type: 'load-diff', id: 'login123' }])
})
