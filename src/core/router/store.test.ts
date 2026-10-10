import { DatabaseSync } from 'node:sqlite'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'vitest'
import type { JobRecord } from './orchestrator'
import { Store, TERMINAL_SCROLLBACK_CAP, type RunRow, type TerminalRow } from './store'

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

function terminal(
  partial: Partial<TerminalRow> & Pick<TerminalRow, 'id' | 'project'>
): TerminalRow {
  return {
    provider: 'claude',
    model: null,
    session_id: null,
    title: 'claude 1',
    created_at: 10,
    updated_at: 10,
    scrollback: '',
    ...partial
  }
}

function job(partial: Partial<JobRecord> & Pick<JobRecord, 'id'>): JobRecord {
  return {
    type: 'planning',
    prompt: 'go',
    provider: 'claude',
    status: 'done',
    output: '',
    failedOver: [],
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

test('saveWindows upserts by provider and name and windows() lists one provider by name', async () => {
  await withStore((store) => {
    store.saveWindows(
      'claude',
      [
        { name: 'seven_day', utilization: 0.4, resetsAt: 20 },
        { name: 'five_hour', utilization: 0.2, resetsAt: null }
      ],
      5
    )
    store.saveWindows('cursor', [{ name: 'five_hour', utilization: 0.9, resetsAt: 30 }], 6)
    store.saveWindows('claude', [{ name: 'five_hour', utilization: 0.3, resetsAt: 25 }], 7)

    expect(store.windows('claude')).toEqual([
      { name: 'five_hour', utilization: 0.3, resetsAt: 25, observedAt: 7 },
      { name: 'seven_day', utilization: 0.4, resetsAt: 20, observedAt: 5 }
    ])
    expect(store.windows('cursor')).toEqual([
      { name: 'five_hour', utilization: 0.9, resetsAt: 30, observedAt: 6 }
    ])
    expect(store.windows('missing')).toEqual([])
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

test('opens an old database, keeps its rows, and reaches the latest schema version', () => {
  const root = mkdtempSync(join(tmpdir(), 'ao-store-mig-'))
  const path = join(root, 'old.sqlite')
  try {
    const legacy = new DatabaseSync(path)
    legacy.exec(`
      CREATE TABLE IF NOT EXISTS runs (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        job_id TEXT NOT NULL,
        provider TEXT NOT NULL,
        job_type TEXT NOT NULL,
        started_at INTEGER NOT NULL,
        duration_ms INTEGER,
        cost_usd REAL,
        tokens INTEGER,
        utilization REAL,
        outcome TEXT NOT NULL,
        session_id TEXT
      );
      CREATE TABLE IF NOT EXISTS resting (
        provider TEXT PRIMARY KEY,
        until INTEGER NOT NULL
      );
      CREATE TABLE IF NOT EXISTS usage_windows (
        provider TEXT NOT NULL,
        name TEXT NOT NULL,
        utilization REAL NOT NULL,
        resets_at INTEGER,
        observed_at INTEGER NOT NULL,
        PRIMARY KEY (provider, name)
      );
      PRAGMA user_version = 0;
    `)
    legacy
      .prepare(
        `INSERT INTO runs (
           job_id, provider, job_type, started_at, duration_ms,
           cost_usd, tokens, utilization, outcome, session_id
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`
      )
      .run('kept', 'claude', 'planning', 10, 1, null, null, null, 'ok', null)
    legacy.prepare('INSERT INTO resting (provider, until) VALUES (?, ?)').run('claude', 99)
    legacy.close()

    const store = new Store(path)
    expect(store.runs()).toHaveLength(1)
    expect(store.runs()[0]).toMatchObject({ job_id: 'kept', provider: 'claude' })
    expect(store.restingUntil('claude')).toBe(99)
    store.saveChat({
      id: 'c1',
      project: '/p',
      title: 'hi',
      created_at: 1,
      updated_at: 2,
      session_id: null,
      session_provider: null
    })
    expect(store.chats('/p')).toHaveLength(1)
    store.close()

    const again = new Store(path)
    expect(again.runs()[0]?.job_id).toBe('kept')
    expect(again.chats('/p')).toHaveLength(1)
    again.close()

    const check = new DatabaseSync(path)
    expect(check.prepare('PRAGMA user_version').get()).toEqual({ user_version: 3 })
    expect(check.prepare(`SELECT name FROM sqlite_master WHERE name = 'terminals'`).get()).toEqual({
      name: 'terminals'
    })
    check.close()
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('opening a current database twice is a no-op for migrations', () => {
  const root = mkdtempSync(join(tmpdir(), 'ao-store-twice-'))
  const path = join(root, 'db.sqlite')
  try {
    const first = new Store(path)
    first.insertRun(row({ provider: 'claude', started_at: 1 }))
    first.close()
    const second = new Store(path)
    expect(second.runs()).toHaveLength(1)
    second.close()
    const third = new Store(path)
    expect(third.runs()).toHaveLength(1)
    third.close()
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('chats, messages, and jobs round-trip with order and upsert rules', async () => {
  await withStore((store) => {
    store.saveChat({
      id: 'c1',
      project: '/a',
      title: 'first',
      created_at: 10,
      updated_at: 20,
      session_id: 's1',
      session_provider: 'claude'
    })
    store.saveChat({
      id: 'c2',
      project: '/a',
      title: 'second',
      created_at: 11,
      updated_at: 30,
      session_id: null,
      session_provider: null
    })
    store.saveChat({
      id: 'c3',
      project: '/b',
      title: 'other',
      created_at: 12,
      updated_at: 40,
      session_id: null,
      session_provider: null
    })
    expect(store.chats('/a').map((chat) => chat.id)).toEqual(['c2', 'c1'])
    expect(store.chat('c1')).toMatchObject({
      title: 'first',
      session_id: 's1',
      session_provider: 'claude'
    })
    store.saveChat({
      id: 'c1',
      project: '/a',
      title: 'first updated',
      created_at: 10,
      updated_at: 50,
      session_id: 's2',
      session_provider: 'cursor'
    })
    expect(store.chat('c1')?.title).toBe('first updated')
    expect(store.chats('/a').map((chat) => chat.id)).toEqual(['c1', 'c2'])

    store.saveMessage('c1', 1, { id: 'm2', role: 'lead', text: 'b', status: 'done' })
    store.saveMessage('c1', 0, { id: 'm1', role: 'user', text: 'a', status: 'done' })
    expect(store.messages('c1').map((message) => message.id)).toEqual(['m1', 'm2'])
    store.saveMessage('c1', 1, { id: 'm2', role: 'lead', text: 'b2', status: 'done' })
    expect(store.messages('c1')[1]).toMatchObject({ id: 'm2', text: 'b2' })

    store.saveJob('/a', 100, job({ id: 'j1', output: 'one' }))
    store.saveJob('/a', 200, job({ id: 'j2', output: 'two' }))
    store.saveJob('/a', 300, job({ id: 'j3', output: 'three' }))
    store.saveJob('/b', 400, job({ id: 'j4', output: 'other' }))
    expect(store.jobs('/a', 2).map((item) => item.id)).toEqual(['j2', 'j3'])
    store.saveJob('/a', 200, job({ id: 'j2', output: 'two-again', status: 'failed' }))
    expect(store.jobs('/a', 10).find((item) => item.id === 'j2')).toMatchObject({
      output: 'two-again',
      status: 'failed'
    })
  })
})

test('deleteChat removes messages and keeps other chats and jobs', async () => {
  await withStore((store) => {
    store.saveChat({
      id: 'c1',
      project: '/a',
      title: 'one',
      created_at: 1,
      updated_at: 1,
      session_id: null,
      session_provider: null
    })
    store.saveChat({
      id: 'c2',
      project: '/a',
      title: 'two',
      created_at: 2,
      updated_at: 2,
      session_id: null,
      session_provider: null
    })
    store.saveMessage('c1', 0, { id: 'm1', role: 'user', text: 'hi', status: 'done' })
    store.saveMessage('c1', 1, { id: 'm2', role: 'lead', text: 'yo', status: 'done' })
    store.saveMessage('c2', 0, { id: 'm3', role: 'user', text: 'other', status: 'done' })
    store.saveJob('/a', 1, job({ id: 'j1', leadMessage: 'm2' }))
    store.deleteChat('c1')
    expect(store.chat('c1')).toBeNull()
    expect(store.messages('c1')).toEqual([])
    expect(store.chats('/a').map((chat) => chat.id)).toEqual(['c2'])
    expect(store.messages('c2').map((message) => message.id)).toEqual(['m3'])
    expect(store.jobs('/a', 10).map((item) => item.id)).toEqual(['j1'])
    store.renameChat('c2', 'renamed')
    expect(store.chat('c2')?.title).toBe('renamed')
  })
})

test('terminals upsert, list oldest first, update scrollback, delete, and cap', async () => {
  await withStore((store) => {
    store.saveTerminal(
      terminal({
        id: 't2',
        project: '/a',
        title: 'claude 2',
        created_at: 20,
        session_id: 's2',
        model: 'opus'
      })
    )
    store.saveTerminal(
      terminal({
        id: 't1',
        project: '/a',
        title: 'claude 1',
        created_at: 10,
        provider: 'cursor'
      })
    )
    store.saveTerminal(terminal({ id: 't3', project: '/b', title: 'other' }))
    expect(store.terminals('/a').map((row) => row.id)).toEqual(['t1', 't2'])
    expect(store.terminals('/a')[1]).toMatchObject({
      provider: 'claude',
      model: 'opus',
      session_id: 's2',
      title: 'claude 2'
    })
    store.saveTerminal(
      terminal({
        id: 't1',
        project: '/a',
        title: 'cursor 1',
        created_at: 10,
        updated_at: 40,
        provider: 'cursor',
        session_id: 'chat-1',
        scrollback: 'hi'
      })
    )
    expect(store.terminals('/a')[0]).toMatchObject({
      title: 'cursor 1',
      session_id: 'chat-1',
      scrollback: 'hi',
      updated_at: 40
    })

    store.updateTerminalScrollback('t2', 'later', 50)
    expect(store.terminals('/a')[1]).toMatchObject({ scrollback: 'later', updated_at: 50 })

    const long = 'x'.repeat(TERMINAL_SCROLLBACK_CAP + 20)
    store.saveTerminal(terminal({ id: 't4', project: '/a', created_at: 30, scrollback: long }))
    expect(store.terminals('/a')[2].scrollback).toBe(long.slice(-TERMINAL_SCROLLBACK_CAP))
    store.updateTerminalScrollback('t4', `ab${long}`, 60)
    expect(store.terminals('/a')[2].scrollback).toBe(`ab${long}`.slice(-TERMINAL_SCROLLBACK_CAP))
    expect(store.terminals('/a')[2].scrollback).toHaveLength(TERMINAL_SCROLLBACK_CAP)

    store.deleteTerminal('t1')
    expect(store.terminals('/a').map((row) => row.id)).toEqual(['t2', 't4'])
    expect(store.terminals('/b').map((row) => row.id)).toEqual(['t3'])
    store.deleteTerminal('missing')
    expect(store.terminals('/a')).toHaveLength(2)
  })
})

test('a corrupt data row is skipped', () => {
  const root = mkdtempSync(join(tmpdir(), 'ao-store-bad-'))
  const path = join(root, 'db.sqlite')
  try {
    const store = new Store(path)
    store.saveChat({
      id: 'c1',
      project: '/a',
      title: 't',
      created_at: 1,
      updated_at: 1,
      session_id: null,
      session_provider: null
    })
    store.saveMessage('c1', 0, { id: 'm1', role: 'user', text: 'ok', status: 'done' })
    store.saveJob('/a', 1, job({ id: 'j1', output: 'ok' }))
    store.close()

    const raw = new DatabaseSync(path)
    raw
      .prepare(`INSERT INTO messages (id, chat_id, seq, data) VALUES (?, ?, ?, ?)`)
      .run('bad', 'c1', 1, '{not-json')
    raw
      .prepare(`INSERT INTO messages (id, chat_id, seq, data) VALUES (?, ?, ?, ?)`)
      .run('noid', 'c1', 2, '{"text":"x"}')
    raw
      .prepare(`INSERT INTO jobs (id, project, created_at, data) VALUES (?, ?, ?, ?)`)
      .run('badj', '/a', 2, 'nope')
    raw.close()

    const again = new Store(path)
    expect(again.messages('c1').map((message) => message.id)).toEqual(['m1'])
    expect(again.jobs('/a', 10).map((item) => item.id)).toEqual(['j1'])
    again.close()
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})
