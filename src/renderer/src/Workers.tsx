import { useEffect, useRef, useState, type KeyboardEvent } from 'react'
import BotAvatar from './BotAvatar'
import RichText from './RichText'
import TerminalPane, { type PanePlacement } from './TerminalPane'
import TerminalTabs from './TerminalTabs'
import type { TerminalBus } from './terminal-bus'

type JobRecord = Awaited<ReturnType<Window['api']['listJobs']>>[number]
type JobType = Parameters<Window['api']['submitJob']>[0]
type TerminalInfo = Awaited<ReturnType<Window['api']['listTerminals']>>[number]
type ProviderId = Parameters<Window['api']['openTerminal']>[0]

const JOB_TYPES: readonly JobType[] = ['planning', 'debugging', 'review', 'refactor', 'boilerplate']

function errorText(err: unknown): string {
  if (err instanceof Error) return err.message
  if (typeof err === 'string') return err
  return 'Request failed'
}

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

function splitPartner(
  terminals: readonly TerminalInfo[],
  selectedId: string,
  recentId: string | null
): string | null {
  if (
    recentId !== null &&
    recentId !== selectedId &&
    terminals.some((info) => info.id === recentId)
  ) {
    return recentId
  }
  const other = terminals.find((info) => info.id !== selectedId)
  return other === undefined ? null : other.id
}

function placementFor(
  id: string,
  selectedId: string | null,
  partnerId: string | null,
  splitOn: boolean
): PanePlacement {
  if (!splitOn || selectedId === null) return id === selectedId ? 'only' : 'hidden'
  if (id === selectedId) return 'left'
  if (id === partnerId) return 'right'
  return 'hidden'
}

function Workers({
  jobs,
  terminals,
  initialTerminalIds,
  bus,
  onJob,
  onTerminal
}: {
  jobs: JobRecord[]
  terminals: TerminalInfo[]
  initialTerminalIds: ReadonlySet<string> | null
  bus: TerminalBus
  onJob: (job: JobRecord) => void
  onTerminal: (info: TerminalInfo) => void
}): React.JSX.Element {
  const [selectedJobId, setSelectedJobId] = useState<string | null>(null)
  const [prompt, setPrompt] = useState('')
  const [jobType, setJobType] = useState<JobType>('planning')
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [recentOther, setRecentOther] = useState<string | null>(null)
  const [split, setSplit] = useState(false)
  const [opening, setOpening] = useState(false)
  const [openError, setOpenError] = useState<string | null>(null)
  const [focusTermId, setFocusTermId] = useState<string | null>(null)
  const [focusTick, setFocusTick] = useState(0)
  const sending = useRef(false)
  const openingRef = useRef(false)
  const selectedRef = useRef<string | null>(null)

  useEffect(() => {
    selectedRef.current = selectedId
  }, [selectedId])

  const newest = jobs.slice().reverse()
  const selectedJob = jobs.find((job) => job.id === selectedJobId) ?? null
  const promptEmpty = prompt.trim() === ''
  const shownId =
    selectedId !== null && terminals.some((info) => info.id === selectedId) ? selectedId : null
  const partnerId =
    split && shownId !== null && terminals.length >= 2
      ? splitPartner(terminals, shownId, recentOther)
      : null
  const splitOn = partnerId !== null

  function requestFocus(id: string): void {
    setFocusTermId(id)
    setFocusTick((tick) => tick + 1)
  }

  function selectTab(id: string | null, source: 'click' | 'arrow'): void {
    const previous = selectedRef.current
    if (previous !== null && previous !== id) setRecentOther(previous)
    selectedRef.current = id
    setSelectedId(id)
    if (source === 'click' && id !== null) requestFocus(id)
  }

  function closeTerminal(id: string): void {
    const index = terminals.findIndex((info) => info.id === id)
    if (selectedRef.current === id) {
      const left = index > 0 ? terminals[index - 1].id : null
      selectedRef.current = left
      setSelectedId(left)
      if (left === null) {
        document.getElementById('worker-tab-jobs')?.focus()
      } else {
        requestFocus(left)
      }
    }
    if (recentOther === id) setRecentOther(null)
    void window.api.closeTerminal(id).catch((err: unknown) => {
      setOpenError(errorText(err))
    })
  }

  async function openProvider(provider: ProviderId): Promise<void> {
    if (openingRef.current) return
    openingRef.current = true
    setOpening(true)
    try {
      const info = await window.api.openTerminal(provider, 100, 30)
      onTerminal(info)
      const previous = selectedRef.current
      if (previous !== null && previous !== info.id) setRecentOther(previous)
      selectedRef.current = info.id
      setSelectedId(info.id)
      requestFocus(info.id)
      setOpenError(null)
    } catch (err: unknown) {
      setOpenError(errorText(err))
    } finally {
      openingRef.current = false
      setOpening(false)
    }
  }

  async function submit(): Promise<void> {
    const text = prompt
    if (text.trim() === '' || sending.current) return
    sending.current = true
    try {
      const job = await window.api.submitJob(jobType, text)
      setPrompt('')
      onJob(job)
      setSelectedJobId(job.id)
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
      if (job) setSelectedJobId(job.id)
      return
    }
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
    event.preventDefault()
    const nextIndex = event.key === 'ArrowDown' ? index + 1 : index - 1
    const next = newest[nextIndex]
    if (!next) return
    setSelectedJobId(next.id)
    document.getElementById(`job-${next.id}`)?.focus()
  }

  const showJobs = shownId === null

  return (
    <div className="workers">
      <div className="panel-head">
        <h2 id="workers-heading">Workers</h2>
      </div>
      <TerminalTabs
        terminals={terminals}
        selectedId={shownId}
        split={split}
        opening={opening}
        onSelect={selectTab}
        onClose={closeTerminal}
        onOpen={(provider) => {
          void openProvider(provider)
        }}
        onToggleSplit={() => {
          setSplit((value) => !value)
        }}
      />
      {openError !== null ? (
        <p className="field-error" role="alert">
          {openError}
        </p>
      ) : null}
      <div className="workers-stage">
        <div
          className={showJobs ? 'workers-jobs' : 'workers-jobs is-hidden'}
          inert={showJobs ? undefined : true}
        >
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
                  const isSelected = job.id === selectedJobId
                  return (
                    <div
                      key={job.id}
                      id={`job-${job.id}`}
                      role="option"
                      aria-selected={isSelected}
                      tabIndex={isSelected || (selectedJobId === null && index === 0) ? 0 : -1}
                      className={isSelected ? 'job-row is-selected' : 'job-row'}
                      onClick={() => setSelectedJobId(job.id)}
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
            {selectedJob === null ? (
              <p className="hint">Select a job to see its output.</p>
            ) : (
              <>
                <div className="output-head">
                  {selectedJob.provider !== null ? (
                    <BotAvatar
                      bot={selectedJob.provider}
                      state={jobAvatarState(selectedJob.status)}
                      size={20}
                    />
                  ) : null}
                  <span>{outputTitle(selectedJob)}</span>
                </div>
                <div className="output-body">
                  <RichText text={selectedJob.output} />
                </div>
                {selectedJob.error ? <pre className="output-error">{selectedJob.error}</pre> : null}
              </>
            )}
          </section>
        </div>
        <div
          className={
            showJobs
              ? 'terminal-stage is-inactive'
              : splitOn
                ? 'terminal-stage is-split'
                : 'terminal-stage'
          }
          inert={showJobs ? true : undefined}
        >
          {splitOn ? <div className="terminal-divider" aria-hidden="true" /> : null}
          {initialTerminalIds === null
            ? null
            : terminals.map((info) => {
                const placement = placementFor(info.id, shownId, partnerId, splitOn)
                return (
                  <TerminalPane
                    key={info.id}
                    info={info}
                    bus={bus}
                    restore={initialTerminalIds.has(info.id)}
                    placement={placement}
                    focusNonce={focusTermId === info.id ? focusTick : 0}
                    onActivate={() => {
                      if (shownId === info.id) return
                      selectTab(info.id, 'click')
                    }}
                  />
                )
              })}
        </div>
      </div>
    </div>
  )
}

export default Workers
