import { useRef, useState, type FocusEvent, type KeyboardEvent } from 'react'

type PlanStatus = Awaited<ReturnType<Window['api']['listPlans']>>[number]
type ModelOption = Awaited<ReturnType<Window['api']['listModels']>>[number]

function errorText(err: unknown): string {
  if (err instanceof Error) return err.message
  if (typeof err === 'string') return err
  return 'Request failed'
}

function ModelField({
  plan,
  disabled,
  models,
  onFocus
}: {
  plan: PlanStatus
  disabled: boolean
  models: ModelOption[] | null
  onFocus: () => void
}): React.JSX.Element {
  const [value, setValue] = useState(plan.model ?? '')
  const [error, setError] = useState<string | null>(null)
  const inputId = `model-${plan.id}`
  const listId = `model-list-${plan.id}`
  const errorId = `model-error-${plan.id}`
  const modelSet = plan.model !== null && plan.model !== ''

  async function commit(raw: string): Promise<void> {
    if (disabled) return
    const next = raw.trim()
    const prev = plan.model ?? ''
    if (next === prev) {
      setValue(prev)
      return
    }
    setValue(next)
    setError(null)
    try {
      await window.api.setModel(plan.id, next === '' ? null : next)
    } catch (err: unknown) {
      setError(errorText(err))
      setValue(prev)
    }
  }

  async function clearModel(): Promise<void> {
    if (disabled) return
    const prev = plan.model ?? ''
    setValue('')
    setError(null)
    try {
      await window.api.setModel(plan.id, null)
    } catch (err: unknown) {
      setError(errorText(err))
      setValue(prev)
    }
  }

  function onKeyDown(event: KeyboardEvent<HTMLInputElement>): void {
    if (event.key !== 'Enter') return
    event.preventDefault()
    void commit(event.currentTarget.value)
  }

  function onBlur(event: FocusEvent<HTMLInputElement>): void {
    if (!event.currentTarget.isConnected) return
    void commit(event.currentTarget.value)
  }

  return (
    <div className="model-picker">
      <label htmlFor={inputId}>Model</label>
      <div className="model-row">
        <input
          id={inputId}
          className="model-input"
          list={listId}
          placeholder="CLI default"
          value={value}
          disabled={disabled}
          spellCheck={false}
          autoComplete="off"
          aria-invalid={error !== null ? true : undefined}
          aria-describedby={error !== null ? errorId : undefined}
          onFocus={onFocus}
          onChange={(event) => {
            setValue(event.target.value)
            if (error !== null) setError(null)
          }}
          onBlur={onBlur}
          onKeyDown={onKeyDown}
        />
        {modelSet ? (
          <button
            type="button"
            className="btn btn-quiet model-clear"
            aria-label={`Use the CLI default model for ${plan.id}`}
            disabled={disabled}
            onMouseDown={(event) => {
              event.preventDefault()
            }}
            onClick={() => {
              void clearModel()
            }}
          >
            ×
          </button>
        ) : null}
      </div>
      <datalist id={listId}>
        {models?.map((option) => (
          <option key={option.id} value={option.id}>
            {option.label}
          </option>
        ))}
      </datalist>
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
  disabled
}: {
  plan: PlanStatus
  disabled: boolean
}): React.JSX.Element {
  const started = useRef(false)
  const [models, setModels] = useState<ModelOption[] | null>(null)

  function onFocus(): void {
    if (started.current) return
    started.current = true
    void window.api.listModels(plan.id).then(
      (list) => {
        setModels(list)
      },
      () => {
        started.current = false
      }
    )
  }

  return (
    <ModelField
      key={plan.model ?? ''}
      plan={plan}
      disabled={disabled}
      models={models}
      onFocus={onFocus}
    />
  )
}

export default ModelPicker
