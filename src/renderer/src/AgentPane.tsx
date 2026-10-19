import { useLayoutEffect, useRef, useState } from 'react'
import BotAvatar from './BotAvatar'
import { editedFilesSummary } from './job-steps'
import { agentStatusLabel, stepWord } from './agent-tabs'

type JobRecord = Awaited<ReturnType<Window['api']['listJobs']>>[number]

function errorText(err: unknown): string {
  if (err instanceof Error) {
    return err.message.replace(/^Error invoking remote method '[^']+': (Error: )?/, '')
  }
  if (typeof err === 'string') return err
  return 'Request failed'
}

function avatarState(status: JobRecord['status']): 'idle' | 'working' | 'resting' | 'error' {
  if (status === 'running') return 'working'
  if (status === 'failed') return 'error'
  if (status === 'queued') return 'resting'
  return 'idle'
}

/** A job's live work in the middle of the window, as a terminal would show it, with Take over. */
function AgentPane({
  job,
  onTakeOver,
  onClose
}: {
  job: JobRecord
  /** Resolves when the job's terminal is open; rejects with a reason to show. */
  onTakeOver: () => Promise<void>
  onClose: () => void
}): React.JSX.Element {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const logRef = useRef<HTMLDivElement | null>(null)
  const stickRef = useRef(true)
  const steps = job.steps ?? []
  const edits = editedFilesSummary(job)
  const live = job.status === 'running' || job.status === 'queued'

  useLayoutEffect(() => {
    const log = logRef.current
    if (log !== null && stickRef.current) log.scrollTop = log.scrollHeight
  }, [steps.length, job.output.length])

  function takeOver(): void {
    setBusy(true)
    setError(null)
    onTakeOver().catch((err: unknown) => {
      setError(errorText(err))
      setBusy(false)
    })
  }

  function stop(): void {
    setError(null)
    window.api.cancelJob(job.id).catch((err: unknown) => {
      setError(errorText(err))
    })
  }

  const meta = [job.provider ?? 'waiting for a worker', job.type, job.access ?? 'read']
  if (job.model !== undefined) meta.push(job.model)

  return (
    <section className="agent-pane" aria-label={`Agent: ${job.title ?? job.type}`}>
      <header className="agent-pane-head">
        <div className="agent-pane-title">
          {job.provider !== null ? (
            <BotAvatar bot={job.provider} state={avatarState(job.status)} size={22} />
          ) : null}
          <div className="agent-pane-name">
            <strong>{job.title ?? job.prompt.slice(0, 60)}</strong>
            <span className="agent-pane-meta">{meta.join(' · ')}</span>
          </div>
          <span className={`status status-${job.status}`}>
            <span className="status-dot" aria-hidden="true" />
            {agentStatusLabel(job)}
          </span>
        </div>
        <div className="agent-pane-actions">
          <button
            type="button"
            className="btn"
            disabled={busy || job.sessionId === undefined}
            title={
              job.sessionId === undefined
                ? 'Take over is possible once the agent has started'
                : live
                  ? 'Stop the job and continue its session in a terminal'
                  : 'Continue this session in a terminal'
            }
            onClick={takeOver}
          >
            {busy ? 'Opening…' : 'Take over'}
          </button>
          {live ? (
            <button type="button" className="btn btn-quiet" onClick={stop}>
              Stop
            </button>
          ) : null}
          <button type="button" className="btn btn-quiet" onClick={onClose}>
            Back to terminals
          </button>
        </div>
      </header>
      {error !== null ? (
        <p className="field-error" role="alert">
          {error}
        </p>
      ) : null}
      <div
        ref={logRef}
        className="agent-pane-log"
        onScroll={(event) => {
          const el = event.currentTarget
          stickRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24
        }}
      >
        <p className="agent-pane-prompt">&gt; {job.prompt}</p>
        {steps.map((step) => (
          <p key={step.id} className={`agent-pane-step is-${step.status}`}>
            <span className="agent-pane-word">{stepWord(step.tool)}</span>
            <span>{step.title}</span>
            {step.edit !== undefined ? (
              <span className="agent-pane-edit">
                <span className="stat-add">{`+${String(step.edit.added)}`}</span>{' '}
                <span className="stat-del">{`−${String(step.edit.removed)}`}</span>
              </span>
            ) : null}
            {step.status === 'failed' ? <span className="agent-pane-failed">failed</span> : null}
          </p>
        ))}
        {job.output !== '' ? <div className="agent-pane-output">{job.output}</div> : null}
        {job.status === 'queued' ? (
          <p className="agent-pane-wait">Waiting for a free worker…</p>
        ) : null}
        {job.error !== undefined ? <p className="agent-pane-error">{job.error}</p> : null}
        {edits !== null && !live ? (
          <p className="agent-pane-done">
            {`Edited ${edits.files === 1 ? '1 file' : `${String(edits.files)} files`} `}
            <span className="stat-add">{`+${String(edits.added)}`}</span>{' '}
            <span className="stat-del">{`−${String(edits.removed)}`}</span>
            {job.change !== undefined ? ' · pick its worktree in the sidebar to review it' : ''}
          </p>
        ) : null}
      </div>
    </section>
  )
}

export default AgentPane
