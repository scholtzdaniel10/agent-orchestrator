import { useEffect, useId, useRef, type KeyboardEvent, type MouseEvent } from 'react'
import { LEG_IT_EFFECTS, LEG_IT_KEPT } from './leg-it'

/** Asked before LEG IT! turns on: what it lets through. Cancel has focus, so Enter is safe. */
export default function LegItConfirm({
  open,
  onConfirm,
  onCancel
}: {
  open: boolean
  onConfirm: () => void
  onCancel: () => void
}): React.JSX.Element | null {
  const cancelRef = useRef<HTMLButtonElement>(null)
  const titleId = useId()
  const bodyId = useId()

  useEffect(() => {
    if (open) cancelRef.current?.focus()
  }, [open])

  if (!open) return null

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>): void {
    if (event.key !== 'Escape') return
    event.preventDefault()
    event.stopPropagation()
    onCancel()
  }

  function onBackdrop(event: MouseEvent<HTMLDivElement>): void {
    if (event.target === event.currentTarget) onCancel()
  }

  return (
    <div
      className="new-task-overlay"
      role="alertdialog"
      aria-modal="true"
      aria-labelledby={titleId}
      aria-describedby={bodyId}
      onMouseDown={onBackdrop}
      onKeyDown={onKeyDown}
    >
      <div className="new-task legit-confirm">
        <div className="panel-head">
          <div className="panel-title">
            <h2 id={titleId}>Turn on LEG IT!?</h2>
          </div>
        </div>
        <div id={bodyId}>
          <ul className="legit-confirm-list">
            {LEG_IT_EFFECTS.map((line) => (
              <li key={line}>{line}</li>
            ))}
          </ul>
          <p className="hint">{LEG_IT_KEPT} Press LEG IT! again to turn it off.</p>
        </div>
        <div className="new-task-actions">
          <button ref={cancelRef} type="button" className="btn btn-quiet" onClick={onCancel}>
            Cancel
          </button>
          <button type="button" className="btn btn-primary btn-legit" onClick={onConfirm}>
            Turn on LEG IT!
          </button>
        </div>
      </div>
    </div>
  )
}
