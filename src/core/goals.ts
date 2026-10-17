import { randomUUID } from 'node:crypto'
import type { JobRecord } from './router'
import { isJobAccess, isJobType, type JobAccess, type JobType } from './types'

/**
 * planning: the lead is reading the code and proposing a round. waiting: a proposal or a question
 * waits for the person. running: the round's jobs are working. checking: the check command runs.
 */
export type GoalStatus =
  'planning' | 'waiting' | 'running' | 'checking' | 'done' | 'stopped' | 'failed'

export interface PlannedJob {
  type: JobType
  access: JobAccess
  prompt: string
}

export interface GoalCheck {
  code: number | null
  /** The end of what the command printed. */
  output: string
  at: number
}

export interface GoalRound {
  n: number
  summary: string
  jobs: PlannedJob[]
  jobIds: string[]
  /** Jobs that failed, or whose changes could not be brought into the goal, with why. */
  problems: string[]
  check: GoalCheck | null
}

export interface Goal {
  id: string
  project: string
  text: string
  /** Shell command that passes when the goal is met; null leaves it to the lead. */
  check: string | null
  maxRounds: number
  /** A small task: the person's own prompt is round 1, with no planning first. */
  quick: boolean
  /** Worktree holding the goal's work; every round's changes are committed there. */
  worktree: string
  status: GoalStatus
  rounds: GoalRound[]
  /** The next round the lead proposes, waiting for the person's approval. */
  proposal: { summary: string; jobs: PlannedJob[] } | null
  question: string | null
  /** What the person told the lead between rounds. */
  notes: string[]
  /** How it ended, or why it is waiting. */
  outcome: string | null
  createdAt: number
  updatedAt: number
}

export interface GoalWorktrees {
  create(project: string, id: string): Promise<{ id: string; path: string; branch: string }>
  folder(id: string): string
  branchOf(id: string): string
  applyInto(
    fromId: string,
    target: string,
    base: string
  ): Promise<{ ok: boolean; empty: boolean; message: string }>
  commitAll(dir: string, message: string): Promise<boolean>
  discard(project: string, id: string): Promise<void>
}

export interface GoalStore {
  saveGoal(id: string, project: string, updatedAt: number, data: string): void
  goals(project: string, limit?: number): string[]
  goal(id: string): string | null
  deleteGoal(id: string): void
}

export interface GoalsDeps {
  store: GoalStore
  worktrees: GoalWorktrees
  submit: (job: PlannedJob, extra: { goal: string; base: string; project: string }) => JobRecord
  job: (id: string) => JobRecord | null
  cancel: (id: string) => void
  /** Asks the lead's plan to read the goal's folder and answer `prompt`; resolves to its text. */
  plan: (goal: Goal, folder: string, prompt: string) => Promise<string>
  runCheck: (command: string, folder: string) => Promise<{ code: number | null; output: string }>
  /** The most a job may be given, as the person set it for the lead. */
  ceiling: () => JobAccess
  /** Names the goal's worktree in the sidebar and Changes. */
  nameWorktree?: (id: string, project: string, name: string) => void
  onUpdate?: (goal: Goal) => void
  now?: () => number
}

export const MAX_JOBS_PER_ROUND = 3
const ACCESS_RANK: Record<JobAccess, number> = { read: 0, edit: 1, full: 2 }
const OUTPUT_TAIL = 6000

export class Goals {
  private readonly deps: GoalsDeps
  private readonly now: () => number
  private readonly finishing = new Set<string>()

  constructor(deps: GoalsDeps) {
    this.deps = deps
    this.now = deps.now ?? Date.now
  }

  list(project: string): Goal[] {
    return this.deps.store
      .goals(project)
      .map(parseGoal)
      .filter((goal): goal is Goal => goal !== null)
  }

  get(id: string): Goal | null {
    const raw = this.deps.store.goal(id)
    return raw === null ? null : parseGoal(raw)
  }

  /**
   * A new goal. Big ones start with the lead planning round 1; a quick one runs the person's own
   * prompt as round 1 straight away.
   */
  async create(input: {
    project: string
    text: string
    check?: string | null
    maxRounds?: number
    quick?: { type: JobType; access: JobAccess }
  }): Promise<Goal> {
    const text = input.text.trim()
    if (text === '') throw new Error('Say what the goal is.')
    const id = randomUUID()
    const created = await this.deps.worktrees.create(input.project, id)
    const now = this.now()
    const check = input.check?.trim() ?? ''
    const goal: Goal = {
      id,
      project: input.project,
      text,
      check: check === '' ? null : check,
      maxRounds: clampRounds(input.maxRounds ?? 5),
      quick: input.quick !== undefined,
      worktree: created.id,
      status: 'planning',
      rounds: [],
      proposal: null,
      question: null,
      notes: [],
      outcome: null,
      createdAt: now,
      updatedAt: now
    }
    this.deps.nameWorktree?.(created.id, input.project, `Goal: ${firstLine(text, 40)}`)
    if (input.quick !== undefined) {
      goal.status = 'waiting'
      goal.proposal = {
        summary: 'Your task',
        jobs: [{ type: input.quick.type, access: input.quick.access, prompt: text }]
      }
      this.save(goal)
      return this.approve(id)
    }
    this.save(goal)
    void this.plan(id)
    return goal
  }

  /** Runs the proposed round, with the person's edits when given. */
  approve(id: string, jobs?: PlannedJob[]): Goal {
    const goal = this.require(id)
    if (goal.status !== 'waiting' || goal.proposal === null) {
      throw new Error('There is no proposed round to approve.')
    }
    const chosen = (jobs ?? goal.proposal.jobs).filter((job) => job.prompt.trim() !== '')
    if (chosen.length === 0) throw new Error('A round needs at least one job.')
    const ceiling = this.deps.ceiling()
    const base = this.deps.worktrees.branchOf(goal.worktree)
    const round: GoalRound = {
      n: goal.rounds.length + 1,
      summary: goal.proposal.summary,
      jobs: chosen,
      jobIds: [],
      problems: [],
      check: null
    }
    goal.rounds.push(round)
    goal.proposal = null
    goal.status = 'running'
    goal.outcome = null
    this.save(goal)
    for (const planned of chosen) {
      const access = ACCESS_RANK[planned.access] > ACCESS_RANK[ceiling] ? ceiling : planned.access
      const record = this.deps.submit(
        { ...planned, access: access === 'read' ? 'edit' : access },
        { goal: goal.id, base, project: goal.project }
      )
      round.jobIds.push(record.id)
    }
    this.save(goal)
    // Jobs that failed at once (a plan signed out, say) never send another update.
    this.onJob(null, goal.id)
    return goal
  }

  /** The person's answer to the lead's question, or a note; the lead plans again with it. */
  answer(id: string, text: string): Goal {
    const goal = this.require(id)
    if (goal.status === 'planning' || goal.status === 'running' || goal.status === 'checking') {
      throw new Error('Wait for this round to finish.')
    }
    const note = text.trim()
    if (note !== '') goal.notes.push(note)
    goal.question = null
    goal.proposal = null
    if (goal.status !== 'waiting') goal.maxRounds = Math.max(goal.maxRounds, goal.rounds.length + 1)
    this.save(goal)
    void this.plan(id)
    return goal
  }

  stop(id: string): Goal {
    const goal = this.require(id)
    const round = goal.rounds[goal.rounds.length - 1]
    if (goal.status === 'running' && round !== undefined) {
      for (const jobId of round.jobIds) {
        const job = this.deps.job(jobId)
        if (job !== null && (job.status === 'queued' || job.status === 'running')) {
          this.deps.cancel(jobId)
        }
      }
    }
    goal.status = 'stopped'
    goal.proposal = null
    goal.outcome = 'Stopped by you. The work so far is in Changes.'
    this.save(goal)
    return goal
  }

  /** Forget a finished or stopped goal and remove its worktree. */
  async remove(id: string): Promise<void> {
    const goal = this.require(id)
    if (goal.status === 'planning' || goal.status === 'running' || goal.status === 'checking') {
      throw new Error('Stop the goal first.')
    }
    this.deps.store.deleteGoal(id)
    try {
      await this.deps.worktrees.discard(goal.project, goal.worktree)
    } catch {
      // Already merged or discarded from Changes.
    }
  }

  /** Called on every job update; a goal moves on once all of its round's jobs have ended. */
  onJob(job: JobRecord | null, goalId?: string): void {
    const id = goalId ?? job?.goal
    if (id === undefined) return
    const goal = this.get(id)
    if (goal === null || goal.status !== 'running') return
    const round = goal.rounds[goal.rounds.length - 1]
    if (round === undefined) return
    if (job !== null && !round.jobIds.includes(job.id)) return
    const ended = round.jobIds.every((jobId) => {
      const record = this.deps.job(jobId)
      return record === null || record.status === 'done' || record.status === 'failed'
    })
    if (ended) void this.finishRound(id)
  }

  /** After a restart: carry on with goals the app was in the middle of. */
  resume(project: string): void {
    for (const goal of this.list(project)) {
      if (goal.status === 'running') {
        this.onJob(null, goal.id)
      } else if (goal.status === 'planning' || goal.status === 'checking') {
        goal.status = 'waiting'
        goal.question = 'Legate closed while this goal was working. Plan the next round again?'
        this.save(goal)
      }
    }
  }

  private async plan(id: string): Promise<void> {
    const goal = this.require(id)
    goal.status = 'planning'
    goal.question = null
    this.save(goal)
    let text: string
    try {
      text = await this.deps.plan(goal, this.deps.worktrees.folder(goal.worktree), planPrompt(goal))
    } catch (err) {
      const fresh = this.require(id)
      fresh.status = 'waiting'
      fresh.question = `The lead could not plan: ${errorText(err)}. Try again?`
      this.save(fresh)
      return
    }
    const fresh = this.require(id)
    if (fresh.status !== 'planning') return
    const parsed = parsePlan(text)
    if (parsed === null) {
      fresh.status = 'waiting'
      fresh.question = `The lead's answer did not include a plan: "${firstLine(text, 160)}". Try again, or tell it what to do.`
    } else if (parsed.decision === 'done') {
      fresh.status = 'done'
      fresh.outcome = parsed.summary || 'The lead judges the goal met. Review it in Changes.'
    } else if (parsed.decision === 'ask') {
      fresh.status = 'waiting'
      fresh.question = parsed.question
    } else {
      fresh.status = 'waiting'
      fresh.proposal = { summary: parsed.summary, jobs: parsed.jobs }
    }
    this.save(fresh)
  }

  private async finishRound(id: string): Promise<void> {
    if (this.finishing.has(id)) return
    this.finishing.add(id)
    try {
      const goal = this.require(id)
      if (goal.status !== 'running') return
      const round = goal.rounds[goal.rounds.length - 1]
      if (round === undefined) return
      const folder = this.deps.worktrees.folder(goal.worktree)
      const base = this.deps.worktrees.branchOf(goal.worktree)
      round.problems = []
      for (const jobId of round.jobIds) {
        const job = this.deps.job(jobId)
        const label = firstLine(job?.prompt ?? jobId, 60)
        if (job === null || job.status === 'failed') {
          round.problems.push(`${label}: ${job?.error ?? 'the job is gone'}`)
        }
        if (job === null || job.change === undefined) continue
        if (job.status === 'done') {
          try {
            const result = await this.deps.worktrees.applyInto(job.change, folder, base)
            if (!result.ok) round.problems.push(`${label}: ${result.message}`)
          } catch (err) {
            round.problems.push(`${label}: ${errorText(err)}`)
          }
        }
        try {
          await this.deps.worktrees.discard(goal.project, job.change)
        } catch {
          // Its changes are in the goal already, or it is gone.
        }
      }
      await this.deps.worktrees.commitAll(folder, `Goal round ${String(round.n)}: ${round.summary}`)

      if (goal.check !== null) {
        goal.status = 'checking'
        this.save(goal)
        let result: { code: number | null; output: string }
        try {
          result = await this.deps.runCheck(goal.check, folder)
        } catch (err) {
          result = { code: null, output: errorText(err) }
        }
        round.check = { code: result.code, output: tail(result.output), at: this.now() }
      }
      const planNext = this.decide(goal, round)
      this.save(goal)
      if (planNext) void this.plan(id)
    } finally {
      this.finishing.delete(id)
    }
  }

  /** After a round: done, stop at a limit, or plan the next one (true). */
  private decide(goal: Goal, round: GoalRound): boolean {
    if (round.check !== null && round.check.code === 0) {
      goal.status = 'done'
      goal.outcome = `The check passed after round ${String(round.n)}. Review the goal in Changes.`
      return false
    }
    if (goal.quick && goal.check === null) {
      goal.status = 'done'
      goal.outcome = 'Done. Review it in Changes.'
      return false
    }
    const previous = goal.rounds[goal.rounds.length - 2]
    if (
      round.check !== null &&
      previous?.check !== null &&
      previous?.check !== undefined &&
      previous.check.code !== 0 &&
      failureKey(previous.check.output) === failureKey(round.check.output)
    ) {
      goal.status = 'waiting'
      goal.question =
        'The check failed the same way twice. Tell the lead what to try, or stop the goal.'
      return false
    }
    if (goal.rounds.length >= goal.maxRounds) {
      goal.status = 'stopped'
      goal.outcome = `Reached the limit of ${String(goal.maxRounds)} rounds. The work so far is in Changes; answer to give it more.`
      return false
    }
    goal.status = 'planning'
    return true
  }

  private require(id: string): Goal {
    const goal = this.get(id)
    if (goal === null) throw new Error('That goal is gone.')
    return goal
  }

  private save(goal: Goal): void {
    goal.updatedAt = this.now()
    this.deps.store.saveGoal(goal.id, goal.project, goal.updatedAt, JSON.stringify(goal))
    this.deps.onUpdate?.(goal)
  }
}

/** What the lead is asked before each round. */
export function planPrompt(goal: Goal): string {
  const lines: string[] = [
    "You plan the next round of work towards a goal in Legate. This folder is the goal's own worktree with every earlier round committed. Read the code as you need; do not change anything.",
    '',
    `Goal: ${goal.text}`,
    goal.check !== null
      ? `Done when this command exits 0 in this folder: ${goal.check}`
      : 'Done when you judge the goal met.',
    `This is round ${String(goal.rounds.length + 1)} of at most ${String(goal.maxRounds)}.`
  ]
  if (goal.rounds.length > 0) {
    lines.push('', 'Earlier rounds:')
    for (const round of goal.rounds) {
      lines.push(`Round ${String(round.n)}: ${round.summary}`)
      for (const job of round.jobs)
        lines.push(`- ${job.type}, ${job.access}: ${firstLine(job.prompt, 200)}`)
      for (const problem of round.problems) lines.push(`  problem: ${problem}`)
      if (round.check !== null) {
        lines.push(
          `  check exited ${String(round.check.code)}:`,
          indent(lastChars(round.check.output, 1500))
        )
      }
    }
  }
  if (goal.notes.length > 0) {
    lines.push('', 'From the person:')
    for (const note of goal.notes) lines.push(`- ${note}`)
  }
  lines.push(
    '',
    'Answer with exactly one JSON block, one of:',
    '```json',
    '{"decision": "jobs", "summary": "<one line: what this round does>", "jobs": [{"type": "refactor", "access": "edit", "prompt": "<what to do>"}]}',
    '{"decision": "done", "summary": "<why the goal is met>"}',
    '{"decision": "ask", "question": "<what you need from the person>"}',
    '```',
    `Rules: at most ${String(MAX_JOBS_PER_ROUND)} jobs; jobs in one round must not edit the same files; each prompt stands alone (the worker sees nothing else) and names the files; keep rounds small; type is planning, debugging, review, refactor or boilerplate; access is edit, or full only when the job must run commands such as tests.`
  )
  return lines.join('\n')
}

export type ParsedPlan =
  | { decision: 'jobs'; summary: string; jobs: PlannedJob[] }
  | { decision: 'done'; summary: string }
  | { decision: 'ask'; question: string }

/** The lead's JSON answer, from a fenced block or the last object in the text; null if unusable. */
export function parsePlan(text: string): ParsedPlan | null {
  const fenced = [...text.matchAll(/```(?:json)?\s*([\s\S]*?)```/g)].map((match) => match[1] ?? '')
  const candidates =
    fenced.length > 0
      ? fenced.reverse()
      : [text.slice(text.indexOf('{'), text.lastIndexOf('}') + 1)]
  for (const candidate of candidates) {
    let value: unknown
    try {
      value = JSON.parse(candidate.trim())
    } catch {
      continue
    }
    const plan = readPlan(value)
    if (plan !== null) return plan
  }
  return null
}

function readPlan(value: unknown): ParsedPlan | null {
  if (typeof value !== 'object' || value === null) return null
  const record = value as Record<string, unknown>
  const summary = typeof record.summary === 'string' ? record.summary.trim() : ''
  if (record.decision === 'done') return { decision: 'done', summary }
  if (record.decision === 'ask') {
    const question = typeof record.question === 'string' ? record.question.trim() : ''
    return question === '' ? null : { decision: 'ask', question }
  }
  if (record.decision !== 'jobs' || !Array.isArray(record.jobs)) return null
  const jobs: PlannedJob[] = []
  for (const raw of record.jobs) {
    if (typeof raw !== 'object' || raw === null) continue
    const job = raw as Record<string, unknown>
    const prompt = typeof job.prompt === 'string' ? job.prompt.trim() : ''
    if (prompt === '') continue
    jobs.push({
      type: typeof job.type === 'string' && isJobType(job.type) ? job.type : 'refactor',
      access:
        typeof job.access === 'string' && isJobAccess(job.access) && job.access !== 'read'
          ? job.access
          : 'edit',
      prompt
    })
  }
  if (jobs.length === 0) return null
  return {
    decision: 'jobs',
    summary: summary || 'Next round',
    jobs: jobs.slice(0, MAX_JOBS_PER_ROUND)
  }
}

/** Two failures are the same when their last lines match, ignoring numbers such as timings. */
export function failureKey(output: string): string {
  return output
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line !== '')
    .slice(-15)
    .join('\n')
    .replace(/\d+(\.\d+)?/g, '#')
    .toLowerCase()
}

function parseGoal(raw: string): Goal | null {
  try {
    const value = JSON.parse(raw) as Goal
    return typeof value.id === 'string' && Array.isArray(value.rounds) ? value : null
  } catch {
    return null
  }
}

function clampRounds(value: number): number {
  if (!Number.isFinite(value)) return 5
  return Math.min(20, Math.max(1, Math.round(value)))
}

function firstLine(text: string, max: number): string {
  const line = text.trim().split(/\r?\n/)[0] ?? ''
  return line.length > max ? `${line.slice(0, max - 1)}…` : line
}

function tail(text: string): string {
  return lastChars(text, OUTPUT_TAIL)
}

function lastChars(text: string, max: number): string {
  return text.length > max ? `…${text.slice(-max)}` : text
}

function indent(text: string): string {
  return text
    .split(/\r?\n/)
    .map((line) => `    ${line}`)
    .join('\n')
}

function errorText(err: unknown): string {
  return err instanceof Error ? err.message : String(err)
}
