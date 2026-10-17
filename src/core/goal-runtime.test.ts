import { expect, test } from 'vitest'
import type { AgentEvent, ProviderId, RunHandle } from './types'
import { planGoal, runGoalCheck, type PlanAdapter, type PlanWorker } from './goal-runtime'

function handle(
  events: AgentEvent[],
  exit: { code: number | null; stderr: string } = { code: 0, stderr: '' }
): RunHandle {
  return {
    events: (async function* (): AsyncGenerator<AgentEvent> {
      for (const event of events) yield event
    })(),
    exit: Promise.resolve(exit),
    kill(): void {
      return undefined
    }
  }
}

function workers(...rows: PlanWorker[]): PlanWorker[] {
  return rows
}

test('planGoal uses the preferred plan, then the first one that is ready', async () => {
  const seen: {
    id: ProviderId
    cwd: string
    prompt: string
    model: string | undefined
    access: string | undefined
  }[] = []
  const adapter = (id: ProviderId): PlanAdapter => ({
    run: (job, cwd, opts) => {
      seen.push({ id, cwd, prompt: job.prompt, model: opts?.model, access: opts?.access })
      return handle([
        { kind: 'text', text: 'draft ' },
        { kind: 'result', ok: true, text: `plan from ${id}` }
      ])
    }
  })
  const both = workers({ id: 'claude', available: true }, { id: 'cursor', available: true })
  const text = await planGoal({
    folder: '/wt/goal',
    prompt: 'plan the round',
    workers: both,
    preferred: 'cursor',
    adapter,
    modelFor: (id) => (id === 'cursor' ? 'composer' : undefined)
  })
  expect(text).toBe('plan from cursor')
  expect(seen).toEqual([
    {
      id: 'cursor',
      cwd: '/wt/goal',
      prompt: 'plan the round',
      model: 'composer',
      access: 'read'
    }
  ])

  seen.length = 0
  const fallback = await planGoal({
    folder: '/wt/goal',
    prompt: 'again',
    workers: workers({ id: 'claude', available: false }, { id: 'cursor', available: true }),
    preferred: 'claude',
    adapter,
    modelFor: () => undefined
  })
  expect(fallback).toBe('plan from cursor')
  expect(seen[0]?.id).toBe('cursor')
  expect(seen[0]?.model).toBeUndefined()
})

test('planGoal keeps spoken text when the result is blank, and names a failed run', async () => {
  const spoken: PlanAdapter = {
    run: () =>
      handle([
        { kind: 'text', text: 'hello' },
        { kind: 'result', ok: true, text: '   ' }
      ])
  }
  const text = await planGoal({
    folder: '/wt',
    prompt: 'plan',
    workers: [{ id: 'claude', available: true }],
    preferred: undefined,
    adapter: () => spoken,
    modelFor: () => undefined
  })
  expect(text).toBe('hello')

  const silent: PlanAdapter = {
    run: () => handle([], { code: 1, stderr: 'noise\nout of credits' })
  }
  await expect(
    planGoal({
      folder: '/wt',
      prompt: 'plan',
      workers: [{ id: 'claude', available: true }],
      preferred: 'claude',
      adapter: () => silent,
      modelFor: () => undefined
    })
  ).rejects.toThrow('out of credits')

  await expect(
    planGoal({
      folder: '/wt',
      prompt: 'plan',
      workers: [{ id: 'claude', available: false }],
      preferred: 'claude',
      adapter: () => spoken,
      modelFor: () => undefined
    })
  ).rejects.toThrow('No plan is signed in and ready.')
})

test('runGoalCheck returns the exit code of a real command', async () => {
  const result = await runGoalCheck('node -e "process.exit(3)"', process.cwd())
  expect(result.code).toBe(3)
})

test('runGoalCheck keeps the end of a long command and both streams', async () => {
  const marked = await runGoalCheck(
    "node -e \"process.stdout.write('A'.repeat(250000)+'END')\"",
    process.cwd()
  )
  expect(marked.code).toBe(0)
  expect(marked.output.length).toBeLessThanOrEqual(200_000)
  expect(marked.output.length).toBeLessThan(250_000)
  expect(marked.output.endsWith('END')).toBe(true)

  const err = await runGoalCheck(
    'node -e "process.stderr.write(\'bye\'); process.exit(3)"',
    process.cwd()
  )
  expect(err.code).toBe(3)
  expect(err.output).toContain('bye')
})
