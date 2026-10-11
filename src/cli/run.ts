import { randomUUID } from 'node:crypto'
import { mergeListedChanges } from '../core/changes'
import type { JobRecord, JobStep, WorkerInfo } from '../core/router/orchestrator'
import type { Routine } from '../core/routines'
import type { WorktreeNameRow } from '../core/router/store'
import {
  accessOf,
  isJobAccess,
  isJobType,
  type ChangeSet,
  type JobAccess,
  type JobType,
  type ProjectInfo,
  type ProviderId
} from '../core/types'
import { TESTED_VERSIONS } from '../core/providers/versions'
import { parseArgs, type Parsed } from './args'
import {
  changeCells,
  changeFooter,
  footerLine,
  headerLine,
  jobCells,
  oneSentence,
  planLine,
  stepLine,
  table
} from './format'
import { earlyExit, usageFor } from './help'
import { resolveId } from './ids'

const LOOKUP = 1000

export class ImmediateExit extends Error {
  readonly code: number

  constructor(code: number) {
    super('exit')
    this.name = 'ImmediateExit'
    this.code = code
  }
}

export interface CliContext {
  version: string
  project: string
  now: () => number
  color: boolean
  /** True when both stdin and stdout are terminals. */
  tty: boolean
  termSize: () => { columns: number; rows: number }
  interactive: (provider: 'claude' | 'cursor') => {
    command: string
    args: string[]
    env?: NodeJS.ProcessEnv
  }
  spawnInherited: (
    spec: { command: string; args: string[]; env?: NodeJS.ProcessEnv },
    cwd: string
  ) => Promise<void>
  stdout: (text: string) => void
  stderr: (text: string) => void
  stdin: {
    isTTY: boolean
    read: () => Promise<string>
    ask: (prompt: string) => Promise<string>
  }
  onSigint: (handler: () => void) => void
  exitNow: (code: number) => void
  orch: {
    init: () => Promise<void>
    workers: () => WorkerInfo[]
    submit: (
      type: JobType,
      prompt: string,
      provider?: ProviderId,
      access?: JobAccess | boolean,
      extra?: { group?: string; leadMessage?: string; routine?: string }
    ) => JobRecord
    cancel: (id: string) => JobRecord
    onUpdate: (cb: (job: JobRecord) => void) => () => void
  }
  store: {
    jobsNewest: (project: string, limit: number) => { createdAt: number; job: JobRecord }[]
    worktreeNames: (project: string) => WorktreeNameRow[]
    setWorktreeName: (row: WorktreeNameRow) => void
    deleteWorktreeName: (id: string) => void
  }
  worktrees: {
    info: (project: string) => Promise<ProjectInfo>
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
    onJobFinished?: (job: JobRecord) => void
  }
  github: {
    openPr: (
      project: string,
      title: string,
      body: string
    ) => Promise<{ url: string; branch: string }>
  }
}

interface RunFlags {
  json: boolean
  quiet: boolean
  verbose: boolean
}

export async function run(argv: string[], ctx: CliContext): Promise<number> {
  try {
    const early = earlyExit(argv, ctx.version, ctx.tty)
    if (early !== null) {
      ;(early.stream === 'stdout' ? ctx.stdout : ctx.stderr)(early.text)
      return early.code
    }
    const parsed = parseArgs(argv)
    if (!parsed.ok) {
      ctx.stderr(`${parsed.message}\n${usageFor(parsed.command)}\n`)
      return 2
    }
    return await dispatch(parsed.parsed, ctx)
  } catch (err) {
    if (err instanceof ImmediateExit) return err.code
    ctx.stderr(`${oneSentence(err)}\n`)
    return 1
  }
}

async function dispatch(parsed: Parsed, ctx: CliContext): Promise<number> {
  const command = parsed.command
  if (command === null || command === 'ui') {
    if (!ctx.tty) {
      ctx.stderr(`Missing command.\n${usageFor(null)}\n`)
      return 2
    }
    if (unexpected(parsed.args, 'ui', ctx)) return 2
    const { openTui } = await import('./tui/open')
    return openTui(ctx)
  }
  if (command === 'status') {
    if (unexpected(parsed.args, 'status', ctx)) return 2
    return status(ctx)
  }
  if (command === 'job') return submitJob(parsed, ctx)
  if (command === 'jobs') return listJobs(parsed, ctx)
  if (command === 'show') return showJob(parsed, ctx)
  if (command === 'changes') {
    if (unexpected(parsed.args, 'changes', ctx)) return 2
    return printChanges(ctx)
  }
  if (command === 'diff') return diffChange(parsed, ctx)
  if (command === 'merge') return mergeChange(parsed, ctx)
  if (command === 'discard') return discardChange(parsed, ctx)
  if (command === 'pr') return openPr(parsed, ctx)
  if (command === 'worktree') return worktree(parsed, ctx)
  if (command === 'routine') return routine(parsed, ctx)
  ctx.stderr(`Unknown command "${command ?? ''}".\n${usageFor(null)}\n`)
  return 2
}

async function status(ctx: CliContext): Promise<number> {
  await ctx.orch.init()
  const now = ctx.now()
  const lines = ctx.orch
    .workers()
    .map((worker) => planLine(worker, TESTED_VERSIONS[worker.id], now, ctx.color))
  lines.push(ctx.project)
  const active = ctx.store
    .jobsNewest(ctx.project, LOOKUP)
    .filter((row) => row.job.status === 'running' || row.job.status === 'queued')
  if (active.length > 0) {
    lines.push(
      table(
        active.map((row) =>
          jobCells({
            id: row.job.id,
            status: row.job.status,
            provider: row.job.provider,
            type: row.job.type,
            access: accessOf(row.job),
            ageMs: now - row.createdAt,
            prompt: row.job.prompt
          })
        )
      )
    )
  }
  ctx.stdout(`${lines.join('\n')}\n`)
  return 0
}

async function submitJob(parsed: Parsed, ctx: CliContext): Promise<number> {
  if (parsed.args.length > 1) {
    unexpected(parsed.args.slice(1), 'job', ctx)
    return 2
  }
  const type = parsed.flags.type ?? 'planning'
  if (!isJobType(type)) {
    ctx.stderr(`Unknown job type "${type}".\n${usageFor('job')}\n`)
    return 2
  }
  const worker = parseWorker(parsed.flags.worker)
  if (worker === null) {
    ctx.stderr(`Worker must be auto, claude, or cursor.\n${usageFor('job')}\n`)
    return 2
  }
  const access = parsed.flags.access ?? 'read'
  if (!isJobAccess(access)) {
    ctx.stderr(`Access must be read, edit, or full.\n${usageFor('job')}\n`)
    return 2
  }
  const prompt = await readPrompt(ctx, parsed.args[0])
  if (prompt === null) {
    ctx.stderr(`Missing prompt.\n${usageFor('job')}\n`)
    return 2
  }
  if (prompt.trim() === '') {
    ctx.stderr(`Empty prompt.\n${usageFor('job')}\n`)
    return 2
  }
  await ctx.orch.init()
  return follow(ctx, () => ctx.orch.submit(type, prompt.trim(), worker, access), runFlags(parsed))
}

async function listJobs(parsed: Parsed, ctx: CliContext): Promise<number> {
  if (unexpected(parsed.args, 'jobs', ctx)) return 2
  const limit = parseLimit(parsed.flags.limit)
  if (limit === null) {
    ctx.stderr(`Limit must be a positive whole number.\n${usageFor('jobs')}\n`)
    return 2
  }
  const now = ctx.now()
  const rows = ctx.store.jobsNewest(ctx.project, limit)
  if (rows.length === 0) return 0
  ctx.stdout(
    `${table(
      rows.map((row) =>
        jobCells({
          id: row.job.id,
          status: row.job.status,
          provider: row.job.provider,
          type: row.job.type,
          access: accessOf(row.job),
          ageMs: now - row.createdAt,
          prompt: row.job.prompt
        })
      )
    )}\n`
  )
  return 0
}

async function showJob(parsed: Parsed, ctx: CliContext): Promise<number> {
  const token = parsed.args[0]
  if (token === undefined) {
    ctx.stderr(`Missing job id.\n${usageFor('show')}\n`)
    return 2
  }
  if (unexpected(parsed.args.slice(1), 'show', ctx)) return 2
  const rows = ctx.store.jobsNewest(ctx.project, LOOKUP)
  const hit = resolveId(
    token,
    rows.map((row) => ({ id: row.job.id })),
    'Job'
  )
  if (!hit.ok) {
    ctx.stderr(`${hit.message}\n`)
    return 2
  }
  const row = rows.find((item) => item.job.id === hit.id)
  if (row === undefined) {
    ctx.stderr(`No job matches "${token}".\n`)
    return 2
  }
  const job = row.job
  const lines = [headerLine(job, ctx.color)]
  for (const step of job.steps ?? []) {
    const line = stepLine(step, true, ctx.color)
    if (line !== null) lines.push(line)
  }
  if (job.output !== '') lines.push(job.output)
  if (job.status === 'failed') lines.push(`failed: ${job.error ?? 'failed'}`)
  ctx.stdout(`${lines.join('\n')}\n`)
  return 0
}

async function printChanges(ctx: CliContext): Promise<number> {
  const changes = await listChanges(ctx)
  if (changes.length === 0) return 0
  ctx.stdout(`${table(changes.map(changeRow))}\n`)
  return 0
}

async function diffChange(parsed: Parsed, ctx: CliContext): Promise<number> {
  const hit = await oneChange(parsed, 'diff', ctx)
  if (typeof hit !== 'string') return hit
  const text = await ctx.worktrees.diff(ctx.project, hit)
  ctx.stdout(text === '' || text.endsWith('\n') ? text : `${text}\n`)
  return 0
}

async function mergeChange(parsed: Parsed, ctx: CliContext): Promise<number> {
  const hit = await oneChange(parsed, 'merge', ctx)
  if (typeof hit !== 'string') return hit
  const result = await ctx.worktrees.merge(ctx.project, hit)
  if (result.ok) ctx.store.deleteWorktreeName(hit)
  ;(result.ok ? ctx.stdout : ctx.stderr)(`${result.message}\n`)
  return result.ok ? 0 : 1
}

async function discardChange(parsed: Parsed, ctx: CliContext): Promise<number> {
  const hit = await oneChange(parsed, 'discard', ctx)
  if (typeof hit !== 'string') return hit
  if (!parsed.flags.yes) {
    if (!ctx.stdin.isTTY) {
      ctx.stderr('Refusing to discard without --yes when input is not a terminal.\n')
      return 2
    }
    const answer = (await ctx.stdin.ask(`Discard ${hit}? [y/N] `)).trim().toLowerCase()
    if (answer !== 'y' && answer !== 'yes') {
      ctx.stdout(`Kept ${hit}.\n`)
      return 0
    }
  }
  await ctx.worktrees.discard(ctx.project, hit)
  ctx.store.deleteWorktreeName(hit)
  ctx.stdout(`Discarded ${hit}.\n`)
  return 0
}

async function openPr(parsed: Parsed, ctx: CliContext): Promise<number> {
  const title = parsed.args[0]
  if (title === undefined) {
    ctx.stderr(`Missing title.\n${usageFor('pr')}\n`)
    return 2
  }
  if (unexpected(parsed.args.slice(1), 'pr', ctx)) return 2
  if (title.trim() === '' || title.trim().length > 120 || /[\r\n]/.test(title)) {
    ctx.stderr(`Title must be 1 to 120 characters on one line.\n${usageFor('pr')}\n`)
    return 2
  }
  const body = parsed.flags.body ?? ''
  if (body.length > 4000) {
    ctx.stderr(`Description must be at most 4000 characters.\n${usageFor('pr')}\n`)
    return 2
  }
  const result = await ctx.github.openPr(ctx.project, title.trim(), body)
  ctx.stdout(`${result.url}\n`)
  return 0
}

async function worktree(parsed: Parsed, ctx: CliContext): Promise<number> {
  if (parsed.sub === 'list') {
    if (unexpected(parsed.args, 'worktree list', ctx)) return 2
    return printChanges(ctx)
  }
  if (parsed.sub === 'path') return worktreePath(parsed, ctx)
  if (parsed.sub === 'create') return worktreeCreate(parsed, ctx)
  ctx.stderr(`Unknown command "worktree ${parsed.sub ?? ''}".\n${usageFor('worktree')}\n`)
  return 2
}

async function worktreePath(parsed: Parsed, ctx: CliContext): Promise<number> {
  const hit = await oneChange(parsed, 'worktree path', ctx)
  if (typeof hit !== 'string') return hit
  ctx.stdout(`${ctx.worktrees.folder(hit)}\n`)
  return 0
}

async function worktreeCreate(parsed: Parsed, ctx: CliContext): Promise<number> {
  if (parsed.args.length > 1) {
    unexpected(parsed.args.slice(1), 'worktree create', ctx)
    return 2
  }
  const given = parsed.args[0]
  let label = ''
  if (given !== undefined) {
    const parsedName = worktreeName(given)
    if (parsedName === null) {
      ctx.stderr(`Name must be 1 to 40 characters on one line.\n${usageFor('worktree create')}\n`)
      return 2
    }
    label = parsedName
  }
  const about = await ctx.worktrees.info(ctx.project)
  if (!about.isRepo) {
    ctx.stderr('This folder is not a git repository, so it cannot have worktrees.\n')
    return 1
  }
  const created = await ctx.worktrees.create(ctx.project, randomUUID())
  ctx.store.setWorktreeName({
    id: created.id,
    project: ctx.project,
    name: label,
    created_at: ctx.now()
  })
  ctx.stdout(`${created.id}  ${created.path}\n`)
  return 0
}

async function routine(parsed: Parsed, ctx: CliContext): Promise<number> {
  if (parsed.sub === 'list') {
    if (unexpected(parsed.args, 'routine list', ctx)) return 2
    const rows = ctx.routines.list().map((item) => [item.name, item.type, item.worker, item.access])
    if (rows.length === 0) return 0
    ctx.stdout(`${table(rows)}\n`)
    return 0
  }
  if (parsed.sub === 'run') return routineRun(parsed, ctx)
  ctx.stderr(`Unknown command "routine ${parsed.sub ?? ''}".\n${usageFor('routine')}\n`)
  return 2
}

async function routineRun(parsed: Parsed, ctx: CliContext): Promise<number> {
  const name = parsed.args[0]
  if (name === undefined || name.trim() === '') {
    ctx.stderr(`Missing routine name.\n${usageFor('routine run')}\n`)
    return 2
  }
  if (unexpected(parsed.args.slice(1), 'routine run', ctx)) return 2
  const matches = ctx.routines.list().filter((item) => item.name === name)
  if (matches.length === 0) {
    ctx.stderr(`No routine named "${name}".\n`)
    return 2
  }
  if (matches.length > 1) {
    ctx.stderr(`Routine name "${name}" matches ${matches.map((item) => item.id).join(' and ')}.\n`)
    return 2
  }
  const routineId = matches[0]?.id
  if (routineId === undefined) {
    ctx.stderr(`No routine named "${name}".\n`)
    return 2
  }
  await ctx.orch.init()
  return follow(ctx, () => ctx.routines.run(routineId), runFlags(parsed))
}

async function follow(ctx: CliContext, begin: () => JobRecord, flags: RunFlags): Promise<number> {
  const color = ctx.color && !flags.json
  const started = ctx.now()
  let currentId: string | null = null
  let latest: JobRecord | null = null
  let headerDone = false
  let settled = false
  let sigints = 0
  const printed = new Set<string>()
  let resolveDone: (job: JobRecord) => void = () => {}
  let rejectDone: (err: unknown) => void = () => {}
  const done = new Promise<JobRecord>((resolve, reject) => {
    resolveDone = resolve
    rejectDone = reject
  })

  const render = (job: JobRecord): void => {
    if (flags.quiet || flags.json) return
    const pending = (job.steps ?? []).filter((step) => {
      if (printed.has(step.id)) return false
      return stepLine(step, flags.verbose, false) !== null
    })
    const terminal = job.status === 'done' || job.status === 'failed'
    if (!headerDone && (job.model !== undefined || terminal || pending.length > 0)) {
      headerDone = true
      ctx.stdout(`${headerLine(job, color)}\n`)
    }
    for (const step of pending) {
      printed.add(step.id)
      const line = stepLine(step, flags.verbose, color)
      if (line !== null) ctx.stdout(`${line}\n`)
    }
  }

  const handle = (job: JobRecord): void => {
    if (currentId === null || job.id !== currentId || settled) return
    latest = job
    render(job)
    if (job.status === 'done' || job.status === 'failed') {
      settled = true
      resolveDone(job)
    }
  }

  const pending: JobRecord[] = []
  const unsub = ctx.orch.onUpdate((job) => {
    if (currentId === null) pending.push(job)
    else handle(job)
  })

  ctx.onSigint(() => {
    sigints += 1
    if (sigints >= 2 || currentId === null) {
      settled = true
      rejectDone(new ImmediateExit(130))
      ctx.exitNow(130)
      return
    }
    try {
      ctx.orch.cancel(currentId)
    } catch {
      settled = true
      rejectDone(new ImmediateExit(130))
      ctx.exitNow(130)
    }
  })

  try {
    const job = begin()
    currentId = job.id
    for (const item of pending) handle(item)
    handle(job)
    const final = settled && latest !== null ? latest : await done
    const elapsed = ctx.now() - started
    if (flags.json) {
      ctx.stdout(`${JSON.stringify(jsonJob(final))}\n`)
    } else {
      await finishText(ctx, final, flags, color, elapsed)
    }
    if (sigints > 0) return 130
    return final.status === 'done' ? 0 : 1
  } finally {
    unsub()
  }
}

async function finishText(
  ctx: CliContext,
  job: JobRecord,
  flags: RunFlags,
  color: boolean,
  elapsed: number
): Promise<void> {
  if (flags.quiet) {
    writeBlock(ctx, job.output)
    if (job.status === 'failed') ctx.stderr(`${footerLine('failed', elapsed, job.error, false)}\n`)
    return
  }
  if (job.status !== 'done' && job.status !== 'failed') return
  writeBlock(ctx, job.output)
  ctx.stdout(`${footerLine(job.status, elapsed, job.error, color)}\n`)
  if (accessOf(job) !== 'read' && job.change !== undefined) {
    const changes = await listChanges(ctx)
    const found = changes.find((change) => change.id === job.change)
    ctx.stdout(
      `${changeFooter(
        job.change,
        found?.files.length ?? 0,
        found?.insertions ?? 0,
        found?.deletions ?? 0
      )}\n`
    )
  }
}

function jsonJob(job: JobRecord): {
  id: string
  status: JobRecord['status']
  provider: JobRecord['provider']
  model: string | null
  access: JobAccess
  output: string
  error: string | null
  change: string | null
  steps: JobStep[]
} {
  return {
    id: job.id,
    status: job.status,
    provider: job.provider,
    model: job.model ?? null,
    access: accessOf(job),
    output: job.output,
    error: job.error ?? null,
    change: job.change ?? null,
    steps: job.steps ?? []
  }
}

async function listChanges(ctx: CliContext): Promise<ChangeSet[]> {
  const listed = await ctx.worktrees.list(ctx.project)
  const ids = await ctx.worktrees.ids(ctx.project)
  return mergeListedChanges({
    listed,
    ids,
    names: ctx.store.worktreeNames(ctx.project),
    folder: (id) => ctx.worktrees.folder(id)
  })
}

async function oneChange(parsed: Parsed, key: string, ctx: CliContext): Promise<string | number> {
  const token = parsed.args[0]
  if (token === undefined) {
    const missing = key === 'worktree path' ? 'Missing change id or name.' : 'Missing change id.'
    ctx.stderr(`${missing}\n${usageFor(key)}\n`)
    return 2
  }
  if (unexpected(parsed.args.slice(1), key, ctx)) return 2
  const changes = await listChanges(ctx)
  const hit = resolveId(
    token,
    changes.map((change) => ({ id: change.id, name: change.name })),
    'Change'
  )
  if (!hit.ok) {
    ctx.stderr(`${hit.message}\n`)
    return 2
  }
  return hit.id
}

async function readPrompt(ctx: CliContext, arg: string | undefined): Promise<string | null> {
  if (arg !== undefined && arg !== '-') return arg
  if (arg === undefined && ctx.stdin.isTTY) return null
  return ctx.stdin.read()
}

function parseWorker(value: string | undefined): ProviderId | undefined | null {
  if (value === undefined || value === 'auto') return undefined
  if (value === 'claude' || value === 'cursor') return value
  return null
}

function parseLimit(value: string | undefined): number | null {
  if (value === undefined) return 20
  if (!/^[1-9]\d{0,8}$/.test(value)) return null
  return Number(value)
}

function worktreeName(value: string): string | null {
  if (/[\r\n]/.test(value)) return null
  const trimmed = value.trim()
  if (trimmed.length < 1 || trimmed.length > 40) return null
  return trimmed
}

function runFlags(parsed: Parsed): RunFlags {
  return {
    json: parsed.flags.json,
    quiet: parsed.flags.quiet,
    verbose: parsed.flags.verbose
  }
}

function unexpected(args: string[], key: string, ctx: CliContext): boolean {
  const arg = args[0]
  if (arg === undefined) return false
  ctx.stderr(`Unexpected argument "${arg}".\n${usageFor(key)}\n`)
  return true
}

function writeBlock(ctx: CliContext, text: string): void {
  if (text === '') return
  ctx.stdout(text.endsWith('\n') ? text : `${text}\n`)
}

function changeRow(change: ChangeSet): string[] {
  return changeCells({
    id: change.id,
    name: change.name,
    files: change.files.length,
    insertions: change.insertions,
    deletions: change.deletions
  })
}
