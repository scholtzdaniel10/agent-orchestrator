import type { JobType } from '../types'
import type { JobRecord, Orchestrator } from '../router'
import type { BridgeTool } from './server'

const JOB_TYPES: readonly JobType[] = ['planning', 'debugging', 'review', 'refactor', 'boilerplate']
const MAX_PROMPT = 20_000
const MAX_OUTPUT = 6_000
const DEFAULT_WAIT_SECONDS = 120
const MAX_WAIT_SECONDS = 300

const JOB_TYPE_SCHEMA: Record<string, unknown> = {
  type: 'object',
  additionalProperties: false,
  required: ['type', 'prompt'],
  properties: {
    type: { type: 'string', enum: [...JOB_TYPES] },
    prompt: { type: 'string', minLength: 1, maxLength: MAX_PROMPT },
    edit: { type: 'boolean' }
  }
}

export function createOrchestratorTools(
  orch: Orchestrator,
  leadTurn?: () => string | null
): BridgeTool[] {
  return [
    {
      name: 'list_workers',
      description: 'Who can run jobs right now and how much allowance each has left.',
      inputSchema: { type: 'object', properties: {}, additionalProperties: false },
      async handler(args) {
        rejectUnknown(args, [])
        return orch.workers().map((worker) => ({
          provider: worker.id,
          available: worker.available,
          headroom: round2(worker.headroom),
          resting_until:
            worker.restingUntil === null ? null : new Date(worker.restingUntil).toISOString(),
          busy: worker.busy,
          queued: worker.queued
        }))
      }
    },
    {
      name: 'send_job',
      description:
        'Hand one job to a worker and return its id immediately. The router chooses the worker. Workers are read-only (they can read files and answer, not edit). The prompt must be self-contained because the worker sees nothing else. Set edit to true only when the job has to change files; its changes go to a separate worktree and wait for the user to review and merge them.',
      inputSchema: JOB_TYPE_SCHEMA,
      async handler(args) {
        rejectUnknown(args, ['type', 'prompt', 'edit'])
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
        const turn = leadTurn?.() ?? null
        const job = orch.submit(
          args.type,
          args.prompt,
          undefined,
          typeof args.edit === 'boolean' ? args.edit : undefined,
          turn !== null ? { leadMessage: turn } : undefined
        )
        return { id: job.id, provider: job.provider, status: job.status }
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
