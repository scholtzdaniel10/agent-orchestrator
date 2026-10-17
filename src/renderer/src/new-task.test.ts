import { expect, test } from 'vitest'
import {
  DEFAULT_COUNT,
  DEFAULT_HOW,
  DEFAULT_KIND,
  DEFAULT_ROUNDS,
  DEFAULT_WHO,
  HOW_OPTIONS,
  JOB_KINDS,
  activeHow,
  asKind,
  clampCount,
  clampRounds,
  howEnabled,
  landingLine,
  planNewTask,
  repoHint,
  submitLabel,
  unavailableHint,
  type How,
  type JobKind,
  type NewTaskContext,
  type NewTaskDraft,
  type Who
} from './new-task'

function draft(patch: Partial<NewTaskDraft> = {}): NewTaskDraft {
  return {
    text: 'Fix the pager',
    who: 'cursor',
    how: 'terminal',
    count: 1,
    kind: 'refactor',
    check: '',
    maxRounds: 5,
    ...patch
  }
}

const ready: NewTaskContext = {
  isRepo: true,
  claudeAvailable: true,
  cursorAvailable: true
}

const notRepo: NewTaskContext = { ...ready, isRepo: false }

test('defaults are Cursor, a terminal, and a refactor', () => {
  expect(DEFAULT_WHO).toBe('cursor')
  expect(DEFAULT_HOW).toBe('terminal')
  expect(DEFAULT_KIND).toBe('refactor')
  expect(DEFAULT_COUNT).toBe(1)
  expect(DEFAULT_ROUNDS).toBe(5)
  expect(JOB_KINDS).toEqual(['debugging', 'refactor', 'boilerplate', 'planning', 'review'])
})

test('how choices explain the three ways to start', () => {
  expect(HOW_OPTIONS.map((option) => [option.id, option.label, option.detail])).toEqual([
    [
      'terminal',
      'In a terminal',
      'Starts the agent in its own worktree. You watch and talk to it.'
    ],
    [
      'background',
      'In the background',
      'Runs as a job in its own worktree. You review the change.'
    ],
    ['goal', 'As a goal', 'The lead plans rounds and asks you before each one.']
  ])
})

test('labels follow the way the work starts', () => {
  expect(submitLabel('terminal')).toBe('Start agent')
  expect(submitLabel('background')).toBe('Run job')
  expect(submitLabel('goal')).toBe('Start goal')
  expect(landingLine(true)).toBe('Changes appear in Changes for you to merge.')
  expect(landingLine(false)).toBe('This job only reads the folder. It does not change files.')
  expect(repoHint(true)).toBeNull()
  expect(repoHint(false)).toBe('Worktrees need a git repository.')
  expect(unavailableHint(true)).toBeNull()
  expect(unavailableHint(false)).toBe('Not ready')
})

test('outside a repository only a background job is allowed', () => {
  expect(howEnabled('terminal', false)).toBe(false)
  expect(howEnabled('goal', false)).toBe(false)
  expect(howEnabled('background', false)).toBe(true)
  expect(howEnabled('terminal', true)).toBe(true)
  expect(activeHow('terminal', false)).toBe('background')
  expect(activeHow('goal', false)).toBe('background')
  expect(activeHow('goal', true)).toBe('goal')
})

test('count stays between 1 and 6, rounds between 1 and 20', () => {
  expect(clampCount(0)).toBe(1)
  expect(clampCount(1)).toBe(1)
  expect(clampCount(6)).toBe(6)
  expect(clampCount(9)).toBe(6)
  expect(clampCount(1.4)).toBe(1)
  expect(clampCount(Number.NaN)).toBe(1)
  expect(clampRounds(0)).toBe(1)
  expect(clampRounds(20)).toBe(20)
  expect(clampRounds(50)).toBe(20)
  expect(clampRounds(Number.NaN)).toBe(5)
  expect(asKind('planning')).toBe('planning')
  expect(asKind('nope')).toBe('refactor')
})

test('empty text is an error and makes no call', () => {
  expect(planNewTask(draft({ text: '   ' }), ready)).toEqual({
    ok: false,
    error: 'Write what should be done.'
  })
  expect(planNewTask(draft({ text: '' }), notRepo)).toEqual({
    ok: false,
    error: 'Write what should be done.'
  })
})

test('a plan that is not ready is an error and makes no call', () => {
  const cursorOut: NewTaskContext = { ...ready, cursorAvailable: false }
  expect(planNewTask(draft({ who: 'cursor' }), cursorOut)).toEqual({
    ok: false,
    error: 'Cursor is not ready. Pick a plan that is, or set it up and try again.'
  })
  const claudeOut: NewTaskContext = { ...ready, claudeAvailable: false }
  expect(planNewTask(draft({ who: 'claude', text: '  Look around  ' }), claudeOut)).toEqual({
    ok: false,
    error: 'Claude is not ready. Pick a plan that is, or set it up and try again.'
  })
  expect(planNewTask(draft({ text: '  ' }), cursorOut).ok).toBe(false)
  expect(planNewTask(draft({ text: '  ' }), cursorOut)).toMatchObject({
    error: 'Write what should be done.'
  })
})

test('a terminal starts that agent, and a check on it is ignored', () => {
  expect(planNewTask(draft({ how: 'terminal', check: '' }), ready)).toEqual({
    ok: true,
    action: 'terminal',
    text: 'Fix the pager',
    who: 'cursor',
    count: 1
  })
  expect(
    planNewTask(
      draft({ how: 'terminal', who: 'claude', count: 9, check: 'pnpm test', text: '  Ship it  ' }),
      ready
    )
  ).toEqual({
    ok: true,
    action: 'terminal',
    text: 'Ship it',
    who: 'claude',
    count: 6
  })
})

test('a background job with no check edits in a worktree', () => {
  for (const check of ['', '   ']) {
    expect(
      planNewTask(draft({ how: 'background', kind: 'debugging', who: 'claude', check }), ready)
    ).toEqual({
      ok: true,
      action: 'job',
      jobType: 'debugging',
      text: 'Fix the pager',
      who: 'claude',
      access: 'edit'
    })
  }
})

test('a background job with a check becomes a quick goal', () => {
  const kinds: JobKind[] = ['debugging', 'refactor', 'boilerplate', 'planning', 'review']
  for (const kind of kinds) {
    expect(planNewTask(draft({ how: 'background', kind, check: '  pnpm test  ' }), ready)).toEqual({
      ok: true,
      action: 'goal',
      created: 'goal',
      input: {
        text: 'Fix the pager',
        check: 'pnpm test',
        quickType: kind,
        quickAccess: 'edit'
      }
    })
  }
})

test('a goal keeps the check and the round cap', () => {
  expect(planNewTask(draft({ how: 'goal', check: '', maxRounds: 5 }), ready)).toEqual({
    ok: true,
    action: 'goal',
    created: 'goal',
    input: { text: 'Fix the pager', check: '', maxRounds: 5 }
  })
  expect(
    planNewTask(draft({ how: 'goal', check: ' pnpm test ', maxRounds: 50, who: 'claude' }), ready)
  ).toEqual({
    ok: true,
    action: 'goal',
    created: 'goal',
    input: { text: 'Fix the pager', check: 'pnpm test', maxRounds: 20 }
  })
})

test('outside a repository every choice is a read-only job', () => {
  const hows: How[] = ['terminal', 'background', 'goal']
  const whos: Who[] = ['cursor', 'claude']
  for (const how of hows) {
    for (const who of whos) {
      expect(
        planNewTask(
          draft({
            how,
            who,
            kind: 'planning',
            check: 'pnpm test',
            count: 4,
            maxRounds: 9,
            text: '  Read the pager  '
          }),
          notRepo
        )
      ).toEqual({
        ok: true,
        action: 'job',
        jobType: 'planning',
        text: 'Read the pager',
        who,
        access: 'read'
      })
    }
  }
})

test('outside a repository a missing plan still makes no call', () => {
  expect(
    planNewTask(draft({ how: 'background', text: 'Read it' }), {
      ...notRepo,
      cursorAvailable: false
    })
  ).toMatchObject({ ok: false })
})
