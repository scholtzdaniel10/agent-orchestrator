/** Live heartbeat: a foreign owner is still around if it beat within this window. */
export const JOB_STALE_MS = 20_000

export const WORKTREE_BUSY_MESSAGE =
  'A job is still running in this worktree. Stop it first, or wait for it to finish.'

export interface GuardJob {
  status: string
  change?: string
  ownerId?: string
  heartbeatAt?: number
}

/** True when a queued or running job (this process or a foreign live one) uses the worktree. */
export function worktreeHasActiveJob(
  jobs: readonly GuardJob[],
  changeId: string,
  now: number
): boolean {
  return jobs.some((job) => job.change === changeId && jobIsLive(job, now))
}

export function assertWorktreeIdle(jobs: readonly GuardJob[], changeId: string, now: number): void {
  if (worktreeHasActiveJob(jobs, changeId, now)) {
    throw new Error(WORKTREE_BUSY_MESSAGE)
  }
}

function jobIsLive(job: GuardJob, now: number): boolean {
  if (job.status !== 'queued' && job.status !== 'running') return false
  if (job.ownerId === undefined || job.heartbeatAt === undefined) return true
  return now - job.heartbeatAt < JOB_STALE_MS
}
