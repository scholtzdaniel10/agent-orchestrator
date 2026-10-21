import { readFileSync } from 'node:fs'
import {
  PROVIDERS,
  planOf,
  type JobType,
  type PaceRules,
  type ProviderId,
  type RouterRules
} from '../types'
import rulesJson from './rules.json'
import type { RunRow, Store, StoredWindow } from './store'

const JOB_TYPES: readonly JobType[] = ['planning', 'debugging', 'review', 'refactor', 'boilerplate']

/**
 * A reading the person copied from their plan's dashboard (Cursor reports no usage itself).
 * Legate's own runs since the reading are added on top until the next one.
 */
export const DASHBOARD_WINDOW = 'dashboard'

const WINDOW_MS: Record<string, number> = {
  five_hour: 5 * 3600_000,
  seven_day: 168 * 3600_000,
  [DASHBOARD_WINDOW]: 30 * 24 * 3600_000
}

export const DEFAULT_PACE: PaceRules = {
  weight: 1.5,
  min: 0.5,
  max: 2,
  atRiskSlack: 0.25,
  atRiskLeft: 0.15
}

/** Jobs one plan may run at once when rules omit or mis-set maxParallel. */
export const DEFAULT_MAX_PARALLEL = 3
const MAX_PARALLEL_MIN = 1
const MAX_PARALLEL_MAX = 8

/** Bundled copy of rules.json. `loadRules()` with no path returns this. */
export const defaultRules: RouterRules = validate(rulesJson)

export interface WindowState {
  name: string
  used: number
  resetsAt: number | null
  lengthMs: number | null
}

export interface PlanUsage {
  /** What the plan reports; empty when only an estimate exists. */
  windows: WindowState[]
  /** Worst window, 0..1. */
  used: number
  /** Reset time of that worst window. */
  resetsAt: number | null
  /** Share of the pace window elapsed minus share of it used; null when unknown. */
  slack: number | null
  /** 1 when unknown. */
  paceFactor: number
  atRisk: boolean
}

export interface Score {
  provider: ProviderId
  fit: number
  headroom: number
  paceFactor: number
  atRisk: boolean
  score: number
}

/**
 * Load router settings. With no path, returns the JSON imported next to this
 * module so the rules still exist after electron-vite bundles the file away
 * from `import.meta.url`.
 */
export function loadRules(path?: string): RouterRules {
  if (path === undefined) return defaultRules
  return validate(JSON.parse(readFileSync(path, 'utf8')) as unknown)
}

/** Fraction of the provider's allowance still unused, in [0, 1]. */
export function headroom(
  provider: ProviderId,
  store: Store,
  rules: RouterRules,
  now: number
): number {
  const resting = store.restingUntil(planOf(provider))
  if (resting !== null && resting > now) return 0
  return clamp01(1 - planUsage(provider, store, rules, now).used)
}

/**
 * Allowance windows the plan has reported, or the rolling weight estimate
 * when it has reported none.
 */
export function planUsage(
  provider: ProviderId,
  store: Store,
  rules: RouterRules,
  now: number
): PlanUsage {
  const plan = planOf(provider)
  const stored = store.windows(plan)
  if (stored.length === 0) {
    const allowance = rules.allowance[plan]
    const since = now - allowance.windowHours * 3600_000
    return {
      windows: [],
      used: clamp01(weightUsed(runsOnPlan(store, plan, since), allowance.units)),
      resetsAt: null,
      slack: null,
      paceFactor: 1,
      atRisk: false
    }
  }

  const windows = stored.map((row) =>
    row.name === DASHBOARD_WINDOW
      ? dashboardWindow(row, store, plan, rules.allowance[plan].units, now)
      : toWindow(row, now)
  )
  let worst = windows[0]
  for (let i = 1; i < windows.length; i++) {
    const window = windows[i]
    if (
      window.used > worst.used ||
      (window.used === worst.used && laterReset(window.resetsAt, worst.resetsAt))
    ) {
      worst = window
    }
  }

  const paceWindow = longestOpen(windows)
  const pace = rules.pace ?? DEFAULT_PACE
  if (paceWindow?.lengthMs == null || paceWindow.resetsAt === null) {
    return {
      windows,
      used: worst.used,
      resetsAt: worst.resetsAt,
      slack: null,
      paceFactor: 1,
      atRisk: false
    }
  }

  const left = clamp01((paceWindow.resetsAt - now) / paceWindow.lengthMs)
  const slack = 1 - left - paceWindow.used
  const paceFactor = clamp(1 + pace.weight * slack, pace.min, pace.max)
  const atRisk = slack >= pace.atRiskSlack && left <= pace.atRiskLeft && 1 - worst.used > 0.1
  return {
    windows,
    used: worst.used,
    resetsAt: worst.resetsAt,
    slack,
    paceFactor,
    atRisk
  }
}

/**
 * One score per candidate listed for this job type and not excluded, in rule
 * order. Fit is 1 for the first rule entry and for a plan at risk of wasting
 * allowance, otherwise `fallbackFit`. An at-risk plan uses `pace.max`.
 */
export function scoreProviders(
  type: JobType,
  candidates: ProviderId[],
  store: Store,
  rules: RouterRules,
  now: number,
  exclude: ProviderId[] = []
): Score[] {
  const order = rules.rules[type]
  const pace = rules.pace ?? DEFAULT_PACE
  const scores: Score[] = []
  for (let i = 0; i < order.length; i++) {
    const provider = order[i]
    if (!candidates.includes(provider) || exclude.includes(provider)) continue
    const usage = planUsage(provider, store, rules, now)
    const atRisk = usage.atRisk
    const fit = i === 0 || atRisk ? 1 : rules.fallbackFit
    const paceFactor = atRisk ? pace.max : usage.paceFactor
    const room = headroom(provider, store, rules, now)
    scores.push({
      provider,
      fit,
      headroom: room,
      paceFactor,
      atRisk,
      score: fit * room * paceFactor
    })
  }
  return scores
}

/**
 * Highest score among providers that are candidates, listed for this job type,
 * and not excluded. Ties keep the earlier rule entry. All-zero scores and an
 * empty field return null.
 */
export function pickProvider(
  type: JobType,
  candidates: ProviderId[],
  store: Store,
  rules: RouterRules,
  now: number,
  exclude: ProviderId[] = [],
  running: Partial<Record<ProviderId, number>> = {}
): ProviderId | null {
  return pickWithReason(type, candidates, store, rules, now, exclude, running)?.provider ?? null
}

/** Same choice as `pickProvider`, plus why that plan won. */
export function pickWithReason(
  type: JobType,
  candidates: ProviderId[],
  store: Store,
  rules: RouterRules,
  now: number,
  exclude: ProviderId[] = [],
  /** Jobs already running on each plan; used to skip a plan at its parallel limit. */
  running: Partial<Record<ProviderId, number>> = {}
): { provider: ProviderId; reason: string } | null {
  const scores = scoreProviders(type, candidates, store, rules, now, exclude)
  let best: Score | null = null
  for (const row of scores) {
    if (row.score > (best?.score ?? 0)) best = row
  }
  if (!best) return null
  let winner = best
  const limit = rules.maxParallel
  if (jobsOnPlan(running, winner.provider) >= limit) {
    for (const row of scores) {
      if (row.provider === winner.provider || row.score <= 0) continue
      if (jobsOnPlan(running, row.provider) >= limit) continue
      if (row.headroom <= 0) continue
      winner = row
      break
    }
  }
  return { provider: winner.provider, reason: pickReason(type, winner, scores, rules) }
}

/** Jobs already running on the plan this worker spends, across every worker that shares it. */
function jobsOnPlan(running: Partial<Record<ProviderId, number>>, provider: ProviderId): number {
  const plan = planOf(provider)
  let total = 0
  for (const id of PROVIDERS) {
    if (planOf(id) === plan) total += running[id] ?? 0
  }
  return total
}

/** Weight estimate for a plan: runs of every worker that spends it. */
function runsOnPlan(store: Store, provider: ProviderId, since: number): RunRow[] {
  const plan = planOf(provider)
  const rows: RunRow[] = []
  for (const id of PROVIDERS) {
    if (planOf(id) !== plan) continue
    rows.push(...store.runsSince(id, since))
  }
  rows.sort((a, b) => a.started_at - b.started_at || (a.id ?? 0) - (b.id ?? 0))
  return rows
}

function pickReason(type: JobType, winner: Score, scores: Score[], rules: RouterRules): string {
  const another = scores.some((row) => row.provider !== winner.provider && row.score > 0)
  if (!another) return 'only plan available'
  if (winner.atRisk) return 'allowance expiring'
  if (winner.provider === rules.rules[type][0]) return 'first choice'
  if (winner.paceFactor > 1.05) return 'behind pace'
  return 'more headroom'
}

function toWindow(row: StoredWindow, now: number): WindowState {
  const lengthMs = WINDOW_MS[row.name] ?? null
  if (row.resetsAt !== null && row.resetsAt <= now) {
    return { name: row.name, used: 0, resetsAt: null, lengthMs }
  }
  return { name: row.name, used: clamp01(row.utilization), resetsAt: row.resetsAt, lengthMs }
}

/**
 * A dashboard reading as stored: the share used and when that cycle resets. A message saying what
 * is wrong when the input is not a percentage from 0 to 100 and a reset time in the future.
 */
export function dashboardReading(
  usedPercent: unknown,
  resetsAt: unknown,
  now: number
): { name: string; utilization: number; resetsAt: number } | string {
  if (typeof usedPercent !== 'number' || !Number.isFinite(usedPercent)) {
    return 'Enter how much is used, as a percentage.'
  }
  if (usedPercent < 0 || usedPercent > 100) return 'Usage must be between 0 and 100%.'
  if (typeof resetsAt !== 'number' || !Number.isFinite(resetsAt)) {
    return 'Enter the date your usage resets.'
  }
  if (resetsAt <= now) return 'The reset date has to be in the future.'
  return { name: DASHBOARD_WINDOW, utilization: usedPercent / 100, resetsAt }
}

/**
 * The dashboard reading plus Legate's runs since it. Once the cycle the reading belongs to has
 * reset, it counts only runs since that reset, and the next reset is a month on.
 */
function dashboardWindow(
  row: StoredWindow,
  store: Store,
  plan: ProviderId,
  units: number,
  now: number
): WindowState {
  const lengthMs = WINDOW_MS[DASHBOARD_WINDOW]
  if (row.resetsAt === null || row.resetsAt > now) {
    const since = runsOnPlan(store, plan, row.observedAt)
    return {
      name: row.name,
      used: clamp01(row.utilization + weightUsed(since, units)),
      resetsAt: row.resetsAt,
      lengthMs
    }
  }
  let months = 1
  let cycleStart = row.resetsAt
  let next = addMonths(row.resetsAt, months)
  while (next <= now) {
    cycleStart = next
    months += 1
    next = addMonths(row.resetsAt, months)
  }
  return {
    name: row.name,
    used: clamp01(weightUsed(runsOnPlan(store, plan, cycleStart), units)),
    resetsAt: next,
    lengthMs
  }
}

/**
 * The same day and time `months` calendar months later, clamped to the month's last day. Counted
 * from one anchor so a cycle that starts on the 31st stays on the 31st after a short month.
 */
export function addMonths(ms: number, months: number): number {
  const date = new Date(ms)
  const day = date.getDate()
  date.setDate(1)
  date.setMonth(date.getMonth() + months)
  const lastDay = new Date(date.getFullYear(), date.getMonth() + 1, 0).getDate()
  date.setDate(Math.min(day, lastDay))
  return date.getTime()
}

/** Pace window: the longest known window that still has a future reset. */
function longestOpen(windows: WindowState[]): WindowState | null {
  let best: WindowState | null = null
  for (const window of windows) {
    if (window.lengthMs === null || window.resetsAt === null) continue
    if (best === null || window.lengthMs > (best.lengthMs ?? 0)) best = window
  }
  return best
}

function laterReset(a: number | null, b: number | null): boolean {
  return (a ?? Number.NEGATIVE_INFINITY) > (b ?? Number.NEGATIVE_INFINITY)
}

function weightUsed(rows: RunRow[], units: number): number {
  let weight = 0
  for (const row of rows) weight += row.cost_usd ?? row.tokens ?? 1
  return weight / units
}

function clamp01(value: number): number {
  return clamp(value, 0, 1)
}

function clamp(value: number, min: number, max: number): number {
  if (value < min) return min
  if (value > max) return max
  return value
}

function validate(raw: unknown): RouterRules {
  if (raw === null || typeof raw !== 'object') {
    throw new Error('router rules must be a JSON object')
  }
  const body = raw as Record<string, unknown>
  if (body.fallbackFit == null) throw new Error('router rules missing fallbackFit')
  if (body.allowance == null) throw new Error('router rules missing allowance')
  if (body.defaultRestHours == null) throw new Error('router rules missing defaultRestHours')
  if (body.rules === null || typeof body.rules !== 'object') {
    throw new Error('router rules missing rules')
  }
  const table = body.rules as Record<string, unknown>
  for (const type of JOB_TYPES) {
    const list = table[type]
    if (!Array.isArray(list) || list.length === 0) {
      throw new Error(`router rules missing a non-empty provider list for job type "${type}"`)
    }
  }
  if (body.pace !== undefined) validatePace(body.pace)
  const maxParallel = readMaxParallel(body.maxParallel)
  return { ...(raw as RouterRules), maxParallel }
}

function readMaxParallel(value: unknown): number {
  if (value === undefined) return DEFAULT_MAX_PARALLEL
  if (typeof value !== 'number' || !Number.isInteger(value)) return DEFAULT_MAX_PARALLEL
  if (value < MAX_PARALLEL_MIN) return MAX_PARALLEL_MIN
  if (value > MAX_PARALLEL_MAX) return MAX_PARALLEL_MAX
  return value
}

function validatePace(pace: unknown): void {
  if (pace === null || typeof pace !== 'object' || Array.isArray(pace)) {
    throw new Error('router rules: invalid pace settings')
  }
  const body = pace as Record<string, unknown>
  const weight = body.weight
  const min = body.min
  const max = body.max
  const atRiskSlack = body.atRiskSlack
  const atRiskLeft = body.atRiskLeft
  if (
    !isFiniteNumber(weight) ||
    !isFiniteNumber(min) ||
    !isFiniteNumber(max) ||
    !isFiniteNumber(atRiskSlack) ||
    !isFiniteNumber(atRiskLeft) ||
    !(min > 0 && min <= 1 && max >= 1) ||
    weight < 0 ||
    atRiskSlack < 0 ||
    atRiskSlack > 1 ||
    atRiskLeft < 0 ||
    atRiskLeft > 1
  ) {
    throw new Error('router rules: invalid pace settings')
  }
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value)
}
