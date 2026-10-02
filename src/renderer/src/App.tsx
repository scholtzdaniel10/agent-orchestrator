import { useEffect, useRef, useState } from 'react'
import LeadChat from './LeadChat'
import UsageMeter from './UsageMeter'
import Workers from './Workers'
import { createTerminalBus, type TerminalBus } from './terminal-bus'

type LeadMessage = Awaited<ReturnType<Window['api']['listLeadMessages']>>[number]
type JobRecord = Awaited<ReturnType<Window['api']['listJobs']>>[number]
type PlanStatus = Awaited<ReturnType<Window['api']['listPlans']>>[number]
type TerminalInfo = Awaited<ReturnType<Window['api']['listTerminals']>>[number]

const terminalBus: TerminalBus = createTerminalBus((listener) =>
  window.api.onTerminalData(listener)
)

const MINUTE_MS = 60_000

function mergeMessage(messages: LeadMessage[], message: LeadMessage): LeadMessage[] {
  const index = messages.findIndex((item) => item.id === message.id)
  if (index === -1) return [...messages, message]
  const next = messages.slice()
  next[index] = message
  return next
}

function mergeMessageList(current: LeadMessage[], list: LeadMessage[]): LeadMessage[] {
  const listed = new Set(list.map((message) => message.id))
  const live = new Map(current.map((message) => [message.id, message]))
  const merged = list.map((message) => live.get(message.id) ?? message)
  for (const message of current) {
    if (!listed.has(message.id)) merged.push(message)
  }
  return merged
}

function mergeJob(jobs: JobRecord[], job: JobRecord): JobRecord[] {
  const index = jobs.findIndex((item) => item.id === job.id)
  if (index === -1) return [...jobs, job]
  const next = jobs.slice()
  next[index] = job
  return next
}

function mergeJobList(current: JobRecord[], list: JobRecord[]): JobRecord[] {
  const listed = new Set(list.map((job) => job.id))
  const live = new Map(current.map((job) => [job.id, job]))
  const merged = list.map((job) => live.get(job.id) ?? job)
  for (const job of current) {
    if (!listed.has(job.id)) merged.push(job)
  }
  return merged
}

function applyTerminalUpdate(
  current: TerminalInfo[],
  info: TerminalInfo,
  removed: boolean
): TerminalInfo[] {
  if (removed) return current.filter((item) => item.id !== info.id)
  const index = current.findIndex((item) => item.id === info.id)
  if (index === -1) return [...current, info]
  const next = current.slice()
  next[index] = info
  return next
}

function mergeTerminalList(
  current: TerminalInfo[],
  list: TerminalInfo[],
  removed: ReadonlySet<string>
): TerminalInfo[] {
  const live = new Map(current.map((item) => [item.id, item]))
  const seen = new Set<string>()
  const merged: TerminalInfo[] = []
  for (const info of list) {
    if (removed.has(info.id)) continue
    seen.add(info.id)
    merged.push(live.get(info.id) ?? info)
  }
  for (const info of current) {
    if (!seen.has(info.id) && !removed.has(info.id)) merged.push(info)
  }
  return merged
}

/** Signed in and not resting. A busy plan still counts: it is in use, not unavailable. */
function readyPlans(plans: readonly PlanStatus[], now: number): number {
  return plans.filter((plan) => {
    if (!plan.available) return false
    return plan.restingUntil === null || plan.restingUntil <= now
  }).length
}

function summaryLine(ready: number, running: number, queued: number): string {
  const plans = ready === 1 ? '1 plan ready' : `${ready} plans ready`
  return `${plans} · ${running} running · ${queued} queued`
}

function App(): React.JSX.Element {
  const [messages, setMessages] = useState<LeadMessage[]>([])
  const [jobs, setJobs] = useState<JobRecord[]>([])
  const [plans, setPlans] = useState<PlanStatus[] | null>(null)
  const [terminals, setTerminals] = useState<TerminalInfo[]>([])
  const [initialTerminalIds, setInitialTerminalIds] = useState<ReadonlySet<string> | null>(null)
  const [now, setNow] = useState(() => Date.now())
  const acceptList = useRef(true)

  useEffect(() => {
    let active = true
    const unsubscribe = window.api.onLeadUpdate((message) => {
      setMessages((current) => mergeMessage(current, message))
    })
    void window.api.listLeadMessages().then((list) => {
      if (!active || !acceptList.current) return
      setMessages((current) => mergeMessageList(current, list))
    })
    return () => {
      active = false
      unsubscribe()
    }
  }, [])

  useEffect(() => {
    let active = true
    const unsubscribe = window.api.onJobUpdate((job) => {
      setJobs((current) => mergeJob(current, job))
    })
    void window.api.listJobs().then((list) => {
      if (!active) return
      setJobs((current) => mergeJobList(current, list))
    })
    return () => {
      active = false
      unsubscribe()
    }
  }, [])

  useEffect(() => {
    let active = true
    let sawUpdate = false
    const unsubscribe = window.api.onPlansUpdate((next) => {
      sawUpdate = true
      setPlans(next)
    })
    void window.api.listPlans().then((list) => {
      if (!active || sawUpdate) return
      setPlans(list)
    })
    return () => {
      active = false
      unsubscribe()
    }
  }, [])

  useEffect(() => {
    let active = true
    const removed = new Set<string>()
    const unsubscribe = window.api.onTerminalUpdate((info, isRemoved) => {
      if (isRemoved) {
        removed.add(info.id)
        terminalBus.forget(info.id)
      } else {
        removed.delete(info.id)
      }
      setTerminals((current) => applyTerminalUpdate(current, info, isRemoved))
    })
    void window.api.listTerminals().then(
      (list) => {
        if (!active) return
        setInitialTerminalIds((current) => current ?? new Set(list.map((info) => info.id)))
        setTerminals((current) => mergeTerminalList(current, list, removed))
      },
      () => {
        if (!active) return
        setInitialTerminalIds((current) => current ?? new Set())
      }
    )
    return () => {
      active = false
      unsubscribe()
    }
  }, [])

  useEffect(() => {
    const timer = window.setInterval(() => {
      setNow(Date.now())
    }, MINUTE_MS)
    return () => {
      window.clearInterval(timer)
    }
  }, [])

  async function resetLead(): Promise<void> {
    await window.api.resetLead()
    acceptList.current = false
    setMessages([])
  }

  const ready = readyPlans(plans ?? [], now)
  const running = jobs.filter((job) => job.status === 'running').length
  const queued = jobs.filter((job) => job.status === 'queued').length

  return (
    <div className="app">
      <header className="app-header">
        <div className="app-name">agent-orchestrator</div>
        <p className="app-summary">{summaryLine(ready, running, queued)}</p>
      </header>
      <section className="panel panel-lead" aria-labelledby="lead-heading">
        <LeadChat messages={messages} onReset={resetLead} />
      </section>
      <section className="panel panel-workers" aria-labelledby="workers-heading">
        <Workers
          jobs={jobs}
          terminals={terminals}
          initialTerminalIds={initialTerminalIds}
          bus={terminalBus}
          onJob={(job) => {
            setJobs((current) => mergeJob(current, job))
          }}
          onTerminal={(info) => {
            setTerminals((current) => applyTerminalUpdate(current, info, false))
          }}
        />
      </section>
      <section className="panel panel-usage" aria-labelledby="usage-heading">
        <UsageMeter plans={plans} now={now} />
      </section>
    </div>
  )
}

export default App
