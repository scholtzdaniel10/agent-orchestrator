import { useEffect, useLayoutEffect, useRef, type KeyboardEvent } from 'react'

export interface MenuEntry {
  label: string
  onSelect?: () => void
  disabled?: boolean
  danger?: boolean
  /** Shown as a ticked option. */
  checked?: boolean
  /** A non-clickable heading. */
  heading?: boolean
  /** Draw a line above this entry. */
  separator?: boolean
  title?: string
}

/** A small popup menu at a screen point: arrow keys, Enter, Escape, outside click. */
function SidebarMenu({
  label,
  at,
  entries,
  onClose
}: {
  label: string
  at: { x: number; y: number }
  entries: MenuEntry[]
  onClose: () => void
}): React.JSX.Element {
  const ref = useRef<HTMLUListElement>(null)
  // Parents re-render often; keep the listeners (and the focus return) from re-running.
  const closeRef = useRef(onClose)
  // Read before the menu takes focus, so focus can go back there.
  const openerRef = useRef<Element | null>(document.activeElement)
  useLayoutEffect(() => {
    closeRef.current = onClose
  })

  useLayoutEffect(() => {
    const element = ref.current
    if (element === null) return
    const rect = element.getBoundingClientRect()
    const x = Math.max(4, Math.min(at.x, window.innerWidth - rect.width - 4))
    const y = Math.max(4, Math.min(at.y, window.innerHeight - rect.height - 4))
    element.style.left = `${String(x)}px`
    element.style.top = `${String(y)}px`
    element.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus()
  }, [at])

  useEffect(() => {
    const opener = openerRef.current
    const onDown = (event: globalThis.MouseEvent): void => {
      if (
        ref.current !== null &&
        event.target instanceof Node &&
        ref.current.contains(event.target)
      )
        return
      closeRef.current()
    }
    const onBlur = (): void => {
      closeRef.current()
    }
    document.addEventListener('mousedown', onDown, true)
    window.addEventListener('blur', onBlur)
    return () => {
      document.removeEventListener('mousedown', onDown, true)
      window.removeEventListener('blur', onBlur)
      if (opener instanceof HTMLElement && opener.isConnected) opener.focus()
    }
  }, [])

  function onKeyDown(event: KeyboardEvent<HTMLUListElement>): void {
    if (event.key === 'Escape' || event.key === 'Tab') {
      event.preventDefault()
      event.stopPropagation()
      onClose()
      return
    }
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
    event.preventDefault()
    const buttons = [
      ...(ref.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? [])
    ]
    if (buttons.length === 0) return
    const index = buttons.findIndex((button) => button === document.activeElement)
    const step = event.key === 'ArrowDown' ? 1 : -1
    buttons[(index + step + buttons.length) % buttons.length]?.focus()
  }

  return (
    <ul
      ref={ref}
      className="side-menu"
      role="menu"
      aria-label={label}
      style={{ left: at.x, top: at.y }}
      onKeyDown={onKeyDown}
      onContextMenu={(event) => {
        event.preventDefault()
      }}
    >
      {entries.map((entry, index) => {
        const sep = entry.separator === true ? ' has-separator' : ''
        if (entry.heading === true) {
          return (
            <li key={index} role="presentation" className={`side-menu-heading${sep}`}>
              {entry.label}
            </li>
          )
        }
        const checkable = entry.checked !== undefined
        return (
          <li key={index} role="none" className={sep === '' ? undefined : 'has-separator'}>
            <button
              type="button"
              role={checkable ? 'menuitemcheckbox' : 'menuitem'}
              aria-checked={checkable ? entry.checked : undefined}
              className={`side-menu-item${entry.danger === true ? ' is-danger' : ''}`}
              disabled={entry.disabled === true}
              title={entry.title}
              onClick={() => {
                onClose()
                entry.onSelect?.()
              }}
            >
              <span className="side-menu-check" aria-hidden="true">
                {entry.checked === true ? '■' : ''}
              </span>
              {entry.label}
            </button>
          </li>
        )
      })}
    </ul>
  )
}

export default SidebarMenu
