import BotAvatar from './BotAvatar'
import ModelPicker from './ModelPicker'

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

function usageAvatarState(word: PlanWord): 'idle' | 'working' | 'resting' {
  if (word === 'not signed in' || word === 'resting') return 'resting'
  if (word === 'busy') return 'working'
  return 'idle'
}

function statusClass(word: PlanWord): string {
  if (word === 'not signed in') return 'status status-out'
  return `status status-${word}`
}

function meterFillClass(id: PlanStatus['id'], left: number): string {
  if (left < 15) return 'meter-fill meter-bad'
  if (left < 30) return 'meter-fill meter-warn'
  return `meter-fill meter-${id}`
}

function UsageHeading(): React.JSX.Element {
  return (
    <div className="panel-head">
      <h2 id="usage-heading">Usage</h2>
    </div>
  )
}

function UsageMeter({
  plans,
  now
}: {
  plans: PlanStatus[] | null
  now: number
}): React.JSX.Element {
  if (plans === null) {
    return (
      <div className="usage">
        <UsageHeading />
        <p className="hint">Checking plans…</p>
      </div>
    )
  }

  if (plans.length === 0) {
    return (
      <div className="usage">
        <UsageHeading />
        <p className="hint">No plans found. Sign in to the claude or agent CLI.</p>
      </div>
    )
  }

  return (
    <div className="usage">
      <UsageHeading />
      <div className="usage-list">
        {plans.map((plan) => {
          const word = planWord(plan, now)
          const left = headroomPercent(plan.used)
          const restingUntil = plan.restingUntil
          const reset = plan.resetsAt === null ? null : resetPhrase(plan.resetsAt, now)
          const showResting = word === 'resting' && restingUntil !== null
          const showQueued = plan.queued > 0
          const hasDetails = showResting || reset !== null || showQueued || plan.atRisk
          return (
            <div key={plan.id} className="usage-card-wrap">
              <div className="usage-card">
                <div className="usage-head">
                  <BotAvatar bot={plan.id} state={usageAvatarState(word)} size={32} />
                  <span className="usage-name">{plan.id}</span>
                  <span className={statusClass(word)}>
                    <span className="status-dot" aria-hidden="true" />
                    {word}
                  </span>
                </div>
                <div className="usage-readout">
                  <div className="usage-figure">
                    <span className="usage-percent">{left}%</span>
                    <span className="usage-left-word">left</span>
                  </div>
                  <div
                    className="meter"
                    role="meter"
                    aria-valuemin={0}
                    aria-valuemax={100}
                    aria-valuenow={left}
                    aria-label={`${plan.id} allowance left`}
                  >
                    <div className={meterFillClass(plan.id, left)} style={{ width: `${left}%` }} />
                  </div>
                </div>
                {hasDetails ? (
                  <div className="usage-details">
                    {showResting && restingUntil !== null ? (
                      <p className="usage-detail">resting until {clock(restingUntil)}</p>
                    ) : null}
                    {reset !== null ? <p className="usage-detail">{reset}</p> : null}
                    {showQueued ? <p className="usage-detail">{plan.queued} queued</p> : null}
                    {plan.atRisk ? (
                      <p className="usage-risk">
                        <span className="usage-risk-mark" aria-hidden="true">
                          !
                        </span>
                        Unused allowance expires soon
                      </p>
                    ) : null}
                  </div>
                ) : null}
                <ModelPicker plan={plan} disabled={word === 'not signed in'} />
              </div>
            </div>
          )
        })}
        <p className="model-hint">A model applies to the next job or lead turn on that plan.</p>
      </div>
    </div>
  )
}

export default UsageMeter
