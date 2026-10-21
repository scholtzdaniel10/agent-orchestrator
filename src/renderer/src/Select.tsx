import {
  Children,
  Fragment,
  isValidElement,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode
} from 'react'
import { createPortal } from 'react-dom'
import {
  edgeEnabled,
  matchTyped,
  nextEnabled,
  placeList,
  type Placement,
  type SelectOption
} from './select-logic'

/** Typed letters within this gap add to one search, as a native select does. */
const TYPE_AHEAD_MS = 500

interface OptionProps {
  value?: string | number
  children?: ReactNode
  disabled?: boolean
  title?: string
  'data-detail'?: string
}

function textOf(node: ReactNode): string {
  if (node === null || node === undefined || typeof node === 'boolean') return ''
  if (typeof node === 'string' || typeof node === 'number') return String(node)
  if (Array.isArray(node)) return node.map(textOf).join('')
  if (isValidElement<{ children?: ReactNode }>(node)) return textOf(node.props.children)
  return ''
}

/** The `<option>` elements among `children`, fragments and arrays included, in order. */
function readOptions(children: ReactNode): SelectOption[] {
  const out: SelectOption[] = []
  Children.forEach(children, (child) => {
    if (!isValidElement(child)) return
    if (child.type === Fragment) {
      out.push(...readOptions((child.props as { children?: ReactNode }).children))
      return
    }
    if (child.type !== 'option') return
    const props = child.props as OptionProps
    const label = textOf(props.children)
    const option: SelectOption = { value: String(props.value ?? label), label }
    if (props['data-detail'] !== undefined) option.detail = props['data-detail']
    if (props.disabled === true) option.disabled = true
    if (props.title !== undefined) option.title = props.title
    out.push(option)
  })
  return out
}

function Chevron(): React.JSX.Element {
  return (
    <svg
      className="select-chevron"
      width="10"
      height="6"
      viewBox="0 0 10 6"
      aria-hidden="true"
      focusable="false"
    >
      <path
        d="M1 1l4 4 4-4"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="square"
      />
    </svg>
  )
}

function Check(): React.JSX.Element {
  return (
    <svg
      className="select-check"
      width="10"
      height="8"
      viewBox="0 0 10 8"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M1 4l3 3 5-6" fill="none" stroke="currentColor" strokeWidth="1.5" />
    </svg>
  )
}

/**
 * A dropdown drawn in Legate's own style, in place of a native `<select>`, whose open list
 * Windows draws itself. Takes the same `<option>` children (`data-detail` adds a second,
 * code-face column) and calls `onChange` with the picked value. The list opens in a layer over
 * the window, so a scrolling panel never clips it.
 */
function Select({
  id,
  className,
  value,
  onChange,
  disabled,
  title,
  children,
  'aria-label': ariaLabel,
  'aria-invalid': ariaInvalid,
  'aria-describedby': ariaDescribedBy
}: {
  id?: string
  className?: string
  value: string
  onChange: (value: string) => void
  disabled?: boolean
  title?: string
  children: ReactNode
  'aria-label'?: string
  'aria-invalid'?: boolean
  'aria-describedby'?: string
}): React.JSX.Element {
  const options = readOptions(children)
  const selected = options.findIndex((option) => option.value === value)
  const current = selected >= 0 ? options[selected] : undefined
  const [open, setOpen] = useState(false)
  const [active, setActive] = useState(-1)
  const [place, setPlace] = useState<Placement | null>(null)
  /** The document the button lives in: the list opens there, a panel in its own window too. */
  const [host, setHost] = useState<HTMLElement | null>(null)
  const buttonRef = useRef<HTMLButtonElement>(null)
  const listRef = useRef<HTMLUListElement>(null)
  const typed = useRef({ text: '', at: 0 })
  const baseId = useId()
  const listId = `${baseId}-list`
  const optionId = (index: number): string => `${baseId}-option-${String(index)}`

  function openList(at: number): void {
    if (disabled === true || options.length === 0) return
    setActive(at >= 0 && options[at]?.disabled !== true ? at : edgeEnabled(options, 'first'))
    setPlace(null)
    setHost(buttonRef.current?.ownerDocument.body ?? null)
    setOpen(true)
  }

  function close(refocus: boolean): void {
    setOpen(false)
    if (refocus) buttonRef.current?.focus()
  }

  function pick(index: number): void {
    const option = options[index]
    if (option === undefined || option.disabled === true) return
    close(true)
    if (option.value !== value) onChange(option.value)
  }

  /** Letters typed within TYPE_AHEAD_MS (by the key events' own times) add up to one search. */
  function typeAhead(key: string, from: number, at: number): number {
    const text = at - typed.current.at < TYPE_AHEAD_MS ? typed.current.text + key : key
    typed.current = { text, at }
    const start = text.length > 1 ? from - 1 : from
    return matchTyped(options, start, text)
  }

  // Measure the list once it is in the document, then place it beside the button.
  useLayoutEffect(() => {
    if (!open) return
    const button = buttonRef.current
    const list = listRef.current
    const view = button?.ownerDocument.defaultView
    if (!button || !list || !view) return
    const rect = button.getBoundingClientRect()
    setPlace(
      placeList(
        rect,
        { width: view.innerWidth, height: view.innerHeight },
        list.scrollHeight,
        list.scrollWidth
      )
    )
  }, [open])

  // Focus moves into the list once it is placed: while hidden for measuring it cannot take focus.
  const placed = place !== null
  useEffect(() => {
    if (open && placed) listRef.current?.focus()
  }, [open, placed])

  // Keep the active row in view as the arrow keys move it.
  useEffect(() => {
    if (!open || active < 0) return
    const row = listRef.current?.querySelector<HTMLElement>(`[data-index="${String(active)}"]`)
    row?.scrollIntoView({ block: 'nearest' })
  }, [open, active, place])

  // A click elsewhere, a resize, or a scroll outside the list closes it.
  useEffect(() => {
    if (!open) return
    const doc = buttonRef.current?.ownerDocument
    const view = doc?.defaultView
    if (!doc || !view) return
    const outside = (event: Event): void => {
      const target = event.target as Node | null
      if (target && (buttonRef.current?.contains(target) || listRef.current?.contains(target))) {
        return
      }
      setOpen(false)
    }
    const scrolled = (event: Event): void => {
      if (event.target instanceof Node && listRef.current?.contains(event.target)) return
      setOpen(false)
    }
    const shut = (): void => setOpen(false)
    doc.addEventListener('mousedown', outside, true)
    doc.addEventListener('scroll', scrolled, true)
    view.addEventListener('resize', shut)
    view.addEventListener('blur', shut)
    return () => {
      doc.removeEventListener('mousedown', outside, true)
      doc.removeEventListener('scroll', scrolled, true)
      view.removeEventListener('resize', shut)
      view.removeEventListener('blur', shut)
    }
  }, [open])

  function onButtonKey(event: KeyboardEvent<HTMLButtonElement>): void {
    if (event.altKey || event.ctrlKey || event.metaKey) return
    if (['ArrowDown', 'ArrowUp', 'Enter', ' '].includes(event.key)) {
      event.preventDefault()
      openList(selected)
      return
    }
    // Typing on a closed dropdown picks the matching option, as a native select does.
    if (event.key.length === 1) {
      const found = typeAhead(event.key, selected, event.timeStamp)
      if (found >= 0 && options[found].value !== value) onChange(options[found].value)
    }
  }

  function onListKey(event: KeyboardEvent<HTMLUListElement>): void {
    const moveTo = (index: number): void => {
      event.preventDefault()
      if (index >= 0) setActive(index)
    }
    switch (event.key) {
      case 'ArrowDown':
        return moveTo(nextEnabled(options, active, 1))
      case 'ArrowUp':
        return moveTo(nextEnabled(options, active, -1))
      case 'Home':
        return moveTo(edgeEnabled(options, 'first'))
      case 'End':
        return moveTo(edgeEnabled(options, 'last'))
      case 'Enter':
      case ' ':
        event.preventDefault()
        return pick(active)
      case 'Escape':
        event.preventDefault()
        event.stopPropagation()
        return close(true)
      case 'Tab':
        return close(false)
      default:
        if (event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey) {
          moveTo(typeAhead(event.key, active, event.timeStamp))
        }
    }
  }

  const list = open ? (
    <ul
      ref={listRef}
      id={listId}
      className="select-list"
      role="listbox"
      tabIndex={-1}
      aria-label={ariaLabel}
      aria-labelledby={ariaLabel === undefined && id !== undefined ? id : undefined}
      aria-activedescendant={active >= 0 ? optionId(active) : undefined}
      style={
        place === null
          ? { top: 0, left: 0, visibility: 'hidden' }
          : { top: place.top, left: place.left, minWidth: place.width, maxHeight: place.maxHeight }
      }
      onKeyDown={onListKey}
    >
      {options.map((option, index) => {
        const classes = ['select-option']
        if (index === active) classes.push('is-active')
        if (index === selected) classes.push('is-selected')
        return (
          <li
            key={`${option.value}-${String(index)}`}
            id={optionId(index)}
            data-index={index}
            className={classes.join(' ')}
            role="option"
            aria-selected={index === selected}
            aria-disabled={option.disabled === true ? true : undefined}
            title={option.title}
            onMouseMove={() => {
              if (option.disabled !== true && index !== active) setActive(index)
            }}
            onMouseDown={(event) => {
              // Keep focus in the list so the click lands before anything closes it.
              event.preventDefault()
            }}
            onClick={() => pick(index)}
          >
            <span className="select-mark">{index === selected ? <Check /> : null}</span>
            <span className="select-label">{option.label}</span>
            {option.detail !== undefined ? (
              <span className="select-detail">{option.detail}</span>
            ) : null}
          </li>
        )
      })}
    </ul>
  ) : null

  return (
    <>
      <button
        ref={buttonRef}
        id={id}
        type="button"
        className={className ? `select ${className}` : 'select'}
        title={title}
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? listId : undefined}
        aria-label={ariaLabel}
        aria-invalid={ariaInvalid}
        aria-describedby={ariaDescribedBy}
        onClick={() => (open ? close(true) : openList(selected))}
        onKeyDown={onButtonKey}
      >
        <span className="select-value">{current?.label ?? ''}</span>
        {current?.detail !== undefined ? (
          <span className="select-detail">{current.detail}</span>
        ) : null}
        <Chevron />
      </button>
      {list !== null && host !== null ? createPortal(list, host) : null}
    </>
  )
}

export default Select
