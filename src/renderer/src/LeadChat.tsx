import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react'
import { RayBurst, Star } from './Star'
import BotAvatar from './BotAvatar'
import RichText from './RichText'

type LeadMessage = Awaited<ReturnType<Window['api']['listLeadMessages']>>[number]
type JobRecord = Awaited<ReturnType<Window['api']['listJobs']>>[number]
type PlanStatus = Awaited<ReturnType<Window['api']['listPlans']>>[number]
type ProviderId = PlanStatus['id']
type PlanChoice = 'auto' | ProviderId

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
  if (err instanceof Error) return err.message
  if (typeof err === 'string') return err
  return 'Request failed'
}

function asPlanChoice(value: string): PlanChoice {
  if (value === 'claude' || value === 'cursor') return value
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
    if (index > 0) nodes.push(<span key={`sep-${String(index)}`} className="meta-sep"> / </span>)
    nodes.push(<span key={`part-${String(index)}`}>{part}</span>)
  })
  return <>{nodes}</>
}

function delegatedMeta(job: JobRecord): ReactNode[] {
  const parts: ReactNode[] = [job.type]
  if (job.edit === true) parts.push('edits')
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
  onReset,
  onShowJob
}: {
  messages: LeadMessage[]
  jobs: JobRecord[]
  plans: PlanStatus[] | null
  onReset: () => Promise<void>
  onShowJob: (id: string) => void
}): React.JSX.Element {
  const [draft, setDraft] = useState('')
  const [sendError, setSendError] = useState<string | null>(null)
  const [choice, setChoice] = useState<PlanChoice>('auto')
  const [planError, setPlanError] = useState<string | null>(null)
  const logRef = useRef<HTMLDivElement>(null)
  const sending = useRef(false)
  const resetting = useRef(false)
  const choiceRef = useRef<PlanChoice>('auto')
  const picked = useRef(false)

  useEffect(() => {
    let active = true
    void window.api.getLeadPlan().then(
      (plan) => {
        if (!active || picked.current) return
        const value: PlanChoice = plan ?? 'auto'
        choiceRef.current = value
        setChoice(value)
      },
      (err: unknown) => {
        if (!active || picked.current) return
        setPlanError(errorText(err))
      }
    )
    return () => {
      active = false
    }
  }, [])

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
  const sendDisabled = draftEmpty || leadStreaming

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

  async function newChat(): Promise<void> {
    if (leadStreaming || resetting.current) return
    resetting.current = true
    try {
      await onReset()
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
          <BotAvatar bot="lead" state={leadStreaming ? 'working' : 'idle'} size={20} />
          <h2 id="lead-heading">Lead</h2>
        </div>
        <div className="head-actions">
          <label htmlFor="lead-plan">Runs on</label>
          <select
            id="lead-plan"
            className="plan-choice"
            title="Switching plans starts a fresh lead session."
            value={choice}
            disabled={leadStreaming}
            aria-invalid={planError !== null ? true : undefined}
            aria-describedby={planError !== null ? 'lead-plan-error' : undefined}
            onChange={(event) => {
              void changePlan(asPlanChoice(event.target.value))
            }}
          >
            <option value="auto">auto</option>
            <option value="claude" disabled={!planAvailable(plans, 'claude')}>
              claude
            </option>
            <option value="cursor" disabled={!planAvailable(plans, 'cursor')}>
              cursor
            </option>
          </select>
          <button
            type="button"
            className="btn btn-quiet"
            disabled={leadStreaming}
            onClick={() => {
              void newChat()
            }}
          >
            New chat
          </button>
        </div>
      </div>
      {planError !== null ? (
        <p id="lead-plan-error" className="field-error" role="alert">
          {planError}
        </p>
      ) : null}
      <div ref={logRef} className="lead-log" role="log" aria-live="polite">
        {messages.length === 0 ? (
          <div className="empty">
            <RayBurst />
            <BotAvatar bot="lead" state="idle" size={40} />
            <p className="hint">
              Ask the lead for something and it will split the work across your plans.
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
                  <span className="lead-meta">
                    {isUser ? 'You' : metaLine(leadMeta(message))}
                  </span>
                  <div className="lead-body">
                    {isUser ? (
                      message.text
                    ) : showDots ? (
                      <StreamingDots />
                    ) : (
                      <RichText text={message.text} />
                    )}
                  </div>
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
        <label htmlFor="lead-input">Message to the lead</label>
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
