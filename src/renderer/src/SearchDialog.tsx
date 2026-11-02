import { useEffect, useId, useRef, useState, type KeyboardEvent, type MouseEvent } from 'react'
import { fieldLabel, highlightSegments, type SearchHit } from '../../core/search'
import {
  hitMeta,
  moveActive,
  resultsLine,
  searchable,
  SEARCH_DEBOUNCE_MS,
  SEARCH_SHOWN
} from './search-view'

interface SearchDialogProps {
  open: boolean
  /** Text to start with, e.g. what the sidebar filter holds. */
  initialQuery: string
  now: number
  onClose: () => void
  onOpenChat: (id: string, messageId: string | undefined) => void
  onOpenJob: (id: string) => void
}

function shownError(err: unknown): string {
  const raw = err instanceof Error ? err.message : typeof err === 'string' ? err : ''
  const text = raw.replace(/^Error invoking remote method '[^']+': (Error: )?/, '').trim()
  return text === '' ? 'Search failed. Try again.' : text
}

function SearchDialogOpen(props: Omit<SearchDialogProps, 'open'>): React.JSX.Element {
  const inputRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLUListElement>(null)
  const asked = useRef(0)
  const [query, setQuery] = useState(props.initialQuery)
  /** The latest answer, with the query it answers. */
  const [result, setResult] = useState<{
    query: string
    hits: SearchHit[]
    error: string | null
  } | null>(null)
  /** The row picked with the arrow keys, for the query it was picked in. */
  const [picked, setPicked] = useState<{ query: string; index: number } | null>(null)
  const titleId = useId()
  const listId = useId()

  const ready = searchable(query)
  const answer = ready && result !== null && result.query === query ? result : null
  const hits = answer?.hits ?? []
  const error = answer?.error ?? null
  const busy = ready && answer === null
  const active =
    hits.length === 0
      ? -1
      : picked !== null && picked.query === query && picked.index < hits.length
        ? picked.index
        : 0

  useEffect(() => {
    inputRef.current?.focus()
    inputRef.current?.select()
  }, [])

  useEffect(() => {
    if (!searchable(query)) return
    const ticket = ++asked.current
    const timer = window.setTimeout(() => {
      window.api.searchHistory(query, SEARCH_SHOWN).then(
        (found) => {
          if (ticket === asked.current) setResult({ query, hits: found, error: null })
        },
        (err: unknown) => {
          if (ticket === asked.current) setResult({ query, hits: [], error: shownError(err) })
        }
      )
    }, SEARCH_DEBOUNCE_MS)
    return () => {
      window.clearTimeout(timer)
    }
  }, [query])

  useEffect(() => {
    if (active < 0) return
    listRef.current
      ?.querySelector(`[data-index="${String(active)}"]`)
      ?.scrollIntoView({ block: 'nearest' })
  }, [active])

  function choose(hit: SearchHit): void {
    props.onClose()
    if (hit.kind === 'chat') props.onOpenChat(hit.id, hit.messageId)
    else props.onOpenJob(hit.id)
  }

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>): void {
    if (event.key === 'Escape') {
      event.preventDefault()
      event.stopPropagation()
      props.onClose()
      return
    }
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault()
      setPicked({
        query,
        index: moveActive(active, event.key === 'ArrowDown' ? 1 : -1, hits.length)
      })
      return
    }
    if (event.key === 'Enter') {
      const hit = hits[active]
      if (hit === undefined) return
      event.preventDefault()
      choose(hit)
    }
  }

  function onBackdrop(event: MouseEvent<HTMLDivElement>): void {
    if (event.target === event.currentTarget) props.onClose()
  }

  const activeId = active >= 0 ? `${listId}-${String(active)}` : undefined
  const status = error ?? (busy ? 'Searching…' : resultsLine(query, hits.length))

  return (
    <div
      className="new-task-overlay"
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      onMouseDown={onBackdrop}
      onKeyDown={onKeyDown}
    >
      <div className="search-dialog">
        <h2 id={titleId} className="visually-hidden">
          Search chats and jobs
        </h2>
        <input
          ref={inputRef}
          className="search-dialog-input"
          type="search"
          value={query}
          maxLength={200}
          placeholder="Search chats and jobs  (all words must match)"
          aria-label="Search chats and jobs"
          role="combobox"
          aria-expanded={hits.length > 0}
          aria-controls={listId}
          aria-activedescendant={activeId}
          aria-autocomplete="list"
          onChange={(event) => {
            setQuery(event.target.value)
          }}
        />
        {hits.length > 0 ? (
          <ul ref={listRef} id={listId} className="search-results" role="listbox">
            {hits.map((hit, index) => (
              <li
                key={`${hit.kind}:${hit.id}`}
                id={`${listId}-${String(index)}`}
                data-index={index}
                role="option"
                aria-selected={index === active}
                className={index === active ? 'search-hit is-active' : 'search-hit'}
                onMouseMove={() => {
                  if (index !== active) setPicked({ query, index })
                }}
                onClick={() => {
                  choose(hit)
                }}
              >
                <div className="search-hit-head">
                  <span className="search-hit-title">
                    {hit.title === '' ? '(untitled)' : hit.title}
                  </span>
                  <span className="search-hit-meta">{hitMeta(hit, props.now)}</span>
                </div>
                <div className="search-hit-snippet">
                  <span className="search-hit-field">{fieldLabel(hit)}</span>
                  {highlightSegments(hit.snippet, hit.marks).map((part, at) =>
                    part.mark ? (
                      <mark key={at}>{part.text}</mark>
                    ) : (
                      <span key={at}>{part.text}</span>
                    )
                  )}
                </div>
              </li>
            ))}
          </ul>
        ) : null}
        <p className={error !== null ? 'search-status is-error' : 'search-status'} role="status">
          {status}
        </p>
      </div>
    </div>
  )
}

/** Full-text search over the open project's saved chats and jobs. Ctrl+Shift+F. */
export default function SearchDialog(props: SearchDialogProps): React.JSX.Element | null {
  if (!props.open) return null
  return <SearchDialogOpen {...props} />
}
