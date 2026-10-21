import { useEffect, useRef, useState } from 'react'
import { planOf } from '../../core/types'
import { Star } from './Star'
import BotAvatar from './BotAvatar'
import ModelPicker from './ModelPicker'
import ResetCalendar from './ResetCalendar'
import {
  agoPhrase,
  dateFromValue,
  dateValue,
  resetChoices,
  resetLabel,
  updatedPhrase
} from './usage-format'

const CURSOR_ESTIMATE =
  "Cursor does not report its usage. This is an estimate from Legate's own jobs."
/** Where Cursor shows the plan's usage so far and the date it resets. */
const CURSOR_DASHBOARD = 'https://cursor.com/dashboard/spending'
/** Window name the main process stores a dashboard reading under. */
const DASHBOARD_WINDOW = 'dashboard'

type PlanStatus = Awaited<ReturnType<Window['api']['listPlans']>>[number]

/** One card per plan. Grok spends Cursor, so its running work shows on that card. */
function meterPlans(plans: readonly PlanStatus[]): PlanStatus[] {
  const running = new Map<PlanStatus['id'], number>()
  const queued = new Map<PlanStatus['id'], number>()
  const busy = new Set<PlanStatus['id']>()
  for (const plan of plans) {
    const spent = planOf(plan.id)
    if (spent === plan.id) continue
    running.set(spent, (running.get(spent) ?? 0) + plan.running)
    queued.set(spent, (queued.get(spent) ?? 0) + plan.queued)
    if (plan.busy) busy.add(spent)
  }
  return plans
    .filter((plan) => planOf(plan.id) === plan.id)
    .map((plan) => {
      const moreRunning = running.get(plan.id) ?? 0
      const moreQueued = queued.get(plan.id) ?? 0
      if (moreRunning === 0 && moreQueued === 0 && !busy.has(plan.id)) return plan
      return {
        ...plan,
        running: plan.running + moreRunning,
        queued: plan.queued + moreQueued,
        busy: plan.busy || busy.has(plan.id)
      }
    })
}

const MINUTE_MS = 60_000
const CURSOR_MODEL_HINT = 'Cursor keeps the last model used as its own default.'

type PlanWord = 'not installed' | 'signed out' | 'resting' | 'busy' | 'ready'

function headroomPercent(used: number): number {
  const raw = Math.round((1 - used) * 100)
  if (!Number.isFinite(raw)) return 0
  if (raw < 0) return 0
  if (raw > 100) return 100
  return raw
}

function planWord(plan: PlanStatus, now: number): PlanWord {
  if (plan.problem === 'not-installed') return 'not installed'
  if (plan.problem === 'signed-out' || !plan.available) return 'signed out'
  if (plan.restingUntil !== null && plan.restingUntil > now) return 'resting'
  if (plan.busy || plan.running > 0) return 'busy'
  return 'ready'
}

function planStatusLabel(plan: PlanStatus, word: PlanWord): string {
  if (word === 'busy' && plan.running > 1) return `${plan.running} running`
  return word
}

function cliVersionLine(plan: PlanStatus): string {
  if (plan.version === null || plan.versionStatus === 'unknown') return 'CLI version unknown'
  if (plan.versionStatus === 'tested') return `CLI ${plan.version}`
  if (plan.versionStatus === 'newer') {
    return `CLI ${plan.version} · newer than tested (${plan.testedVersion})`
  }
  return `CLI ${plan.version} · older than tested (${plan.testedVersion}) — update it`
}

function problemNotice(plan: PlanStatus): string | null {
  if (plan.problem === 'not-installed') {
    return plan.id === 'claude'
      ? 'Claude Code is not installed. Install it, then run `claude` once in a terminal to sign in.'
      : 'The Cursor CLI is not installed. Install it, then run `agent login` in a terminal.'
  }
  if (plan.problem === 'signed-out') {
    return plan.id === 'claude'
      ? 'Claude Code is signed out. Run `claude` in a terminal and sign in.'
      : 'The Cursor CLI is signed out. Run `agent login` in a terminal.'
  }
  return null
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
  if (word === 'not installed' || word === 'signed out' || word === 'resting') return 'resting'
  if (word === 'busy') return 'working'
  return 'idle'
}

function statusClass(word: PlanWord, running: number): string {
  if (word === 'not installed' || word === 'signed out') return 'status status-out'
  if (word === 'busy' && running > 1) return 'status status-running'
  return `status status-${word}`
}

function planUnusable(word: PlanWord): boolean {
  return word === 'not installed' || word === 'signed out'
}

function meterFillClass(id: PlanStatus['id'], left: number): string {
  if (left < 15) return 'meter-fill meter-bad'
  if (left < 30) return 'meter-fill meter-warn'
  return `meter-fill meter-${id}`
}

function windowLabel(name: string): string {
  if (name === 'five_hour') return '5-hour'
  if (name === 'seven_day') return 'weekly'
  if (name === DASHBOARD_WINDOW) return 'this month'
  return name
}

function Chevron({ up }: { up: boolean }): React.JSX.Element {
  return (
    <svg width="10" height="6" viewBox="0 0 10 6" aria-hidden="true" focusable="false">
      <path
        d={up ? 'M1 5 5 1l4 4' : 'M1 1l4 4 4-4'}
        fill="none"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  )
}

/** The slider's position for what is typed: clamped to 0–100, 0 while it is not a number yet. */
function sliderValue(percent: string): number {
  const value = Number(percent)
  if (percent.trim() === '' || !Number.isFinite(value)) return 0
  return Math.min(100, Math.max(0, value))
}

/** "37% used · 63% left", as the card will show it once saved. */
function usedSummary(percent: string): string {
  const value = Number(percent)
  if (percent.trim() === '' || !Number.isFinite(value) || value < 0 || value > 100) {
    return 'enter 0 to 100'
  }
  const used = Math.round(value * 10) / 10
  const left = Math.round((100 - value) * 10) / 10
  return `${String(used)}% used · ${String(left)}% left`
}

/**
 * Cursor reports no usage, so the person can copy it from their dashboard. Legate then adds its
 * own Cursor and Grok jobs on top until the next reading.
 */
function CursorReading({ plan, now }: { plan: PlanStatus; now: number }): React.JSX.Element {
  const reading = plan.windows.find((slot) => slot.name === DASHBOARD_WINDOW)
  const [editing, setEditing] = useState(false)
  const [percent, setPercent] = useState('')
  const [resets, setResets] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)

  function startEditing(): void {
    // Start from what the card shows now, so a new reading is a nudge from there.
    setPercent(String(reading === undefined ? 0 : Math.round(reading.used * 100)))
    // Keep the date of the last reading when it is still one of the choices.
    const last = reading?.resetsAt != null ? dateValue(reading.resetsAt) : ''
    setResets(resetChoices(now).some((choice) => choice.value === last) ? last : '')
    setError(null)
    setEditing(true)
  }

  function save(): void {
    const resetsAt = dateFromValue(resets)
    const used = percent.trim() === '' ? Number.NaN : Number(percent)
    setSaving(true)
    window.api
      .setCursorReading(used, resetsAt ?? Number.NaN)
      .then(
        (result) => {
          if (result.ok) {
            setEditing(false)
            setError(null)
          } else {
            setError(result.error ?? 'Could not save the reading.')
          }
        },
        (err: unknown) => {
          setError(err instanceof Error ? err.message : String(err))
        }
      )
      .finally(() => {
        setSaving(false)
      })
  }

  function clear(): void {
    window.api.clearCursorReading().catch((err: unknown) => {
      setError(err instanceof Error ? err.message : String(err))
    })
  }

  if (editing) {
    return (
      <form
        className="usage-reading-form"
        onSubmit={(event) => {
          event.preventDefault()
          save()
        }}
      >
        <p className="hint">
          Copy these from{' '}
          <a href={CURSOR_DASHBOARD} target="_blank" rel="noreferrer">
            your Cursor spending page
          </a>
          , for the usage your Legate jobs draw on.
        </p>
        <div className="usage-reading-field">
          <div className="usage-reading-label">
            <span>Used</span>
            <span className="usage-reading-value">{usedSummary(percent)}</span>
          </div>
          <div className="usage-reading-percent">
            <input
              type="range"
              min={0}
              max={100}
              step={1}
              value={sliderValue(percent)}
              autoFocus
              aria-label="Percent of Cursor usage used"
              style={{ '--fill': `${String(sliderValue(percent))}%` } as React.CSSProperties}
              onChange={(event) => setPercent(event.target.value)}
            />
            {/* Fine-tune after a drag: one percent per click, like the slider's arrow keys. */}
            <span className="usage-reading-step">
              <button
                type="button"
                aria-label="One percent more"
                disabled={sliderValue(percent) >= 100}
                onClick={() =>
                  setPercent(String(Math.min(100, Math.round(sliderValue(percent)) + 1)))
                }
              >
                <Chevron up />
              </button>
              <button
                type="button"
                aria-label="One percent less"
                disabled={sliderValue(percent) <= 0}
                onClick={() =>
                  setPercent(String(Math.max(0, Math.round(sliderValue(percent)) - 1)))
                }
              >
                <Chevron up={false} />
              </button>
            </span>
          </div>
        </div>
        <div className="usage-reading-field">
          <div className="usage-reading-label">
            <span>Resets on</span>
            <span className="usage-reading-value">
              {resets === '' ? 'pick a day' : resetLabel(dateFromValue(resets) ?? now, now)}
            </span>
          </div>
          <ResetCalendar value={resets} now={now} onChange={setResets} />
        </div>
        {error !== null ? <p className="field-error">{error}</p> : null}
        <div className="usage-reading-actions">
          <button type="submit" className="btn btn-compact" disabled={saving}>
            Save
          </button>
          <button
            type="button"
            className="btn btn-quiet btn-compact"
            onClick={() => {
              setEditing(false)
              setError(null)
            }}
          >
            Cancel
          </button>
        </div>
      </form>
    )
  }

  return (
    <>
      <p className="usage-fresh hint">
        <span>
          {reading === undefined || plan.usageAt === null
            ? CURSOR_ESTIMATE
            : `From your dashboard, read ${agoPhrase(plan.usageAt, now)}, plus Legate's jobs since.`}
        </span>
        <span className="usage-reading-actions">
          <button
            type="button"
            className="btn btn-quiet btn-compact"
            title="Enter the usage shown on cursor.com/dashboard/spending"
            onClick={startEditing}
          >
            {reading === undefined ? 'Set from dashboard' : 'Update'}
          </button>
          {reading !== undefined ? (
            <button type="button" className="btn btn-quiet btn-compact" onClick={clear}>
              Clear
            </button>
          ) : null}
        </span>
      </p>
      {error !== null ? <p className="error-text">{error}</p> : null}
    </>
  )
}

function WarnGlyph(): React.JSX.Element {
  return (
    <svg
      className="usage-risk-mark"
      width="12"
      height="12"
      viewBox="0 0 16 16"
      aria-hidden="true"
      focusable="false"
    >
      <path
        fill="currentColor"
        fillRule="evenodd"
        d="M8 1.5 15 14H1L8 1.5ZM7.25 6h1.5v4.2h-1.5V6Zm0 5.4h1.5V13h-1.5v-1.6Z"
      />
    </svg>
  )
}

function UsageHeading({ onOpenSetup }: { onOpenSetup: () => void }): React.JSX.Element {
  return (
    <div className="panel-head">
      <div className="panel-title">
        <Star size={12} />
        <h2 id="usage-heading">Usage</h2>
      </div>
      <button type="button" className="btn btn-quiet btn-compact" onClick={onOpenSetup}>
        Setup
      </button>
    </div>
  )
}

function UsageMeter({
  plans,
  now,
  onOpenSetup
}: {
  plans: PlanStatus[] | null
  now: number
  onOpenSetup: () => void
}): React.JSX.Element {
  const [checking, setChecking] = useState(false)
  const checkingRef = useRef(false)
  const [refreshing, setRefreshing] = useState(false)
  const [refreshError, setRefreshError] = useState<string | null>(null)

  function refresh(): void {
    setRefreshing(true)
    window.api
      .refreshUsage()
      .then(
        (result) => {
          setRefreshError(result.ok ? null : (result.error ?? 'Could not read the usage.'))
        },
        (err: unknown) => {
          setRefreshError(err instanceof Error ? err.message : String(err))
        }
      )
      .finally(() => {
        setRefreshing(false)
      })
  }

  // Opening Usage asks Claude for its numbers; the main process skips it if they are fresh.
  useEffect(() => {
    let current = true
    window.api.refreshUsage().then(
      (result) => {
        if (current && !result.ok) setRefreshError(result.error ?? null)
      },
      () => {}
    )
    return () => {
      current = false
    }
  }, [])

  async function checkAgain(): Promise<void> {
    if (checkingRef.current) return
    checkingRef.current = true
    setChecking(true)
    try {
      await window.api.recheckPlans()
    } finally {
      checkingRef.current = false
      setChecking(false)
    }
  }

  if (plans === null) {
    return (
      <div className="usage">
        <UsageHeading onOpenSetup={onOpenSetup} />
        <p className="hint">Checking plans…</p>
      </div>
    )
  }

  if (plans.length === 0) {
    return (
      <div className="usage">
        <UsageHeading onOpenSetup={onOpenSetup} />
        <p className="hint">No plans found. Sign in to the claude or agent CLI.</p>
      </div>
    )
  }

  return (
    <div className="usage">
      <UsageHeading onOpenSetup={onOpenSetup} />
      <div className="usage-list">
        {meterPlans(plans).map((plan) => {
          const word = planWord(plan, now)
          const notice = problemNotice(plan)
          const left = headroomPercent(plan.used)
          const restingUntil = plan.restingUntil
          const hasWindows = plan.windows.length > 0
          const reset =
            hasWindows || plan.resetsAt === null ? null : resetPhrase(plan.resetsAt, now)
          const showResting = word === 'resting' && restingUntil !== null
          const showQueued = plan.queued > 0
          const showRunning = plan.running > 1
          const hasDetails =
            showResting || reset !== null || showQueued || showRunning || plan.atRisk
          return (
            <div key={plan.id} className="usage-card-wrap">
              <div className="usage-card">
                <div className="usage-head">
                  <BotAvatar bot={plan.id} state={usageAvatarState(word)} size={32} />
                  <span className="usage-name">{plan.id}</span>
                  <span className={statusClass(word, plan.running)}>
                    <span className="status-dot" aria-hidden="true" />
                    {planStatusLabel(plan, word)}
                  </span>
                </div>
                {notice !== null ? (
                  <div className="usage-problem">
                    <p className="hint">{notice}</p>
                    <button
                      type="button"
                      className="btn btn-compact"
                      disabled={checking}
                      onClick={() => {
                        void checkAgain()
                      }}
                    >
                      Check again
                    </button>
                  </div>
                ) : (
                  <>
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
                        <div
                          className={meterFillClass(plan.id, left)}
                          style={{ width: `${left}%` }}
                        />
                      </div>
                      {hasWindows ? (
                        <div className="usage-windows">
                          {plan.windows.map((slot, index) => {
                            const slotLeft = headroomPercent(slot.used)
                            const slotLabel = windowLabel(slot.name)
                            const slotReset =
                              slot.resetsAt === null ? null : resetPhrase(slot.resetsAt, now)
                            return (
                              <div key={`${slot.name}-${index}`} className="usage-window">
                                <div className="usage-window-top">
                                  <span className="usage-window-label" title={slotLabel}>
                                    {slotLabel}
                                  </span>
                                  <span className="usage-window-left">{slotLeft}% left</span>
                                </div>
                                <div
                                  className="meter meter-thin"
                                  role="meter"
                                  aria-valuemin={0}
                                  aria-valuemax={100}
                                  aria-valuenow={slotLeft}
                                  aria-label={`${plan.id} ${slotLabel} allowance left`}
                                >
                                  <div
                                    className={meterFillClass(plan.id, slotLeft)}
                                    style={{ width: `${slotLeft}%` }}
                                  />
                                </div>
                                {slotReset !== null ? (
                                  <span className="usage-window-reset" title={slotReset}>
                                    {slotReset}
                                  </span>
                                ) : null}
                              </div>
                            )
                          })}
                        </div>
                      ) : null}
                    </div>
                    {plan.id === 'claude' ? (
                      <p className="usage-fresh hint">
                        <span>
                          {plan.usageAt === null
                            ? 'not read yet'
                            : updatedPhrase(plan.usageAt, now)}
                        </span>
                        <button
                          type="button"
                          className="btn btn-quiet btn-compact"
                          disabled={refreshing}
                          title="Ask Claude for its usage now. Counts your use outside Legate too."
                          onClick={refresh}
                        >
                          {refreshing ? 'Reading…' : 'Refresh'}
                        </button>
                      </p>
                    ) : planOf(plan.id) === 'cursor' ? (
                      <CursorReading plan={plan} now={now} />
                    ) : null}
                    {plan.id === 'claude' && refreshError !== null ? (
                      <p className="error-text">{refreshError}</p>
                    ) : null}
                    {hasDetails ? (
                      <div className="usage-details">
                        {showResting && restingUntil !== null ? (
                          <p className="usage-detail">resting until {clock(restingUntil)}</p>
                        ) : null}
                        {reset !== null ? <p className="usage-detail">{reset}</p> : null}
                        {showRunning ? (
                          <p className="usage-detail">{plan.running} running</p>
                        ) : null}
                        {showQueued ? <p className="usage-detail">{plan.queued} queued</p> : null}
                        {plan.atRisk ? (
                          <p className="usage-risk">
                            <WarnGlyph />
                            Unused allowance expires soon
                          </p>
                        ) : null}
                      </div>
                    ) : null}
                  </>
                )}
                <ModelPicker
                  plan={plan}
                  disabled={planUnusable(word)}
                  labelTitle={plan.id === 'cursor' ? CURSOR_MODEL_HINT : undefined}
                />
                {plan.problem === 'not-installed' ? null : (
                  <p className="hint">{cliVersionLine(plan)}</p>
                )}
              </div>
            </div>
          )
        })}
      </div>
      <p className="model-hint">A model applies to the next job or lead turn on that plan.</p>
    </div>
  )
}

export default UsageMeter
