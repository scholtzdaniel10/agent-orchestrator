import { useEffect, useState, type FocusEvent, type KeyboardEvent } from 'react'

type PlanStatus = Awaited<ReturnType<Window['api']['listPlans']>>[number]
type ModelOption = Awaited<ReturnType<Window['api']['listModels']>>[number]

const OTHER = '__other__'

function errorText(err: unknown): string {
  if (err instanceof Error) {
    // Electron prefixes errors from the main process; show only the message itself.
    return err.message.replace(/^Error invoking remote method '[^']+': (Error: )?/, '')
  }
  if (typeof err === 'string') return err
  return 'Request failed'
}

function ModelField({
  plan,
  disabled,
  models,
  labelTitle
}: {
  plan: PlanStatus
  disabled: boolean
  models: ModelOption[] | null
  labelTitle?: string
}): React.JSX.Element {
  const saved = plan.model ?? ''
  const [typing, setTyping] = useState(false)
  const [typed, setTyped] = useState(saved)
  const [error, setError] = useState<string | null>(null)
  const selectId = `model-${plan.id}`
  const otherId = `model-other-${plan.id}`
  const errorId = `model-error-${plan.id}`
  const hintId = `model-hint-${plan.id}`
  const describedBy = [labelTitle !== undefined ? hintId : null, error !== null ? errorId : null]
    .filter((id): id is string => id !== null)
    .join(' ')
  const listed = models ?? []
  // A model typed by hand, or saved before the list arrived, still needs a row of its own.
  const savedIsListed = saved === '' || listed.some((option) => option.id === saved)

  async function save(next: string): Promise<void> {
    if (disabled) return
    setError(null)
    try {
      await window.api.setModel(plan.id, next === '' ? null : next)
    } catch (err: unknown) {
      setError(errorText(err))
    }
  }

  function onSelect(next: string): void {
    if (next === OTHER) {
      setTyped(saved)
      setTyping(true)
      return
    }
    setTyping(false)
    if (next !== saved) void save(next)
  }

  function commitTyped(raw: string): void {
    const next = raw.trim()
    setTyping(false)
    if (next !== saved) void save(next)
  }

  function onKeyDown(event: KeyboardEvent<HTMLInputElement>): void {
    if (event.key === 'Escape') {
      event.preventDefault()
      setTyping(false)
      return
    }
    if (event.key !== 'Enter') return
    event.preventDefault()
    commitTyped(event.currentTarget.value)
  }

  function onBlur(event: FocusEvent<HTMLInputElement>): void {
    if (!event.currentTarget.isConnected) return
    commitTyped(event.currentTarget.value)
  }

  return (
    <div className="model-picker">
      <label htmlFor={selectId} title={labelTitle}>
        Model
      </label>
      {labelTitle !== undefined ? (
        <span id={hintId} className="visually-hidden">
          {labelTitle}
        </span>
      ) : null}
      <div className="model-row">
        <select
          id={selectId}
          className="model-input"
          value={typing ? OTHER : saved}
          disabled={disabled}
          aria-invalid={error !== null ? true : undefined}
          aria-describedby={describedBy === '' ? undefined : describedBy}
          onChange={(event) => {
            onSelect(event.target.value)
          }}
        >
          <option value="">CLI default</option>
          {savedIsListed ? null : <option value={saved}>{saved}</option>}
          {listed.map((option) => (
            <option key={option.id} value={option.id}>
              {option.label === option.id ? option.id : `${option.label} · ${option.id}`}
            </option>
          ))}
          <option value={OTHER}>Other…</option>
        </select>
      </div>
      {typing ? (
        <div className="model-row">
          <label htmlFor={otherId} className="visually-hidden">
            Model name
          </label>
          <input
            id={otherId}
            className="model-input"
            placeholder="Model name, then Enter"
            value={typed}
            disabled={disabled}
            spellCheck={false}
            autoComplete="off"
            autoFocus
            onChange={(event) => {
              setTyped(event.target.value)
            }}
            onBlur={onBlur}
            onKeyDown={onKeyDown}
          />
        </div>
      ) : null}
      {error !== null ? (
        <p id={errorId} className="field-error" role="alert">
          {error}
        </p>
      ) : null}
    </div>
  )
}

function ModelPicker({
  plan,
  disabled,
  labelTitle
}: {
  plan: PlanStatus
  disabled: boolean
  labelTitle?: string
}): React.JSX.Element {
  const [models, setModels] = useState<ModelOption[] | null>(null)
  const { id, available } = plan

  // Load the list up front: every model has to be there the first time the menu opens.
  useEffect(() => {
    if (!available) return
    let active = true
    void window.api.listModels(id).then(
      (list) => {
        if (active) setModels(list)
      },
      () => {
        // The menu still offers the CLI default and a typed name.
      }
    )
    return () => {
      active = false
    }
  }, [id, available])

  return (
    <ModelField
      key={plan.model ?? ''}
      plan={plan}
      disabled={disabled}
      models={models}
      labelTitle={labelTitle}
    />
  )
}

export default ModelPicker
