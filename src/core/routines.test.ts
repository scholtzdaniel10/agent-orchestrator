import { expect, test } from 'vitest'
import type { JobRecord } from './router/orchestrator'
import type { RoutineRow } from './router/store'
import { Routines, parseRoutine, type Routine, type SubmitJob } from './routines'
import type { JobAccess, JobType, ProviderId } from './types'

class FakeStore {
  rows: RoutineRow[] = []

  listRoutines(project: string): RoutineRow[] {
    return this.rows
      .filter((row) => row.project === project)
      .slice()
      .sort((a, b) => a.position - b.position)
  }

  saveRoutine(row: RoutineRow): void {
    const index = this.rows.findIndex((item) => item.id === row.id)
    if (index >= 0) this.rows[index] = { ...row }
    else this.rows.push({ ...row })
  }

  deleteRoutine(id: string): void {
    this.rows = this.rows.filter((row) => row.id !== id)
  }
}

interface SubmitCall {
  type: JobType
  prompt: string
  provider: ProviderId | undefined
  access: JobAccess | undefined
  extra: { routine?: string } | undefined
}

function makeRoutines(partial?: {
  project?: string | (() => string)
  id?: () => string
  submit?: SubmitJob
}): {
  routines: Routines
  store: FakeStore
  calls: SubmitCall[]
  jobs: JobRecord[]
} {
  const store = new FakeStore()
  const calls: SubmitCall[] = []
  const jobs: JobRecord[] = []
  const submit: SubmitJob =
    partial?.submit ??
    ((type, prompt, provider, access, extra) => {
      calls.push({ type, prompt, provider, access, extra })
      const job: JobRecord = {
        id: `job-${String(jobs.length + 1)}`,
        type,
        prompt,
        provider: provider ?? 'claude',
        status: 'queued',
        output: '',
        failedOver: [],
        access: access ?? 'read'
      }
      if (extra?.routine !== undefined) job.routine = extra.routine
      jobs.push(job)
      return job
    })
  const routines = new Routines({
    store,
    submit,
    project: partial?.project ?? '/proj',
    id: partial?.id ?? (() => 'id-1')
  })
  return { routines, store, calls, jobs }
}

function sample(partial?: Partial<Routine>): Routine {
  return {
    id: 'r1',
    name: 'Review it',
    prompt: 'review the last change',
    type: 'review',
    worker: 'auto',
    access: 'read',
    ...partial
  }
}

test('save validates, lists in order, and remove deletes', () => {
  const { routines, store } = makeRoutines({
    id: () => 'generated'
  })
  const first = routines.save({
    name: '  One  ',
    prompt: 'do one',
    type: 'planning',
    worker: 'claude',
    access: 'edit'
  })
  expect(first).toMatchObject({
    id: 'generated',
    name: 'One',
    prompt: 'do one',
    type: 'planning',
    worker: 'claude',
    access: 'edit'
  })
  expect(first).not.toHaveProperty('after')
  const second = routines.save(
    sample({
      id: 'r2',
      name: 'Two',
      after: { jobType: 'debugging' }
    })
  )
  expect(routines.list().map((item) => item.id)).toEqual(['generated', 'r2'])
  expect(routines.list()[1]).toEqual(second)
  expect(store.rows[0]).toMatchObject({ project: '/proj', position: 0 })
  expect(store.rows[1]).toMatchObject({ project: '/proj', position: 1 })

  routines.save(sample({ id: 'r2', name: 'Two renamed', prompt: 'later' }))
  expect(routines.list()[1]).toMatchObject({ name: 'Two renamed', prompt: 'later' })
  expect(store.rows[1].position).toBe(1)

  routines.remove('r2')
  expect(routines.list().map((item) => item.id)).toEqual(['generated'])
  expect(() => routines.remove('')).toThrow('invalid routine id')
})

test('save refuses a 31st routine on the same project', () => {
  const { routines } = makeRoutines({
    id: () => {
      throw new Error('should not generate')
    }
  })
  for (let i = 0; i < 30; i++) {
    routines.save(sample({ id: `r${String(i)}`, name: `n${String(i)}` }))
  }
  expect(() => routines.save(sample({ id: 'r30', name: 'too many' }))).toThrow(
    'This project already has 30 routines. Delete one before adding another.'
  )
  routines.save(sample({ id: 'r0', name: 'updated' }))
  expect(routines.list()[0].name).toBe('updated')
})

test('parseRoutine rejects bad fields', () => {
  expect(() => parseRoutine(null)).toThrow('invalid routine')
  expect(() => parseRoutine(sample({ name: '' }))).toThrow(
    'Name must be 1 to 40 characters on one line.'
  )
  expect(() => parseRoutine(sample({ name: 'a\nb' }))).toThrow(
    'Name must be 1 to 40 characters on one line.'
  )
  expect(() => parseRoutine(sample({ name: 'x'.repeat(41) }))).toThrow(
    'Name must be 1 to 40 characters on one line.'
  )
  expect(() => parseRoutine(sample({ prompt: '' }))).toThrow('Prompt must be 1 to 8000 characters.')
  expect(() => parseRoutine(sample({ prompt: 'x'.repeat(8001) }))).toThrow(
    'Prompt must be 1 to 8000 characters.'
  )
  expect(() => parseRoutine({ ...sample(), type: 'nope' })).toThrow('unknown job type')
  expect(() => parseRoutine({ ...sample(), worker: 'both' })).toThrow(
    'worker must be auto, claude, cursor, or grok'
  )
  expect(() => parseRoutine({ ...sample(), access: 'write' })).toThrow(
    'access must be read, edit, or full'
  )
  expect(() => parseRoutine({ ...sample(), after: { jobType: 'nope' } })).toThrow(
    'unknown job type'
  )
})

test('run submits the saved prompt and marks the job with the routine name', () => {
  const { routines, calls } = makeRoutines()
  routines.save(
    sample({
      worker: 'cursor',
      access: 'full',
      type: 'refactor'
    })
  )
  const job = routines.run('r1')
  expect(job).toMatchObject({
    type: 'refactor',
    prompt: 'review the last change',
    provider: 'cursor',
    routine: 'Review it'
  })
  expect(calls).toEqual([
    {
      type: 'refactor',
      prompt: 'review the last change',
      provider: 'cursor',
      access: 'full',
      extra: { routine: 'Review it' }
    }
  ])
  expect(() => routines.run('missing')).toThrow(
    'That routine is gone. Refresh the list and try again.'
  )
})

test('run with auto does not name a provider', () => {
  const { routines, calls } = makeRoutines()
  routines.save(sample({ worker: 'auto' }))
  routines.run('r1')
  expect(calls[0]?.provider).toBeUndefined()
})

test('onJobFinished runs matching routines once with a follow-on prompt', () => {
  const { routines, calls } = makeRoutines()
  routines.save(
    sample({
      id: 'after-review',
      name: 'Follow review',
      type: 'refactor',
      prompt: 'clean it up',
      after: { jobType: 'review' }
    })
  )
  routines.save(
    sample({
      id: 'after-plan',
      name: 'Skip',
      prompt: 'nope',
      after: { jobType: 'planning' }
    })
  )
  routines.save(sample({ id: 'manual', name: 'Manual', prompt: 'only click' }))
  const finished: JobRecord = {
    id: 'orig',
    type: 'review',
    prompt: 'please review this',
    provider: 'claude',
    status: 'done',
    output: 'ok',
    failedOver: []
  }
  routines.onJobFinished(finished)
  routines.onJobFinished(finished)
  expect(calls).toHaveLength(1)
  expect(calls[0]).toMatchObject({
    type: 'refactor',
    provider: undefined,
    access: 'read',
    extra: { routine: 'Follow review' }
  })
  expect(calls[0]?.prompt).toBe('clean it up\n\nThis follows the job: "please review this"')
})

test('a finished job started by a routine does not trigger another', () => {
  const { routines, calls } = makeRoutines()
  routines.save(
    sample({
      after: { jobType: 'review' }
    })
  )
  routines.onJobFinished({
    id: 'from-routine',
    type: 'review',
    prompt: 'go',
    provider: 'claude',
    status: 'done',
    output: '',
    failedOver: [],
    routine: 'Review it'
  })
  expect(calls).toEqual([])
})

test('a failed job does not trigger routines', () => {
  const { routines, calls } = makeRoutines()
  routines.save(sample({ after: { jobType: 'review' } }))
  routines.onJobFinished({
    id: 'bad',
    type: 'review',
    prompt: 'go',
    provider: 'claude',
    status: 'failed',
    output: '',
    failedOver: []
  })
  expect(calls).toEqual([])
})

test('the follow-on prompt uses the first 300 characters of the finished job', () => {
  const { routines, calls } = makeRoutines()
  routines.save(sample({ prompt: 'next', after: { jobType: 'planning' } }))
  const long = 'a'.repeat(400)
  routines.onJobFinished({
    id: 'long',
    type: 'planning',
    prompt: long,
    provider: 'cursor',
    status: 'done',
    output: '',
    failedOver: []
  })
  expect(calls[0]?.prompt).toBe(`next\n\nThis follows the job: "${'a'.repeat(300)}"`)
})

test('corrupt stored rows are skipped', () => {
  const { routines, store } = makeRoutines()
  store.saveRoutine({ id: 'bad', project: '/proj', position: 0, data: '{not-json' })
  store.saveRoutine({
    id: 'ok',
    project: '/proj',
    position: 1,
    data: JSON.stringify({
      name: 'Good',
      prompt: 'go',
      type: 'review',
      worker: 'auto',
      access: 'read'
    })
  })
  expect(routines.list().map((item) => item.id)).toEqual(['ok'])
})
