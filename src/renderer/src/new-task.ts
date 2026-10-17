/** Pure decisions for the New task dialog: which call to make, validation, and labels. */

export type Who = 'claude' | 'cursor'
export type How = 'terminal' | 'background' | 'goal'
export type JobKind = 'debugging' | 'refactor' | 'boilerplate' | 'planning' | 'review'

export const WHO_OPTIONS: readonly { id: Who; label: string }[] = [
  { id: 'cursor', label: 'Cursor' },
  { id: 'claude', label: 'Claude' }
]

export const HOW_OPTIONS: readonly { id: How; label: string; detail: string }[] = [
  {
    id: 'terminal',
    label: 'In a terminal',
    detail: 'Starts the agent in its own worktree. You watch and talk to it.'
  },
  {
    id: 'background',
    label: 'In the background',
    detail: 'Runs as a job in its own worktree. You review the change.'
  },
  {
    id: 'goal',
    label: 'As a goal',
    detail: 'The lead plans rounds and asks you before each one.'
  }
]

export const JOB_KINDS: readonly JobKind[] = [
  'debugging',
  'refactor',
  'boilerplate',
  'planning',
  'review'
]

export const DEFAULT_WHO: Who = 'cursor'
export const DEFAULT_HOW: How = 'terminal'
export const DEFAULT_KIND: JobKind = 'refactor'
export const DEFAULT_COUNT = 1
export const DEFAULT_ROUNDS = 5
export const COUNT_MIN = 1
export const COUNT_MAX = 6
export const ROUNDS_MIN = 1
export const ROUNDS_MAX = 20

export interface NewTaskDraft {
  text: string
  who: Who
  how: How
  count: number
  kind: JobKind
  check: string
  maxRounds: number
}

export interface NewTaskContext {
  isRepo: boolean
  claudeAvailable: boolean
  cursorAvailable: boolean
}

/** Fields passed to `createGoal`. A check on a background job sets the quick fields. */
export interface NewTaskGoalInput {
  text: string
  check: string
  maxRounds?: number
  quickType?: JobKind
  quickAccess?: 'edit'
}

export type NewTaskPlan =
  | { ok: false; error: string }
  | { ok: true; action: 'terminal'; text: string; who: Who; count: number }
  | {
      ok: true
      action: 'job'
      jobType: JobKind
      text: string
      who: Who
      access: 'read' | 'edit'
    }
  | { ok: true; action: 'goal'; input: NewTaskGoalInput; created: 'goal' }

export function providerReady(who: Who, ctx: NewTaskContext): boolean {
  return who === 'claude' ? ctx.claudeAvailable : ctx.cursorAvailable
}

export function unavailableHint(ready: boolean): string | null {
  return ready ? null : 'Not ready'
}

export function howEnabled(how: How, isRepo: boolean): boolean {
  return isRepo || how === 'background'
}

/** Outside a git repo the only way to start work is a read-only background job. */
export function activeHow(how: How, isRepo: boolean): How {
  return howEnabled(how, isRepo) ? how : 'background'
}

export function repoHint(isRepo: boolean): string | null {
  return isRepo ? null : 'Worktrees need a git repository.'
}

export function submitLabel(how: How): string {
  if (how === 'terminal') return 'Start agent'
  if (how === 'background') return 'Run job'
  return 'Start goal'
}

export function landingLine(isRepo: boolean): string {
  if (!isRepo) return 'This job only reads the folder. It does not change files.'
  return 'Pick it in the sidebar, then Review to merge.'
}

export function clampCount(value: number): number {
  if (!Number.isFinite(value)) return DEFAULT_COUNT
  return Math.min(COUNT_MAX, Math.max(COUNT_MIN, Math.round(value)))
}

export function clampRounds(value: number): number {
  if (!Number.isFinite(value)) return DEFAULT_ROUNDS
  return Math.min(ROUNDS_MAX, Math.max(ROUNDS_MIN, Math.round(value)))
}

export function asKind(value: string): JobKind {
  for (const kind of JOB_KINDS) {
    if (kind === value) return kind
  }
  return DEFAULT_KIND
}

function providerName(who: Who): string {
  return who === 'cursor' ? 'Cursor' : 'Claude'
}

export function planNewTask(draft: NewTaskDraft, ctx: NewTaskContext): NewTaskPlan {
  const text = draft.text.trim()
  if (text === '') return { ok: false, error: 'Write what should be done.' }
  if (!providerReady(draft.who, ctx)) {
    return {
      ok: false,
      error: `${providerName(draft.who)} is not ready. Pick a plan that is, or set it up and try again.`
    }
  }

  const how = activeHow(draft.how, ctx.isRepo)
  const check = draft.check.trim()
  if (!ctx.isRepo) {
    return {
      ok: true,
      action: 'job',
      jobType: draft.kind,
      text,
      who: draft.who,
      access: 'read'
    }
  }
  if (how === 'terminal') {
    return { ok: true, action: 'terminal', text, who: draft.who, count: clampCount(draft.count) }
  }
  if (how === 'background') {
    if (check === '') {
      return {
        ok: true,
        action: 'job',
        jobType: draft.kind,
        text,
        who: draft.who,
        access: 'edit'
      }
    }
    return {
      ok: true,
      action: 'goal',
      created: 'goal',
      input: { text, check, quickType: draft.kind, quickAccess: 'edit' }
    }
  }
  return {
    ok: true,
    action: 'goal',
    created: 'goal',
    input: { text, check, maxRounds: clampRounds(draft.maxRounds) }
  }
}
