import {
  isJobAccess,
  isProviderId,
  type JobAccess,
  type JobType,
  type LeadProposal,
  type ProposedJob
} from '../types'
import type { JobRecord, Orchestrator } from '../router'
import type { BridgeTool } from './server'

const JOB_TYPES: readonly JobType[] = ['planning', 'debugging', 'review', 'refactor', 'boilerplate']
const MAX_PROMPT = 20_000
const MAX_OUTPUT = 6_000
const MAX_PROPOSED_JOBS = 4
const MAX_TITLE = 80
const MAX_SUMMARY = 600
const DEFAULT_WAIT_SECONDS = 120
const MAX_WAIT_SECONDS = 300

const JOB_TYPE_SCHEMA: Record<string, unknown> = {
  type: 'object',
  additionalProperties: false,
  required: ['type', 'prompt'],
  properties: {
    type: { type: 'string', enum: [...JOB_TYPES] },
    prompt: { type: 'string', minLength: 1, maxLength: MAX_PROMPT },
    edit: { type: 'boolean' },
    access: { type: 'string', enum: ['read', 'edit', 'full'] }
  }
}

const ACCESS_RANK: Record<JobAccess, number> = { read: 0, edit: 1, full: 2 }
const ACCESS_LABEL: Record<JobAccess, string> = {
  read: 'read only',
  edit: 'edit files',
  full: 'do anything'
}

/**
 * The lead's tools. With `propose`, the lead proposes jobs for the person to run instead of
 * sending them itself, so `send_job` is left out.
 */
export function createOrchestratorTools(
  orch: Orchestrator,
  leadTurn?: () => string | null,
  maxAccess?: () => JobAccess,
  propose?: (turn: string, proposal: LeadProposal) => void
): BridgeTool[] {
  const ceiling = (): JobAccess => {
    const value = maxAccess?.()
    return isJobAccess(value) ? value : 'read'
  }
  const tools: BridgeTool[] = [
    {
      name: 'list_workers',
      description:
        'Who can run jobs right now, how much allowance each has left, and the access ceiling the person set (read, edit, or full). A job cannot be given more than that.',
      inputSchema: { type: 'object', properties: {}, additionalProperties: false },
      async handler(args) {
        rejectUnknown(args, [])
        return {
          max_access: ceiling(),
          workers: orch.workers().map((worker) => ({
            provider: worker.id,
            available: worker.available,
            headroom: round2(worker.headroom),
            resting_until:
              worker.restingUntil === null ? null : new Date(worker.restingUntil).toISOString(),
            busy: worker.busy,
            queued: worker.queued
          }))
        }
      }
    },
    {
      name: 'send_job',
      description:
        'Hand one job to a worker and return its id immediately. The router chooses the worker. The prompt must be self-contained because the worker sees nothing else. Access is read (files only), edit (change files in a separate worktree the user reviews and merges), or full (edit plus commands, without asking). The person sets a ceiling; a request above it is lowered. Pass access as that string, or the old edit boolean (true means edit).',
      inputSchema: JOB_TYPE_SCHEMA,
      async handler(args) {
        rejectUnknown(args, ['type', 'prompt', 'edit', 'access'])
        if (!isJobType(args.type)) {
          throw new Error('type must be planning, debugging, review, refactor, or boilerplate')
        }
        if (typeof args.prompt !== 'string') throw new Error('prompt must be a non-empty string')
        if (args.prompt.length > MAX_PROMPT) {
          throw new Error('prompt must be at most 20000 characters')
        }
        if (args.prompt.trim() === '') throw new Error('prompt must be a non-empty string')
        if (args.edit !== undefined && typeof args.edit !== 'boolean') {
          throw new Error('edit must be a boolean')
        }
        if (args.access !== undefined && !isJobAccess(args.access)) {
          throw new Error('access must be read, edit, or full')
        }
        const requested = requestedAccess(args)
        const max = ceiling()
        const granted = ACCESS_RANK[requested] > ACCESS_RANK[max] ? max : requested
        const turn = leadTurn?.() ?? null
        const job = orch.submit(
          args.type,
          args.prompt,
          undefined,
          granted,
          turn !== null ? { leadMessage: turn } : undefined
        )
        const result: {
          id: string
          provider: JobRecord['provider']
          status: JobRecord['status']
          note?: string
        } = { id: job.id, provider: job.provider, status: job.status }
        if (granted !== requested) {
          result.note = `Access lowered to ${granted}: the person allows ${ACCESS_LABEL[granted]}.`
        }
        return result
      }
    },
    {
      name: 'get_status',
      description:
        'Return every job on the board, oldest first, or one job when id is set. Includes jobs the user started directly. Prompts are cut to 120 characters. Outputs are not included.',
      inputSchema: {
        type: 'object',
        additionalProperties: false,
        properties: { id: { type: 'string' } }
      },
      async handler(args) {
        rejectUnknown(args, ['id'])
        if (args.id === undefined) return orch.list().map(statusView)
        if (typeof args.id !== 'string') throw new Error('id must be a string')
        const job = orch.get(args.id)
        if (!job) throw new Error(`Unknown job id: ${args.id}`)
        return statusView(job)
      }
    },
    {
      name: 'get_result',
      description:
        'Wait until a job is done or failed, or until the wait ends, then return its status and output (last 6000 characters). If it is still queued or running, the current status is returned so the caller can ask again.',
      inputSchema: {
        type: 'object',
        additionalProperties: false,
        required: ['id'],
        properties: {
          id: { type: 'string' },
          wait_seconds: { type: 'number', minimum: 0, maximum: MAX_WAIT_SECONDS }
        }
      },
      async handler(args) {
        rejectUnknown(args, ['id', 'wait_seconds'])
        if (typeof args.id !== 'string') throw new Error('id must be a string')
        const current = orch.get(args.id)
        if (!current) throw new Error(`Unknown job id: ${args.id}`)
        const waitMs = waitMilliseconds(args.wait_seconds)
        const job =
          waitMs === 0 || isTerminal(current.status)
            ? current
            : await waitForJob(orch, args.id, waitMs)
        return resultView(job)
      }
    }
  ]
  if (propose === undefined) return tools
  const proposeTool: BridgeTool = {
    name: 'propose_jobs',
    description:
      'Propose up to 4 jobs for the person to run. Nothing starts until they press Run, and they may edit or remove jobs first. Each job needs a short title (a few words), a type, an access level (read, edit or full; above the ceiling it is lowered), and a self-contained prompt, because the worker sees nothing else. Leave worker out to let the router choose, or name claude, cursor or grok. Call it once per turn with the whole plan and a one-line summary.',
    inputSchema: {
      type: 'object',
      additionalProperties: false,
      required: ['summary', 'jobs'],
      properties: {
        summary: { type: 'string', minLength: 1, maxLength: MAX_SUMMARY },
        jobs: {
          type: 'array',
          minItems: 1,
          maxItems: MAX_PROPOSED_JOBS,
          items: {
            type: 'object',
            additionalProperties: false,
            required: ['title', 'type', 'prompt'],
            properties: {
              title: { type: 'string', minLength: 1, maxLength: MAX_TITLE },
              type: { type: 'string', enum: [...JOB_TYPES] },
              access: { type: 'string', enum: ['read', 'edit', 'full'] },
              worker: { type: 'string', enum: ['claude', 'cursor', 'grok'] },
              prompt: { type: 'string', minLength: 1, maxLength: MAX_PROMPT }
            }
          }
        }
      }
    },
    async handler(args) {
      rejectUnknown(args, ['summary', 'jobs'])
      const turn = leadTurn?.() ?? null
      if (turn === null) throw new Error('propose_jobs only works during a lead turn')
      if (typeof args.summary !== 'string' || args.summary.trim() === '') {
        throw new Error('summary must be a non-empty string')
      }
      if (!Array.isArray(args.jobs) || args.jobs.length === 0) {
        throw new Error('jobs must be a non-empty list')
      }
      if (args.jobs.length > MAX_PROPOSED_JOBS) {
        throw new Error(`propose at most ${String(MAX_PROPOSED_JOBS)} jobs`)
      }
      const max = ceiling()
      const lowered: string[] = []
      const jobs = args.jobs.map((raw: unknown, index: number): ProposedJob => {
        const job = proposedJob(raw, index)
        if (ACCESS_RANK[job.access] > ACCESS_RANK[max]) {
          lowered.push(job.title)
          return { ...job, access: max }
        }
        return job
      })
      propose(turn, { summary: args.summary.trim().slice(0, MAX_SUMMARY), jobs, status: 'waiting' })
      const result: { status: string; jobs: number; note?: string } = {
        status: 'waiting for the person to press Run',
        jobs: jobs.length
      }
      if (lowered.length > 0) {
        result.note = `Access lowered to ${max} (the person allows ${ACCESS_LABEL[max]}) for: ${lowered.join(', ')}.`
      }
      return result
    }
  }
  return tools.flatMap((tool) => (tool.name === 'send_job' ? [proposeTool] : [tool]))
}

/** A plan the person edited, checked like the lead's own proposal. */
export function proposedJobs(value: unknown): ProposedJob[] {
  if (!Array.isArray(value) || value.length === 0) throw new Error('jobs must be a non-empty list')
  if (value.length > MAX_PROPOSED_JOBS) {
    throw new Error(`propose at most ${String(MAX_PROPOSED_JOBS)} jobs`)
  }
  return value.map((raw: unknown, index: number) => proposedJob(raw, index))
}

function proposedJob(raw: unknown, index: number): ProposedJob {
  const where = `jobs[${String(index)}]`
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) {
    throw new Error(`${where} must be an object`)
  }
  const item = raw as Record<string, unknown>
  for (const key of Object.keys(item)) {
    if (!['title', 'type', 'access', 'worker', 'prompt'].includes(key)) {
      throw new Error(`${where}: unexpected argument: ${key}`)
    }
  }
  if (typeof item.title !== 'string' || item.title.trim() === '') {
    throw new Error(`${where}.title must be a non-empty string`)
  }
  if (!isJobType(item.type)) {
    throw new Error(`${where}.type must be planning, debugging, review, refactor, or boilerplate`)
  }
  if (item.access !== undefined && !isJobAccess(item.access)) {
    throw new Error(`${where}.access must be read, edit, or full`)
  }
  if (item.worker !== undefined && !isProviderId(item.worker)) {
    throw new Error(`${where}.worker must be claude, cursor, or grok`)
  }
  if (typeof item.prompt !== 'string' || item.prompt.trim() === '') {
    throw new Error(`${where}.prompt must be a non-empty string`)
  }
  if (item.prompt.length > MAX_PROMPT) {
    throw new Error(`${where}.prompt must be at most 20000 characters`)
  }
  const job: ProposedJob = {
    title: item.title.trim().slice(0, MAX_TITLE),
    type: item.type,
    access: isJobAccess(item.access) ? item.access : 'read',
    prompt: item.prompt
  }
  if (isProviderId(item.worker)) job.worker = item.worker
  return job
}

function statusView(job: JobRecord): {
  id: string
  type: JobType
  provider: JobRecord['provider']
  status: JobRecord['status']
  failed_over: JobRecord['failedOver']
  prompt: string
} {
  return {
    id: job.id,
    type: job.type,
    provider: job.provider,
    status: job.status,
    failed_over: [...job.failedOver],
    prompt: job.prompt.slice(0, 120)
  }
}

function resultView(job: JobRecord): {
  id: string
  status: JobRecord['status']
  provider: JobRecord['provider']
  failed_over: JobRecord['failedOver']
  output: string
  truncated: boolean
  edit: boolean
  change: string | null
  error?: string
} {
  const view = {
    id: job.id,
    status: job.status,
    provider: job.provider,
    failed_over: [...job.failedOver],
    output: job.output.slice(-MAX_OUTPUT),
    truncated: job.output.length > MAX_OUTPUT,
    edit: job.edit === true,
    change: job.change ?? null
  }
  if (job.error !== undefined) return { ...view, error: job.error }
  return view
}

function waitForJob(orch: Orchestrator, id: string, waitMs: number): Promise<JobRecord> {
  return new Promise((resolve, reject) => {
    let settled = false
    const timer = setTimeout(() => {
      finish(orch.get(id))
    }, waitMs)
    const unsub = orch.onUpdate((job) => {
      if (job.id !== id) return
      if (isTerminal(job.status)) finish(job)
    })

    function finish(job: JobRecord | null): void {
      if (settled) return
      settled = true
      clearTimeout(timer)
      unsub()
      if (!job) {
        reject(new Error(`Unknown job id: ${id}`))
        return
      }
      resolve(job)
    }

    const again = orch.get(id)
    if (!again || isTerminal(again.status)) finish(again)
  })
}

function waitMilliseconds(value: unknown): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) return DEFAULT_WAIT_SECONDS * 1000
  const seconds = Math.min(MAX_WAIT_SECONDS, Math.max(0, value))
  return seconds * 1000
}

function isTerminal(status: JobRecord['status']): boolean {
  return status === 'done' || status === 'failed'
}

function requestedAccess(args: Record<string, unknown>): JobAccess {
  if (isJobAccess(args.access)) return args.access
  if (args.edit === true) return 'edit'
  return 'read'
}

function isJobType(value: unknown): value is JobType {
  return typeof value === 'string' && (JOB_TYPES as readonly string[]).includes(value)
}

function rejectUnknown(args: Record<string, unknown>, allowed: readonly string[]): void {
  for (const key of Object.keys(args)) {
    if (!allowed.includes(key)) throw new Error(`unexpected argument: ${key}`)
  }
}

function round2(value: number): number {
  return Math.round(value * 100) / 100
}
