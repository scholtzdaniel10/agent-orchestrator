import { randomUUID } from 'node:crypto'

/**
 * daily: at a time each day. hourly: every few hours. ci-failed: checks fail on an open pull
 * request from the checked-out branch. new-issue: a GitHub issue is opened. main-broken: the newest
 * run on the default branch failed while it is checked out.
 */
export type TriggerKind = 'daily' | 'hourly' | 'ci-failed' | 'new-issue' | 'main-broken'

export const TRIGGER_KINDS: TriggerKind[] = [
  'daily',
  'hourly',
  'ci-failed',
  'new-issue',
  'main-broken'
]

export interface Trigger {
  id: string
  project: string
  kind: TriggerKind
  /** daily: "HH:MM" local time. */
  at: string | null
  /** hourly: every this many hours. */
  hours: number | null
  /** The goal, or for GitHub triggers extra instructions added to it. */
  text: string
  check: string | null
  maxRounds: number
  enabled: boolean
  lastFired: number | null
  /** GitHub items already seen, so each fires once. */
  seen: string[]
  /** False until the first look, which only records what already exists. */
  primed: boolean
  lastError: string | null
  createdAt: number
}

export interface PullRequestFact {
  number: number
  title: string
  branch: string
  sha: string
  failing: boolean
}

export interface IssueFact {
  number: number
  title: string
  body: string
}

export interface RunFact {
  id: number
  name: string
  status: string
  conclusion: string
}

export interface GithubFacts {
  /** The branch checked out in the project, or null when detached. */
  branch: string | null
  defaultBranch: string | null
  pulls: PullRequestFact[]
  issues: IssueFact[]
  /** Newest first, on the default branch. */
  mainRuns: RunFact[]
}

/** What a GitHub trigger would fire on: a key to remember, the goal text, and a run whose log helps. */
export interface Firing {
  key: string
  text: string
  runId: number | null
  branch: string | null
}

export function isTriggerKind(value: unknown): value is TriggerKind {
  return typeof value === 'string' && (TRIGGER_KINDS as string[]).includes(value)
}

export function isScheduled(kind: TriggerKind): boolean {
  return kind === 'daily' || kind === 'hourly'
}

/** When a scheduled trigger fires next, in epoch ms; null for GitHub triggers or a bad time. */
export function nextDue(trigger: Trigger, now: number): number | null {
  if (trigger.kind === 'hourly') {
    const hours = Math.max(1, trigger.hours ?? 24)
    const from = trigger.lastFired ?? trigger.createdAt
    return from + hours * 3_600_000
  }
  if (trigger.kind !== 'daily') return null
  const match = /^(\d{1,2}):(\d{2})$/.exec(trigger.at ?? '')
  if (match === null) return null
  const hour = Number(match[1])
  const minute = Number(match[2])
  if (hour > 23 || minute > 59) return null
  const due = new Date(now)
  due.setHours(hour, minute, 0, 0)
  // Today's time is due unless it already fired at or after it.
  if (trigger.lastFired !== null && trigger.lastFired >= due.getTime())
    due.setDate(due.getDate() + 1)
  if (trigger.lastFired === null && trigger.createdAt > due.getTime())
    due.setDate(due.getDate() + 1)
  return due.getTime()
}

export function isDue(trigger: Trigger, now: number): boolean {
  if (!trigger.enabled || !isScheduled(trigger.kind)) return false
  const due = nextDue(trigger, now)
  return due !== null && now >= due
}

/** Everything a GitHub trigger could fire on now, before removing what it has seen. */
export function githubFirings(trigger: Trigger, facts: GithubFacts): Firing[] {
  const extra = trigger.text.trim() === '' ? '' : `\n\nAlso: ${trigger.text.trim()}`
  if (trigger.kind === 'ci-failed') {
    if (facts.branch === null) return []
    return facts.pulls
      .filter((pull) => pull.failing && pull.branch === facts.branch)
      .map((pull) => ({
        key: `pr:${String(pull.number)}:${pull.sha}`,
        text: `CI fails on pull request #${String(pull.number)} "${pull.title}" (branch ${pull.branch}). Find why and make the failing checks pass. Do not weaken or skip tests.${extra}`,
        runId: null,
        branch: pull.branch
      }))
  }
  if (trigger.kind === 'new-issue') {
    return facts.issues.map((issue) => ({
      key: `issue:${String(issue.number)}`,
      text: `Resolve GitHub issue #${String(issue.number)}: ${issue.title}\n\n${issue.body.trim().slice(0, 4000)}${extra}`,
      runId: null,
      branch: null
    }))
  }
  if (trigger.kind === 'main-broken') {
    if (facts.defaultBranch === null || facts.branch !== facts.defaultBranch) return []
    const newest = facts.mainRuns.find((run) => run.status === 'completed')
    if (newest === undefined || newest.conclusion !== 'failure') return []
    return [
      {
        key: `run:${String(newest.id)}`,
        text: `The "${newest.name}" run fails on ${facts.defaultBranch}. Find why and fix it. Do not weaken or skip tests.${extra}`,
        runId: newest.id,
        branch: facts.defaultBranch
      }
    ]
  }
  return []
}

/**
 * Firings not seen before. The first look only primes: it records what exists without firing,
 * so turning on "new issue" does not start a goal for every old issue.
 */
export function freshFirings(
  trigger: Trigger,
  facts: GithubFacts
): { fire: Firing[]; seen: string[] } {
  const all = githubFirings(trigger, facts)
  const seen = new Set(trigger.seen)
  const fire = trigger.primed ? all.filter((firing) => !seen.has(firing.key)) : []
  for (const firing of all) seen.add(firing.key)
  return { fire, seen: [...seen].slice(-300) }
}

export function newTrigger(input: {
  project: string
  kind: TriggerKind
  at?: string | null
  hours?: number | null
  text?: string
  check?: string | null
  maxRounds?: number
  now: number
}): Trigger {
  const text = (input.text ?? '').trim()
  if (isScheduled(input.kind) && text === '') throw new Error('Say what the scheduled goal is.')
  if (input.kind === 'daily' && !/^\d{1,2}:\d{2}$/.test(input.at ?? '')) {
    throw new Error('Give a time such as 07:30.')
  }
  const check = (input.check ?? '').trim()
  return {
    id: randomUUID(),
    project: input.project,
    kind: input.kind,
    at: input.kind === 'daily' ? (input.at ?? null) : null,
    hours:
      input.kind === 'hourly' ? Math.min(168, Math.max(1, Math.round(input.hours ?? 24))) : null,
    text,
    check: check === '' ? null : check,
    maxRounds: Math.min(20, Math.max(1, Math.round(input.maxRounds ?? 5))),
    enabled: true,
    lastFired: null,
    seen: [],
    primed: isScheduled(input.kind),
    lastError: null,
    createdAt: input.now
  }
}

/** Reads `gh --json` output for the three GitHub facts; anything unreadable counts as none. */
export function parsePulls(json: string): PullRequestFact[] {
  const rows = parseArray(json)
  const out: PullRequestFact[] = []
  for (const row of rows) {
    if (typeof row.number !== 'number' || typeof row.headRefName !== 'string') continue
    const checks = Array.isArray(row.statusCheckRollup) ? row.statusCheckRollup : []
    const failing = checks.some((check: unknown) => {
      if (typeof check !== 'object' || check === null) return false
      const c = check as Record<string, unknown>
      const verdict = String(c.conclusion ?? c.state ?? '').toUpperCase()
      return verdict === 'FAILURE' || verdict === 'ERROR' || verdict === 'TIMED_OUT'
    })
    out.push({
      number: row.number,
      title: typeof row.title === 'string' ? row.title : '',
      branch: row.headRefName,
      sha: typeof row.headRefOid === 'string' ? row.headRefOid : '',
      failing
    })
  }
  return out
}

export function parseIssues(json: string): IssueFact[] {
  return parseArray(json).flatMap((row) =>
    typeof row.number === 'number'
      ? [
          {
            number: row.number,
            title: typeof row.title === 'string' ? row.title : '',
            body: typeof row.body === 'string' ? row.body : ''
          }
        ]
      : []
  )
}

export function parseRuns(json: string): RunFact[] {
  return parseArray(json).flatMap((row) =>
    typeof row.databaseId === 'number'
      ? [
          {
            id: row.databaseId,
            name: typeof row.name === 'string' ? row.name : 'workflow',
            status: typeof row.status === 'string' ? row.status : '',
            conclusion: typeof row.conclusion === 'string' ? row.conclusion : ''
          }
        ]
      : []
  )
}

function parseArray(json: string): Record<string, unknown>[] {
  try {
    const value: unknown = JSON.parse(json)
    return Array.isArray(value)
      ? value.filter(
          (row): row is Record<string, unknown> => typeof row === 'object' && row !== null
        )
      : []
  } catch {
    return []
  }
}
