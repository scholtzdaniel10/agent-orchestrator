import { readFileSync } from 'node:fs'
import type { JobType, ProviderId, RouterRules } from '../types'
import rulesJson from './rules.json'
import type { RunRow, Store } from './store'

const JOB_TYPES: readonly JobType[] = ['planning', 'debugging', 'review', 'refactor', 'boilerplate']

/** Bundled copy of rules.json. `loadRules()` with no path returns this. */
export const defaultRules: RouterRules = validate(rulesJson)

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
  const resting = store.restingUntil(provider)
  if (resting !== null && resting > now) return 0

  const allowance = rules.allowance[provider]
  const since = now - allowance.windowHours * 3600_000
  const reported = store.lastUtilization(provider, since)
  const used =
    reported !== null ? reported : weightUsed(store.runsSince(provider, since), allowance.units)
  return clamp01(1 - used)
}

/**
 * Highest `fit * headroom` among providers that are candidates, listed for
 * this job type, and not excluded. Fit is 1 for the first rule entry and
 * `fallbackFit` after that. Ties keep the earlier rule entry. All-zero scores
 * and an empty field return null.
 */
export function pickProvider(
  type: JobType,
  candidates: ProviderId[],
  store: Store,
  rules: RouterRules,
  now: number,
  exclude: ProviderId[] = []
): ProviderId | null {
  const order = rules.rules[type]
  let best: ProviderId | null = null
  let bestScore = 0
  for (let i = 0; i < order.length; i++) {
    const provider = order[i]
    if (!candidates.includes(provider) || exclude.includes(provider)) continue
    const fit = i === 0 ? 1 : rules.fallbackFit
    const score = fit * headroom(provider, store, rules, now)
    if (score > bestScore) {
      bestScore = score
      best = provider
    }
  }
  return best
}

function weightUsed(rows: RunRow[], units: number): number {
  let weight = 0
  for (const row of rows) weight += row.cost_usd ?? row.tokens ?? 1
  return weight / units
}

function clamp01(value: number): number {
  if (value < 0) return 0
  if (value > 1) return 1
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
  return raw as RouterRules
}
