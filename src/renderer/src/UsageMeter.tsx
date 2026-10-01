import { useEffect, useState } from 'react'

type PlanStatus = Awaited<ReturnType<Window['api']['listPlans']>>[number]

const MINUTE_MS = 60_000

type PlanWord = 'not signed in' | 'resting' | 'busy' | 'ready'

function headroomPercent(used: number): number {
  const raw = Math.round((1 - used) * 100)
  if (!Number.isFinite(raw)) return 0
  if (raw < 0) return 0
  if (raw > 100) return 100
  return raw
}

function planWord(plan: PlanStatus, now: number): PlanWord {
  if (!plan.available) return 'not signed in'
  if (plan.restingUntil !== null && plan.restingUntil > now) return 'resting'
  if (plan.busy) return 'busy'
  return 'ready'
}

function clock(ms: number): string {
  const date = new Date(ms)
  const hours = date.getHours().toString().padStart(2, '0')
  const minutes = date.getMinutes().toString().padStart(2, '0')
  return `${hours}:${minutes}`
}

function resetPhrase(resetsAt: number, now: number): string | null {
  const remaining = resetsAt - now
  if (remaining <= 0) return null
  if (remaining < MINUTE_MS) return 'resets in under 1m'
  const days = Math.floor(remaining / (24 * 60 * MINUTE_MS))
  const hours = Math.floor(remaining / (60 * MINUTE_MS)) % 24
  const minutes = Math.floor(remaining / MINUTE_MS) % 60
  const parts: string[] = []
  if (days > 0) parts.push(`${days}d`)
  if (hours > 0) parts.push(`${hours}h`)
  if (minutes > 0) parts.push(`${minutes}m`)
  return `resets in ${parts.join(' ')}`
}

function UsageMeter(): React.JSX.Element {
  const [plans, setPlans] = useState<PlanStatus[] | null>(null)
  const [now, setNow] = useState(() => Date.now())

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
    const timer = window.setInterval(() => {
      setNow(Date.now())
    }, MINUTE_MS)
    return () => {
      window.clearInterval(timer)
    }
  }, [])

  if (plans === null) {
    return (
      <div className="usage">
        <p className="hint">Checking plans…</p>
      </div>
    )
  }

  if (plans.length === 0) {
    return (
      <div className="usage">
        <p className="hint">No plans found. Sign in to the claude or agent CLI.</p>
      </div>
    )
  }

  return (
    <div className="usage">
      <div className="usage-list">
        {plans.map((plan) => {
          const word = planWord(plan, now)
          const left = headroomPercent(plan.used)
          const dim = word === 'not signed in' || word === 'resting'
          const restingUntil = plan.restingUntil
          const reset = plan.resetsAt === null ? null : resetPhrase(plan.resetsAt, now)
          return (
            <div key={plan.id} className={dim ? 'usage-card is-dim' : 'usage-card'}>
              <div className="usage-head">
                <span className="usage-name">{plan.id}</span>
                <span className="badge">{word}</span>
              </div>
              <div className="usage-meter-row">
                <div
                  className="meter"
                  role="meter"
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={left}
                  aria-label={`${plan.id} allowance left`}
                >
                  <div
                    className={left < 15 ? 'meter-fill is-low' : 'meter-fill'}
                    style={{ width: `${left}%` }}
                  />
                </div>
                <span className="usage-left">{left}% left</span>
              </div>
              {word === 'resting' && restingUntil !== null ? (
                <p className="usage-detail">resting until {clock(restingUntil)}</p>
              ) : null}
              {reset !== null ? <p className="usage-detail">{reset}</p> : null}
              {plan.queued > 0 ? <p className="usage-detail">{plan.queued} queued</p> : null}
              {plan.atRisk ? (
                <p className="usage-risk">
                  <span className="usage-risk-mark" aria-hidden="true">
                    !
                  </span>
                  Unused allowance expires soon
                </p>
              ) : null}
            </div>
          )
        })}
      </div>
    </div>
  )
}

export default UsageMeter
