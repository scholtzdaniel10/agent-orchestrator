import { performance } from 'node:perf_hooks'
import { expect, test } from 'vitest'
import { loadRules, pickWithReason } from '../../../../agent-orchestrator/src/core/router/router.ts'
import type { RunRow, StoredWindow } from '../../../../agent-orchestrator/src/core/router/store.ts'
import type { ProviderId } from '../../../../agent-orchestrator/src/core/types.ts'

const NOW = 1_800_000_000_000
const RUNS = 10_000

function providerName(index: number): string {
  return ['claude', 'cursor', 'grok', 'local'][index % 4]
}

const runs: RunRow[] = []
for (let i = 0; i < RUNS; i += 1) {
  runs.push({
    id: i,
    job_id: `job-${i}`,
    provider: providerName(i) as ProviderId,
    job_type: 'planning',
    started_at: NOW - i,
    duration_ms: 1,
    cost_usd: 1,
    tokens: 10,
    utilization: null,
    outcome: 'ok',
    session_id: null
  })
}

function windows(): StoredWindow[] {
  return [
    { name: 'five_hour', utilization: 0.2, resetsAt: NOW + 3_600_000, observedAt: NOW },
    { name: 'seven_day', utilization: 0.45, resetsAt: NOW + 168 * 3_600_000, observedAt: NOW }
  ]
}

const byPlan = new Map<string, StoredWindow[]>([
  ['claude', windows()],
  ['cursor', windows()],
  ['local', windows()]
])

const store = {
  restingUntil(): null {
    return null
  },
  windows(plan: string): StoredWindow[] {
    return byPlan.get(plan) ?? []
  },
  runsSince(provider: string, since: number): RunRow[] {
    return runs.filter((row) => row.provider === provider && row.started_at >= since)
  }
}

test('time pickWithReason', () => {
  const rules = loadRules()
  // The TypeScript router has three providers. `local` is not in its ProviderId
  // union or rules.json, so this times the same 10_000 runs and two windows on
  // the three plans that router can score.
  const candidates: ProviderId[] = ['claude', 'cursor', 'grok']
  let sink = 0
  for (let i = 0; i < 200; i += 1) {
    const pick = pickWithReason('planning', candidates, store, rules, NOW, [], {})
    sink += pick ? 1 : 0
  }
  const samples: number[] = []
  const iterations = 2_000
  for (let sample = 0; sample < 101; sample += 1) {
    const start = performance.now()
    for (let i = 0; i < iterations; i += 1) {
      const pick = pickWithReason('planning', candidates, store, rules, NOW, [], {})
      sink += pick ? 1 : 0
    }
    samples.push(((performance.now() - start) * 1_000_000) / iterations)
  }
  samples.sort((a, b) => a - b)
  const median = samples[50]
  console.log(`ts_median_ns ${median}`)
  console.log(`ts_sink ${sink}`)
  expect(median).toBeGreaterThan(0)
})
