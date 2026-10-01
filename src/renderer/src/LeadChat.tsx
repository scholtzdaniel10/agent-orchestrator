import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from 'react'

type LeadMessage = Awaited<ReturnType<Window['api']['listLeadMessages']>>[number]

function mergeMessage(messages: LeadMessage[], message: LeadMessage): LeadMessage[] {
  const index = messages.findIndex((item) => item.id === message.id)
  if (index === -1) return [...messages, message]
  const next = messages.slice()
  next[index] = message
  return next
}

function mergeMessageList(current: LeadMessage[], list: LeadMessage[]): LeadMessage[] {
  const listed = new Set(list.map((message) => message.id))
  const live = new Map(current.map((message) => [message.id, message]))
  const merged = list.map((message) => live.get(message.id) ?? message)
  for (const message of current) {
    if (!listed.has(message.id)) merged.push(message)
  }
  return merged
}

function errorText(err: unknown): string {
  if (err instanceof Error) return err.message
  if (typeof err === 'string') return err
  return 'Request failed'
}

function leadMeta(message: LeadMessage): string {
  const name = message.provider ? `lead · ${message.provider}` : 'lead'
  if (message.status === 'streaming') return `${name} · working…`
  return name
}

function LeadChat(): React.JSX.Element {
  const [messages, setMessages] = useState<LeadMessage[]>([])
  const [draft, setDraft] = useState('')
  const [sendError, setSendError] = useState<string | null>(null)
  const logRef = useRef<HTMLDivElement>(null)
  const sending = useRef(false)
  const resetting = useRef(false)
  const acceptList = useRef(true)

  useEffect(() => {
    let active = true
    const unsubscribe = window.api.onLeadUpdate((message) => {
      setMessages((current) => mergeMessage(current, message))
    })
    void window.api.listLeadMessages().then((list) => {
      if (!active || !acceptList.current) return
      setMessages((current) => mergeMessageList(current, list))
    })
    return () => {
      active = false
      unsubscribe()
    }
  }, [])

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
      await window.api.resetLead()
      acceptList.current = false
      setMessages([])
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
      <div ref={logRef} className="lead-log" role="log" aria-live="polite">
        {messages.length === 0 ? (
          <p className="hint">
            Ask the lead for something and it will split the work across your plans.
          </p>
        ) : (
          messages.map((message) => {
            const isUser = message.role === 'user'
            const isError = message.status === 'error'
            const bubble = isUser ? 'lead-msg lead-msg-user' : 'lead-msg lead-msg-lead'
            return (
              <div key={message.id} className={bubble}>
                {isUser ? null : <span className="lead-meta">{leadMeta(message)}</span>}
                <div className={isError ? 'lead-text output-error' : 'lead-text'}>
                  {message.text}
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
        className="lead-form"
        onSubmit={(event) => {
          event.preventDefault()
          void send()
        }}
      >
        <label htmlFor="lead-input">Message to the lead</label>
        <textarea
          id="lead-input"
          rows={3}
          value={draft}
          aria-describedby={sendError !== null ? 'lead-error' : undefined}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={onDraftKeyDown}
        />
        <div className="lead-actions">
          <button type="submit" disabled={sendDisabled}>
            Send
          </button>
          <button type="button" disabled={anyStreaming} onClick={() => void newChat()}>
            New chat
          </button>
        </div>
      </form>
    </div>
  )
}

export default LeadChat
