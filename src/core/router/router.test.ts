import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'vitest'
import type { JobType } from '../types'
import { headroom, loadRules, pickProvider } from './router'
import { Store, type RunRow } from './store'

const JOB_TYPES: readonly JobType[] = ['planning', 'debugging', 'review', 'refactor', 'boilerplate']

const NOW = 50_000_000

function insert(
  store: Store,
  partial: Partial<RunRow> & Pick<RunRow, 'provider' | 'started_at'>
): void {
  store.insertRun({
    job_id: 'job',
    job_type: 'planning',
    duration_ms: 1,
    cost_usd: null,
    tokens: null,
    utilization: null,
    outcome: 'ok',
    session_id: null,
    ...partial
  })
}

function withStore(fn: (store: Store) => void): void {
  const store = new Store(':memory:')
  try {
    fn(store)
  } finally {
    store.close()
  }
}

test('loadRules() parses the bundled file and covers every job type', () => {
  const rules = loadRules()
  expect(rules.fallbackFit).toBe(0.5)
  expect(rules.defaultRestHours).toBe(5)
  expect(rules.allowance.claude).toEqual({ windowHours: 5, units: 15 })
  expect(rules.allowance.cursor).toEqual({ windowHours: 720, units: 20_000_000 })
  for (const type of JOB_TYPES) {
    expect(rules.rules[type]?.length).toBeGreaterThan(0)
  }
})

test('loadRules rejects a file missing a job type', () => {
  const dir = mkdtempSync(join(tmpdir(), 'router-rules-'))
  try {
    const path = join(dir, 'rules.json')
    writeFileSync(
      path,
      JSON.stringify({
        rules: { planning: ['claude'] },
        fallbackFit: 0.5,
        allowance: { claude: { windowHours: 5, units: 15 } },
        defaultRestHours: 5
      })
    )
    expect(() => loadRules(path)).toThrow(/debugging/)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('headroom uses reported utilization, else summed weight, and resting is 0', () => {
  const rules = loadRules()
  withStore((store) => {
    // cost 0 must not fall through to tokens. null cost uses tokens, then 1.
    // weights 0 + 5 + 1 = 6; claude allowance is 15; headroom = 1 - 6/15.
    insert(store, { provider: 'claude', started_at: NOW - 1, cost_usd: 0, tokens: 100 })
    insert(store, { provider: 'claude', started_at: NOW - 1, tokens: 5 })
    insert(store, { provider: 'claude', started_at: NOW - 1 })
    expect(headroom('claude', store, rules, NOW)).toBe(1 - 6 / 15)
  })

  withStore((store) => {
    const windowMs = rules.allowance.claude.windowHours * 3600_000
    insert(store, { provider: 'claude', started_at: NOW - windowMs - 1, cost_usd: 15 })
    insert(store, { provider: 'claude', started_at: NOW - windowMs, cost_usd: 3 })
    expect(headroom('claude', store, rules, NOW)).toBe(1 - 3 / 15)

    // Reported utilization wins over weight, including a literal 0.
    insert(store, { provider: 'claude', started_at: NOW - 1, utilization: 0, cost_usd: 15 })
    expect(headroom('claude', store, rules, NOW)).toBe(1)

    insert(store, {
      provider: 'cursor',
      started_at: NOW - 1,
      utilization: 0.75,
      cost_usd: 15
    })
    expect(headroom('cursor', store, rules, NOW)).toBe(0.25)

    store.setResting('cursor', NOW + 1)
    expect(headroom('cursor', store, rules, NOW)).toBe(0)
    // Rest ends at `until`, so equal-to-now is usable again.
    store.setResting('cursor', NOW)
    expect(headroom('cursor', store, rules, NOW)).toBe(0.25)
  })
})

test('pickProvider prefers the first choice, then fit × headroom', () => {
  const rules = loadRules()
  const both = ['claude', 'cursor'] as const

  withStore((store) => {
    expect(pickProvider('planning', [...both], store, rules, NOW)).toBe('claude')
    expect(pickProvider('boilerplate', [...both], store, rules, NOW)).toBe('cursor')

    // claude utilization 0.75 → headroom 0.25 → score 1 * 0.25 = 0.25
    // cursor unused → headroom 1 → score fallbackFit 0.5 * 1 = 0.5
    // 0.5 > 0.25, so the fallback wins.
    insert(store, { provider: 'claude', started_at: NOW - 1, utilization: 0.75 })
    expect(pickProvider('planning', [...both], store, rules, NOW)).toBe('cursor')

    // Tie: claude headroom 0.5 → score 0.5; cursor score 0.5 * 1 = 0.5.
    // The earlier rule entry wins.
    insert(store, { provider: 'claude', started_at: NOW, utilization: 0.5 })
    expect(pickProvider('planning', [...both], store, rules, NOW)).toBe('claude')

    store.setResting('claude', NOW + 5_000)
    expect(pickProvider('planning', [...both], store, rules, NOW)).toBe('cursor')
    store.setResting('cursor', NOW + 5_000)
    expect(pickProvider('planning', [...both], store, rules, NOW)).toBeNull()

    store.clearResting('claude')
    store.clearResting('cursor')
    expect(pickProvider('planning', [...both], store, rules, NOW, ['claude'])).toBe('cursor')
    expect(pickProvider('planning', [...both], store, rules, NOW, ['claude', 'cursor'])).toBeNull()

    insert(store, { provider: 'claude', started_at: NOW, utilization: 1 })
    insert(store, { provider: 'cursor', started_at: NOW, utilization: 1 })
    expect(pickProvider('planning', [...both], store, rules, NOW)).toBeNull()
  })
})
