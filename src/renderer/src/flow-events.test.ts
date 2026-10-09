import { expect, test } from 'vitest'
import type {
  ChangeSet,
  JobRecord,
  LeadMessage,
  PlanStatus,
  TerminalInfo
} from '../../preload/api-types'
import { diffFlow, formatHoursMinutes, observeFlow, type FlowSnapshot } from './flow-events'

const NOW = Date.UTC(2026, 9, 2, 8, 0, 0)

function plan(partial: Partial<PlanStatus> & Pick<PlanStatus, 'id'>): PlanStatus {
  return {
    available: true,
    used: 0.2,
    restingUntil: null,
    resetsAt: null,
    atRisk: false,
    busy: false,
    running: 0,
    queued: 0,
    model: null,
    windows: [],
    ...partial
  }
}

function job(partial: Partial<JobRecord> & Pick<JobRecord, 'id' | 'status'>): JobRecord {
  return {
    type: 'planning',
    prompt: 'prompt',
    provider: 'claude',
    output: '',
    failedOver: [],
    ...partial
  }
}

function lead(
  partial: Partial<LeadMessage> & Pick<LeadMessage, 'id' | 'role' | 'status'>
): LeadMessage {
  return {
    text: '',
    ...partial
  }
}

function terminal(partial: Partial<TerminalInfo> & Pick<TerminalInfo, 'id'>): TerminalInfo {
  return {
    provider: 'claude',
    title: 'claude 1',
    status: 'running',
    exitCode: null,
    model: null,
    startedAt: NOW,
    ...partial
  }
}

function changeSet(partial: Partial<ChangeSet> & Pick<ChangeSet, 'id'>): ChangeSet {
  return {
    branch: `orch/${partial.id}`,
    path: partial.id,
    files: [{ path: 'notes.txt', insertions: 1, deletions: 0 }],
    insertions: 1,
    deletions: 0,
    ...partial
  }
}

function snap(partial: Partial<FlowSnapshot> = {}): FlowSnapshot {
  return {
    jobs: partial.jobs ?? [],
    plans: partial.plans ?? [],
    lead: partial.lead ?? [],
    terminals: partial.terminals ?? [],
    changes: partial.changes ?? []
  }
}

function clone(snapshot: FlowSnapshot): FlowSnapshot {
  return {
    jobs: snapshot.jobs.map((item) => ({ ...item, failedOver: [...item.failedOver] })),
    plans: snapshot.plans.map((item) => ({ ...item })),
    lead: snapshot.lead.map((item) => ({ ...item })),
    terminals: snapshot.terminals.map((item) => ({ ...item })),
    changes: snapshot.changes.map((item) => ({
      ...item,
      files: item.files.map((file) => ({ ...file }))
    }))
  }
}

test('a new lead message starts a turn and a user message does not', () => {
  const prev = snap()
  const next = snap({
    lead: [
      lead({ id: 'u', role: 'user', status: 'done', text: 'hello' }),
      lead({ id: 'a', role: 'lead', status: 'streaming', text: '' })
    ]
  })
  expect(diffFlow(prev, next, NOW)).toEqual([{ at: NOW, actor: 'lead', text: 'turn started' }])
})

test('a lead turn finishing or failing is recorded once', () => {
  const streaming = lead({ id: 'a', role: 'lead', status: 'streaming', text: 'partial' })
  const done = diffFlow(
    snap({ lead: [streaming] }),
    snap({ lead: [{ ...streaming, status: 'done', text: 'full' }] }),
    NOW
  )
  const failed = diffFlow(
    snap({ lead: [streaming] }),
    snap({ lead: [{ ...streaming, status: 'error', text: 'nope' }] }),
    NOW
  )
  const stillStreaming = diffFlow(
    snap({ lead: [streaming] }),
    snap({ lead: [{ ...streaming, text: 'more' }] }),
    NOW
  )
  expect(done).toEqual([
    { at: NOW, actor: 'lead', text: 'turn finished', result: 'ok', tone: 'ok' }
  ])
  expect(failed).toEqual([{ at: NOW, actor: 'lead', text: 'turn failed', tone: 'bad' }])
  expect(stillStreaming).toEqual([])
})

test('a new job with a reason appends it after the plan', () => {
  const prev = snap()
  const next = snap({
    jobs: [
      job({
        id: 'j1',
        status: 'queued',
        provider: 'cursor',
        type: 'review',
        reason: 'allowance expiring'
      }),
      job({
        id: 'j2',
        status: 'queued',
        provider: null,
        type: 'refactor',
        reason: 'chosen'
      })
    ]
  })
  expect(diffFlow(prev, next, NOW)).toEqual([
    { at: NOW, actor: 'router', text: 'review → cursor · allowance expiring' },
    { at: NOW, actor: 'router', text: 'refactor → no plan · chosen' }
  ])
})

test('a new running job keeps the reason on the router line only', () => {
  const next = snap({
    jobs: [
      job({
        id: 'j',
        status: 'running',
        provider: 'claude',
        type: 'boilerplate',
        reason: 'first choice'
      })
    ],
    plans: [plan({ id: 'claude' })]
  })
  expect(diffFlow(snap(), next, NOW)).toEqual([
    { at: NOW, actor: 'router', text: 'boilerplate → claude · first choice' },
    { at: NOW, actor: 'claude', text: 'boilerplate running' }
  ])
})

test('a new job names its plan, or no plan', () => {
  const prev = snap()
  const next = snap({
    jobs: [
      job({ id: 'j1', status: 'queued', provider: 'cursor', type: 'review' }),
      job({ id: 'j2', status: 'queued', provider: null, type: 'refactor' })
    ]
  })
  expect(diffFlow(prev, next, NOW)).toEqual([
    { at: NOW, actor: 'router', text: 'review → cursor' },
    { at: NOW, actor: 'router', text: 'refactor → no plan' }
  ])
})

test('a job becoming running is attributed to its provider', () => {
  const queued = job({ id: 'j', status: 'queued', provider: 'claude', type: 'debugging' })
  expect(
    diffFlow(snap({ jobs: [queued] }), snap({ jobs: [{ ...queued, status: 'running' }] }), NOW)
  ).toEqual([{ at: NOW, actor: 'claude', text: 'debugging running' }])
})

test('a job becoming done or failed records the result', () => {
  const running = job({ id: 'j', status: 'running', provider: 'cursor', type: 'boilerplate' })
  const done = diffFlow(
    snap({ jobs: [running] }),
    snap({ jobs: [{ ...running, status: 'done' }] }),
    NOW
  )
  const error = 'x'.repeat(50)
  const failed = diffFlow(
    snap({ jobs: [running] }),
    snap({ jobs: [{ ...running, status: 'failed', error }] }),
    NOW
  )
  expect(done).toEqual([
    { at: NOW, actor: 'cursor', text: 'boilerplate done', result: 'ok', tone: 'ok' }
  ])
  expect(failed).toEqual([
    {
      at: NOW,
      actor: 'cursor',
      text: 'boilerplate failed',
      result: error.slice(0, 40),
      tone: 'bad'
    }
  ])
})

test('a job whose failedOver grew records the handoff', () => {
  const before = job({ id: 'j', status: 'running', provider: 'claude', type: 'planning' })
  const after = job({
    id: 'j',
    status: 'queued',
    provider: 'cursor',
    type: 'planning',
    failedOver: ['claude']
  })
  expect(diffFlow(snap({ jobs: [before] }), snap({ jobs: [after] }), NOW)).toEqual([
    { at: NOW, actor: 'router', text: 'planning failover claude → cursor', tone: 'warn' }
  ])
})

test('a plan entering or leaving rest is recorded', () => {
  const until = NOW + 90 * 60 * 1000
  const awake = plan({ id: 'claude' })
  const resting = plan({ id: 'claude', restingUntil: until })
  expect(diffFlow(snap({ plans: [awake] }), snap({ plans: [resting] }), NOW)).toEqual([
    {
      at: NOW,
      actor: 'claude',
      text: `resting until ${formatHoursMinutes(until)}`,
      tone: 'warn'
    }
  ])
  expect(diffFlow(snap({ plans: [resting] }), snap({ plans: [awake] }), NOW)).toEqual([
    { at: NOW, actor: 'claude', text: 'back', tone: 'ok' }
  ])
})

test('a plan model change names the new model or the CLI default', () => {
  const claude = plan({ id: 'claude', model: 'opus' })
  const cursor = plan({ id: 'cursor', model: 'fast' })
  expect(
    diffFlow(
      snap({ plans: [claude, cursor] }),
      snap({
        plans: [
          { ...claude, model: 'sonnet' },
          { ...cursor, model: null }
        ]
      }),
      NOW
    )
  ).toEqual([
    { at: NOW, actor: 'claude', text: 'model → sonnet' },
    { at: NOW, actor: 'cursor', text: 'model → CLI default' }
  ])
})

test('terminals opening, ending, and disappearing are recorded', () => {
  const running = terminal({ id: 't1', provider: 'claude', title: 'claude 1' })
  const opened = diffFlow(snap(), snap({ terminals: [running] }), NOW)
  const endedOk = diffFlow(
    snap({ terminals: [running] }),
    snap({ terminals: [{ ...running, status: 'exited', exitCode: 0 }] }),
    NOW
  )
  const endedBad = diffFlow(
    snap({ terminals: [running] }),
    snap({ terminals: [{ ...running, status: 'exited', exitCode: 7 }] }),
    NOW
  )
  const closed = diffFlow(snap({ terminals: [running] }), snap(), NOW)
  expect(opened).toEqual([{ at: NOW, actor: 'claude', text: 'terminal opened (claude 1)' }])
  expect(endedOk).toEqual([
    {
      at: NOW,
      actor: 'claude',
      text: 'terminal ended (claude 1)',
      result: 'code 0',
      tone: 'ok'
    }
  ])
  expect(endedBad).toEqual([
    {
      at: NOW,
      actor: 'claude',
      text: 'terminal ended (claude 1)',
      result: 'code 7',
      tone: 'bad'
    }
  ])
  expect(closed).toEqual([{ at: NOW, actor: 'claude', text: 'terminal closed (claude 1)' }])
})

test('the first snapshot yields nothing', () => {
  const first = snap({
    jobs: [job({ id: 'j', status: 'running', provider: 'claude', edit: true, change: 'abcd1234' })],
    lead: [lead({ id: 'a', role: 'lead', status: 'done', text: 'hi' })],
    terminals: [terminal({ id: 't' })],
    plans: [plan({ id: 'claude', restingUntil: NOW + 60_000, model: 'opus' })],
    changes: [changeSet({ id: 'abcd1234' })]
  })
  expect(observeFlow(null, first, NOW)).toEqual([])
})

test('an unchanged snapshot yields nothing', () => {
  const current = snap({
    jobs: [job({ id: 'j', status: 'running', provider: 'claude', output: 'partial' })],
    lead: [lead({ id: 'a', role: 'lead', status: 'streaming', text: 'partial', model: 'opus' })],
    plans: [plan({ id: 'claude', model: 'opus', busy: true })],
    terminals: [terminal({ id: 't', status: 'exited', exitCode: 0 })],
    changes: [changeSet({ id: 'abcd1234' })]
  })
  expect(diffFlow(current, clone(current), NOW)).toEqual([])
  const edited = clone(current)
  edited.jobs[0] = { ...edited.jobs[0], output: 'more text' }
  edited.lead[0] = { ...edited.lead[0], text: 'more text' }
  expect(diffFlow(current, edited, NOW)).toEqual([])
})

test('several changes in one diff stay in lead, router, plan, then terminal order', () => {
  const until = NOW + 30 * 60 * 1000
  const streaming = lead({ id: 'a', role: 'lead', status: 'streaming' })
  const prev = snap({
    lead: [streaming],
    jobs: [
      job({ id: 'j1', status: 'running', provider: 'claude', type: 'debugging' }),
      job({ id: 'j3', status: 'running', provider: 'claude', type: 'refactor' })
    ],
    plans: [plan({ id: 'claude', model: 'old' }), plan({ id: 'cursor', model: null })],
    terminals: [terminal({ id: 'old', provider: 'claude', title: 'claude 1' })]
  })
  const next = snap({
    lead: [
      { ...streaming, status: 'done', text: 'done' },
      lead({ id: 'b', role: 'lead', status: 'streaming' })
    ],
    jobs: [
      job({ id: 'j1', status: 'done', provider: 'claude', type: 'debugging' }),
      job({ id: 'j2', status: 'running', provider: 'cursor', type: 'review' }),
      job({
        id: 'j3',
        status: 'queued',
        provider: 'cursor',
        type: 'refactor',
        failedOver: ['claude']
      })
    ],
    plans: [
      plan({ id: 'claude', model: 'old', restingUntil: until }),
      plan({ id: 'cursor', model: 'gpt' })
    ],
    terminals: [terminal({ id: 'new', provider: 'cursor', title: 'cursor 1' })]
  })
  expect(diffFlow(prev, next, NOW).map((event) => `${event.actor}: ${event.text}`)).toEqual([
    'lead: turn finished',
    'lead: turn started',
    'router: review → cursor',
    'router: refactor failover claude → cursor',
    'claude: debugging done',
    `claude: resting until ${formatHoursMinutes(until)}`,
    'cursor: review running',
    'cursor: model → gpt',
    'cursor: terminal opened (cursor 1)',
    'claude: terminal closed (claude 1)'
  ])
})

test('an editing job ends the router line with edits', () => {
  const next = snap({
    jobs: [
      job({
        id: 'j',
        status: 'running',
        provider: 'claude',
        type: 'boilerplate',
        edit: true,
        reason: 'first choice'
      })
    ],
    plans: [plan({ id: 'claude' })]
  })
  expect(diffFlow(snap(), next, NOW)).toEqual([
    { at: NOW, actor: 'router', text: 'boilerplate → claude · first choice · edits' },
    { at: NOW, actor: 'claude', text: 'boilerplate running' }
  ])
})

test('an editing job without a reason still ends with edits', () => {
  expect(
    diffFlow(
      snap(),
      snap({
        jobs: [job({ id: 'j', status: 'queued', provider: null, type: 'review', edit: true })]
      }),
      NOW
    )
  ).toEqual([{ at: NOW, actor: 'router', text: 'review → no plan · edits' }])
})

test('a change ready event uses the provider of the job that owns it', () => {
  const item = changeSet({ id: 'abcd1234' })
  const owned = job({ id: 'j', status: 'done', provider: 'cursor', change: 'abcd1234' })
  expect(diffFlow(snap({ jobs: [owned] }), snap({ jobs: [owned], changes: [item] }), NOW)).toEqual([
    { at: NOW, actor: 'cursor', text: 'change ready (1 file)', tone: 'ok' }
  ])
})

test('a change with no matching job, or a job with no provider, is ready on the router', () => {
  const lone = changeSet({
    id: 'aaaaaaaa',
    files: [
      { path: 'a.txt', insertions: 1, deletions: 0 },
      { path: 'b.txt', insertions: 2, deletions: 1 }
    ]
  })
  const unassigned = changeSet({ id: 'bbbbbbbb' })
  const owned = job({ id: 'j', status: 'done', provider: null, change: 'bbbbbbbb' })
  expect(diffFlow(snap(), snap({ changes: [lone] }), NOW)).toEqual([
    { at: NOW, actor: 'router', text: 'change ready (2 files)', tone: 'ok' }
  ])
  expect(
    diffFlow(snap({ jobs: [owned] }), snap({ jobs: [owned], changes: [unassigned] }), NOW)
  ).toEqual([{ at: NOW, actor: 'router', text: 'change ready (1 file)', tone: 'ok' }])
})

test('a change that disappears is closed by the router', () => {
  const staying = changeSet({ id: 'aaaaaaaa' })
  const leaving = changeSet({ id: 'bbbbbbbb' })
  const arriving = changeSet({
    id: 'cccccccc',
    files: [
      { path: 'a.txt', insertions: 1, deletions: 0 },
      { path: 'b.txt', insertions: 1, deletions: 0 },
      { path: 'c.txt', insertions: 1, deletions: 0 }
    ]
  })
  const jobs = [job({ id: 'j', status: 'done', provider: 'claude', change: 'cccccccc' })]
  expect(
    diffFlow(
      snap({ jobs, changes: [staying, leaving] }),
      snap({ jobs, changes: [staying, arriving] }),
      NOW
    ).map((event) => `${event.actor}: ${event.text}`)
  ).toEqual(['claude: change ready (3 files)', 'router: change closed (bbbbbbbb)'])
})

test('a job that is new and already running yields the router line and the running line', () => {
  const next = snap({
    jobs: [job({ id: 'j', status: 'running', provider: 'claude', type: 'boilerplate' })],
    plans: [plan({ id: 'claude' })]
  })
  expect(diffFlow(snap(), next, NOW)).toEqual([
    { at: NOW, actor: 'router', text: 'boilerplate → claude' },
    { at: NOW, actor: 'claude', text: 'boilerplate running' }
  ])
})
