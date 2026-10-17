import { expect, test } from 'vitest'
import {
  changeCells,
  changeFooter,
  checkReport,
  footerLine,
  formatAge,
  formatDuration,
  goalCells,
  headerLine,
  jobCells,
  planLine,
  proposalText,
  type PlanView,
  stepLine,
  table
} from './format'

const read = {
  id: 's1',
  tool: 'read' as const,
  title: 'Read src/app.ts',
  status: 'done' as const
}

function plan(partial: Partial<PlanView> & Pick<PlanView, 'id'>): PlanView {
  return {
    available: true,
    headroom: 0.8,
    restingUntil: null,
    busy: false,
    running: 0,
    version: '2.1.292',
    versionStatus: 'tested',
    atRisk: false,
    ...partial
  }
}

test('step lines name the tool, the size, and a failure', () => {
  expect(stepLine(read, false, false)).toBe('READ   Read src/app.ts')
  expect(
    stepLine(
      {
        id: 's2',
        tool: 'edit',
        title: 'Edited src/app.ts',
        status: 'done',
        edit: { added: 4, removed: 1 }
      },
      false,
      false
    )
  ).toBe('EDIT   Edited src/app.ts +4 −1')
  expect(
    stepLine({ id: 's3', tool: 'shell', title: 'Ran pnpm test', status: 'failed' }, false, false)
  ).toBe('RUN    Ran pnpm test (failed)')
  expect(stepLine({ id: 's4', tool: 'say', title: 'hello', status: 'done' }, false, false)).toBe(
    null
  )
  expect(stepLine({ id: 's4', tool: 'say', title: 'hello', status: 'done' }, true, false)).toBe(
    'SAY    hello'
  )
  expect(stepLine({ ...read, status: 'running' }, true, false)).toBeNull()
})

test('colour wraps the tool word and leaves the title readable', () => {
  const line = stepLine(read, false, true)
  expect(line).toContain('\u001b[36mREAD\u001b[0m')
  expect(line).toContain('Read src/app.ts')
  expect(stepLine(read, false, false)).not.toContain('\u001b')
})

test('header, footer, durations, and ages', () => {
  expect(
    headerLine(
      {
        provider: 'claude',
        model: 'sonnet',
        type: 'planning',
        access: 'read',
        reason: 'first choice'
      },
      false
    )
  ).toBe('claude · sonnet · planning · read · first choice')
  expect(headerLine({ provider: 'claude', type: 'planning', access: 'read' }, false)).toBe(
    'claude · planning · read'
  )
  expect(headerLine({ provider: 'claude', type: 'planning', access: 'read' }, true)).toContain(
    '\u001b[38;2;255;160;90mclaude\u001b[0m'
  )
  expect(footerLine('done', 42_000, undefined, false)).toBe('done in 42s')
  expect(footerLine('failed', 1500, 'boom', false)).toBe('failed: boom')
  expect(footerLine('done', 42_000, undefined, true)).toContain('\u001b[')
  expect(formatDuration(0)).toBe('0s')
  expect(formatDuration(61_000)).toBe('1m 1s')
  expect(formatDuration(120_000)).toBe('2m')
  expect(formatDuration(3_600_000)).toBe('1h')
  expect(formatAge(30_000)).toBe('0m')
  expect(formatAge(5 * 60_000)).toBe('5m')
  expect(formatAge(2 * 3_600_000)).toBe('2h')
  expect(formatAge(3 * 86_400_000)).toBe('3d')
})

test('tables pad every column but the last', () => {
  expect(
    table([
      ['id', 'done', 'prompt'],
      ['abcd', 'running', 'hi']
    ])
  ).toBe(['id    done     prompt', 'abcd  running  hi'].join('\n'))
  expect(
    jobCells({
      id: 'abcdef123456',
      status: 'done',
      provider: 'claude',
      type: 'planning',
      access: 'read',
      ageMs: 60_000,
      prompt: 'one\ntwo'
    })
  ).toEqual(['abcdef12', 'done', 'claude', 'planning', 'read', '1m', 'one two'])
  expect(
    changeCells({ id: 'abcd1234', name: 'login', files: 2, insertions: 10, deletions: 3 })
  ).toEqual(['abcd1234', 'login', '2 files', '+10 −3'])
})

test('plan lines use the app wording', () => {
  expect(planLine(plan({ id: 'claude' }), '2.1.292', 0, false)).toBe(
    'claude · ready · CLI 2.1.292 · 80% left'
  )
  expect(
    planLine(
      plan({
        id: 'cursor',
        available: false,
        problem: 'not-installed',
        version: null,
        versionStatus: 'unknown'
      }),
      '2026.10.01',
      0,
      false
    )
  ).toBe(
    'cursor · not installed · The Cursor CLI is not installed. Install it, then run `agent login` in a terminal.'
  )
  expect(
    planLine(plan({ id: 'claude', version: '9.0.0', versionStatus: 'newer' }), '2.1.292', 0, false)
  ).toBe('claude · ready · CLI 9.0.0 · newer than tested (2.1.292) · 80% left')
  expect(
    planLine(
      plan({ id: 'claude', version: '1.0.0', versionStatus: 'older', headroom: 0.5 }),
      '2.1.292',
      0,
      false
    )
  ).toBe('claude · ready · CLI 1.0.0 · older than tested (2.1.292) — update it · 50% left')
  expect(planLine(plan({ id: 'claude' }), '2.1.292', 0, true)).toContain('\u001b[')
})

test('a goal list row, a proposal, and a check', () => {
  expect(goalCells({ status: 'done', used: 2, max: 5, text: 'Newer goal\nmore detail' })).toEqual([
    'done',
    '2/5',
    'Newer goal'
  ])
  expect(
    proposalText(1, 'Add the form', [
      {
        type: 'refactor',
        access: 'edit',
        prompt: 'Write the form\nUse the field.\nKeep the types.\nHidden line.'
      },
      { type: 'review', access: 'full', prompt: 'Check it' }
    ])
  ).toBe(
    [
      'Round 1: Add the form',
      'refactor · edit',
      'Write the form',
      'Use the field.',
      'Keep the types.',
      '…',
      'review · full',
      'Check it'
    ].join('\n')
  )
  expect(checkReport(0, 'all green\n')).toBe('Check passed\nall green')
  const lines = Array.from({ length: 20 }, (_, index) => `line ${String(index)}`)
  expect(checkReport(3, lines.join('\n'))).toBe(
    ['Check failed (exit 3)', ...lines.slice(-15)].join('\n')
  )
})

test('a change footer names the diff command', () => {
  expect(changeFooter('abcd1234', 2, 4, 1)).toBe(
    'change abcd1234 · 2 files +4 −1 · legate diff abcd1234'
  )
  expect(changeFooter('abcd1234', 1, 1, 0)).toBe(
    'change abcd1234 · 1 file +1 −0 · legate diff abcd1234'
  )
})
