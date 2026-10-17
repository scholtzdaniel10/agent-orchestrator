import {
  useEffect,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent,
  type MouseEvent
} from 'react'
import BotAvatar from './BotAvatar'
import { Star } from './Star'
import {
  COUNT_MAX,
  COUNT_MIN,
  DEFAULT_COUNT,
  DEFAULT_HOW,
  DEFAULT_KIND,
  DEFAULT_ROUNDS,
  DEFAULT_WHO,
  HOW_OPTIONS,
  JOB_KINDS,
  ROUNDS_MAX,
  ROUNDS_MIN,
  WHO_OPTIONS,
  activeHow,
  asKind,
  clampCount,
  clampRounds,
  howEnabled,
  landingLine,
  planNewTask,
  providerReady,
  repoHint,
  submitLabel,
  unavailableHint,
  type How,
  type JobKind,
  type NewTaskContext,
  type Who
} from './new-task'

interface NewTaskDialogProps {
  open: boolean
  onClose: () => void
  isRepo: boolean
  claudeAvailable: boolean
  cursorAvailable: boolean
  /** Starts agents in new worktrees with the task (the app already does this for the sidebar form). */
  onStartTerminal: (text: string, provider: 'claude' | 'cursor', count: number) => void
  /** After a job or goal was created, so the app can show the right tab. */
  onCreated: (kind: 'job' | 'goal') => void
}

function shownError(err: unknown): string {
  const raw =
    err instanceof Error ? err.message : typeof err === 'string' ? err : 'Could not start that.'
  const text = raw.replace(/^Error invoking remote method '[^']+': (Error: )?/, '').trim()
  return text === '' ? 'Could not start that. Try again.' : text
}

function NewTaskDialogOpen(props: Omit<NewTaskDialogProps, 'open'>): React.JSX.Element {
  const textRef = useRef<HTMLTextAreaElement>(null)
  const sending = useRef(false)
  const [text, setText] = useState('')
  const [who, setWho] = useState<Who>(DEFAULT_WHO)
  const [howChoice, setHowChoice] = useState<How>(DEFAULT_HOW)
  const [count, setCount] = useState(DEFAULT_COUNT)
  const [kind, setKind] = useState<JobKind>(DEFAULT_KIND)
  const [check, setCheck] = useState('')
  const [rounds, setRounds] = useState(DEFAULT_ROUNDS)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const titleId = 'new-task-title'
  const whoLabelId = 'new-task-who'
  const howLabelId = 'new-task-how'

  const ctx: NewTaskContext = {
    isRepo: props.isRepo,
    claudeAvailable: props.claudeAvailable,
    cursorAvailable: props.cursorAvailable
  }
  const how = activeHow(howChoice, props.isRepo)
  const outsideRepo = repoHint(props.isRepo)

  useEffect(() => {
    textRef.current?.focus()
  }, [])

  async function run(): Promise<void> {
    if (sending.current) return
    const plan = planNewTask(
      { text, who, how: howChoice, count, kind, check, maxRounds: rounds },
      ctx
    )
    if (!plan.ok) {
      setError(plan.error)
      return
    }
    sending.current = true
    setError(null)
    if (plan.action === 'terminal') {
      props.onStartTerminal(plan.text, plan.who, plan.count)
      sending.current = false
      props.onClose()
      return
    }
    setBusy(true)
    try {
      if (plan.action === 'job') {
        await window.api.submitJob(plan.jobType, plan.text, plan.who, plan.access)
        props.onCreated('job')
      } else {
        await window.api.createGoal(plan.input)
        props.onCreated(plan.created)
      }
      props.onClose()
    } catch (err: unknown) {
      setError(shownError(err))
    } finally {
      sending.current = false
      setBusy(false)
    }
  }

  function onSubmit(event: FormEvent<HTMLFormElement>): void {
    event.preventDefault()
    void run()
  }

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>): void {
    if (event.key === 'Escape') {
      event.preventDefault()
      event.stopPropagation()
      props.onClose()
      return
    }
    if (event.key === 'Enter' && event.ctrlKey) {
      event.preventDefault()
      void run()
    }
  }

  function onBackdrop(event: MouseEvent<HTMLDivElement>): void {
    if (event.target === event.currentTarget) props.onClose()
  }

  return (
    <div
      className="new-task-overlay"
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      onMouseDown={onBackdrop}
      onKeyDown={onKeyDown}
    >
      <form className="new-task" onSubmit={onSubmit}>
        <div className="panel-head">
          <div className="panel-title">
            <Star size={12} />
            <h2 id={titleId}>New task</h2>
          </div>
        </div>

        <label className="new-task-field" htmlFor="new-task-text">
          What should be done?
          <textarea
            id="new-task-text"
            ref={textRef}
            rows={4}
            value={text}
            placeholder="Fix the off-by-one in the pager and add a test"
            onChange={(event) => {
              setText(event.target.value)
              setError(null)
            }}
          />
        </label>

        <div className="new-task-section">
          <p id={whoLabelId} className="new-task-kicker">
            Who
          </p>
          <div className="new-task-who" role="radiogroup" aria-labelledby={whoLabelId}>
            {WHO_OPTIONS.map((option) => {
              const ready = providerReady(option.id, ctx)
              const hint = unavailableHint(ready)
              const selected = who === option.id
              return (
                <button
                  key={option.id}
                  type="button"
                  role="radio"
                  aria-checked={selected}
                  className={selected ? 'new-task-choice is-selected' : 'new-task-choice'}
                  disabled={!ready}
                  title={hint ?? undefined}
                  onClick={() => {
                    setWho(option.id)
                    setError(null)
                  }}
                >
                  <BotAvatar bot={option.id} state={ready ? 'idle' : 'error'} size={18} />
                  <span className="new-task-choice-body">
                    <span className="new-task-choice-name">{option.label}</span>
                    {hint !== null ? <span className="new-task-choice-hint">{hint}</span> : null}
                  </span>
                </button>
              )
            })}
          </div>
        </div>

        <div className="new-task-section">
          <p id={howLabelId} className="new-task-kicker">
            How
          </p>
          <div className="new-task-how" role="radiogroup" aria-labelledby={howLabelId}>
            {HOW_OPTIONS.map((option) => {
              const enabled = howEnabled(option.id, props.isRepo)
              const selected = how === option.id
              return (
                <button
                  key={option.id}
                  type="button"
                  role="radio"
                  aria-checked={selected}
                  className={selected ? 'new-task-choice is-selected' : 'new-task-choice'}
                  disabled={!enabled}
                  title={!enabled && outsideRepo !== null ? outsideRepo : undefined}
                  onClick={() => {
                    setHowChoice(option.id)
                    setError(null)
                  }}
                >
                  <span className="new-task-choice-body">
                    <span className="new-task-choice-name">{option.label}</span>
                    <span className="new-task-choice-detail">{option.detail}</span>
                  </span>
                </button>
              )
            })}
          </div>
          {outsideRepo !== null ? <p className="hint">{outsideRepo}</p> : null}
          {how === 'terminal' ? (
            <label className="new-task-count">
              How many
              <input
                type="number"
                min={COUNT_MIN}
                max={COUNT_MAX}
                value={count}
                onChange={(event) => {
                  setCount(clampCount(event.target.valueAsNumber))
                }}
              />
            </label>
          ) : null}
          {how === 'background' ? (
            <label className="new-task-field">
              Kind
              <select
                value={kind}
                onChange={(event) => {
                  setKind(asKind(event.target.value))
                }}
              >
                {JOB_KINDS.map((item) => (
                  <option key={item} value={item}>
                    {item}
                  </option>
                ))}
              </select>
            </label>
          ) : null}
          {props.isRepo && (how === 'background' || how === 'goal') ? (
            <label className="new-task-field">
              Check
              <input
                className="new-task-check"
                value={check}
                maxLength={2000}
                placeholder="Optional, e.g. pnpm test"
                onChange={(event) => {
                  setCheck(event.target.value)
                  setError(null)
                }}
              />
            </label>
          ) : null}
          {how === 'goal' ? (
            <label className="new-task-rounds">
              At most
              <input
                type="number"
                min={ROUNDS_MIN}
                max={ROUNDS_MAX}
                value={rounds}
                onChange={(event) => {
                  setRounds(clampRounds(event.target.valueAsNumber))
                }}
              />
              rounds
            </label>
          ) : null}
        </div>

        {error !== null ? (
          <p className="field-error" role="alert">
            {error}
          </p>
        ) : null}

        <div className="new-task-foot">
          <p className="hint">{landingLine(props.isRepo)}</p>
          <div className="new-task-actions">
            <button type="button" className="btn btn-quiet" onClick={props.onClose}>
              Cancel
            </button>
            <button type="submit" className="btn btn-primary" disabled={busy}>
              {busy ? 'Starting…' : submitLabel(how)}
            </button>
          </div>
        </div>
      </form>
    </div>
  )
}

export default function NewTaskDialog({
  open,
  ...rest
}: NewTaskDialogProps): React.JSX.Element | null {
  if (!open) return null
  return <NewTaskDialogOpen {...rest} />
}
