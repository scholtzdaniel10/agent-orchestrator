import { randomUUID } from 'node:crypto'
import type { JobRecord } from './router/orchestrator'
import type { RoutineRow } from './router/store'
import { isJobAccess, isJobType, type JobAccess, type JobType, type ProviderId } from './types'

const NAME_MAX = 40
const PROMPT_MAX = 8_000
const ID_MAX = 64
const PER_PROJECT = 30
const FOLLOW_PROMPT = 300

export type RoutineWorker = 'auto' | ProviderId

export interface Routine {
  id: string
  name: string
  prompt: string
  type: JobType
  worker: RoutineWorker
  access: JobAccess
  after?: { jobType: JobType }
}

export interface RoutineStore {
  listRoutines(project: string): RoutineRow[]
  saveRoutine(row: RoutineRow): void
  deleteRoutine(id: string): void
}

export type SubmitJob = (
  type: JobType,
  prompt: string,
  provider?: ProviderId,
  access?: JobAccess,
  extra?: { group?: string; leadMessage?: string; routine?: string }
) => JobRecord

export class Routines {
  private readonly store: RoutineStore
  private readonly submitJob: SubmitJob
  private readonly project: string | (() => string)
  private readonly id: () => string
  private readonly seenDone = new Set<string>()

  constructor(opts: {
    store: RoutineStore
    submit: SubmitJob
    project: string | (() => string)
    id?: () => string
  }) {
    this.store = opts.store
    this.submitJob = opts.submit
    this.project = opts.project
    this.id = opts.id ?? randomUUID
  }

  list(): Routine[] {
    const out: Routine[] = []
    for (const row of this.store.listRoutines(this.projectDir())) {
      const parsed = routineFromRow(row)
      if (parsed !== null) out.push(parsed)
    }
    return out
  }

  save(input: unknown): Routine {
    const routine = parseRoutine(input, this.id)
    const project = this.projectDir()
    const rows = this.store.listRoutines(project)
    const existing = rows.find((row) => row.id === routine.id)
    if (existing === undefined && rows.length >= PER_PROJECT) {
      throw new Error('This project already has 30 routines. Delete one before adding another.')
    }
    const position =
      existing !== undefined
        ? existing.position
        : rows.reduce((max, row) => Math.max(max, row.position), -1) + 1
    this.store.saveRoutine({
      id: routine.id,
      project,
      position,
      data: serializeRoutine(routine)
    })
    return routine
  }

  remove(id: string): void {
    if (typeof id !== 'string' || id === '' || id.length > ID_MAX) {
      throw new Error('invalid routine id')
    }
    this.store.deleteRoutine(id)
  }

  run(id: string): JobRecord {
    const routine = this.list().find((item) => item.id === id)
    if (routine === undefined) {
      throw new Error('That routine is gone. Refresh the list and try again.')
    }
    return this.submitRoutine(routine, routine.prompt)
  }

  onJobFinished(job: JobRecord): void {
    if (job.status !== 'done') return
    if (job.routine !== undefined) return
    if (this.seenDone.has(job.id)) return
    this.seenDone.add(job.id)
    const snippet = job.prompt.slice(0, FOLLOW_PROMPT)
    const suffix = `\n\nThis follows the job: "${snippet}"`
    for (const routine of this.list()) {
      if (routine.after?.jobType !== job.type) continue
      try {
        this.submitRoutine(routine, `${routine.prompt}${suffix}`)
      } catch (err) {
        console.error(err)
      }
    }
  }

  private submitRoutine(routine: Routine, prompt: string): JobRecord {
    const provider = routine.worker === 'auto' ? undefined : routine.worker
    return this.submitJob(routine.type, prompt, provider, routine.access, {
      routine: routine.name
    })
  }

  private projectDir(): string {
    return typeof this.project === 'function' ? this.project() : this.project
  }
}

export function parseRoutine(input: unknown, makeId: () => string = randomUUID): Routine {
  if (typeof input !== 'object' || input === null) throw new Error('invalid routine')
  const row = input as Record<string, unknown>
  let id = row.id
  if (id === undefined || id === null || id === '') id = makeId()
  if (typeof id !== 'string' || id.length < 1 || id.length > ID_MAX) {
    throw new Error('invalid routine id')
  }
  if (typeof row.name !== 'string' || /[\r\n]/.test(row.name)) {
    throw new Error('Name must be 1 to 40 characters on one line.')
  }
  const name = row.name.trim()
  if (name.length < 1 || name.length > NAME_MAX) {
    throw new Error('Name must be 1 to 40 characters on one line.')
  }
  if (typeof row.prompt !== 'string') {
    throw new Error('Prompt must be 1 to 8000 characters.')
  }
  const prompt = row.prompt
  if (prompt.length < 1 || prompt.length > PROMPT_MAX) {
    throw new Error('Prompt must be 1 to 8000 characters.')
  }
  if (!isJobType(row.type)) throw new Error('unknown job type')
  const worker = row.worker
  if (worker !== 'auto' && worker !== 'claude' && worker !== 'cursor') {
    throw new Error('worker must be auto, claude, or cursor')
  }
  if (!isJobAccess(row.access)) throw new Error('access must be read, edit, or full')
  const routine: Routine = {
    id,
    name,
    prompt,
    type: row.type,
    worker,
    access: row.access
  }
  if (row.after !== undefined && row.after !== null) {
    if (typeof row.after !== 'object') throw new Error('invalid after')
    const jobType = (row.after as { jobType?: unknown }).jobType
    if (!isJobType(jobType)) throw new Error('unknown job type')
    routine.after = { jobType }
  }
  return routine
}

function serializeRoutine(routine: Routine): string {
  const data: Record<string, unknown> = {
    name: routine.name,
    prompt: routine.prompt,
    type: routine.type,
    worker: routine.worker,
    access: routine.access
  }
  if (routine.after !== undefined) data.after = routine.after
  return JSON.stringify(data)
}

function routineFromRow(row: RoutineRow): Routine | null {
  try {
    const parsed: unknown = JSON.parse(row.data)
    if (typeof parsed !== 'object' || parsed === null) return null
    return parseRoutine({ ...(parsed as Record<string, unknown>), id: row.id })
  } catch {
    return null
  }
}
