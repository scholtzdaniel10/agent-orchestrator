import { expect, test } from 'vitest'
import { Store, type RunRow } from './store'

function row(partial: Partial<RunRow> & Pick<RunRow, 'provider' | 'started_at'>): RunRow {
  return {
    job_id: 'job',
    job_type: 'planning',
    duration_ms: 10,
    cost_usd: null,
    tokens: null,
    utilization: null,
    outcome: 'ok',
    session_id: null,
    ...partial
  }
}

async function withStore(fn: (store: Store) => void | Promise<void>): Promise<void> {
  const store = new Store(':memory:')
  try {
    await fn(store)
  } finally {
    store.close()
  }
}

test('inserts runs and returns them oldest first', async () => {
  await withStore((store) => {
    store.insertRun(
      row({ provider: 'claude', started_at: 300, cost_usd: 0, tokens: 9, outcome: 'error' })
    )
    store.insertRun(
      row({ provider: 'cursor', started_at: 100, job_id: 'c', utilization: 0.2, session_id: 's' })
    )
    store.insertRun(
      row({ provider: 'claude', started_at: 200, job_id: 'b', tokens: 4, outcome: 'limit' })
    )

    const all = store.runs()
    expect(all.map((run) => run.started_at)).toEqual([100, 200, 300])
    expect(all[0]).toMatchObject({
      provider: 'cursor',
      job_id: 'c',
      utilization: 0.2,
      session_id: 's',
      cost_usd: null
    })
    expect(all[2]).toMatchObject({ cost_usd: 0, tokens: 9, outcome: 'error' })

    expect(store.runs('claude').map((run) => run.job_id)).toEqual(['b', 'job'])
    expect(store.runsSince('claude', 200).map((run) => run.started_at)).toEqual([200, 300])
    expect(store.runsSince('claude', 250).map((run) => run.started_at)).toEqual([300])
  })
})

test('lastUtilization picks the newest non-null value inside the window', async () => {
  await withStore((store) => {
    store.insertRun(row({ provider: 'claude', started_at: 50, utilization: 0.9 }))
    store.insertRun(row({ provider: 'claude', started_at: 100, utilization: 0.2 }))
    store.insertRun(row({ provider: 'claude', started_at: 300, utilization: 0.4 }))
    store.insertRun(row({ provider: 'claude', started_at: 300, utilization: null, cost_usd: 8 }))
    store.insertRun(row({ provider: 'claude', started_at: 400, utilization: null }))
    store.insertRun(row({ provider: 'cursor', started_at: 500, utilization: 0.1 }))

    // 400 is newer but null, so the newest row that has a value wins (0.4 at t=300).
    expect(store.lastUtilization('claude', 100)).toBe(0.4)
    expect(store.lastUtilization('claude', 0)).toBe(0.4)
    expect(store.lastUtilization('claude', 401)).toBeNull()
    expect(store.lastUtilization('claude', 1000)).toBeNull()
    expect(store.lastUtilization('cursor', 0)).toBe(0.1)
    // A reported 0 is a real value, not "missing".
    store.insertRun(row({ provider: 'cursor', started_at: 600, utilization: 0 }))
    expect(store.lastUtilization('cursor', 0)).toBe(0)
  })
})

test('resting is set, replaced, and cleared', async () => {
  await withStore((store) => {
    expect(store.restingUntil('claude')).toBeNull()
    store.setResting('claude', 50)
    expect(store.restingUntil('claude')).toBe(50)
    store.setResting('claude', 80)
    expect(store.restingUntil('claude')).toBe(80)
    store.setResting('cursor', 10)
    store.clearResting('claude')
    expect(store.restingUntil('claude')).toBeNull()
    expect(store.restingUntil('cursor')).toBe(10)
    store.clearResting('claude')
    expect(store.restingUntil('missing')).toBeNull()
  })
})
