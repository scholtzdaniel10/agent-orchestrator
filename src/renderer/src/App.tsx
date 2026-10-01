import { useEffect, useRef, useState, type KeyboardEvent } from 'react'

type JobRecord = Awaited<ReturnType<Window['api']['listJobs']>>[number]
type JobType = Parameters<Window['api']['submitJob']>[0]

const JOB_TYPES: readonly JobType[] = ['planning', 'debugging', 'review', 'refactor', 'boilerplate']

function mergeJob(jobs: JobRecord[], job: JobRecord): JobRecord[] {
  const index = jobs.findIndex((item) => item.id === job.id)
  if (index === -1) return [...jobs, job]
  const next = jobs.slice()
  next[index] = job
  return next
}

function mergeList(current: JobRecord[], list: JobRecord[]): JobRecord[] {
  const listed = new Set(list.map((job) => job.id))
  const live = new Map(current.map((job) => [job.id, job]))
  const merged = list.map((job) => live.get(job.id) ?? job)
  for (const job of current) {
    if (!listed.has(job.id)) merged.push(job)
  }
  return merged
}

function App(): React.JSX.Element {
  const [jobs, setJobs] = useState<JobRecord[]>([])
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [prompt, setPrompt] = useState('')
  const [jobType, setJobType] = useState<JobType>('planning')
  const sending = useRef(false)

  useEffect(() => {
    let active = true
    const unsubscribe = window.api.onJobUpdate((job) => {
      setJobs((prev) => mergeJob(prev, job))
    })
    void window.api.listJobs().then((list) => {
      if (!active) return
      setJobs((prev) => mergeList(prev, list))
    })
    return () => {
      active = false
      unsubscribe()
    }
  }, [])

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
      setJobs((prev) => mergeJob(prev, job))
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
    <div className="app">
      <form
        className="job-box"
        onSubmit={(event) => {
          event.preventDefault()
          void submit()
        }}
      >
        <label htmlFor="prompt">Prompt</label>
        <textarea
          id="prompt"
          rows={5}
          value={prompt}
          onChange={(event) => setPrompt(event.target.value)}
          onKeyDown={onPromptKeyDown}
        />
        <div className="job-actions">
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
          <button type="submit" disabled={promptEmpty}>
            Submit
          </button>
        </div>
      </form>

      <section className="jobs" aria-label="Jobs">
        <h2>Jobs</h2>
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
                  className={isSelected ? 'job-row selected' : 'job-row'}
                  onClick={() => setSelectedId(job.id)}
                  onKeyDown={(event) => onRowKeyDown(event, index)}
                >
                  <span className="job-id" title={job.id}>
                    {job.id.slice(0, 8)}
                  </span>
                  <span>{job.type}</span>
                  <span>{job.provider ?? '—'}</span>
                  <span className={`badge badge-${job.status}`}>{job.status}</span>
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
        <h2>Output</h2>
        {selected === null ? (
          <p className="hint">Select a job to see its output.</p>
        ) : (
          <>
            <pre className="output">{selected.output}</pre>
            {selected.error ? <pre className="output-error">{selected.error}</pre> : null}
          </>
        )}
      </section>
    </div>
  )
}

export default App
