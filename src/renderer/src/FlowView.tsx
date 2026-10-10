import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { Star } from './Star'
import BotAvatar from './BotAvatar'
import { formatHoursMinutes, type FlowEvent, type FlowSnapshot } from './flow-events'

type JobRecord = FlowSnapshot['jobs'][number]
type PlanStatus = FlowSnapshot['plans'][number]
type LeadMessage = FlowSnapshot['lead'][number]
type TerminalInfo = FlowSnapshot['terminals'][number]

function headroomPercent(used: number): number {
  const raw = Math.round((1 - used) * 100)
  if (!Number.isFinite(raw)) return 0
  if (raw < 0) return 0
  if (raw > 100) return 100
  return raw
}

function fillClass(id: PlanStatus['id'], left: number): string {
  if (left < 15) return 'meter-fill meter-bad'
  if (left < 30) return 'meter-fill meter-warn'
  return `meter-fill meter-${id}`
}

function formatElapsed(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000))
  const minutes = Math.floor(total / 60)
  const seconds = total % 60
  if (minutes === 0) return `${seconds}s`
  return `${minutes}m ${seconds}s`
}

function formatLogTime(ms: number): string {
  const date = new Date(ms)
  const hours = date.getHours().toString().padStart(2, '0')
  const minutes = date.getMinutes().toString().padStart(2, '0')
  const seconds = date.getSeconds().toString().padStart(2, '0')
  return `${hours}:${minutes}:${seconds}`
}

function leadAvatar(messages: readonly LeadMessage[]): 'idle' | 'working' | 'error' {
  if (messages.some((message) => message.role === 'lead' && message.status === 'streaming')) {
    return 'working'
  }
  for (let index = messages.length - 1; index >= 0; index--) {
    const message = messages[index]
    if (message.role !== 'lead') continue
    return message.status === 'error' ? 'error' : 'idle'
  }
  return 'idle'
}

function leadDetail(messages: readonly LeadMessage[]): { provider?: string; model?: string } {
  for (let index = messages.length - 1; index >= 0; index--) {
    const message = messages[index]
    if (message.role !== 'lead') continue
    if (message.provider !== undefined || message.model !== undefined) {
      return { provider: message.provider, model: message.model }
    }
  }
  return {}
}

function leadLine(state: 'idle' | 'working' | 'error'): string {
  if (state === 'working') return 'working…'
  if (state === 'error') return 'error'
  return 'idle'
}

function latestJob(jobs: readonly JobRecord[], id: PlanStatus['id']): JobRecord | undefined {
  for (let index = jobs.length - 1; index >= 0; index--) {
    if (jobs[index].provider === id) return jobs[index]
  }
  return undefined
}

function runningJob(jobs: readonly JobRecord[], id: PlanStatus['id']): JobRecord | undefined {
  for (let index = jobs.length - 1; index >= 0; index--) {
    const job = jobs[index]
    if (job.provider === id && job.status === 'running') return job
  }
  return undefined
}

function runningCount(jobs: readonly JobRecord[], id: PlanStatus['id']): number {
  let count = 0
  for (const job of jobs) {
    if (job.provider === id && job.status === 'running') count += 1
  }
  return count
}

function workerAvatar(
  plan: PlanStatus,
  jobs: readonly JobRecord[],
  now: number
): 'idle' | 'working' | 'resting' | 'error' {
  const resting = !plan.available || (plan.restingUntil !== null && plan.restingUntil > now)
  if (resting) return 'resting'
  if (plan.busy || runningJob(jobs, plan.id) !== undefined) return 'working'
  const latest = latestJob(jobs, plan.id)
  if (latest !== undefined && latest.status === 'failed') return 'error'
  return 'idle'
}

function terminalSuffix(terminals: readonly TerminalInfo[], id: PlanStatus['id']): string {
  const count = terminals.filter((terminal) => terminal.provider === id).length
  if (count === 0) return ''
  return ` · ${count} ${count === 1 ? 'terminal' : 'terminals'}`
}

function changeSuffix(
  changes: FlowSnapshot['changes'],
  jobs: readonly JobRecord[],
  id: PlanStatus['id']
): string {
  const ids = new Set<string>()
  for (const job of jobs) {
    if (job.provider === id && job.change !== undefined) ids.add(job.change)
  }
  let count = 0
  for (const item of changes) {
    if (ids.has(item.id)) count += 1
  }
  if (count === 0) return ''
  return ` · ${count} ${count === 1 ? 'change' : 'changes'}`
}

function planFeeding(jobs: readonly JobRecord[], id: PlanStatus['id']): boolean {
  return jobs.some(
    (job) => job.provider === id && (job.status === 'running' || job.status === 'queued')
  )
}

function boxClass(tone: 'lead' | 'claude' | 'cursor' | null, flash: boolean): string {
  const names = ['flow-box']
  if (tone !== null) names.push(`is-${tone}`)
  if (flash) names.push('is-flash')
  return names.join(' ')
}

function resultClass(tone: FlowEvent['tone']): string {
  if (tone === undefined) return 'flow-result'
  return `flow-result is-${tone}`
}

interface Point {
  x: number
  y: number
}

function round(value: number): string {
  return value.toFixed(1)
}

function sidePoint(root: DOMRect, el: HTMLElement, side: 'left' | 'right'): Point {
  const rect = el.getBoundingClientRect()
  return {
    x: (side === 'left' ? rect.left : rect.right) - root.left,
    y: rect.top + rect.height / 2 - root.top
  }
}

function straight(from: Point, to: Point): string {
  return `M ${round(from.x)} ${round(from.y)} L ${round(to.x)} ${round(to.y)}`
}

function elbow(from: Point, to: Point, busX: number): string {
  return `M ${round(from.x)} ${round(from.y)} H ${round(busX)} V ${round(to.y)} H ${round(to.x)}`
}

function paint(root: HTMLElement, name: string, d: string): void {
  const path = root.querySelector(`[data-flow-path="${name}"]`)
  if (path instanceof SVGPathElement) path.setAttribute('d', d)
  const dot = root.querySelector(`[data-flow-dot="${name}"]`)
  if (dot instanceof HTMLElement) dot.style.setProperty('offset-path', `path('${d}')`)
}

function findBox(root: HTMLElement, name: string): HTMLElement | null {
  const el = root.querySelector(`[data-flow="${name}"]`)
  return el instanceof HTMLElement ? el : null
}

function WarnMark(): React.JSX.Element {
  return (
    <span className="flow-risk" title="Unused allowance expires soon">
      <svg width="12" height="12" viewBox="0 0 16 16" aria-hidden="true" focusable="false">
        <path
          fill="currentColor"
          fillRule="evenodd"
          d="M8 1.5 15 14H1L8 1.5ZM7.25 6h1.5v4.2h-1.5V6Zm0 5.4h1.5V13h-1.5v-1.6Z"
        />
      </svg>
    </span>
  )
}

function YouGlyph(): React.JSX.Element {
  return (
    <svg
      className="flow-you-icon"
      width="14"
      height="14"
      viewBox="0 0 16 16"
      aria-hidden="true"
      focusable="false"
    >
      <path
        fill="currentColor"
        d="M8 8a3 3 0 1 0 0-6 3 3 0 0 0 0 6Zm0 1.5c-2.67 0-8 1.34-8 4V15h16v-1.5c0-2.66-5.33-4-8-4Z"
      />
    </svg>
  )
}

function EventLog({ events }: { events: FlowEvent[] }): React.JSX.Element {
  const ref = useRef<HTMLDivElement>(null)
  const stickRef = useRef(true)

  function onScroll(): void {
    const el = ref.current
    if (el === null) return
    stickRef.current = el.scrollHeight - el.scrollTop - el.clientHeight <= 20
  }

  useLayoutEffect(() => {
    const el = ref.current
    if (el === null || !stickRef.current) return
    el.scrollTop = el.scrollHeight
  }, [events])

  return (
    <div className="flow-log">
      <div className="flow-log-head">Activity</div>
      <div className="flow-log-body" role="log" aria-live="off" ref={ref} onScroll={onScroll}>
        {events.length === 0 ? (
          <p className="flow-empty">No activity yet. Events appear here as work happens.</p>
        ) : (
          events.map((event) => (
            <div key={event.id} className="flow-event">
              <span className="flow-time">{formatLogTime(event.at)}</span>
              <span className={`flow-actor flow-actor-${event.actor}`}>{event.actor}</span>
              <span className="flow-text">{event.text}</span>
              <span className={resultClass(event.tone)}>{event.result ?? ''}</span>
            </div>
          ))
        )}
      </div>
    </div>
  )
}

function Topology({
  jobs,
  plans,
  lead,
  terminals,
  changes,
  now,
  elapsedNow,
  starts,
  flashes
}: {
  jobs: JobRecord[]
  plans: PlanStatus[]
  lead: LeadMessage[]
  terminals: TerminalInfo[]
  changes: FlowSnapshot['changes']
  now: number
  elapsedNow: number
  starts: Record<string, number>
  flashes: Record<string, number>
}): React.JSX.Element {
  const rootRef = useRef<HTMLDivElement>(null)
  const streaming = lead.some(
    (message) => message.role === 'lead' && message.status === 'streaming'
  )
  const state = leadAvatar(lead)
  const detail = leadDetail(lead)

  useLayoutEffect(() => {
    const root = rootRef.current
    if (root === null) return

    const draw = (): void => {
      const base = root.getBoundingClientRect()
      if (base.width === 0 || base.height === 0) return
      const you = findBox(root, 'you')
      const leadBox = findBox(root, 'lead')
      const router = findBox(root, 'router')
      if (you !== null && leadBox !== null) {
        paint(
          root,
          'you-lead',
          straight(sidePoint(base, you, 'right'), sidePoint(base, leadBox, 'left'))
        )
      }
      if (leadBox !== null && router !== null) {
        paint(
          root,
          'lead-router',
          straight(sidePoint(base, leadBox, 'right'), sidePoint(base, router, 'left'))
        )
      }
      if (router === null) return
      const from = sidePoint(base, router, 'right')
      const targets: Array<{ id: string; point: Point }> = []
      for (const plan of plans) {
        const box = findBox(root, `worker-${plan.id}`)
        if (box === null) continue
        targets.push({ id: plan.id, point: sidePoint(base, box, 'left') })
      }
      const busX =
        targets.length === 0
          ? from.x
          : (from.x + targets.reduce((sum, target) => sum + target.point.x, 0) / targets.length) / 2
      for (const target of targets) {
        const aligned = targets.length === 1 && Math.abs(from.y - target.point.y) <= 1
        const d = aligned ? straight(from, target.point) : elbow(from, target.point, busX)
        paint(root, `worker-${target.id}`, d)
      }
    }

    draw()
    const observer = new ResizeObserver(draw)
    observer.observe(root)
    return () => {
      observer.disconnect()
    }
  }, [plans, jobs, lead, terminals, changes, elapsedNow, streaming, now, starts, flashes])

  return (
    <div className="topo" ref={rootRef}>
      <div className="topo-overlay">
        <svg className="flow-wires" aria-hidden="true">
          <path
            data-flow-path="you-lead"
            className={streaming ? 'flow-wire is-lead' : 'flow-wire'}
          />
          <path
            data-flow-path="lead-router"
            className={streaming ? 'flow-wire is-router' : 'flow-wire'}
          />
          {plans.map((plan) => (
            <path
              key={plan.id}
              data-flow-path={`worker-${plan.id}`}
              className={planFeeding(jobs, plan.id) ? `flow-wire is-${plan.id}` : 'flow-wire'}
            />
          ))}
        </svg>
        {streaming ? (
          <span className="flow-dot is-lead" data-flow-dot="you-lead" aria-hidden="true" />
        ) : null}
        {streaming ? (
          <span className="flow-dot is-router" data-flow-dot="lead-router" aria-hidden="true" />
        ) : null}
        {plans.map((plan) =>
          planFeeding(jobs, plan.id) ? (
            <span
              key={plan.id}
              className={`flow-dot is-${plan.id}`}
              data-flow-dot={`worker-${plan.id}`}
              aria-hidden="true"
            />
          ) : null
        )}
      </div>
      <div className="flow-box topo-you" data-flow="you">
        <YouGlyph />
        <span className="flow-name">you</span>
      </div>
      <div
        className={`topo-lead ${boxClass(state === 'idle' ? null : 'lead', false)}`}
        data-flow="lead"
      >
        <div className="flow-line">
          <BotAvatar bot="lead" state={state} size={20} />
          <span className="flow-line-text">
            <span className="flow-name is-lead">lead</span>
            {detail.provider !== undefined ? (
              <span className="flow-keep">{` · ${detail.provider}`}</span>
            ) : null}
            {detail.model !== undefined ? (
              <span className="flow-model">{` · ${detail.model}`}</span>
            ) : null}
          </span>
        </div>
        <div className="flow-sub">
          <span className="flow-sub-text">{leadLine(state)}</span>
        </div>
      </div>
      <div className="flow-box topo-router" data-flow="router">
        <div className="flow-router-label">router</div>
        {plans.map((plan) => {
          const left = headroomPercent(plan.used)
          return (
            <div key={plan.id} className="flow-plan-row">
              <span className={`flow-plan-name is-${plan.id}`}>{plan.id}</span>
              <div
                className="meter meter-thin"
                role="meter"
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={left}
                aria-label={`${plan.id} allowance left`}
              >
                <div className={fillClass(plan.id, left)} style={{ width: `${left}%` }} />
              </div>
              <span className="flow-pct">{left}%</span>
              <span className="flow-plan-trail">
                {plan.running > 1 ? (
                  <span className="flow-queued">{`${plan.running} running`}</span>
                ) : null}
                {plan.queued > 0 ? (
                  <span className="flow-queued">{`${plan.queued} queued`}</span>
                ) : null}
                {plan.atRisk ? <WarnMark /> : null}
              </span>
            </div>
          )
        })}
      </div>
      <div className="topo-workers">
        {plans.map((plan) => {
          const avatar = workerAvatar(plan, jobs, now)
          const running = runningJob(jobs, plan.id)
          const parallel = Math.max(plan.running, runningCount(jobs, plan.id))
          const suffix = terminalSuffix(terminals, plan.id)
          const waiting = changeSuffix(changes, jobs, plan.id)
          const extra = `${suffix}${waiting}`
          const tone = avatar === 'working' || avatar === 'error' ? plan.id : null
          let text = `idle${extra}`
          let elapsed: string | null = null
          if (plan.problem === 'not-installed') {
            text = `not installed${extra}`
          } else if (plan.problem === 'signed-out' || !plan.available) {
            text = `signed out${extra}`
          } else if (plan.restingUntil !== null && plan.restingUntil > now) {
            text = `resting until ${formatHoursMinutes(plan.restingUntil)}${extra}`
          } else if (running !== undefined) {
            const started = starts[running.id] ?? elapsedNow
            const more = parallel - 1
            text = more > 0 ? `${running.type} · +${more} more · ` : `${running.type} · running · `
            elapsed = formatElapsed(elapsedNow - started)
          }
          return (
            <div
              key={plan.id}
              data-flow={`worker-${plan.id}`}
              className={boxClass(tone, flashes[plan.id] !== undefined)}
            >
              <div className="flow-line">
                <BotAvatar bot={plan.id} state={avatar} size={20} />
                <span className="flow-line-text">
                  <span className={`flow-name is-${plan.id}`}>{plan.id}</span>
                  <span className="flow-model">{` · ${plan.model ?? 'CLI default'}`}</span>
                </span>
              </div>
              <div className="flow-sub">
                <span className="flow-sub-text">{text}</span>
                {elapsed !== null ? <span className="flow-elapsed">{elapsed}</span> : null}
                {elapsed !== null && extra !== '' ? (
                  <span className="flow-sub-text">{extra}</span>
                ) : null}
              </div>
            </div>
          )
        })}
      </div>
    </div>
  )
}

function FlowView({
  jobs,
  plans,
  lead,
  terminals,
  changes,
  events,
  now,
  starts,
  flashes
}: {
  jobs: JobRecord[]
  plans: PlanStatus[]
  lead: LeadMessage[]
  terminals: TerminalInfo[]
  changes: FlowSnapshot['changes']
  events: FlowEvent[]
  now: number
  starts: Record<string, number>
  flashes: Record<string, number>
}): React.JSX.Element {
  const running = jobs.some((job) => job.status === 'running')
  const [tick, setTick] = useState(0)

  useEffect(() => {
    if (!running) return
    const timer = window.setInterval(() => {
      setTick(Date.now())
    }, 1000)
    return () => {
      window.clearInterval(timer)
    }
  }, [running])

  const elapsedNow = tick === 0 ? now : tick

  return (
    <div className="flow">
      <div className="panel-head">
        <div className="panel-title">
          <Star size={12} />
          <h2 id="flow-heading">Flow</h2>
          <span className="flow-live">
            <span className="flow-live-dot" aria-hidden="true" />
            live
          </span>
        </div>
      </div>
      <div className="flow-body">
        <div className="flow-topology">
          <Topology
            jobs={jobs}
            plans={plans}
            lead={lead}
            terminals={terminals}
            changes={changes}
            now={now}
            elapsedNow={elapsedNow}
            starts={starts}
            flashes={flashes}
          />
        </div>
        <EventLog events={events} />
      </div>
    </div>
  )
}

export default FlowView
