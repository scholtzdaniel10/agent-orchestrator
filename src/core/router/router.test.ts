import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'vitest'
import type { JobType } from '../types'
import {
  DEFAULT_PACE,
  headroom,
  loadRules,
  pickProvider,
  pickWithReason,
  planUsage,
  scoreProviders
} from './router'
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
  expect(rules.pace).toEqual(DEFAULT_PACE)
  for (const type of JOB_TYPES) {
    expect(rules.rules[type]?.length).toBeGreaterThan(0)
  }
})

test('loadRules(path) loads an edited copy of the rules', () => {
  const dir = mkdtempSync(join(tmpdir(), 'router-rules-'))
  try {
    const path = join(dir, 'rules.json')
    const edited = { ...loadRules(), fallbackFit: 0.9 }
    edited.rules = { ...edited.rules, boilerplate: ['claude', 'cursor'] }
    writeFileSync(path, JSON.stringify(edited))
    const rules = loadRules(path)
    expect(rules.fallbackFit).toBe(0.9)
    expect(rules.rules.boilerplate).toEqual(['claude', 'cursor'])
    expect(loadRules().rules.boilerplate).toEqual(['cursor', 'claude'])
  } finally {
    rmSync(dir, { recursive: true, force: true })
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

test('loadRules accepts a file with no pace block and rejects a bad one', () => {
  const dir = mkdtempSync(join(tmpdir(), 'router-rules-'))
  try {
    const path = join(dir, 'rules.json')
    const bundled = loadRules()
    const withoutPace = { ...bundled }
    delete withoutPace.pace
    writeFileSync(path, JSON.stringify(withoutPace))
    expect(loadRules(path).pace).toBeUndefined()
    expect(loadRules(path).fallbackFit).toBe(0.5)

    writeFileSync(
      path,
      JSON.stringify({
        ...bundled,
        pace: { weight: 0, min: 1, max: 1, atRiskSlack: 0, atRiskLeft: 1 }
      })
    )
    expect(loadRules(path).pace).toEqual({
      weight: 0,
      min: 1,
      max: 1,
      atRiskSlack: 0,
      atRiskLeft: 1
    })

    const bad = [
      { weight: -1, min: 0.5, max: 2, atRiskSlack: 0.25, atRiskLeft: 0.15 },
      { weight: 1.5, min: 0, max: 2, atRiskSlack: 0.25, atRiskLeft: 0.15 },
      { weight: 1.5, min: 1.1, max: 2, atRiskSlack: 0.25, atRiskLeft: 0.15 },
      { weight: 1.5, min: 0.5, max: 0.9, atRiskSlack: 0.25, atRiskLeft: 0.15 },
      { weight: Number.NaN, min: 0.5, max: 2, atRiskSlack: 0.25, atRiskLeft: 0.15 },
      { weight: 1.5, min: 0.5, max: 2, atRiskSlack: 1.1, atRiskLeft: 0.15 },
      { weight: 1.5, min: 0.5, max: 2, atRiskSlack: 0.25, atRiskLeft: -0.01 },
      { min: 0.5, max: 2, atRiskSlack: 0.25, atRiskLeft: 0.15 },
      null,
      'fast'
    ]
    for (const pace of bad) {
      writeFileSync(path, JSON.stringify({ ...bundled, pace }))
      expect(() => loadRules(path)).toThrow('router rules: invalid pace settings')
    }
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('headroom uses stored windows, else summed weight, and resting is 0', () => {
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

    // A stored window wins over weight, including a literal 0.
    store.saveWindows('claude', [{ name: 'reported', utilization: 0, resetsAt: NOW + 1 }], NOW)
    expect(headroom('claude', store, rules, NOW)).toBe(1)

    store.saveWindows('cursor', [{ name: 'reported', utilization: 0.75, resetsAt: NOW + 1 }], NOW)
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

    // claude used 0.75 → headroom 0.25 → score 1 * 0.25 = 0.25
    // cursor unused → headroom 1 → score fallbackFit 0.5 * 1 = 0.5
    // 0.5 > 0.25, so the fallback wins. An unknown window name does not change pace.
    store.saveWindows('claude', [{ name: 'reported', utilization: 0.75, resetsAt: NOW + 1 }], NOW)
    expect(pickProvider('planning', [...both], store, rules, NOW)).toBe('cursor')

    // Tie: claude headroom 0.5 → score 0.5; cursor score 0.5 * 1 = 0.5.
    // The earlier rule entry wins.
    store.saveWindows('claude', [{ name: 'reported', utilization: 0.5, resetsAt: NOW + 1 }], NOW)
    expect(pickProvider('planning', [...both], store, rules, NOW)).toBe('claude')

    store.setResting('claude', NOW + 5_000)
    expect(pickProvider('planning', [...both], store, rules, NOW)).toBe('cursor')
    store.setResting('cursor', NOW + 5_000)
    expect(pickProvider('planning', [...both], store, rules, NOW)).toBeNull()

    store.clearResting('claude')
    store.clearResting('cursor')
    expect(pickProvider('planning', [...both], store, rules, NOW, ['claude'])).toBe('cursor')
    expect(pickProvider('planning', [...both], store, rules, NOW, ['claude', 'cursor'])).toBeNull()
    expect(
      scoreProviders('planning', [...both], store, rules, NOW, ['cursor']).map(
        (row) => row.provider
      )
    ).toEqual(['claude'])

    store.saveWindows('claude', [{ name: 'reported', utilization: 1, resetsAt: NOW + 1 }], NOW)
    store.saveWindows('cursor', [{ name: 'reported', utilization: 1, resetsAt: NOW + 1 }], NOW)
    expect(pickProvider('planning', [...both], store, rules, NOW)).toBeNull()
  })
})

const PACE_NOW = 1_800_000_000_000
const WEEK_MS = 168 * 3600_000
const HOUR5_MS = 5 * 3600_000

test('planUsage estimates from run weight when no windows are stored', () => {
  const rules = loadRules()
  withStore((store) => {
    insert(store, { provider: 'claude', started_at: PACE_NOW - 1, cost_usd: 0, tokens: 100 })
    insert(store, { provider: 'claude', started_at: PACE_NOW - 1, tokens: 5 })
    insert(store, { provider: 'claude', started_at: PACE_NOW - 1 })
    expect(planUsage('claude', store, rules, PACE_NOW)).toEqual({
      windows: [],
      used: 6 / 15,
      resetsAt: null,
      slack: null,
      paceFactor: 1,
      atRisk: false
    })
  })
})

test('planUsage reads windows, pace, rollover, and custom pace settings', () => {
  const rules = loadRules()
  withStore((store) => {
    store.saveWindows(
      'claude',
      [
        { name: 'five_hour', utilization: 0.2, resetsAt: PACE_NOW + HOUR5_MS / 2 },
        { name: 'seven_day', utilization: 0.45, resetsAt: PACE_NOW + (WEEK_MS * 2) / 5 }
      ],
      PACE_NOW
    )
    const paced = planUsage('claude', store, rules, PACE_NOW)
    expect(paced.used).toBe(0.45)
    expect(paced.resetsAt).toBe(PACE_NOW + (WEEK_MS * 2) / 5)
    expect(paced.slack).toBeCloseTo(0.15)
    expect(paced.paceFactor).toBeCloseTo(1.225)
    expect(paced.atRisk).toBe(false)
    expect(paced.windows.map((window) => window.name)).toEqual(['five_hour', 'seven_day'])
  })

  withStore((store) => {
    store.saveWindows(
      'claude',
      [{ name: 'seven_day', utilization: 0.8, resetsAt: PACE_NOW }],
      PACE_NOW
    )
    const rolled = planUsage('claude', store, rules, PACE_NOW)
    expect(rolled.used).toBe(0)
    expect(rolled.windows).toEqual([
      { name: 'seven_day', used: 0, resetsAt: null, lengthMs: WEEK_MS }
    ])
    expect(rolled.slack).toBeNull()
    expect(rolled.paceFactor).toBe(1)
    expect(rolled.atRisk).toBe(false)
  })

  withStore((store) => {
    store.saveWindows(
      'claude',
      [{ name: 'seven_day', utilization: 0.9, resetsAt: PACE_NOW + WEEK_MS / 2 }],
      PACE_NOW
    )
    expect(planUsage('claude', store, rules, PACE_NOW).paceFactor).toBe(0.5)
  })

  withStore((store) => {
    store.saveWindows(
      'claude',
      [{ name: 'seven_day', utilization: 0.5, resetsAt: PACE_NOW + WEEK_MS / 10 }],
      PACE_NOW
    )
    expect(planUsage('claude', store, rules, PACE_NOW).atRisk).toBe(true)
  })

  withStore((store) => {
    store.saveWindows(
      'claude',
      [
        { name: 'five_hour', utilization: 0.9375, resetsAt: PACE_NOW + HOUR5_MS / 2 },
        { name: 'seven_day', utilization: 0.5, resetsAt: PACE_NOW + WEEK_MS / 10 }
      ],
      PACE_NOW
    )
    const tight = planUsage('claude', store, rules, PACE_NOW)
    expect(tight.used).toBe(0.9375)
    expect(tight.slack).toBeCloseTo(0.4)
    expect(tight.atRisk).toBe(false)
  })

  withStore((store) => {
    store.saveWindows(
      'claude',
      [
        { name: 'session', utilization: 0.8, resetsAt: PACE_NOW + 1000 },
        { name: 'seven_day', utilization: 0.25, resetsAt: PACE_NOW + WEEK_MS / 2 }
      ],
      PACE_NOW
    )
    const mixed = planUsage('claude', store, rules, PACE_NOW)
    expect(mixed.used).toBe(0.8)
    expect(mixed.windows.find((window) => window.name === 'session')?.lengthMs).toBeNull()
    expect(mixed.slack).toBeCloseTo(0.25)
    expect(mixed.paceFactor).toBeCloseTo(1.375)
    expect(mixed.atRisk).toBe(false)
  })

  withStore((store) => {
    store.saveWindows(
      'claude',
      [{ name: 'seven_day', utilization: 0.5, resetsAt: PACE_NOW + WEEK_MS / 10 }],
      PACE_NOW
    )
    const custom = {
      ...rules,
      pace: { weight: 4, min: 0.2, max: 1.1, atRiskSlack: 0.99, atRiskLeft: 0.05 }
    }
    const usage = planUsage('claude', store, custom, PACE_NOW)
    expect(usage.paceFactor).toBe(1.1)
    expect(usage.atRisk).toBe(false)
  })
})

test('pace changes who wins and why', () => {
  const rules = loadRules()
  const both = ['claude', 'cursor'] as const

  withStore((store) => {
    store.saveWindows(
      'claude',
      [{ name: 'seven_day', utilization: 0, resetsAt: PACE_NOW + WEEK_MS / 2 }],
      PACE_NOW
    )
    expect(pickWithReason('planning', [...both], store, rules, PACE_NOW)).toEqual({
      provider: 'claude',
      reason: 'first choice'
    })
    expect(scoreProviders('planning', [...both], store, rules, PACE_NOW)).toEqual([
      {
        provider: 'claude',
        fit: 1,
        headroom: 1,
        paceFactor: 1.75,
        atRisk: false,
        score: 1.75
      },
      { provider: 'cursor', fit: 0.5, headroom: 1, paceFactor: 1, atRisk: false, score: 0.5 }
    ])
  })

  withStore((store) => {
    store.saveWindows(
      'claude',
      [{ name: 'reported', utilization: 0.5, resetsAt: PACE_NOW + 1000 }],
      PACE_NOW
    )
    store.saveWindows(
      'cursor',
      [{ name: 'seven_day', utilization: 0, resetsAt: PACE_NOW + WEEK_MS / 2 }],
      PACE_NOW
    )
    expect(pickWithReason('planning', [...both], store, rules, PACE_NOW)).toEqual({
      provider: 'cursor',
      reason: 'behind pace'
    })
    expect(scoreProviders('planning', [...both], store, rules, PACE_NOW)).toEqual([
      { provider: 'claude', fit: 1, headroom: 0.5, paceFactor: 1, atRisk: false, score: 0.5 },
      {
        provider: 'cursor',
        fit: 0.5,
        headroom: 1,
        paceFactor: 1.75,
        atRisk: false,
        score: 0.875
      }
    ])
  })

  withStore((store) => {
    store.saveWindows(
      'cursor',
      [{ name: 'seven_day', utilization: 0.25, resetsAt: PACE_NOW + WEEK_MS / 8 }],
      PACE_NOW
    )
    expect(pickWithReason('planning', [...both], store, rules, PACE_NOW)).toEqual({
      provider: 'cursor',
      reason: 'allowance expiring'
    })
    expect(scoreProviders('planning', [...both], store, rules, PACE_NOW)).toEqual([
      { provider: 'claude', fit: 1, headroom: 1, paceFactor: 1, atRisk: false, score: 1 },
      { provider: 'cursor', fit: 1, headroom: 0.75, paceFactor: 2, atRisk: true, score: 1.5 }
    ])
  })

  withStore((store) => {
    expect(pickWithReason('planning', ['claude'], store, rules, PACE_NOW)).toEqual({
      provider: 'claude',
      reason: 'only plan available'
    })
    store.setResting('cursor', PACE_NOW + 1)
    expect(pickWithReason('planning', [...both], store, rules, PACE_NOW)).toEqual({
      provider: 'claude',
      reason: 'only plan available'
    })
  })

  withStore((store) => {
    store.saveWindows(
      'claude',
      [{ name: 'reported', utilization: 0.75, resetsAt: PACE_NOW + 1 }],
      PACE_NOW
    )
    expect(pickWithReason('planning', [...both], store, rules, PACE_NOW)).toEqual({
      provider: 'cursor',
      reason: 'more headroom'
    })
    expect(scoreProviders('planning', [...both], store, rules, PACE_NOW)).toEqual([
      { provider: 'claude', fit: 1, headroom: 0.25, paceFactor: 1, atRisk: false, score: 0.25 },
      { provider: 'cursor', fit: 0.5, headroom: 1, paceFactor: 1, atRisk: false, score: 0.5 }
    ])
  })
})
