import type { JobRecord, LeadMessage, PlanStatus, TerminalInfo } from '../../preload/api-types'

export interface FlowEvent {
  id: number
  at: number
  actor: 'lead' | 'router' | 'claude' | 'cursor'
  text: string
  result?: string
  tone?: 'ok' | 'bad' | 'warn'
}

export interface FlowSnapshot {
  jobs: JobRecord[]
  plans: PlanStatus[]
  lead: LeadMessage[]
  terminals: TerminalInfo[]
}

type FlowChange = Omit<FlowEvent, 'id'>

/** The first snapshot has nothing to compare with, so it yields no events. */
export function observeFlow(
  prev: FlowSnapshot | null,
  next: FlowSnapshot,
  now: number
): FlowChange[] {
  if (prev === null) return []
  return diffFlow(prev, next, now)
}

export function diffFlow(prev: FlowSnapshot, next: FlowSnapshot, now: number): FlowChange[] {
  return [
    ...diffLead(prev.lead, next.lead, now),
    ...diffRouter(prev.jobs, next.jobs, now),
    ...diffWorkers(prev, next, now),
    ...diffTerminals(prev.terminals, next.terminals, now)
  ]
}

function change(
  now: number,
  actor: FlowEvent['actor'],
  text: string,
  tone?: FlowEvent['tone'],
  result?: string
): FlowChange {
  const event: FlowChange = { at: now, actor, text }
  if (result !== undefined) event.result = result
  if (tone !== undefined) event.tone = tone
  return event
}

function diffLead(
  prev: readonly LeadMessage[],
  next: readonly LeadMessage[],
  now: number
): FlowChange[] {
  const prevById = new Map(prev.map((message) => [message.id, message]))
  const events: FlowChange[] = []
  for (const message of next) {
    if (message.role !== 'lead') continue
    const before = prevById.get(message.id)
    if (before === undefined) {
      events.push(change(now, 'lead', 'turn started'))
      continue
    }
    if (before.status !== 'streaming') continue
    if (message.status === 'done') {
      events.push(change(now, 'lead', 'turn finished', 'ok', 'ok'))
    } else if (message.status === 'error') {
      events.push(change(now, 'lead', 'turn failed', 'bad'))
    }
  }
  return events
}

function diffRouter(
  prev: readonly JobRecord[],
  next: readonly JobRecord[],
  now: number
): FlowChange[] {
  const prevById = new Map(prev.map((job) => [job.id, job]))
  const events: FlowChange[] = []
  for (const job of next) {
    const before = prevById.get(job.id)
    if (before === undefined) {
      const target = job.provider ?? 'no plan'
      const routed = `${job.type} → ${target}`
      events.push(change(now, 'router', job.reason ? `${routed} · ${job.reason}` : routed))
    }
    if (before === undefined || job.failedOver.length <= before.failedOver.length) continue
    const added = job.failedOver.slice(before.failedOver.length)
    for (let index = 0; index < added.length; index++) {
      const from = added[index]
      const to = index + 1 < added.length ? added[index + 1] : (job.provider ?? 'no plan')
      events.push(change(now, 'router', `${job.type} failover ${from} → ${to}`, 'warn'))
    }
  }
  return events
}

function planOrder(prev: FlowSnapshot, next: FlowSnapshot): Array<'claude' | 'cursor'> {
  const ids: Array<'claude' | 'cursor'> = []
  const seen = new Set<string>()
  const add = (id: 'claude' | 'cursor'): void => {
    if (seen.has(id)) return
    seen.add(id)
    ids.push(id)
  }
  for (const plan of next.plans) add(plan.id)
  for (const plan of prev.plans) add(plan.id)
  for (const job of next.jobs) {
    if (job.provider !== null) add(job.provider)
  }
  for (const job of prev.jobs) {
    if (job.provider !== null) add(job.provider)
  }
  return ids
}

function isResting(plan: PlanStatus, now: number): boolean {
  return plan.restingUntil !== null && plan.restingUntil > now
}

export function formatHoursMinutes(ms: number): string {
  const date = new Date(ms)
  const hours = date.getHours().toString().padStart(2, '0')
  const minutes = date.getMinutes().toString().padStart(2, '0')
  return `${hours}:${minutes}`
}

function diffWorkers(prev: FlowSnapshot, next: FlowSnapshot, now: number): FlowChange[] {
  const prevJobs = new Map(prev.jobs.map((job) => [job.id, job]))
  const prevPlans = new Map(prev.plans.map((plan) => [plan.id, plan]))
  const nextPlans = new Map(next.plans.map((plan) => [plan.id, plan]))
  const events: FlowChange[] = []

  for (const planId of planOrder(prev, next)) {
    for (const job of next.jobs) {
      if (job.provider !== planId) continue
      const before = prevJobs.get(job.id)
      if (job.status === 'running' && (before === undefined || before.status !== 'running')) {
        events.push(change(now, planId, `${job.type} running`))
      }
      if (job.status === 'done' && (before === undefined || before.status !== 'done')) {
        events.push(change(now, planId, `${job.type} done`, 'ok', 'ok'))
      }
      if (job.status === 'failed' && (before === undefined || before.status !== 'failed')) {
        events.push(
          change(now, planId, `${job.type} failed`, 'bad', (job.error ?? '').slice(0, 40))
        )
      }
    }

    const beforePlan = prevPlans.get(planId)
    const afterPlan = nextPlans.get(planId)
    if (beforePlan === undefined || afterPlan === undefined) continue
    const wasResting = isResting(beforePlan, now)
    const nowResting = isResting(afterPlan, now)
    if (!wasResting && nowResting && afterPlan.restingUntil !== null) {
      events.push(
        change(now, planId, `resting until ${formatHoursMinutes(afterPlan.restingUntil)}`, 'warn')
      )
    } else if (wasResting && !nowResting) {
      events.push(change(now, planId, 'back', 'ok'))
    }
    if (beforePlan.model !== afterPlan.model) {
      events.push(change(now, planId, `model → ${afterPlan.model ?? 'CLI default'}`))
    }
  }

  return events
}

function diffTerminals(
  prev: readonly TerminalInfo[],
  next: readonly TerminalInfo[],
  now: number
): FlowChange[] {
  const prevById = new Map(prev.map((terminal) => [terminal.id, terminal]))
  const nextIds = new Set(next.map((terminal) => terminal.id))
  const events: FlowChange[] = []

  for (const terminal of next) {
    const before = prevById.get(terminal.id)
    if (before === undefined) {
      events.push(change(now, terminal.provider, `terminal opened (${terminal.title})`))
    }
    if (terminal.status === 'exited' && (before === undefined || before.status !== 'exited')) {
      const code = terminal.exitCode
      events.push(
        change(
          now,
          terminal.provider,
          `terminal ended (${terminal.title})`,
          code === 0 ? 'ok' : 'bad',
          `code ${code ?? 'null'}`
        )
      )
    }
  }

  for (const terminal of prev) {
    if (!nextIds.has(terminal.id)) {
      events.push(change(now, terminal.provider, `terminal closed (${terminal.title})`))
    }
  }

  return events
}
