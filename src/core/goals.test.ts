import { expect, test } from 'vitest'
import { failureKey, Goals, parsePlan, planPrompt, type GoalsDeps, type PlannedJob } from './goals'
import type { JobRecord } from './router'

function memoryStore(): GoalsDeps['store'] {
  const rows = new Map<string, { project: string; updatedAt: number; data: string }>()
  return {
    saveGoal(id, project, updatedAt, data) {
      rows.set(id, { project, updatedAt, data })
    },
    goals(project) {
      return [...rows.values()]
        .filter((row) => row.project === project)
        .sort((a, b) => b.updatedAt - a.updatedAt)
        .map((row) => row.data)
    },
    goal(id) {
      return rows.get(id)?.data ?? null
    },
    deleteGoal(id) {
      rows.delete(id)
    }
  }
}

const flush = async (): Promise<void> => {
  for (let i = 0; i < 20; i += 1) await new Promise((resolve) => setTimeout(resolve, 0))
}

interface Harness {
  goals: Goals
  jobs: Map<string, JobRecord>
  submitted: { job: PlannedJob; extra: { goal: string; base: string; project: string } }[]
  applied: string[]
  commits: string[]
  discarded: string[]
  plans: string[]
  finish: (id: string, status: 'done' | 'failed', change?: string) => void
}

function harness(opts: {
  planReplies?: string[]
  checks?: { code: number | null; output: string }[]
  ceiling?: 'read' | 'edit' | 'full'
}): Harness {
  const jobs = new Map<string, JobRecord>()
  const submitted: Harness['submitted'] = []
  const applied: string[] = []
  const commits: string[] = []
  const discarded: string[] = []
  const plans: string[] = []
  const replies = [...(opts.planReplies ?? [])]
  const checks = [...(opts.checks ?? [])]
  let next = 0
  const goals: Goals = new Goals({
    store: memoryStore(),
    worktrees: {
      create: (_project, id) =>
        Promise.resolve({ id: `g-${id.slice(0, 4)}`, path: '/wt/goal', branch: 'orch/goal' }),
      folder: () => '/wt/goal',
      branchOf: () => 'orch/goal',
      applyInto: (fromId) => {
        applied.push(fromId)
        return Promise.resolve({ ok: true, empty: false, message: 'applied' })
      },
      commitAll: (_dir, message) => {
        commits.push(message)
        return Promise.resolve(true)
      },
      discard: (_project, id) => {
        discarded.push(id)
        return Promise.resolve()
      }
    },
    submit: (job, extra) => {
      next += 1
      const record = {
        id: `job-${String(next)}`,
        type: job.type,
        prompt: job.prompt,
        provider: null,
        status: 'running',
        output: '',
        failedOver: [],
        access: job.access,
        goal: extra.goal,
        base: extra.base
      } as JobRecord
      jobs.set(record.id, record)
      submitted.push({ job, extra })
      return record
    },
    job: (id) => jobs.get(id) ?? null,
    cancel: (id) => {
      const job = jobs.get(id)
      if (job !== undefined) job.status = 'failed'
    },
    plan: (_goal, _folder, prompt) => {
      plans.push(prompt)
      const reply = replies.shift()
      return reply === undefined ? Promise.reject(new Error('no reply')) : Promise.resolve(reply)
    },
    runCheck: () => Promise.resolve(checks.shift() ?? { code: 0, output: 'ok' }),
    ceiling: () => opts.ceiling ?? 'full'
  })
  const finish = (id: string, status: 'done' | 'failed', change?: string): void => {
    const job = jobs.get(id)
    if (job === undefined) throw new Error(`no ${id}`)
    job.status = status
    if (change !== undefined) job.change = change
    if (status === 'failed') job.error = 'boom'
    goals.onJob(job)
  }
  return { goals, jobs, submitted, applied, commits, discarded, plans, finish }
}

const JOBS = (prompt: string, access = 'edit'): string =>
  'Here is the plan.\n```json\n' +
  JSON.stringify({
    decision: 'jobs',
    summary: `do ${prompt}`,
    jobs: [{ type: 'refactor', access, prompt }]
  }) +
  '\n```'

test('parsePlan reads jobs, done and ask, and refuses anything else', () => {
  expect(parsePlan(JOBS('fix login'))).toEqual({
    decision: 'jobs',
    summary: 'do fix login',
    jobs: [{ type: 'refactor', access: 'edit', prompt: 'fix login' }]
  })
  expect(parsePlan('{"decision":"done","summary":"all green"}')).toEqual({
    decision: 'done',
    summary: 'all green'
  })
  expect(parsePlan('```json\n{"decision":"ask","question":"Which file?"}\n```')).toEqual({
    decision: 'ask',
    question: 'Which file?'
  })
  expect(parsePlan('I think we should refactor.')).toBeNull()
  expect(parsePlan('{"decision":"jobs","jobs":[]}')).toBeNull()
  const many = parsePlan(
    JSON.stringify({
      decision: 'jobs',
      jobs: [1, 2, 3, 4, 5].map((n) => ({ type: 'nope', access: 'read', prompt: `p${String(n)}` }))
    })
  )
  expect(many?.decision === 'jobs' ? many.jobs : []).toEqual([
    { type: 'refactor', access: 'edit', prompt: 'p1' },
    { type: 'refactor', access: 'edit', prompt: 'p2' },
    { type: 'refactor', access: 'edit', prompt: 'p3' }
  ])
})

test('failureKey ignores numbers such as timings', () => {
  expect(failureKey('FAIL auth.test.ts\nexpected 1 got 2\n(12ms)')).toBe(
    failureKey('FAIL auth.test.ts\nexpected 3 got 4\n(80ms)')
  )
  expect(failureKey('FAIL auth.test.ts')).not.toBe(failureKey('FAIL files.test.ts'))
})

test('a quick task runs your prompt at once, folds its change in, and ends when the check passes', async () => {
  const h = harness({ checks: [{ code: 0, output: 'all passed' }] })
  const goal = await h.goals.create({
    project: '/p',
    text: 'Fix the off-by-one in pager.ts',
    check: 'pnpm test -- pager',
    quick: { type: 'debugging', access: 'edit' }
  })
  expect(h.plans).toEqual([])
  expect(goal.status).toBe('running')
  expect(h.submitted[0]?.extra).toEqual({ goal: goal.id, base: 'orch/goal', project: '/p' })
  h.finish('job-1', 'done', 'change-1')
  await flush()
  expect(h.applied).toEqual(['change-1'])
  expect(h.discarded).toEqual(['change-1'])
  expect(h.commits).toEqual(['Goal round 1: Your task'])
  const done = h.goals.get(goal.id)
  expect(done?.status).toBe('done')
  expect(done?.rounds[0]?.check).toMatchObject({ code: 0, output: 'all passed' })
})

test('a quick task without a check is done after its one job', async () => {
  const h = harness({})
  const goal = await h.goals.create({
    project: '/p',
    text: 'Rename getUser to loadUser',
    quick: { type: 'refactor', access: 'edit' }
  })
  h.finish('job-1', 'done', 'change-1')
  await flush()
  expect(h.goals.get(goal.id)?.status).toBe('done')
})

test('a planned goal waits for approval each round and stops when the check fails the same way twice', async () => {
  const h = harness({
    planReplies: [JOBS('make login keep the session'), JOBS('fix the failing test')],
    checks: [
      { code: 1, output: 'FAIL session.test.ts\nexpected token 1 (12ms)' },
      { code: 1, output: 'FAIL session.test.ts\nexpected token 2 (40ms)' }
    ]
  })
  const goal = await h.goals.create({ project: '/p', text: 'Keep the session', check: 'pnpm test' })
  await flush()
  let now = h.goals.get(goal.id)
  expect(now?.status).toBe('waiting')
  expect(now?.proposal?.jobs[0]?.prompt).toBe('make login keep the session')
  expect(h.submitted).toEqual([])

  h.goals.approve(goal.id)
  h.finish('job-1', 'done', 'change-1')
  await flush()
  now = h.goals.get(goal.id)
  expect(now?.status).toBe('waiting')
  expect(now?.proposal?.summary).toBe('do fix the failing test')
  expect(h.plans[1]).toContain('check exited 1')
  expect(h.plans[1]).toContain('FAIL session.test.ts')

  h.goals.approve(goal.id, [{ type: 'debugging', access: 'full', prompt: 'my own fix' }])
  expect(h.submitted[1]?.job.prompt).toBe('my own fix')
  h.finish('job-2', 'done', 'change-2')
  await flush()
  now = h.goals.get(goal.id)
  expect(now?.status).toBe('waiting')
  expect(now?.question).toContain('failed the same way twice')
  expect(now?.proposal).toBeNull()
})

test('the round limit stops a goal; an answer gives it one more round', async () => {
  const h = harness({
    planReplies: [JOBS('first'), JOBS('second')],
    checks: [{ code: 1, output: 'one error' }]
  })
  const goal = await h.goals.create({ project: '/p', text: 'Goal', check: 'make', maxRounds: 1 })
  await flush()
  h.goals.approve(goal.id)
  h.finish('job-1', 'done', 'c1')
  await flush()
  const stopped = h.goals.get(goal.id)
  expect(stopped?.status).toBe('stopped')
  expect(stopped?.outcome).toContain('limit of 1 rounds')
  h.goals.answer(goal.id, 'Try the other file')
  await flush()
  const again = h.goals.get(goal.id)
  expect(again?.status).toBe('waiting')
  expect(again?.maxRounds).toBe(2)
  expect(h.plans[1]).toContain('Try the other file')
})

test('an answer without a plan, a failed planner, and stop all leave the goal readable', async () => {
  const h = harness({ planReplies: ['I would refactor things.'] })
  const goal = await h.goals.create({ project: '/p', text: 'Goal' })
  await flush()
  expect(h.goals.get(goal.id)?.question).toContain('did not include a plan')
  h.goals.answer(goal.id, 'Plan it')
  await flush()
  expect(h.goals.get(goal.id)?.question).toContain('could not plan')
  expect(h.goals.stop(goal.id).status).toBe('stopped')
  await h.goals.remove(goal.id)
  expect(h.goals.get(goal.id)).toBeNull()
})

test("jobs are capped at the person's ceiling, and read becomes edit", async () => {
  const h = harness({ planReplies: [JOBS('run the tests', 'full')], ceiling: 'edit' })
  const goal = await h.goals.create({ project: '/p', text: 'Goal' })
  await flush()
  h.goals.approve(goal.id)
  expect(h.submitted[0]?.job.access).toBe('edit')
})

test('a failed job is reported to the lead as a problem', async () => {
  const h = harness({ planReplies: [JOBS('one'), JOBS('two')], checks: [{ code: 1, output: 'x' }] })
  const goal = await h.goals.create({ project: '/p', text: 'Goal', check: 'make' })
  await flush()
  h.goals.approve(goal.id)
  h.finish('job-1', 'failed')
  await flush()
  expect(h.goals.get(goal.id)?.rounds[0]?.problems[0]).toContain('boom')
  expect(planPrompt(h.goals.get(goal.id)!)).toContain('problem: one: boom')
})
