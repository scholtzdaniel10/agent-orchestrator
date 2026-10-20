import { expect, test } from 'vitest'
import {
  printsSomething,
  SLEEP_AFTER_MS,
  terminalsToSleep,
  terminalsToWake
} from './terminal-sleep'

type TerminalInfo = Awaited<ReturnType<Window['api']['listTerminals']>>[number]
type JobRecord = Awaited<ReturnType<Window['api']['listJobs']>>[number]

const NOW = 10_000_000

function term(partial: Partial<TerminalInfo> & Pick<TerminalInfo, 'id'>): TerminalInfo {
  return {
    provider: 'grok',
    title: partial.id,
    status: 'running',
    exitCode: null,
    model: null,
    startedAt: NOW - 10 * SLEEP_AFTER_MS,
    ...partial
  }
}

test('only quiet, hidden agent terminals with no job in their worktree sleep', () => {
  const terminals = [
    term({ id: 'quiet', change: 'aaaa1111' }),
    term({ id: 'shown', change: 'aaaa1111' }),
    term({ id: 'chatty', change: 'bbbb2222' }),
    term({ id: 'shell', provider: 'shell', change: 'cccc3333' }),
    term({ id: 'busy', change: 'dddd4444' }),
    term({ id: 'fresh', startedAt: NOW - 1000 }),
    term({ id: 'exited', status: 'exited' }),
    term({ id: 'asleep', status: 'sleeping' })
  ]
  const jobs = [
    { id: 'j', status: 'running', change: 'dddd4444' } as JobRecord,
    { id: 'k', status: 'done', change: 'aaaa1111' } as JobRecord
  ]
  expect(
    terminalsToSleep({
      terminals,
      visible: new Set(['shown']),
      activity: { chatty: NOW - 5000 },
      jobs,
      now: NOW
    })
  ).toEqual(['quiet'])
})

test('a sleeping terminal wakes when it is shown again', () => {
  const terminals = [term({ id: 'a', status: 'sleeping' }), term({ id: 'b', status: 'sleeping' })]
  expect(terminalsToWake(terminals, new Set(['b']))).toEqual(['b'])
})

test('a window title change or bare control codes do not count as output', () => {
  expect(printsSomething('\u001b]0;npm exec mcp-remote https://example\u0007')).toBe(false)
  expect(printsSomething('\u001b]0;Windows PowerShell\u0007\u001b[?25h\u001b[2J\r\n')).toBe(false)
  expect(printsSomething('\u001b[38;1H  Grok 4.7 256K High\u001b[K')).toBe(true)
  expect(printsSomething('Reading src/app.ts\r\n')).toBe(true)
})
