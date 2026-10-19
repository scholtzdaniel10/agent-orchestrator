import { randomUUID } from 'node:crypto'
import { basename } from 'node:path'
import { mergeListedChanges } from '../../core/changes'
import type { JobRecord, WorkerInfo } from '../../core/router/orchestrator'
import type { Routine } from '../../core/routines'
import {
  accessOf,
  type ChangeSet,
  type JobAccess,
  type JobType,
  type ProviderId
} from '../../core/types'
import { oneSentence, planShort } from '../format'
import {
  initialState,
  reduce,
  type Action,
  type Effect,
  type JobItem,
  type RoutineItem,
  type TuiState,
  type WorktreeItem
} from './state'

export interface AgentSpec {
  command: string
  args: string[]
  env?: NodeJS.ProcessEnv
}

export interface TuiHost {
  project: string
  now: () => number
  color: boolean
  columns: () => number
  rows: () => number
  orch: {
    init: () => Promise<void>
    workers: () => WorkerInfo[]
    submit: (type: JobType, prompt: string, provider?: ProviderId, access?: JobAccess) => JobRecord
    cancel: (id: string) => JobRecord
    onUpdate: (cb: (job: JobRecord) => void) => () => void
  }
  store: {
    jobsNewest: (project: string, limit: number) => { createdAt: number; job: JobRecord }[]
    worktreeNames: (project: string) => { id: string; name: string; created_at: number }[]
    setWorktreeName: (row: {
      id: string
      project: string
      name: string
      created_at: number
    }) => void
    deleteWorktreeName: (id: string) => void
  }
  worktrees: {
    info: (project: string) => Promise<{ path: string; isRepo: boolean; branch: string | null }>
    list: (project: string) => Promise<ChangeSet[]>
    ids: (project: string) => Promise<string[]>
    folder: (id: string) => string
    diff: (project: string, id: string) => Promise<string>
    merge: (project: string, id: string) => Promise<{ ok: boolean; message: string }>
    discard: (project: string, id: string) => Promise<void>
    create: (
      project: string,
      jobId: string
    ) => Promise<{ id: string; path: string; branch: string }>
  }
  routines: {
    list: () => Routine[]
    run: (id: string) => JobRecord
    onJobFinished: (job: JobRecord) => void
  }
  interactive: (provider: ProviderId) => AgentSpec
  spawnInherited: (spec: AgentSpec, cwd: string) => Promise<void>
}

export interface Controls {
  suspend: (run: () => Promise<void>) => Promise<void>
  exit: () => void
}

export interface SessionView {
  state: TuiState
  dispatch: (action: Action) => void
  bind: (controls: Controls) => void
}

export interface SessionMount {
  update: (view: SessionView) => void
  unmount: () => void
}

const JOBS = 100
const POLL_MS = 2_000

export async function runSession(
  host: TuiHost,
  mount: (view: SessionView) => SessionMount
): Promise<number> {
  let state = initialState({
    projectName: basename(host.project),
    columns: host.columns(),
    rows: host.rows(),
    color: host.color,
    now: host.now()
  })
  const controls: Controls = {
    suspend: async (run) => {
      await run()
    },
    exit: () => {}
  }
  let alive = true
  let unsub = (): void => {}
  let timer: ReturnType<typeof setInterval> | undefined
  let mounted: SessionMount | null = null
  let finish: (code: number) => void = () => {}
  const closed = new Promise<number>((resolve) => {
    finish = resolve
  })

  const bind = (next: Controls): void => {
    controls.suspend = next.suspend
    controls.exit = next.exit
  }
  const view = (): SessionView => ({
    state,
    dispatch,
    bind
  })

  let draining = false
  const pending: Action[] = []

  function dispatch(action: Action): void {
    if (!alive) return
    pending.push(action)
    if (draining) return
    draining = true
    try {
      while (pending.length > 0 && alive) {
        const next = pending.shift()
        if (next === undefined) break
        const result = reduce(state, next)
        state = result.state
        mounted?.update(view())
        for (const effect of result.effects) void runEffect(effect)
      }
    } finally {
      draining = false
    }
  }

  async function runEffect(effect: Effect): Promise<void> {
    try {
      if (effect.type === 'quit') {
        for (const id of effect.stop) {
          try {
            host.orch.cancel(id)
          } catch {
            // The job may already have finished.
          }
        }
        alive = false
        controls.exit()
        finish(0)
        return
      }
      if (effect.type === 'submit') {
        const provider = effect.worker === 'auto' ? undefined : effect.worker
        const job = host.orch.submit(effect.jobType, effect.prompt, provider, effect.access)
        dispatch({ type: 'submitted', job: jobItem(job, host.now(), true) })
        return
      }
      if (effect.type === 'cancel') {
        dispatch({ type: 'job', job: jobItem(host.orch.cancel(effect.id), host.now(), false) })
        return
      }
      if (effect.type === 'run-routine') {
        const job = host.routines.run(effect.id)
        dispatch({ type: 'submitted', job: jobItem(job, host.now(), true) })
        return
      }
      if (effect.type === 'load-diff') {
        const text = await host.worktrees.diff(host.project, effect.id)
        dispatch({ type: 'diff', id: effect.id, text })
        return
      }
      if (effect.type === 'load-changes') {
        await reloadLists()
        return
      }
      if (effect.type === 'merge') {
        const result = await host.worktrees.merge(host.project, effect.id)
        if (result.ok) host.store.deleteWorktreeName(effect.id)
        else dispatch({ type: 'error', message: result.message })
        await reloadLists()
        return
      }
      if (effect.type === 'discard') {
        await host.worktrees.discard(host.project, effect.id)
        host.store.deleteWorktreeName(effect.id)
        await reloadLists()
        return
      }
      if (effect.type === 'create-worktree') {
        const about = await host.worktrees.info(host.project)
        if (!about.isRepo) {
          dispatch({
            type: 'error',
            message: 'This folder is not a git repository, so it cannot have worktrees.'
          })
          return
        }
        const created = await host.worktrees.create(host.project, randomUUID())
        host.store.setWorktreeName({
          id: created.id,
          project: host.project,
          name: effect.name,
          created_at: host.now()
        })
        await reloadLists()
        dispatch({ type: 'select-worktree', id: created.id })
        return
      }
      if (effect.type === 'open-agent') {
        const spec = host.interactive(effect.provider)
        await controls.suspend(async () => {
          await host.spawnInherited(spec, effect.cwd)
        })
        await reloadLists()
      }
    } catch (err) {
      dispatch({ type: 'error', message: oneSentence(err) })
    }
  }

  async function reloadLists(): Promise<void> {
    if (!alive) return
    const now = host.now()
    dispatch({ type: 'plans', plans: host.orch.workers().map((worker) => planItem(worker, now)) })
    dispatch({ type: 'worktrees', ...(await worktreeSnapshot(host)) })
    dispatch({
      type: 'jobs',
      jobs: host.store
        .jobsNewest(host.project, JOBS)
        .map((row) => jobItem(row.job, row.createdAt, false))
    })
    dispatch({ type: 'routines', routines: host.routines.list().map(routineItem) })
    dispatch({ type: 'tick', now })
  }

  mounted = mount(view())
  try {
    await host.orch.init()
    unsub = host.orch.onUpdate((job) => {
      const prev = state.jobs.find((item) => item.id === job.id)
      if (job.status === 'done' && prev?.status !== 'done') host.routines.onJobFinished(job)
      const known = state.jobs.find((item) => item.id === job.id)
      dispatch({
        type: 'job',
        job: jobItem(job, known?.createdAt ?? host.now(), false)
      })
    })
    await reloadLists()
    timer = setInterval(() => {
      void reloadLists()
    }, POLL_MS)
    timer.unref()
    return await closed
  } finally {
    alive = false
    if (timer !== undefined) clearInterval(timer)
    unsub()
    mounted.unmount()
  }
}

async function worktreeSnapshot(host: TuiHost): Promise<{
  project: WorktreeItem
  items: WorktreeItem[]
  branch: string | null
}> {
  const info = await host.worktrees.info(host.project)
  const listed = await host.worktrees.list(host.project)
  const ids = await host.worktrees.ids(host.project)
  const changes = mergeListedChanges({
    listed,
    ids,
    names: host.store.worktreeNames(host.project),
    folder: (id) => host.worktrees.folder(id)
  })
  return {
    branch: info.branch,
    project: {
      id: '',
      project: true,
      label: basename(info.path !== '' ? info.path : host.project),
      branch: info.branch,
      added: 0,
      removed: 0,
      files: [],
      path: info.path !== '' ? info.path : host.project
    },
    items: changes.map(worktreeItem)
  }
}

function worktreeItem(change: ChangeSet): WorktreeItem {
  const named = change.name !== undefined && change.name !== ''
  return {
    id: change.id,
    project: false,
    label: named ? (change.name ?? change.id) : change.id,
    branch: change.branch,
    added: change.insertions,
    removed: change.deletions,
    files: change.files.map((file) => ({
      path: file.path,
      added: file.insertions,
      removed: file.deletions
    })),
    path: change.path
  }
}

export function jobItem(job: JobRecord, createdAt: number, startedHere: boolean): JobItem {
  const edits = editsFromSteps(job.steps ?? [])
  return {
    id: job.id,
    status: job.status,
    provider: job.provider,
    type: job.type,
    access: accessOf(job),
    model: job.model ?? '',
    reason: job.reason ?? '',
    prompt: job.prompt,
    output: job.output,
    error: job.error ?? '',
    steps: job.steps ?? [],
    createdAt,
    startedHere,
    change: job.change ?? null,
    files: edits.files,
    added: edits.added,
    removed: edits.removed
  }
}

function editsFromSteps(steps: JobRecord['steps']): {
  files: number
  added: number
  removed: number
} {
  const paths = new Set<string>()
  let added = 0
  let removed = 0
  for (const step of steps ?? []) {
    if (step.edit === undefined) continue
    paths.add(step.edit.path)
    added += step.edit.added
    removed += step.edit.removed
  }
  return { files: paths.size, added, removed }
}

function planItem(worker: WorkerInfo, now: number): { id: string; text: string } {
  return { id: worker.id, text: planShort(worker, now) }
}

function routineItem(routine: Routine): RoutineItem {
  return {
    id: routine.id,
    name: routine.name,
    type: routine.type,
    worker: routine.worker,
    access: routine.access
  }
}
