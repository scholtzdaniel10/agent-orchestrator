import { expect, test } from 'vitest'
import {
  freshFirings,
  githubFirings,
  isDue,
  newTrigger,
  nextDue,
  parseIssues,
  parsePulls,
  parseRuns,
  type GithubFacts,
  type Trigger
} from './triggers'

const H = 3_600_000

function facts(partial: Partial<GithubFacts> = {}): GithubFacts {
  return {
    branch: 'feature',
    defaultBranch: 'main',
    pulls: [],
    issues: [],
    mainRuns: [],
    ...partial
  }
}

function at(day: number, hour: number, minute = 0): number {
  return new Date(2026, 9, day, hour, minute).getTime()
}

test('a daily trigger is due at its time, once per day', () => {
  const trigger = newTrigger({
    project: '/p',
    kind: 'daily',
    at: '07:30',
    text: 'Update dependencies and run the tests',
    now: at(17, 6)
  })
  expect(isDue(trigger, at(17, 7, 29))).toBe(false)
  expect(isDue(trigger, at(17, 7, 30))).toBe(true)
  const fired: Trigger = { ...trigger, lastFired: at(17, 7, 31) }
  expect(isDue(fired, at(17, 23))).toBe(false)
  expect(isDue(fired, at(18, 7, 30))).toBe(true)
})

test('a daily trigger made after today’s time waits for tomorrow', () => {
  const trigger = newTrigger({
    project: '/p',
    kind: 'daily',
    at: '07:30',
    text: 'x',
    now: at(17, 9)
  })
  expect(isDue(trigger, at(17, 10))).toBe(false)
  expect(nextDue(trigger, at(17, 10))).toBe(at(18, 7, 30))
})

test('an hourly trigger is due every few hours; a disabled one never', () => {
  const trigger = newTrigger({ project: '/p', kind: 'hourly', hours: 6, text: 'x', now: 0 })
  expect(isDue(trigger, 5 * H)).toBe(false)
  expect(isDue(trigger, 6 * H)).toBe(true)
  expect(isDue({ ...trigger, lastFired: 6 * H }, 11 * H)).toBe(false)
  expect(isDue({ ...trigger, enabled: false }, 99 * H)).toBe(false)
})

test('new triggers check their input', () => {
  expect(() => newTrigger({ project: '/p', kind: 'daily', at: 'soon', text: 'x', now: 0 })).toThrow(
    'Give a time'
  )
  expect(() => newTrigger({ project: '/p', kind: 'hourly', text: ' ', now: 0 })).toThrow(
    'scheduled goal'
  )
  const github = newTrigger({ project: '/p', kind: 'new-issue', now: 0 })
  expect(github.primed).toBe(false)
  expect(github.text).toBe('')
})

test('CI failures fire only for the checked-out branch, once per commit', () => {
  const trigger = { ...newTrigger({ project: '/p', kind: 'ci-failed', now: 0 }), primed: true }
  const pulls = [
    { number: 12, title: 'Login', branch: 'feature', sha: 'aaa', failing: true },
    { number: 13, title: 'Other', branch: 'other', sha: 'bbb', failing: true },
    { number: 14, title: 'Green', branch: 'feature', sha: 'ccc', failing: false }
  ]
  const first = freshFirings(trigger, facts({ pulls }))
  expect(first.fire.map((firing) => firing.key)).toEqual(['pr:12:aaa'])
  expect(first.fire[0]?.text).toContain('#12 "Login"')
  const again = freshFirings({ ...trigger, seen: first.seen }, facts({ pulls }))
  expect(again.fire).toEqual([])
  const pushed = freshFirings(
    { ...trigger, seen: first.seen },
    facts({ pulls: [{ ...pulls[0]!, sha: 'ddd' }] })
  )
  expect(pushed.fire.map((firing) => firing.key)).toEqual(['pr:12:ddd'])
})

test('the first look at issues only remembers them; later ones fire', () => {
  const trigger = newTrigger({ project: '/p', kind: 'new-issue', text: 'Keep it small', now: 0 })
  const old = [{ number: 1, title: 'Old', body: '' }]
  const primed = freshFirings(trigger, facts({ issues: old }))
  expect(primed.fire).toEqual([])
  const next = freshFirings(
    { ...trigger, primed: true, seen: primed.seen },
    facts({ issues: [...old, { number: 2, title: 'Crash on save', body: 'Steps…' }] })
  )
  expect(next.fire).toHaveLength(1)
  expect(next.fire[0]?.text).toContain('issue #2: Crash on save')
  expect(next.fire[0]?.text).toContain('Also: Keep it small')
})

test('main-broken fires for a failed newest run, only with the default branch checked out', () => {
  const trigger = { ...newTrigger({ project: '/p', kind: 'main-broken', now: 0 }), primed: true }
  const mainRuns = [
    { id: 9, name: 'CI', status: 'completed', conclusion: 'failure' },
    { id: 8, name: 'CI', status: 'completed', conclusion: 'success' }
  ]
  expect(githubFirings(trigger, facts({ branch: 'main', mainRuns }))[0]).toMatchObject({
    key: 'run:9',
    runId: 9
  })
  expect(githubFirings(trigger, facts({ branch: 'feature', mainRuns }))).toEqual([])
  expect(githubFirings(trigger, facts({ branch: 'main', mainRuns: [mainRuns[1]!] }))).toEqual([])
})

test('gh JSON is read safely', () => {
  expect(
    parsePulls(
      JSON.stringify([
        {
          number: 3,
          title: 'T',
          headRefName: 'b',
          headRefOid: 's',
          statusCheckRollup: [{ conclusion: 'SUCCESS' }, { state: 'FAILURE' }]
        }
      ])
    )
  ).toEqual([{ number: 3, title: 'T', branch: 'b', sha: 's', failing: true }])
  expect(parseIssues('[{"number":4,"title":"I"}]')).toEqual([{ number: 4, title: 'I', body: '' }])
  expect(
    parseRuns('[{"databaseId":5,"name":"CI","status":"completed","conclusion":"failure"}]')
  ).toEqual([{ id: 5, name: 'CI', status: 'completed', conclusion: 'failure' }])
  expect(parsePulls('not json')).toEqual([])
})
