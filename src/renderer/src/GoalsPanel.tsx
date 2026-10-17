import { useEffect, useState } from 'react'
import { Star } from './Star'

type Goal = Awaited<ReturnType<Window['api']['listGoals']>>[number]
type PlannedJob = Goal['rounds'][number]['jobs'][number]
type JobRecord = Awaited<ReturnType<Window['api']['listJobs']>>[number]

const JOB_TYPES: PlannedJob['type'][] = [
  'planning',
  'debugging',
  'review',
  'refactor',
  'boilerplate'
]
const ACCESS: PlannedJob['access'][] = ['edit', 'full']
const STATUS_TEXT: Record<Goal['status'], string> = {
  planning: 'planning',
  waiting: 'needs you',
  running: 'working',
  checking: 'checking',
  done: 'done',
  stopped: 'stopped',
  failed: 'failed'
}

function errorText(err: unknown): string {
  if (!(err instanceof Error)) return String(err)
  return err.message.replace(/^Error invoking remote method '[^']+': (Error: )?/, '')
}

function firstLine(text: string, max = 90): string {
  const line = text.trim().split(/\r?\n/)[0] ?? ''
  return line.length > max ? `${line.slice(0, max - 1)}…` : line
}

function upsert(list: Goal[], goal: Goal): Goal[] {
  const rest = list.filter((entry) => entry.id !== goal.id)
  return [goal, ...rest].sort((a, b) => b.createdAt - a.createdAt)
}

/** Goals: the lead plans a round, you approve it, jobs run, the check runs, and again. */
function GoalsPanel({
  jobs,
  onViewChange,
  onShowJob
}: {
  jobs: JobRecord[]
  onViewChange: (id: string) => void
  onShowJob: (id: string) => void
}): React.JSX.Element {
  const [goals, setGoals] = useState<Goal[] | null>(null)
  const [selected, setSelected] = useState<string | null>(null)
  const [creating, setCreating] = useState(false)
  const [text, setText] = useState('')
  const [check, setCheck] = useState('')
  const [rounds, setRounds] = useState(5)
  const [draft, setDraft] = useState<{ goal: string; round: number; jobs: PlannedJob[] } | null>(
    null
  )
  const [note, setNote] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let live = true
    window.api.listGoals().then(
      (list) => {
        if (live) setGoals(list)
      },
      (err: unknown) => {
        if (live) setError(errorText(err))
      }
    )
    const off = window.api.onGoalUpdate((goal) => {
      setGoals((prev) => upsert(prev ?? [], goal))
    })
    return () => {
      live = false
      off()
    }
  }, [])

  const goal = goals?.find((entry) => entry.id === selected) ?? goals?.[0] ?? null
  const proposal = goal?.status === 'waiting' ? goal.proposal : null
  const edited =
    goal !== null &&
    proposal !== null &&
    draft?.goal === goal.id &&
    draft.round === goal.rounds.length
      ? draft.jobs
      : (proposal?.jobs ?? [])

  async function act(task: () => Promise<unknown>): Promise<void> {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      await task()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  function editJob(index: number, patch: Partial<PlannedJob>): void {
    if (goal === null) return
    const jobsNow = edited.map((job, at) => (at === index ? { ...job, ...patch } : job))
    setDraft({ goal: goal.id, round: goal.rounds.length, jobs: jobsNow })
  }

  function removeJob(index: number): void {
    if (goal === null) return
    setDraft({
      goal: goal.id,
      round: goal.rounds.length,
      jobs: edited.filter((_job, at) => at !== index)
    })
  }

  function jobStatus(id: string): string {
    return jobs.find((job) => job.id === id)?.status ?? 'ended'
  }

  return (
    <div className="goals">
      <div className="panel-head">
        <div className="panel-title">
          <Star size={12} />
          <h2>Goals</h2>
        </div>
        <button
          type="button"
          className="btn btn-compact"
          onClick={() => {
            setCreating((on) => !on)
          }}
        >
          {creating ? 'Close' : 'New goal'}
        </button>
      </div>
      {creating ? (
        <form
          className="goal-form"
          onSubmit={(event) => {
            event.preventDefault()
            void act(async () => {
              const made = await window.api.createGoal({ text, check, maxRounds: rounds })
              setGoals((prev) => upsert(prev ?? [], made))
              setSelected(made.id)
              setText('')
              setCheck('')
              setCreating(false)
            })
          }}
        >
          <textarea
            rows={3}
            value={text}
            placeholder="What should be true when this is done? e.g. Login keeps the session after a token refresh."
            aria-label="Goal"
            onChange={(event) => setText(event.target.value)}
          />
          <input
            value={check}
            placeholder="Check, optional: a command that passes when it is done, e.g. pnpm test"
            aria-label="Check command"
            onChange={(event) => setCheck(event.target.value)}
          />
          <div className="goal-form-foot">
            <label className="goal-rounds">
              At most
              <input
                type="number"
                min={1}
                max={20}
                value={rounds}
                onChange={(event) => setRounds(Number(event.target.value) || 1)}
              />
              rounds
            </label>
            <button type="submit" className="btn btn-compact" disabled={busy || text.trim() === ''}>
              Start
            </button>
          </div>
          <p className="hint">
            The lead plans each round and waits for your OK. Changes collect in one worktree for you
            to review in Changes.
          </p>
        </form>
      ) : null}
      {error !== null ? <p className="field-error">{error}</p> : null}
      {goals !== null && goals.length === 0 && !creating ? (
        <p className="hint">
          No goals yet. Start one here, or add a check to a job in Jobs to fix until it passes.
        </p>
      ) : null}
      {goals !== null && goals.length > 0 ? (
        <ul className="goal-list">
          {goals.map((entry) => (
            <li key={entry.id}>
              <button
                type="button"
                className={entry.id === goal?.id ? 'goal-row is-selected' : 'goal-row'}
                onClick={() => {
                  setSelected(entry.id)
                }}
              >
                <span className={`goal-status is-${entry.status}`}>
                  {STATUS_TEXT[entry.status]}
                </span>
                <span className="goal-text">{firstLine(entry.text)}</span>
                <span className="goal-round">
                  {entry.quick
                    ? 'task'
                    : `${String(entry.rounds.length)}/${String(entry.maxRounds)}`}
                </span>
              </button>
            </li>
          ))}
        </ul>
      ) : null}
      {goal !== null ? (
        <section className="goal-detail" aria-label="Goal">
          <p className="goal-title">{goal.text}</p>
          {goal.check !== null ? (
            <p className="goal-check">
              Check: <code>{goal.check}</code>
            </p>
          ) : null}

          {goal.status === 'planning' ? (
            <p className="goal-state">
              The lead is reading the code and planning round {goal.rounds.length + 1}…
            </p>
          ) : null}
          {goal.status === 'checking' ? <p className="goal-state">Running the check…</p> : null}

          {proposal !== null ? (
            <div className="goal-proposal">
              <p className="goal-state">
                Round {goal.rounds.length + 1}: {proposal.summary}
              </p>
              {edited.map((job, index) => (
                <div key={index} className="goal-job">
                  <div className="goal-job-head">
                    <select
                      value={job.type}
                      aria-label="Job type"
                      onChange={(event) =>
                        editJob(index, { type: event.target.value as PlannedJob['type'] })
                      }
                    >
                      {JOB_TYPES.map((type) => (
                        <option key={type} value={type}>
                          {type}
                        </option>
                      ))}
                    </select>
                    <select
                      value={job.access}
                      aria-label="Access"
                      onChange={(event) =>
                        editJob(index, { access: event.target.value as PlannedJob['access'] })
                      }
                    >
                      {ACCESS.map((value) => (
                        <option key={value} value={value}>
                          {value === 'full' ? 'edit + run commands' : 'edit files'}
                        </option>
                      ))}
                    </select>
                    <button
                      type="button"
                      className="btn btn-quiet btn-compact"
                      aria-label="Leave this job out"
                      onClick={() => removeJob(index)}
                    >
                      ×
                    </button>
                  </div>
                  <textarea
                    rows={4}
                    value={job.prompt}
                    aria-label="Job prompt"
                    onChange={(event) => editJob(index, { prompt: event.target.value })}
                  />
                </div>
              ))}
              <div className="goal-actions">
                <button
                  type="button"
                  className="btn btn-compact"
                  disabled={busy || edited.length === 0}
                  onClick={() => {
                    void act(() => window.api.approveGoal(goal.id, edited))
                  }}
                >
                  Approve round
                </button>
                <button
                  type="button"
                  className="btn btn-quiet btn-compact"
                  disabled={busy}
                  onClick={() => {
                    void act(() => window.api.stopGoal(goal.id))
                  }}
                >
                  Stop
                </button>
              </div>
            </div>
          ) : null}

          {goal.status === 'waiting' && goal.question !== null ? (
            <p className="goal-question">{goal.question}</p>
          ) : null}
          {goal.outcome !== null ? <p className="goal-outcome">{goal.outcome}</p> : null}

          {goal.status === 'waiting' || goal.status === 'stopped' || goal.status === 'failed' ? (
            <div className="goal-answer">
              <textarea
                rows={2}
                value={note}
                placeholder={
                  proposal !== null
                    ? 'Or tell the lead what to change, and it plans the round again.'
                    : 'Tell the lead what to do next.'
                }
                aria-label="Note for the lead"
                onChange={(event) => setNote(event.target.value)}
              />
              <button
                type="button"
                className="btn btn-quiet btn-compact"
                disabled={busy}
                onClick={() => {
                  void act(async () => {
                    await window.api.answerGoal(goal.id, note)
                    setNote('')
                  })
                }}
              >
                {proposal !== null ? 'Plan again' : 'Continue'}
              </button>
            </div>
          ) : null}

          {goal.status === 'running' ? (
            <div className="goal-actions">
              <button
                type="button"
                className="btn btn-quiet btn-compact"
                disabled={busy}
                onClick={() => {
                  void act(() => window.api.stopGoal(goal.id))
                }}
              >
                Stop
              </button>
            </div>
          ) : null}

          {goal.status === 'done' || goal.status === 'stopped' || goal.status === 'failed' ? (
            <div className="goal-actions">
              <button
                type="button"
                className="btn btn-compact"
                onClick={() => {
                  onViewChange(goal.worktree)
                }}
              >
                Review in Changes
              </button>
              <button
                type="button"
                className="btn btn-quiet btn-compact"
                disabled={busy}
                onClick={() => {
                  void act(async () => {
                    await window.api.removeGoal(goal.id)
                    setGoals((prev) => (prev ?? []).filter((entry) => entry.id !== goal.id))
                    setSelected(null)
                  })
                }}
              >
                Remove
              </button>
            </div>
          ) : null}

          {goal.rounds.length > 0 ? (
            <ol className="goal-rounds-list">
              {goal.rounds
                .slice()
                .reverse()
                .map((round) => (
                  <li key={round.n} className="goal-round-item">
                    <p className="goal-round-head">
                      Round {round.n}: {round.summary}
                    </p>
                    <ul className="goal-round-jobs">
                      {round.jobIds.map((id, index) => (
                        <li key={id}>
                          <button
                            type="button"
                            className="goal-round-job"
                            onClick={() => {
                              onShowJob(id)
                            }}
                          >
                            <span className={`goal-job-status is-${jobStatus(id)}`}>
                              {jobStatus(id)}
                            </span>
                            {firstLine(round.jobs[index]?.prompt ?? id, 70)}
                          </button>
                        </li>
                      ))}
                    </ul>
                    {round.problems.map((problem) => (
                      <p key={problem} className="goal-problem">
                        {problem}
                      </p>
                    ))}
                    {round.check !== null ? (
                      <details className="goal-check-result" open={round.n === goal.rounds.length}>
                        <summary className={round.check.code === 0 ? 'is-pass' : 'is-fail'}>
                          Check{' '}
                          {round.check.code === 0
                            ? 'passed'
                            : `failed (exit ${String(round.check.code)})`}
                        </summary>
                        <pre>{round.check.output.slice(-3000)}</pre>
                      </details>
                    ) : null}
                  </li>
                ))}
            </ol>
          ) : null}
        </section>
      ) : null}
    </div>
  )
}

export default GoalsPanel
