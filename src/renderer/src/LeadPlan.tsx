import { useState } from 'react'
import { PROVIDERS } from '../../core/types'
import BotAvatar from './BotAvatar'

type LeadMessage = Awaited<ReturnType<Window['api']['listLeadMessages']>>[number]
type LeadProposal = NonNullable<LeadMessage['proposal']>
type ProposedJob = LeadProposal['jobs'][number]
type ProviderId = (typeof PROVIDERS)[number]

const ACCESS_LABEL: Record<ProposedJob['access'], string> = {
  read: 'Read only',
  edit: 'Edit files',
  full: 'Full access'
}

function workerName(id: ProviderId): string {
  if (id === 'claude') return 'Claude'
  if (id === 'grok') return 'Grok'
  return 'Cursor'
}

function errorText(err: unknown): string {
  if (err instanceof Error) {
    return err.message.replace(/^Error invoking remote method '[^']+': (Error: )?/, '')
  }
  if (typeof err === 'string') return err
  return 'Request failed'
}

/** The lead's proposed jobs, editable, with Run and Dismiss. Nothing starts before Run. */
function LeadPlan({
  proposal,
  onRun,
  onDismiss
}: {
  proposal: LeadProposal
  onRun: (jobs: ProposedJob[]) => Promise<void>
  onDismiss: () => Promise<void>
}): React.JSX.Element {
  const [jobs, setJobs] = useState<ProposedJob[]>(() => proposal.jobs.map((job) => ({ ...job })))
  const [open, setOpen] = useState<number | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const runnable = jobs.filter((job) => job.prompt.trim() !== '')

  function change(index: number, patch: Partial<ProposedJob>): void {
    setJobs((current) => current.map((job, i) => (i === index ? { ...job, ...patch } : job)))
  }

  function remove(index: number): void {
    setJobs((current) => current.filter((_, i) => i !== index))
    setOpen(null)
  }

  function act(work: () => Promise<void>): void {
    setBusy(true)
    setError(null)
    work()
      .catch((err: unknown) => {
        setError(errorText(err))
      })
      .finally(() => {
        setBusy(false)
      })
  }

  return (
    <div className="lead-plan" aria-label="The lead's plan">
      <h3 className="lead-delegated-title">Plan</h3>
      <p className="lead-plan-summary">{proposal.summary}</p>
      <ol className="lead-plan-list">
        {jobs.map((job, index) => {
          const expanded = open === index
          return (
            <li key={index} className="lead-plan-card">
              <div className="lead-plan-row">
                {job.worker !== undefined ? (
                  <BotAvatar bot={job.worker} state="idle" size={18} />
                ) : (
                  <BotAvatar bot="lead" state="idle" size={18} />
                )}
                <input
                  className="lead-plan-title"
                  aria-label={`Title of job ${String(index + 1)}`}
                  value={job.title}
                  maxLength={80}
                  onChange={(event) => {
                    change(index, { title: event.target.value })
                  }}
                />
                <button
                  type="button"
                  className="btn btn-quiet btn-compact"
                  aria-expanded={expanded}
                  onClick={() => {
                    setOpen(expanded ? null : index)
                  }}
                >
                  {expanded ? 'Less' : 'Edit'}
                </button>
                <button
                  type="button"
                  className="tab-close"
                  aria-label={`Remove ${job.title}`}
                  onClick={() => {
                    remove(index)
                  }}
                >
                  ×
                </button>
              </div>
              <div className="lead-plan-meta">
                <select
                  aria-label={`Worker for ${job.title}`}
                  value={job.worker ?? 'auto'}
                  onChange={(event) => {
                    const value = event.target.value
                    const worker = PROVIDERS.find((id) => id === value)
                    change(index, { worker })
                  }}
                >
                  <option value="auto">Router picks</option>
                  {PROVIDERS.map((id) => (
                    <option key={id} value={id}>
                      {workerName(id)}
                    </option>
                  ))}
                </select>
                <select
                  aria-label={`Access for ${job.title}`}
                  value={job.access}
                  onChange={(event) => {
                    const value = event.target.value
                    change(index, {
                      access: value === 'edit' || value === 'full' ? value : 'read'
                    })
                  }}
                >
                  {(['read', 'edit', 'full'] as const).map((access) => (
                    <option key={access} value={access}>
                      {ACCESS_LABEL[access]}
                    </option>
                  ))}
                </select>
                <span className="lead-plan-type">{job.type}</span>
              </div>
              {expanded ? (
                <textarea
                  className="lead-plan-prompt"
                  aria-label={`Prompt for ${job.title}`}
                  rows={6}
                  value={job.prompt}
                  onChange={(event) => {
                    change(index, { prompt: event.target.value })
                  }}
                />
              ) : (
                <p className="lead-job-prompt" title={job.prompt}>
                  {job.prompt}
                </p>
              )}
            </li>
          )
        })}
      </ol>
      {error !== null ? (
        <p className="error-text" role="alert">
          {error}
        </p>
      ) : null}
      <div className="lead-plan-actions">
        <button
          type="button"
          className="btn btn-primary"
          disabled={busy || runnable.length === 0}
          onClick={() => {
            act(() => onRun(runnable))
          }}
        >
          {runnable.length === 1 ? 'Run 1 agent' : `Run ${String(runnable.length)} agents`}
        </button>
        <button
          type="button"
          className="btn btn-quiet"
          disabled={busy}
          onClick={() => {
            act(onDismiss)
          }}
        >
          Dismiss
        </button>
      </div>
    </div>
  )
}

export default LeadPlan
