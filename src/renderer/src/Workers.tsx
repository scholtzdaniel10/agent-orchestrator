import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react'
import { RayBurst, Star } from './Star'
import BotAvatar from './BotAvatar'
import ChangesTab from './ChangesTab'
import RichText from './RichText'
import TerminalPane, { type PanePlacement } from './TerminalPane'
import TerminalTabs from './TerminalTabs'
import type { TerminalBus } from './terminal-bus'

type ChangeSet = Awaited<ReturnType<Window['api']['listChanges']>>[number]
type JobRecord = Awaited<ReturnType<Window['api']['listJobs']>>[number]
type JobType = Parameters<Window['api']['submitJob']>[0]
type TerminalInfo = Awaited<ReturnType<Window['api']['listTerminals']>>[number]
type PlanStatus = Awaited<ReturnType<Window['api']['listPlans']>>[number]
type ProviderId = Parameters<Window['api']['openTerminal']>[0]
type PlanChoice = 'auto' | ProviderId | 'both'

const JOB_TYPES: readonly JobType[] = ['planning', 'debugging', 'review', 'refactor', 'boilerplate']

function errorText(err: unknown): string {
  if (err instanceof Error) return err.message
  if (typeof err === 'string') return err
  return 'Request failed'
}

function asPlanChoice(value: string): PlanChoice {
  if (value === 'claude' || value === 'cursor' || value === 'both') return value
  return 'auto'
}

function planRank(provider: JobRecord['provider']): number {
  if (provider === 'claude') return 0
  if (provider === 'cursor') return 1
  return 2
}

function planAvailable(plans: readonly PlanStatus[] | null, id: ProviderId): boolean {
  return plans !== null && plans.some((plan) => plan.id === id && plan.available)
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
  if (job.reason) parts.push(job.reason)
  if (job.edit === true) parts.push('edits')
  return parts.join(' · ')
}

function visibleChange(job: JobRecord, changes: readonly ChangeSet[]): string | null {
  const id = job.change
  if (id === undefined) return null
  return changes.some((item) => item.id === id) ? id : null
}

function FailoverArrow(): React.JSX.Element {
  return (
    <svg
      className="failover-arrow"
      width="10"
      height="10"
      viewBox="0 0 10 10"
      aria-hidden="true"
      focusable="false"
    >
      <path
        d="M1.5 5h5.5M5 2.5 7.5 5 5 7.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.25"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  )
}

function metaLine(parts: ReactNode[]): React.JSX.Element {
  const nodes: ReactNode[] = []
  parts.forEach((part, index) => {
    if (index > 0) nodes.push(<span key={`sep-${String(index)}`} className="meta-sep"> / </span>)
    nodes.push(<span key={`part-${String(index)}`}>{part}</span>)
  })
  return <>{nodes}</>
}

function jobMetaParts(job: JobRecord): ReactNode[] {
  const parts: ReactNode[] = [job.type]
  if (job.group !== undefined) parts.push('compare')
  if (job.leadMessage !== undefined) parts.push('from lead')
  if (job.edit === true) parts.push('edits')
  if (job.reason) parts.push(<span className="job-reason-mono">{job.reason}</span>)
  if (job.model) parts.push(<span className="job-model">{job.model}</span>)
  parts.push(<span className="job-id">{job.id.slice(0, 8)}</span>)
  return parts
}

function EmptyState({
  title,
  guidance
}: {
  title: string
  guidance: string
}): React.JSX.Element {
  return (
    <div className="empty-state">
      <RayBurst />
      <p className="empty-state-title">{title}</p>
      <p className="empty-state-guidance">{guidance}</p>
    </div>
  )
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

function canStop(job: JobRecord): boolean {
  return job.status === 'queued' || job.status === 'running'
}

function CompareOutput({
  groupJobs,
  changes,
  onViewChange,
  stoppingId,
  onStop
}: {
  groupJobs: JobRecord[]
  changes: readonly ChangeSet[]
  onViewChange: (id: string) => void
  stoppingId: string | null
  onStop: (id: string) => void
}): React.JSX.Element {
  const [merging, setMerging] = useState(false)
  const [messages, setMessages] = useState<Record<string, string>>({})
  const mergingRef = useRef(false)
  const ordered = groupJobs.slice().sort((a, b) => planRank(a.provider) - planRank(b.provider))
  const eitherBusy = ordered.some((job) => job.status === 'queued' || job.status === 'running')

  async function mergeThis(job: JobRecord): Promise<void> {
    const changeId = visibleChange(job, changes)
    if (changeId === null || mergingRef.current || eitherBusy) return
    mergingRef.current = true
    setMerging(true)
    try {
      const result = await window.api.mergeChange(changeId)
      if (!result.ok) {
        setMessages((prev) => ({ ...prev, [job.id]: result.message }))
        return
      }
      setMessages((prev) => {
        const next = { ...prev }
        delete next[job.id]
        return next
      })
      for (const other of ordered) {
        if (other.id === job.id) continue
        const otherChange = visibleChange(other, changes)
        if (otherChange !== null) await window.api.discardChange(otherChange)
      }
    } catch (err: unknown) {
      setMessages((prev) => ({ ...prev, [job.id]: errorText(err) }))
    } finally {
      mergingRef.current = false
      setMerging(false)
    }
  }

  return (
    <div className="compare-output">
      {ordered.map((job) => {
        const changeId = visibleChange(job, changes)
        const change =
          changeId === null ? undefined : changes.find((item) => item.id === changeId)
        const waiting =
          job.output === '' && (job.status === 'queued' || job.status === 'running')
        const mergeTitle = eitherBusy ? 'Wait for both to finish' : undefined
        const message = messages[job.id]
        return (
          <div key={job.id} className="compare-col">
            <div className="output-head">
              {job.provider !== null ? (
                <BotAvatar
                  bot={job.provider}
                  state={jobAvatarState(job.status)}
                  size={20}
                  title={job.provider}
                />
              ) : null}
              <span>
                {job.provider ?? '—'}
                {job.model ? ` · ${job.model}` : ''}
              </span>
              <span className={`status status-${job.status}`}>
                <span className="status-dot" aria-hidden="true" />
                {job.status}
              </span>
              {canStop(job) ? (
                <button
                  type="button"
                  className="btn btn-compact"
                  disabled={stoppingId === job.id}
                  onClick={() => onStop(job.id)}
                >
                  Stop
                </button>
              ) : null}
            </div>
            <div className="output-body">
              {waiting ? (
                <p className="compare-wait">
                  {job.status === 'queued' ? 'Waiting…' : 'Working…'}
                </p>
              ) : (
                <RichText text={job.output} />
              )}
            </div>
            {job.error ? <pre className="output-error">{job.error}</pre> : null}
            {message !== undefined ? <pre className="output-error">{message}</pre> : null}
            {change !== undefined ? (
              <div className="compare-foot">
                <span className="change-stats">
                  {`${change.files.length === 1 ? '1 file' : `${String(change.files.length)} files`} · `}
                  <span className="stat-add">{`+${String(change.insertions)}`}</span>{' '}
                  <span className="stat-del">{`−${String(change.deletions)}`}</span>
                </span>
                <button
                  type="button"
                  className="btn btn-quiet btn-compact"
                  onClick={() => onViewChange(change.id)}
                >
                  View change
                </button>
                <button
                  type="button"
                  className="btn btn-primary btn-compact"
                  disabled={merging || eitherBusy}
                  title={mergeTitle}
                  onClick={() => {
                    void mergeThis(job)
                  }}
                >
                  Merge this, discard the other
                </button>
              </div>
            ) : null}
          </div>
        )
      })}
    </div>
  )
}

function Workers({
  jobs,
  plans,
  terminals,
  initialTerminalIds,
  changes,
  isRepo,
  bus,
  showJob,
  onJob,
  onTerminal
}: {
  jobs: JobRecord[]
  plans: PlanStatus[] | null
  terminals: TerminalInfo[]
  initialTerminalIds: ReadonlySet<string> | null
  changes: ChangeSet[]
  isRepo: boolean
  bus: TerminalBus
  showJob?: { id: string; nonce: number } | null
  onJob: (job: JobRecord) => void
  onTerminal: (info: TerminalInfo) => void
}): React.JSX.Element {
  const [selectedJobId, setSelectedJobId] = useState<string | null>(null)
  const [prompt, setPrompt] = useState('')
  const [jobType, setJobType] = useState<JobType>('planning')
  const [worker, setWorker] = useState<PlanChoice>('auto')
  const [editFiles, setEditFiles] = useState(false)
  const [mainTab, setMainTab] = useState<'jobs' | 'changes'>('jobs')
  const [selectedChangeId, setSelectedChangeId] = useState<string | null>(null)
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [recentOther, setRecentOther] = useState<string | null>(null)
  const [split, setSplit] = useState(false)
  const [opening, setOpening] = useState(false)
  const [openError, setOpenError] = useState<string | null>(null)
  const [focusTermId, setFocusTermId] = useState<string | null>(null)
  const [focusTick, setFocusTick] = useState(0)
  const [stoppingId, setStoppingId] = useState<string | null>(null)
  const sending = useRef(false)
  const openingRef = useRef(false)
  const stoppingRef = useRef(false)
  const selectedRef = useRef<string | null>(null)

  useEffect(() => {
    selectedRef.current = selectedId
  }, [selectedId])

  useEffect(() => {
    if (showJob == null) return
    const previous = selectedRef.current
    if (previous !== null) setRecentOther(previous)
    selectedRef.current = null
    setSelectedId(null)
    setMainTab('jobs')
    setSelectedJobId(showJob.id)
    const id = showJob.id
    requestAnimationFrame(() => {
      document.getElementById(`job-${id}`)?.scrollIntoView({ block: 'nearest' })
    })
  }, [showJob])

  const newest = jobs.slice().reverse()
  const selectedJob = jobs.find((job) => job.id === selectedJobId) ?? null
  const promptEmpty = prompt.trim() === ''
  const claudeReady = planAvailable(plans, 'claude')
  const cursorReady = planAvailable(plans, 'cursor')
  const bothReady = claudeReady && cursorReady
  const noPlanReady = plans !== null && !claudeReady && !cursorReady
  const choiceBlocked =
    (worker === 'claude' && !claudeReady) ||
    (worker === 'cursor' && !cursorReady) ||
    (worker === 'both' && !bothReady)
  const submitDisabled = promptEmpty || noPlanReady || choiceBlocked
  const shownId =
    selectedId !== null && terminals.some((info) => info.id === selectedId) ? selectedId : null
  const partnerId =
    split && shownId !== null && terminals.length >= 2
      ? splitPartner(terminals, shownId, recentOther)
      : null
  const splitOn = partnerId !== null

  async function stopJob(id: string): Promise<void> {
    if (stoppingRef.current) return
    stoppingRef.current = true
    setStoppingId(id)
    try {
      const job = await window.api.cancelJob(id)
      onJob(job)
      setOpenError(null)
    } catch (err: unknown) {
      setOpenError(errorText(err))
    } finally {
      stoppingRef.current = false
      setStoppingId(null)
    }
  }

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
        setMainTab('jobs')
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
      if (worker === 'both') {
        const group = crypto.randomUUID()
        const edit = isRepo && editFiles
        const results: JobRecord[] = []
        let submitError: string | null = null
        for (const provider of ['claude', 'cursor'] as const) {
          try {
            const job = await window.api.submitJob(jobType, text, provider, edit, group)
            results.push(job)
            onJob(job)
          } catch (err: unknown) {
            submitError = errorText(err)
          }
        }
        if (results.length > 0) {
          setPrompt('')
          setSelectedJobId(results[0].id)
        }
        if (submitError !== null) setOpenError(submitError)
        else setOpenError(null)
        return
      }
      const provider = worker === 'auto' ? undefined : worker
      const job = await window.api.submitJob(jobType, text, provider, isRepo && editFiles)
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

  const showJobs = shownId === null && mainTab === 'jobs'
  const showChanges = shownId === null && mainTab === 'changes'

  function showMain(tab: 'jobs' | 'changes'): void {
    const previous = selectedRef.current
    if (previous !== null) setRecentOther(previous)
    selectedRef.current = null
    setSelectedId(null)
    setMainTab(tab)
  }

  function viewChange(id: string): void {
    setSelectedChangeId(id)
    showMain('changes')
  }

  return (
    <div className="workers">
      <div className="panel-head">
        <div className="panel-title">
          <Star size={12} />
          <h2 id="workers-heading">Workers</h2>
        </div>
      </div>
      <TerminalTabs
        terminals={terminals}
        active={shownId ?? mainTab}
        changeCount={changes.length}
        split={split}
        opening={opening}
        onSelect={(id, source) => {
          if (id === 'jobs' || id === 'changes') {
            showMain(id)
            return
          }
          selectTab(id, source)
        }}
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
          <form
            className="composer"
            onSubmit={(event) => {
              event.preventDefault()
              void submit()
            }}
          >
            <label className="sr-only" htmlFor="prompt">
              Prompt
            </label>
            <div className="composer-box">
              <textarea
                id="prompt"
                rows={3}
                value={prompt}
                placeholder="Hand a job straight to a worker."
                onChange={(event) => setPrompt(event.target.value)}
                onKeyDown={onPromptKeyDown}
              />
              <div className="composer-foot">
                <div className="composer-tools">
                  <label htmlFor="job-type">Job type</label>
                  <select
                    id="job-type"
                    className="job-type"
                    value={jobType}
                    onChange={(event) => setJobType(event.target.value as JobType)}
                  >
                    {JOB_TYPES.map((type) => (
                      <option key={type} value={type}>
                        {type}
                      </option>
                    ))}
                  </select>
                  <label htmlFor="job-worker">Worker</label>
                  <select
                    id="job-worker"
                    className="plan-choice"
                    value={worker}
                    onChange={(event) => setWorker(asPlanChoice(event.target.value))}
                  >
                    <option value="auto">auto</option>
                    <option value="claude" disabled={!claudeReady}>
                      claude
                    </option>
                    <option value="cursor" disabled={!cursorReady}>
                      cursor
                    </option>
                    <option
                      value="both"
                      disabled={!bothReady}
                      title={
                        bothReady
                          ? undefined
                          : 'Both Claude and Cursor need to be ready to compare.'
                      }
                    >
                      both (compare)
                    </option>
                  </select>
                  <label
                    className="edit-toggle"
                    htmlFor="job-edit"
                    title={isRepo ? undefined : 'Open a git repository to let jobs edit files.'}
                  >
                    <input
                      id="job-edit"
                      type="checkbox"
                      checked={editFiles}
                      disabled={!isRepo}
                      title={isRepo ? undefined : 'Open a git repository to let jobs edit files.'}
                      onChange={(event) => setEditFiles(event.target.checked)}
                    />
                    Let it edit files
                  </label>
                </div>
                <button className="btn btn-primary" type="submit" disabled={submitDisabled}>
                  Submit
                </button>
              </div>
            </div>
            {noPlanReady ? (
              <p className="hint">No plan is ready. See Usage on the right.</p>
            ) : null}
          </form>

          <section className="jobs" aria-label="Jobs">
            {newest.length === 0 ? (
              <EmptyState
                title="No jobs yet"
                guidance="Jobs you submit, and jobs the lead hands out, appear here."
              />
            ) : (
              <div role="listbox" aria-label="Jobs" className="job-list">
                {newest.map((job, index) => {
                  const isSelected = job.id === selectedJobId
                  const failoverChain = [...job.failedOver, job.provider ?? '—']
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
                        ) : (
                          <span className="job-avatar-slot" aria-hidden="true" />
                        )}
                        <span className="job-title" title={job.prompt}>
                          {job.prompt}
                        </span>
                        <span className={`status status-${job.status}`}>
                          <span className="status-dot" aria-hidden="true" />
                          {job.status}
                        </span>
                      </div>
                      <div className="job-meta">
                        {metaLine(jobMetaParts(job))}
                        {visibleChange(job, changes) !== null ? (
                          <button
                            type="button"
                            className="btn btn-quiet btn-compact"
                            onClick={(event) => {
                              event.stopPropagation()
                              const id = job.change
                              if (id !== undefined) viewChange(id)
                            }}
                            onKeyDown={(event) => event.stopPropagation()}
                          >
                            View change
                          </button>
                        ) : null}
                      </div>
                      {job.failedOver.length > 0 ? (
                        <div
                          className="failover"
                          aria-label={`Failed over from ${failoverChain[0]} to ${failoverChain[failoverChain.length - 1]}`}
                        >
                          <span className="failover-label">Failover</span>
                          {failoverChain.map((name, hop) => (
                            <span key={`${job.id}-failover-${hop}-${name}`} className="failover-hop">
                              {hop > 0 ? <FailoverArrow /> : null}
                              <span>{name}</span>
                            </span>
                          ))}
                        </div>
                      ) : null}
                    </div>
                  )
                })}
              </div>
            )}
          </section>

          <section className="output-pane" aria-label="Output">
            {selectedJob === null ? (
              <EmptyState
                title="No job selected"
                guidance="Select a job to see its output."
              />
            ) : selectedJob.group !== undefined &&
              jobs.some((job) => job.group === selectedJob.group && job.id !== selectedJob.id) ? (
              <CompareOutput
                groupJobs={jobs.filter((job) => job.group === selectedJob.group)}
                changes={changes}
                onViewChange={viewChange}
                stoppingId={stoppingId}
                onStop={(id) => {
                  void stopJob(id)
                }}
              />
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
                  <span title={outputTitle(selectedJob)}>{outputTitle(selectedJob)}</span>
                  {canStop(selectedJob) ? (
                    <button
                      type="button"
                      className="btn btn-compact"
                      disabled={stoppingId === selectedJob.id}
                      onClick={() => {
                        void stopJob(selectedJob.id)
                      }}
                    >
                      Stop
                    </button>
                  ) : null}
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
          className={showChanges ? 'workers-changes' : 'workers-changes is-hidden'}
          inert={showChanges ? undefined : true}
        >
          <ChangesTab
            changes={changes}
            jobs={jobs}
            selectedId={selectedChangeId}
            onSelect={setSelectedChangeId}
          />
        </div>
        <div
          className={
            shownId === null
              ? 'terminal-stage is-inactive'
              : splitOn
                ? 'terminal-stage is-split'
                : 'terminal-stage'
          }
          inert={shownId === null ? true : undefined}
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
