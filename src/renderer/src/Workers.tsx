import { useRef, useState, type KeyboardEvent } from 'react'
import BotAvatar from './BotAvatar'
import RichText from './RichText'

type JobRecord = Awaited<ReturnType<Window['api']['listJobs']>>[number]
type JobType = Parameters<Window['api']['submitJob']>[0]

const JOB_TYPES: readonly JobType[] = ['planning', 'debugging', 'review', 'refactor', 'boilerplate']

function jobAvatarState(status: JobRecord['status']): 'idle' | 'working' | 'resting' | 'error' {
  if (status === 'running') return 'working'
  if (status === 'failed') return 'error'
  if (status === 'queued') return 'resting'
  return 'idle'
}

function outputTitle(job: JobRecord): string {
  const parts: string[] = [job.type]
  if (job.provider !== null) parts.push(job.provider)
  if (job.model) parts.push(job.model)
  return parts.join(' · ')
}

function Workers({
  jobs,
  onJob
}: {
  jobs: JobRecord[]
  onJob: (job: JobRecord) => void
}): React.JSX.Element {
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [prompt, setPrompt] = useState('')
  const [jobType, setJobType] = useState<JobType>('planning')
  const sending = useRef(false)

  const newest = jobs.slice().reverse()
  const selected = jobs.find((job) => job.id === selectedId) ?? null
  const promptEmpty = prompt.trim() === ''

  async function submit(): Promise<void> {
    const text = prompt
    if (text.trim() === '' || sending.current) return
    sending.current = true
    try {
      const job = await window.api.submitJob(jobType, text)
      setPrompt('')
      onJob(job)
      setSelectedId(job.id)
    } finally {
      sending.current = false
    }
  }

  function onPromptKeyDown(event: KeyboardEvent<HTMLTextAreaElement>): void {
    if (event.key === 'Enter' && event.ctrlKey && !promptEmpty) {
      event.preventDefault()
      void submit()
    }
  }

  function onRowKeyDown(event: KeyboardEvent<HTMLDivElement>, index: number): void {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault()
      const job = newest[index]
      if (job) setSelectedId(job.id)
      return
    }
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
    event.preventDefault()
    const nextIndex = event.key === 'ArrowDown' ? index + 1 : index - 1
    const next = newest[nextIndex]
    if (!next) return
    setSelectedId(next.id)
    document.getElementById(`job-${next.id}`)?.focus()
  }

  return (
    <div className="workers">
      <div className="panel-head">
        <h2 id="workers-heading">Workers</h2>
      </div>
      <p className="hint">Hand a job straight to a worker.</p>
      <form
        className="composer"
        onSubmit={(event) => {
          event.preventDefault()
          void submit()
        }}
      >
        <label htmlFor="prompt">Prompt</label>
        <div className="composer-box">
          <textarea
            id="prompt"
            rows={3}
            value={prompt}
            onChange={(event) => setPrompt(event.target.value)}
            onKeyDown={onPromptKeyDown}
          />
          <div className="composer-foot">
            <div className="composer-tools">
              <label htmlFor="job-type">Job type</label>
              <select
                id="job-type"
                value={jobType}
                onChange={(event) => setJobType(event.target.value as JobType)}
              >
                {JOB_TYPES.map((type) => (
                  <option key={type} value={type}>
                    {type}
                  </option>
                ))}
              </select>
            </div>
            <button className="btn btn-primary" type="submit" disabled={promptEmpty}>
              Submit
            </button>
          </div>
        </div>
      </form>

      <section className="jobs" aria-label="Jobs">
        {newest.length === 0 ? (
          <p className="hint">No jobs yet.</p>
        ) : (
          <div role="listbox" aria-label="Jobs" className="job-list">
            {newest.map((job, index) => {
              const isSelected = job.id === selectedId
              return (
                <div
                  key={job.id}
                  id={`job-${job.id}`}
                  role="option"
                  aria-selected={isSelected}
                  tabIndex={isSelected || (selectedId === null && index === 0) ? 0 : -1}
                  className={isSelected ? 'job-row is-selected' : 'job-row'}
                  onClick={() => setSelectedId(job.id)}
                  onKeyDown={(event) => onRowKeyDown(event, index)}
                >
                  <div className="job-main">
                    {job.provider !== null ? (
                      <BotAvatar
                        bot={job.provider}
                        state={jobAvatarState(job.status)}
                        size={20}
                        title={job.provider}
                      />
                    ) : null}
                    <span className="chip">{job.type}</span>
                    <span className="job-id" title={job.id}>
                      {job.id.slice(0, 8)}
                    </span>
                    <span className="job-grow" />
                    {job.model ? (
                      <span className="job-model" title={job.model}>
                        {job.model}
                      </span>
                    ) : null}
                    <span className={`status status-${job.status}`}>
                      <span className="status-dot" aria-hidden="true" />
                      {job.status}
                    </span>
                  </div>
                  {job.failedOver.length > 0 ? (
                    <span className="failover">
                      ↪ {[...job.failedOver, job.provider ?? '—'].join('→')}
                    </span>
                  ) : null}
                </div>
              )
            })}
          </div>
        )}
      </section>

      <section className="output-pane" aria-label="Output">
        {selected === null ? (
          <p className="hint">Select a job to see its output.</p>
        ) : (
          <>
            <div className="output-head">
              {selected.provider !== null ? (
                <BotAvatar
                  bot={selected.provider}
                  state={jobAvatarState(selected.status)}
                  size={20}
                />
              ) : null}
              <span>{outputTitle(selected)}</span>
            </div>
            <div className="output-body">
              <RichText text={selected.output} />
            </div>
            {selected.error ? <pre className="output-error">{selected.error}</pre> : null}
          </>
        )}
      </section>
    </div>
  )
}

export default Workers
