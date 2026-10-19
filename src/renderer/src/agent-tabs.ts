import type { StepTool } from '../../core/types'

type JobRecord = Awaited<ReturnType<Window['api']['listJobs']>>[number]

const TOOL_WORD: Record<StepTool, string> = {
  say: 'SAY',
  think: 'THINK',
  read: 'READ',
  edit: 'EDIT',
  shell: 'RUN',
  search: 'SEARCH',
  web: 'WEB',
  mcp: 'MCP',
  other: 'TOOL'
}

export function stepWord(tool: StepTool): string {
  return TOOL_WORD[tool]
}

export function agentStatusLabel(job: JobRecord): string {
  if (job.status === 'queued') return 'waiting'
  if (job.status === 'running') return 'working'
  if (job.status === 'failed') return job.error === 'stopped' ? 'stopped' : 'failed'
  return 'done'
}

/**
 * Jobs shown as agent tabs in the middle: the ones started from a lead plan in this window
 * (`opened`, in that order), then any other plan job that is still waiting or working, so a
 * restart still shows them. A tab the person closed stays closed.
 */
export function agentJobs(
  jobs: readonly JobRecord[],
  opened: readonly string[],
  closed: ReadonlySet<string>
): JobRecord[] {
  const byId = new Map(jobs.map((job) => [job.id, job]))
  const shown: JobRecord[] = []
  const seen = new Set<string>()
  for (const id of opened) {
    const job = byId.get(id)
    if (job === undefined || closed.has(id) || seen.has(id)) continue
    shown.push(job)
    seen.add(id)
  }
  for (const job of jobs) {
    if (seen.has(job.id) || closed.has(job.id)) continue
    if (job.title === undefined || job.leadMessage === undefined) continue
    if (job.status !== 'queued' && job.status !== 'running') continue
    shown.push(job)
    seen.add(job.id)
  }
  return shown
}

export function agentTabLabel(job: JobRecord): string {
  return job.title ?? job.prompt.slice(0, 40)
}
