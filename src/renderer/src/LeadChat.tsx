import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode
} from 'react'
import { isProviderId, planOf, PROVIDERS } from '../../core/types'
import { RayBurst, Star } from './Star'
import BotAvatar from './BotAvatar'
import { editedFilesSummary, latestStepTitle } from './job-steps'
import LeadPlan from './LeadPlan'
import RichText from './RichText'
import SidebarMenu from './SidebarMenu'
import Select from './Select'

type LeadMessage = Awaited<ReturnType<Window['api']['listLeadMessages']>>[number]
type JobRecord = Awaited<ReturnType<Window['api']['listJobs']>>[number]
type PlanStatus = Awaited<ReturnType<Window['api']['listPlans']>>[number]
type ProviderId = PlanStatus['id']
type PlanChoice = 'auto' | ProviderId
type JobAccess = 'read' | 'edit' | 'full'

function asAccess(value: string): JobAccess {
  if (value === 'edit' || value === 'full') return value
  return 'read'
}

function accessMark(job: JobRecord): string | null {
  if (job.access === 'full') return 'FULL'
  if (job.edit === true || job.access === 'edit') return 'EDIT'
  return null
}

function jobAvatarState(status: JobRecord['status']): 'idle' | 'working' | 'resting' | 'error' {
  if (status === 'running') return 'working'
  if (status === 'failed') return 'error'
  if (status === 'queued') return 'resting'
  return 'idle'
}

function jobsForMessage(jobs: readonly JobRecord[], messageId: string): JobRecord[] {
  return jobs.filter((job) => job.leadMessage === messageId)
}

function errorText(err: unknown): string {
  if (err instanceof Error) {
    // Electron prefixes errors from the main process; show only the message itself.
    return err.message.replace(/^Error invoking remote method '[^']+': (Error: )?/, '')
  }
  if (typeof err === 'string') return err
  return 'Request failed'
}

function asPlanChoice(value: string): PlanChoice {
  if (isProviderId(value)) return value
  return 'auto'
}

function planAvailable(plans: readonly PlanStatus[] | null, id: ProviderId): boolean {
  return plans !== null && plans.some((plan) => plan.id === id && plan.available)
}

function leadMeta(message: LeadMessage): ReactNode[] {
  const parts: ReactNode[] = []
  if (message.kind === 'report') {
    parts.push('Report')
  }
  parts.push('Lead')
  if (message.provider) parts.push(message.provider)
  if (message.model) parts.push(message.model)
  if (message.status === 'streaming') parts.push('working…')
  return parts
}

function messageClass(message: LeadMessage): string {
  if (message.status === 'error') return 'lead-msg lead-msg-error'
  if (message.role === 'user') return 'lead-msg lead-msg-user'
  return 'lead-msg'
}

function metaLine(parts: ReactNode[]): React.JSX.Element {
  const nodes: ReactNode[] = []
  parts.forEach((part, index) => {
    if (index > 0)
      nodes.push(
        <span key={`sep-${String(index)}`} className="meta-sep">
          {' '}
          /{' '}
        </span>
      )
    nodes.push(<span key={`part-${String(index)}`}>{part}</span>)
  })
  return <>{nodes}</>
}

function delegatedMeta(job: JobRecord): ReactNode[] {
  const parts: ReactNode[] = [job.type]
  const mark = accessMark(job)
  if (mark !== null) parts.push(mark)
  if (job.reason) parts.push(job.reason)
  return parts
}

function StreamingDots(): React.JSX.Element {
  return (
    <span className="dots" aria-hidden="true">
      <span className="dot" />
      <span className="dot" />
      <span className="dot" />
    </span>
  )
}

function LeadChat({
  messages,
  jobs,
  plans,
  mode,
  onReset,
  onOpenTerminal,
  onShowJob,
  onRunPlan,
  settingsNonce = 0
}: {
  messages: LeadMessage[]
  jobs: JobRecord[]
  plans: PlanStatus[] | null
  /** Plan this chat talks to directly, or null for a lead chat. */
  mode: ProviderId | null
  onReset: (direct?: ProviderId) => Promise<void>
  onOpenTerminal: (provider: ProviderId) => void
  onShowJob: (id: string) => void
  /** Starts a lead plan with the person's edits and shows its agents in the middle. */
  onRunPlan: (
    messageId: string,
    jobs: NonNullable<LeadMessage['proposal']>['jobs']
  ) => Promise<void>
  /** Bumped when Settings changes the lead plan or access, so they are read again. */
  settingsNonce?: number
}): React.JSX.Element {
  const [draft, setDraft] = useState('')
  const [sendError, setSendError] = useState<string | null>(null)
  const [choice, setChoice] = useState<PlanChoice>('auto')
  const [planError, setPlanError] = useState<string | null>(null)
  const [workerAccess, setWorkerAccess] = useState<JobAccess>('read')
  const accessRef = useRef<JobAccess>('read')
  const logRef = useRef<HTMLDivElement>(null)
  const sending = useRef(false)
  const resetting = useRef(false)
  const choiceRef = useRef<PlanChoice>('auto')
  const picked = useRef(false)
  const [newMenu, setNewMenu] = useState<{ x: number; y: number } | null>(null)

  useEffect(() => {
    let active = true
    const fresh = settingsNonce > 0
    void window.api.getLeadPlan().then(
      (plan) => {
        if (!active || (picked.current && !fresh)) return
        const value: PlanChoice = plan ?? 'auto'
        choiceRef.current = value
        setChoice(value)
      },
      (err: unknown) => {
        if (!active || picked.current) return
        setPlanError(errorText(err))
      }
    )
    void window.api.getLeadAccess().then(
      (access) => {
        if (!active) return
        accessRef.current = access
        setWorkerAccess(access)
      },
      (err: unknown) => {
        if (!active) return
        setPlanError(errorText(err))
      }
    )
    return () => {
      active = false
    }
  }, [settingsNonce])

  async function changeAccess(value: JobAccess): Promise<void> {
    const previous = accessRef.current
    accessRef.current = value
    setWorkerAccess(value)
    setPlanError(null)
    try {
      await window.api.setLeadAccess(value)
    } catch (err: unknown) {
      if (accessRef.current !== value) return
      accessRef.current = previous
      setWorkerAccess(previous)
      setPlanError(errorText(err))
    }
  }

  // The lead's own model on the chosen plan; workers keep the plan's model from Settings.
  const [leadModel, setLeadModel] = useState<{
    plan: ProviderId
    options: { id: string; label: string }[]
    value: string | null
  } | null>(null)
  const [modelError, setModelError] = useState<string | null>(null)

  useEffect(() => {
    if (choice === 'auto') return
    let current = true
    Promise.all([window.api.listModels(choice), window.api.getLeadModel(choice)]).then(
      ([options, value]) => {
        if (current) setLeadModel({ plan: choice, options, value })
      },
      (err: unknown) => {
        if (current) setModelError(errorText(err))
      }
    )
    return () => {
      current = false
    }
  }, [choice, settingsNonce])

  const modelShown = choice !== 'auto' && leadModel !== null && leadModel.plan === choice
  const modelOptions = modelShown
    ? leadModel.value !== null && !leadModel.options.some((option) => option.id === leadModel.value)
      ? [...leadModel.options, { id: leadModel.value, label: leadModel.value }]
      : leadModel.options
    : []

  async function changeLeadModel(plan: ProviderId, value: string | null): Promise<void> {
    setModelError(null)
    setLeadModel((current) =>
      current !== null && current.plan === plan ? { ...current, value } : current
    )
    try {
      await window.api.setLeadModel(plan, value)
    } catch (err: unknown) {
      setModelError(errorText(err))
    }
  }

  async function changePlan(value: PlanChoice): Promise<void> {
    const previous = choiceRef.current
    picked.current = true
    choiceRef.current = value
    setChoice(value)
    setPlanError(null)
    try {
      await window.api.setLeadPlan(value === 'auto' ? null : value)
    } catch (err: unknown) {
      if (choiceRef.current !== value) return
      choiceRef.current = previous
      setChoice(previous)
      setPlanError(errorText(err))
    }
  }

  useLayoutEffect(() => {
    const log = logRef.current
    if (log === null) return
    log.scrollTop = log.scrollHeight
  }, [messages])

  const draftEmpty = draft.trim() === ''
  const leadStreaming = messages.some(
    (message) => message.role === 'lead' && message.status === 'streaming'
  )
  const noPlanReady =
    plans !== null &&
    (mode === null ? !plans.some((plan) => plan.available) : !planAvailable(plans, mode))
  const sendDisabled = draftEmpty || leadStreaming || noPlanReady

  async function send(): Promise<void> {
    const text = draft
    if (text.trim() === '' || leadStreaming || sending.current) return
    sending.current = true
    setDraft('')
    setSendError(null)
    try {
      await window.api.sendLead(text)
    } catch (err: unknown) {
      setDraft(text)
      setSendError(errorText(err))
    } finally {
      sending.current = false
    }
  }

  async function newChat(direct?: ProviderId): Promise<void> {
    if (leadStreaming || resetting.current) return
    resetting.current = true
    try {
      await onReset(direct)
      setSendError(null)
    } catch (err: unknown) {
      setSendError(errorText(err))
    } finally {
      resetting.current = false
    }
  }

  function onDraftKeyDown(event: KeyboardEvent<HTMLTextAreaElement>): void {
    if (event.key !== 'Enter' || !event.ctrlKey) return
    event.preventDefault()
    void send()
  }

  return (
    <div className="lead-chat">
      <div className="panel-head">
        <div className="panel-title">
          <Star size={12} />
          <BotAvatar bot={mode ?? 'lead'} state={leadStreaming ? 'working' : 'idle'} size={20} />
          <h2 id="lead-heading">{mode === null ? 'Lead' : `Just ${mode}`}</h2>
        </div>
        <div className="head-actions">
          <button
            type="button"
            className="btn btn-quiet"
            aria-haspopup="menu"
            aria-expanded={newMenu !== null}
            disabled={leadStreaming}
            onClick={(event) => {
              const rect = event.currentTarget.getBoundingClientRect()
              setNewMenu({ x: rect.left, y: rect.bottom + 4 })
            }}
          >
            New chat ▾
          </button>
          {newMenu !== null ? (
            <SidebarMenu
              label="New chat"
              at={newMenu}
              onClose={() => {
                setNewMenu(null)
              }}
              entries={[
                {
                  label: 'Lead chat',
                  title: 'The lead splits the work across your plans',
                  onSelect: () => void newChat()
                },
                ...PROVIDERS.map((id) => ({
                  label: `Just ${id}`,
                  title: `Talk straight to ${id}: read only, no delegation`,
                  disabled: !planAvailable(plans, planOf(id)),
                  onSelect: () => void newChat(id)
                }))
              ]}
            />
          ) : null}
        </div>
      </div>
      <div className="lead-options">
        {mode !== null ? (
          <>
            <span className="hint" title="Reads the project. No edits, no commands, no delegation.">
              read only
            </span>
            <button
              type="button"
              className="btn btn-quiet"
              title={`Open ${mode} in a terminal in this project`}
              onClick={() => {
                onOpenTerminal(mode)
              }}
            >
              Open in terminal
            </button>
          </>
        ) : (
          <>
            <span className="lead-option">
              <label htmlFor="lead-plan">Runs on</label>
              <Select
                id="lead-plan"
                className="plan-choice"
                title="Switching plans starts a fresh lead session."
                value={choice}
                disabled={leadStreaming}
                aria-invalid={planError !== null ? true : undefined}
                aria-describedby={planError !== null ? 'lead-plan-error' : undefined}
                onChange={(next) => {
                  void changePlan(asPlanChoice(next))
                }}
              >
                <option value="auto">auto</option>
                {PROVIDERS.map((id) => (
                  <option key={id} value={id} disabled={!planAvailable(plans, planOf(id))}>
                    {id}
                  </option>
                ))}
              </Select>
            </span>
            {modelShown ? (
              <>
                <span className="lead-option">
                  <label htmlFor="lead-model">Model</label>
                  <Select
                    id="lead-model"
                    className="plan-choice lead-model"
                    title="The lead's own model. Workers keep the model set in Settings."
                    value={leadModel.value ?? ''}
                    disabled={leadStreaming}
                    onChange={(next) => {
                      const value = next
                      void changeLeadModel(leadModel.plan, value === '' ? null : value)
                    }}
                  >
                    <option value="">as its jobs</option>
                    {modelOptions.map((option) => (
                      <option key={option.id} value={option.id}>
                        {option.label}
                      </option>
                    ))}
                  </Select>
                </span>
              </>
            ) : null}
            <span className="lead-option">
              <label htmlFor="lead-access">Workers may</label>
              <Select
                id="lead-access"
                className="plan-choice lead-access"
                value={workerAccess}
                disabled={leadStreaming}
                onChange={(next) => {
                  void changeAccess(asAccess(next))
                }}
              >
                <option value="read">read only</option>
                <option value="edit">edit files</option>
                <option value="full">do anything</option>
              </Select>
            </span>
          </>
        )}
      </div>
      {modelError !== null ? (
        <p className="error-text" role="alert">
          {modelError}
        </p>
      ) : null}
      {planError !== null ? (
        <p id="lead-plan-error" className="error-text" role="alert">
          {planError}
        </p>
      ) : null}
      <div ref={logRef} className="lead-log" role="log" aria-live="polite">
        {messages.length === 0 ? (
          <div className="empty">
            <RayBurst />
            <BotAvatar bot="lead" state="idle" size={40} />
            <p className="hint">
              {noPlanReady
                ? 'No plan is ready. See Usage in the panel.'
                : mode === null
                  ? 'Ask the lead for something and it will split the work across your plans.'
                  : `Talk straight to ${mode}. It can read this project but not change files or run commands. For that, open it in a terminal.`}
            </p>
          </div>
        ) : (
          messages.map((message) => {
            const isUser = message.role === 'user'
            const showDots = !isUser && message.status === 'streaming' && message.text === ''
            const delegated = isUser ? [] : jobsForMessage(jobs, message.id)
            const waitingJobs = delegated.filter(
              (job) => job.status === 'queued' || job.status === 'running'
            ).length
            return (
              <div key={message.id} className={isUser ? 'lead-row lead-row-user' : 'lead-row'}>
                <div className={messageClass(message)}>
                  <span className="lead-meta">{isUser ? 'You' : metaLine(leadMeta(message))}</span>
                  <div className="lead-body">
                    {isUser ? (
                      message.text
                    ) : showDots ? (
                      <StreamingDots />
                    ) : (
                      <RichText text={message.text} />
                    )}
                  </div>
                  {!isUser &&
                  message.proposal?.status === 'waiting' &&
                  message.status !== 'streaming' ? (
                    <LeadPlan
                      key={message.id}
                      proposal={message.proposal}
                      onRun={(planned) => onRunPlan(message.id, planned)}
                      onDismiss={() => window.api.dismissLeadProposal(message.id)}
                    />
                  ) : null}
                  {!isUser && message.proposal?.status === 'dismissed' ? (
                    <p className="lead-wait hint">Plan dismissed. Nothing ran.</p>
                  ) : null}
                  {!isUser && waitingJobs > 0 && message.status !== 'streaming' ? (
                    <p className="lead-wait hint">
                      Waiting for {String(waitingJobs)} job{waitingJobs === 1 ? '' : 's'}. The lead
                      will report when they finish.
                    </p>
                  ) : null}
                  {delegated.length > 0 ? (
                    <div className="lead-delegated">
                      <h3 className="lead-delegated-title">Delegated</h3>
                      <div className="lead-delegated-list">
                        {delegated.map((job) => {
                          const label = `Show job: ${job.prompt.slice(0, 60)}`
                          const stepTitle = latestStepTitle(job)
                          const edits = editedFilesSummary(job)
                          return (
                            <button
                              key={job.id}
                              type="button"
                              className="lead-job-card"
                              aria-label={label}
                              onClick={() => onShowJob(job.id)}
                            >
                              <div className="lead-job-card-main">
                                {job.provider !== null ? (
                                  <BotAvatar
                                    bot={job.provider}
                                    state={jobAvatarState(job.status)}
                                    size={20}
                                    title={job.provider}
                                  />
                                ) : null}
                                <span className="lead-job-plan">
                                  {job.provider ?? 'waiting for a worker'}
                                </span>
                                <span className={`status status-${job.status}`}>
                                  <span className="status-dot" aria-hidden="true" />
                                  {job.status}
                                </span>
                              </div>
                              <div className="lead-job-card-meta">
                                {metaLine(delegatedMeta(job))}
                              </div>
                              {job.status === 'running' && stepTitle !== undefined ? (
                                <p className="lead-job-step" title={stepTitle}>
                                  {stepTitle}
                                </p>
                              ) : null}
                              {edits !== null ? (
                                <p className="lead-job-edits">
                                  {`Edited ${edits.files === 1 ? '1 file' : `${String(edits.files)} files`} `}
                                  <span className="stat-add">{`+${String(edits.added)}`}</span>{' '}
                                  <span className="stat-del">{`−${String(edits.removed)}`}</span>
                                </p>
                              ) : null}
                              <p className="lead-job-prompt" title={job.prompt}>
                                {job.prompt}
                              </p>
                            </button>
                          )
                        })}
                      </div>
                    </div>
                  ) : null}
                </div>
              </div>
            )
          })
        )}
      </div>
      {sendError !== null ? (
        <pre id="lead-error" className="output-error" role="alert">
          {sendError}
        </pre>
      ) : null}
      <form
        className="composer"
        onSubmit={(event) => {
          event.preventDefault()
          void send()
        }}
      >
        <label htmlFor="lead-input">
          {mode === null ? 'Message to the lead' : `Message to ${mode}`}
        </label>
        <div className="composer-box">
          <textarea
            id="lead-input"
            rows={3}
            value={draft}
            aria-describedby={sendError !== null ? 'lead-error' : undefined}
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={onDraftKeyDown}
          />
          <div className="composer-foot">
            <span className="composer-hint">Ctrl+Enter to send</span>
            <button className="btn btn-primary" type="submit" disabled={sendDisabled}>
              Send
            </button>
          </div>
        </div>
      </form>
    </div>
  )
}

export default LeadChat
