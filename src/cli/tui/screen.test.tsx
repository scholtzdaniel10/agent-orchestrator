import { useState, type JSX } from 'react'
import { render } from 'ink-testing-library'
import { expect, test } from 'vitest'
import { App } from './app'
import { Screen } from './screen'
import {
  initialState,
  reduce,
  type Action,
  type JobItem,
  type TuiState,
  type WorktreeItem
} from './state'

function job(partial: Partial<JobItem> & Pick<JobItem, 'id'>): JobItem {
  return {
    status: 'done',
    provider: 'cursor',
    type: 'boilerplate',
    access: 'full',
    model: 'Grok 4.6',
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
    branch: null,
    added: 0,
    removed: 0,
    files: [],
    path: `/wt/${partial.id}`,
    ...partial
  }
}

function screen(partial: Partial<TuiState> = {}): TuiState {
  return {
    ...initialState({
      projectName: 'agent-orchestrator',
      columns: 120,
      rows: 30,
      color: true,
      now: 20_000
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
        files: [{ path: 'src/core/github.ts', added: 10, removed: 3 }]
      }),
      tree({ id: '3687475d', label: '3687475d' })
    ],
    worktreeId: 'login123',
    jobs: [
      job({ id: 'aaaa1111', status: 'running', prompt: 'Add a button', createdAt: 12_000 }),
      job({
        id: 'bbbb2222',
        status: 'done',
        provider: 'claude',
        prompt: 'Review the diff',
        createdAt: 8_000
      }),
      job({ id: 'cccc3333', status: 'failed', prompt: 'Fix the crash', createdAt: 4_000 })
    ],
    jobId: 'aaaa1111',
    focus: 'jobs',
    pane: 'job',
    ...partial
  }
}

function frame(state: TuiState): string {
  const view = render(<Screen state={state} onDraft={() => {}} onSubmit={() => {}} />)
  const text = view.lastFrame() ?? ''
  view.unmount()
  return text
}

test('the main screen at 120x30 lists two worktrees and three jobs', () => {
  const text = frame(screen())
  expect(text).toContain('WORKTREES  2')
  expect(text).toContain('JOBS  1 running')
  expect(text).toContain('login fix')
  expect(text).toContain('3687475d')
  expect(text).toContain('no changes')
  expect(text).toContain('Add a button')
  expect(text).toContain('Review the diff')
  expect(text).toContain('Fix the crash')
  expect(text).toContain('▌')
  expect(text).toContain('n new job')
  expect(text).toContain('type: planning · worker: auto · access: read')
  expect(text).toContain('Legate · agent-orchestrator · v1')
})

test('the same screen at 80x24 keeps the short footer', () => {
  const text = frame(screen({ columns: 80, rows: 24 }))
  expect(text).toContain('login fix')
  expect(text).toContain('running')
  expect(text).toContain('failed')
  expect(text).toContain('? help')
  expect(text).toContain('q quit')
  expect(text).not.toContain('n new job')
})

test('a job shows its steps and the edit summary', () => {
  const text = frame(
    screen({
      jobs: [
        job({
          id: 'aaaa1111',
          status: 'done',
          prompt: 'Add a button',
          output: 'Done.',
          files: 2,
          added: 10,
          removed: 3,
          steps: [
            { id: 's1', tool: 'read', title: 'src/core/github.ts', status: 'done' },
            {
              id: 's2',
              tool: 'edit',
              title: 'src/core/github.ts',
              status: 'done',
              edit: { added: 4, removed: 1 }
            },
            { id: 's3', tool: 'shell', title: 'pnpm test', status: 'failed' },
            { id: 's4', tool: 'say', title: 'looking', status: 'done' }
          ]
        })
      ],
      jobId: 'aaaa1111'
    })
  )
  expect(text).toContain('READ')
  expect(text).toContain('src/core/github.ts')
  expect(text).toContain('EDIT')
  expect(text).toContain('+4')
  expect(text).toContain('RUN')
  expect(text).toContain('pnpm test')
  expect(text).toContain('(failed)')
  expect(text).toContain('SAY')
  expect(text).toContain('Edited 2 files +10 −3')
  expect(text).toContain('Done.')
})

test('the diff view shows added and removed lines', () => {
  const text = frame(
    screen({
      pane: 'diff',
      diffId: 'login123',
      diffText: '+++ b/src/app.ts\n--- a/src/app.ts\n@@ -1 +1 @@\n-gone\n+hello\n',
      follow: false,
      scroll: 0,
      focus: 'detail'
    })
  )
  expect(text).toContain('Diff login fix')
  expect(text).toContain('+hello')
  expect(text).toContain('-gone')
})

test('ctrl+t cycles the job type and does not insert the letter', async () => {
  function Harness(): JSX.Element {
    const [state, setState] = useState(() => screen({ columns: 100, rows: 24 }))
    const dispatch = (action: Action): void => {
      setState((prev) => reduce(prev, action).state)
    }
    return <App state={state} dispatch={dispatch} bind={() => {}} />
  }
  const view = render(<Harness />)
  await new Promise((resolve) => setTimeout(resolve, 30))
  view.stdin.write('n')
  await new Promise((resolve) => setTimeout(resolve, 30))
  view.stdin.write('ab')
  await new Promise((resolve) => setTimeout(resolve, 30))
  view.stdin.write('\x14')
  await new Promise((resolve) => setTimeout(resolve, 30))
  const text = view.lastFrame() ?? ''
  view.unmount()
  expect(text).toContain('type: debugging')
  expect(text).toContain('ab')
  expect(text).not.toContain('abt')
})

test('the help screen lists the keys', () => {
  const text = frame(screen({ pane: 'help', focus: 'detail', follow: false, scroll: 0, rows: 40 }))
  expect(text).toContain('Keys')
  expect(text).toContain('Tab           switch pane')
  expect(text).toContain('Ctrl+T        job type')
  expect(text).toContain('q or Ctrl+C   quit')
})

test('without colour the screen keeps plain headings, the ▸ marker and inverse focus', () => {
  const text = frame(screen({ color: false }))
  expect(text).toContain('Worktrees  2')
  expect(text).toContain('Jobs  1 running')
  expect(text).toContain('▸')
  expect(text).toContain('Legate · agent-orchestrator · v1')
})

test('the boxes fill the terminal height', () => {
  const text = frame(screen({ columns: 120, rows: 30 }))
  expect(text.split('\n')).toHaveLength(30)
})

test('the project row explains itself instead of repeating the title', () => {
  const text = frame(screen({ worktreeId: '', focus: 'worktrees', pane: 'worktree' }))
  expect(text).toContain('The project folder.')
  expect(text.split('agent-orchestrator · v1')).toHaveLength(3)
})
