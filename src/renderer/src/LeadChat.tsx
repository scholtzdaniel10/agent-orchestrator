import { useLayoutEffect, useRef, useState, type KeyboardEvent } from 'react'
import BotAvatar from './BotAvatar'
import RichText from './RichText'

type LeadMessage = Awaited<ReturnType<Window['api']['listLeadMessages']>>[number]

function errorText(err: unknown): string {
  if (err instanceof Error) return err.message
  if (typeof err === 'string') return err
  return 'Request failed'
}

function leadMeta(message: LeadMessage): string {
  const parts = ['Lead']
  if (message.provider) parts.push(message.provider)
  if (message.model) parts.push(message.model)
  if (message.status === 'streaming') parts.push('working…')
  return parts.join(' · ')
}

function leadAvatarState(status: LeadMessage['status']): 'idle' | 'working' | 'error' {
  if (status === 'streaming') return 'working'
  if (status === 'error') return 'error'
  return 'idle'
}

function bubbleClass(message: LeadMessage): string {
  if (message.status === 'error') return 'lead-bubble lead-bubble-error'
  if (message.role === 'user') return 'lead-bubble lead-bubble-user'
  return 'lead-bubble'
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
  onReset
}: {
  messages: LeadMessage[]
  onReset: () => Promise<void>
}): React.JSX.Element {
  const [draft, setDraft] = useState('')
  const [sendError, setSendError] = useState<string | null>(null)
  const logRef = useRef<HTMLDivElement>(null)
  const sending = useRef(false)
  const resetting = useRef(false)

  useLayoutEffect(() => {
    const log = logRef.current
    if (log === null) return
    log.scrollTop = log.scrollHeight
  }, [messages])

  const draftEmpty = draft.trim() === ''
  const leadStreaming = messages.some(
    (message) => message.role === 'lead' && message.status === 'streaming'
  )
  const anyStreaming = messages.some((message) => message.status === 'streaming')
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
    if (anyStreaming || resetting.current) return
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
          <BotAvatar bot="lead" state={leadStreaming ? 'working' : 'idle'} size={20} />
          <h2 id="lead-heading">Lead</h2>
        </div>
        <button
          type="button"
          className="btn btn-quiet"
          disabled={anyStreaming}
          onClick={() => {
            void newChat()
          }}
        >
          New chat
        </button>
      </div>
      <div ref={logRef} className="lead-log" role="log" aria-live="polite">
        {messages.length === 0 ? (
          <div className="empty">
            <BotAvatar bot="lead" state="idle" size={40} />
            <p className="hint">
              Ask the lead for something and it will split the work across your plans.
            </p>
          </div>
        ) : (
          messages.map((message) => {
            const isUser = message.role === 'user'
            const showDots = !isUser && message.status === 'streaming' && message.text === ''
            return (
              <div key={message.id} className={isUser ? 'lead-row lead-row-user' : 'lead-row'}>
                {isUser ? null : (
                  <BotAvatar bot="lead" state={leadAvatarState(message.status)} size={28} />
                )}
                <div className={bubbleClass(message)}>
                  {isUser ? null : <span className="lead-meta">{leadMeta(message)}</span>}
                  <div className="lead-body">
                    {isUser ? (
                      message.text
                    ) : showDots ? (
                      <StreamingDots />
                    ) : (
                      <RichText text={message.text} />
                    )}
                  </div>
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
