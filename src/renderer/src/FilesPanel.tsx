import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type UIEvent
} from 'react'
import { parseDiff } from './diff-lines'
import {
  FILE_LINE_CAP,
  FILE_ROW_HEIGHT,
  STATUS_MARK,
  buildTree,
  filesCountLabel,
  flattenVisible,
  formatBytes,
  parentPath,
  windowRange,
  type FileEntry,
  type FileStatus,
  type VisibleRow
} from './file-tree'

type FileList = { entries: FileEntry[]; truncated: boolean }
type FileContent = Awaited<ReturnType<Window['api']['readFile']>>

const expandedByWhere = new Map<string, Set<string>>()
const splitByWhere = new Map<string, number>()
const selectedByWhere = new Map<string, string | null>()

const DIFF_LINE_CAP = FILE_LINE_CAP

function whereKey(where: string | null): string {
  return where ?? ''
}

function errorText(err: unknown): string {
  if (err instanceof Error) {
    return err.message.replace(/^Error invoking remote method '[^']+': (Error: )?/, '')
  }
  if (typeof err === 'string') return err
  return 'Request failed'
}

function rowId(row: VisibleRow): string {
  return `files-${row.kind}-${row.path}`
}

function markClass(status: FileStatus): string {
  return `files-mark is-${status}`
}

function FilesPanel({ where, label }: { where: string | null; label: string }): React.JSX.Element {
  const key = whereKey(where)
  const [seenWhere, setSeenWhere] = useState(where)
  const [list, setList] = useState<FileList | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [filter, setFilter] = useState('')
  const [changedOnly, setChangedOnly] = useState(false)
  const [expanded, setExpanded] = useState<Set<string>>(
    () => new Set(expandedByWhere.get(key) ?? [])
  )
  const [split, setSplit] = useState(() => splitByWhere.get(key) ?? 0.5)
  const [selectedPath, setSelectedPath] = useState<string | null>(
    () => selectedByWhere.get(key) ?? null
  )
  const [focusedPath, setFocusedPath] = useState<string | null>(null)
  const [content, setContent] = useState<{ path: string; value: FileContent } | null>(null)
  const [diff, setDiff] = useState<{ path: string; value: string } | null>(null)
  const [preview, setPreview] = useState<'file' | 'changes'>('file')
  const [opening, setOpening] = useState(false)
  const [openError, setOpenError] = useState<string | null>(null)
  const [scrollTop, setScrollTop] = useState(0)
  const [viewH, setViewH] = useState(320)
  const loadId = useRef(0)
  const lastAt = useRef(0)
  const pending = useRef<ReturnType<typeof setTimeout> | null>(null)
  const whereRef = useRef(where)
  const treeRef = useRef<HTMLDivElement>(null)
  const bodyRef = useRef<HTMLDivElement>(null)

  if (seenWhere !== where) {
    setSeenWhere(where)
    const nextKey = whereKey(where)
    setExpanded(new Set(expandedByWhere.get(nextKey) ?? []))
    setSplit(splitByWhere.get(nextKey) ?? 0.5)
    setSelectedPath(selectedByWhere.get(nextKey) ?? null)
    setFocusedPath(null)
    setFilter('')
    setChangedOnly(false)
    setPreview('file')
    setContent(null)
    setDiff(null)
    setList(null)
    setError(null)
    setLoading(true)
    setOpenError(null)
  }

  const load = useCallback((immediate: boolean): void => {
    const go = (): void => {
      lastAt.current = Date.now()
      pending.current = null
      const id = loadId.current + 1
      loadId.current = id
      const asked = whereRef.current
      setLoading(true)
      setError(null)
      void window.api.listFiles(asked).then(
        (result) => {
          if (id !== loadId.current) return
          setList(result)
          setLoading(false)
        },
        (err: unknown) => {
          if (id !== loadId.current) return
          setError(errorText(err))
          setLoading(false)
        }
      )
    }
    if (pending.current !== null) {
      clearTimeout(pending.current)
      pending.current = null
    }
    if (immediate) {
      go()
      return
    }
    const wait = Math.max(0, 1000 - (Date.now() - lastAt.current))
    if (wait === 0) {
      go()
      return
    }
    pending.current = setTimeout(go, wait)
  }, [])

  useEffect(() => {
    whereRef.current = where
    load(true)
    return () => {
      if (pending.current !== null) clearTimeout(pending.current)
    }
  }, [where, load])

  useEffect(() => {
    const onFocus = (): void => {
      load(false)
    }
    window.addEventListener('focus', onFocus)
    return () => {
      window.removeEventListener('focus', onFocus)
    }
  }, [load])

  useEffect(() => {
    return window.api.onChangesUpdate(() => {
      load(false)
    })
  }, [load])

  const tree = useMemo(() => buildTree(list?.entries ?? []), [list])
  const rows = useMemo(
    () => flattenVisible(tree, expanded, { filter, changedOnly }),
    [tree, expanded, filter, changedOnly]
  )
  const selected =
    selectedPath === null ? null : (list?.entries.find((row) => row.path === selectedPath) ?? null)
  const shownContent = content !== null && content.path === selectedPath ? content.value : null
  const shownDiff = diff !== null && diff.path === selectedPath ? diff.value : null

  useEffect(() => {
    if (selectedPath === null) return
    const status = list?.entries.find((row) => row.path === selectedPath)?.status
    if (status === undefined) return
    const path = selectedPath
    let active = true
    void window.api.readFile(where, path).then(
      (next) => {
        if (active) setContent({ path, value: next })
      },
      () => {
        if (active) setContent({ path, value: { kind: 'missing' } })
      }
    )
    if (status !== 'clean') {
      void window.api.fileDiff(where, path).then(
        (next) => {
          if (active) setDiff({ path, value: next })
        },
        () => {
          if (active) setDiff({ path, value: '' })
        }
      )
    }
    return () => {
      active = false
    }
  }, [where, selectedPath, list])

  const narrowed = filter.trim() !== '' || changedOnly
  const shownFiles = rows.filter((row) => row.kind === 'file').length
  const totalFiles = list?.entries.length ?? 0
  const count = filesCountLabel(shownFiles, totalFiles, narrowed)
  const focusedIndex = Math.max(
    0,
    rows.findIndex((row) => row.path === (focusedPath ?? selectedPath))
  )
  const range = windowRange(rows.length, scrollTop, viewH, FILE_ROW_HEIGHT)
  const slice = rows.slice(range.start, range.end)

  useEffect(() => {
    const el = treeRef.current
    if (el === null) return
    const top = focusedIndex * FILE_ROW_HEIGHT
    const bottom = top + FILE_ROW_HEIGHT
    if (top < el.scrollTop) el.scrollTop = top
    else if (bottom > el.scrollTop + el.clientHeight) el.scrollTop = bottom - el.clientHeight
  }, [focusedIndex])

  function rememberExpanded(next: Set<string>): void {
    expandedByWhere.set(key, next)
    setExpanded(next)
  }

  function toggleFolder(path: string): void {
    const next = new Set(expanded)
    if (next.has(path)) next.delete(path)
    else next.add(path)
    rememberExpanded(next)
  }

  function selectFile(path: string): void {
    setSelectedPath(path)
    selectedByWhere.set(key, path)
    setFocusedPath(path)
    setPreview('file')
    setOpenError(null)
  }

  function focusRow(row: VisibleRow): void {
    setFocusedPath(row.path)
    if (row.kind === 'file') {
      setSelectedPath(row.path)
      selectedByWhere.set(key, row.path)
      setOpenError(null)
    }
  }

  async function openSelected(): Promise<void> {
    if (selected === null || opening) return
    setOpening(true)
    setOpenError(null)
    try {
      await window.api.openFile(where, selected.path)
    } catch (err: unknown) {
      setOpenError(errorText(err))
    } finally {
      setOpening(false)
    }
  }

  function onTreeKeyDown(event: KeyboardEvent<HTMLDivElement>): void {
    const row = rows[focusedIndex]
    if (row === undefined) return
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault()
      const nextIndex = event.key === 'ArrowDown' ? focusedIndex + 1 : focusedIndex - 1
      const next = rows[nextIndex]
      if (next) focusRow(next)
      return
    }
    if (event.key === 'ArrowRight') {
      event.preventDefault()
      if (row.kind === 'dir') {
        if (!expanded.has(row.path) && filter.trim() === '' && !changedOnly) {
          toggleFolder(row.path)
        } else {
          const child = rows[focusedIndex + 1]
          if (child && child.depth > row.depth) focusRow(child)
        }
      }
      return
    }
    if (event.key === 'ArrowLeft') {
      event.preventDefault()
      const filtering = filter.trim() !== '' || changedOnly
      if (row.kind === 'dir' && expanded.has(row.path) && !filtering) {
        toggleFolder(row.path)
        return
      }
      const parent = parentPath(row.path)
      if (parent === null) return
      const target = rows.find((item) => item.kind === 'dir' && item.path === parent)
      if (target) focusRow(target)
      return
    }
    if (event.key === 'Enter') {
      event.preventDefault()
      if (row.kind === 'file') void openSelected()
      else toggleFolder(row.path)
    }
  }

  function onTreeScroll(event: UIEvent<HTMLDivElement>): void {
    const el = event.currentTarget
    setScrollTop(el.scrollTop)
    setViewH(el.clientHeight)
  }

  function onSplitDown(event: ReactPointerEvent<HTMLDivElement>): void {
    event.preventDefault()
    const body = bodyRef.current
    if (body === null) return
    const pointer = event.pointerId
    const target = event.currentTarget
    target.setPointerCapture(pointer)
    const onMove = (move: PointerEvent): void => {
      const rect = body.getBoundingClientRect()
      if (rect.height <= 0) return
      const next = Math.min(0.82, Math.max(0.18, (move.clientY - rect.top) / rect.height))
      setSplit(next)
      splitByWhere.set(key, next)
    }
    const onUp = (): void => {
      target.releasePointerCapture(pointer)
      target.removeEventListener('pointermove', onMove)
      target.removeEventListener('pointerup', onUp)
    }
    target.addEventListener('pointermove', onMove)
    target.addEventListener('pointerup', onUp)
  }

  useEffect(() => {
    const el = treeRef.current
    if (el !== null) setViewH(el.clientHeight)
  }, [list, split])

  const showLoading = loading && list === null
  const dirty = selected !== null && selected.status !== 'clean'
  const mode = dirty && preview === 'changes' ? 'changes' : 'file'
  const bytes = shownContent !== null && shownContent.kind !== 'missing' ? shownContent.bytes : null

  return (
    <div className="files-panel">
      <div className="files-head">
        <span className="files-label">{label}</span>
        <span className="files-count">{list === null ? '' : count}</span>
        <button type="button" className="btn btn-quiet btn-compact" onClick={() => load(false)}>
          Refresh
        </button>
      </div>
      <div className="files-tools">
        <input
          type="search"
          className="files-filter"
          placeholder="Filter files"
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
          aria-label="Filter files"
        />
        <button
          type="button"
          className="btn btn-quiet btn-compact"
          aria-pressed={changedOnly}
          onClick={() => setChangedOnly((on) => !on)}
        >
          Changed
        </button>
      </div>
      {showLoading ? <p className="files-status">Loading files…</p> : null}
      {error !== null ? (
        <div className="files-error">
          <p className="error-text" role="alert">
            {error}
          </p>
          <button type="button" className="btn btn-quiet btn-compact" onClick={() => load(true)}>
            Retry
          </button>
        </div>
      ) : null}
      {list?.truncated === true ? <p className="files-status">List cut at 20 000 files.</p> : null}
      <div ref={bodyRef} className="files-body">
        <div
          ref={treeRef}
          className="files-tree"
          style={{ flex: `${String(split)} 1 0` }}
          role="tree"
          aria-label="Files"
          aria-busy={loading}
          tabIndex={0}
          onKeyDown={onTreeKeyDown}
          onScroll={onTreeScroll}
        >
          <div className="files-tree-inner" style={{ height: rows.length * FILE_ROW_HEIGHT }}>
            <div
              className="files-tree-window"
              style={{ transform: `translateY(${String(range.start * FILE_ROW_HEIGHT)}px)` }}
            >
              {slice.map((row) => {
                const focused = row.path === (focusedPath ?? selectedPath)
                const chosen = row.kind === 'file' && row.path === selectedPath
                const open =
                  row.kind === 'dir' &&
                  (expanded.has(row.path) || filter.trim() !== '' || changedOnly)
                return (
                  <div
                    key={`${row.kind}:${row.path}`}
                    id={rowId(row)}
                    role="treeitem"
                    aria-level={row.depth + 1}
                    aria-selected={chosen}
                    aria-expanded={row.kind === 'dir' ? open : undefined}
                    tabIndex={-1}
                    className={
                      'files-row' +
                      (chosen ? ' is-selected' : '') +
                      (focused ? ' is-focused' : '') +
                      (row.status === 'deleted' ? ' is-deleted' : '')
                    }
                    style={{ paddingLeft: 8 + row.depth * 12, height: FILE_ROW_HEIGHT }}
                    onClick={() => {
                      setFocusedPath(row.path)
                      if (row.kind === 'dir') toggleFolder(row.path)
                      else selectFile(row.path)
                    }}
                  >
                    {row.kind === 'dir' ? (
                      <span className={open ? 'files-chevron is-open' : 'files-chevron'} />
                    ) : (
                      <span className="files-chevron-gap" />
                    )}
                    <span className="files-name">{row.name}</span>
                    {row.kind === 'dir' && row.hasChanged ? <span className="files-dot" /> : null}
                    {row.kind === 'file' && row.status !== 'clean' ? (
                      <span className={markClass(row.status)}>{STATUS_MARK[row.status]}</span>
                    ) : null}
                  </div>
                )
              })}
            </div>
          </div>
        </div>
        <div
          className="files-split"
          role="separator"
          aria-orientation="horizontal"
          aria-label="Resize preview"
          onPointerDown={onSplitDown}
        />
        <div className="files-preview" style={{ flex: `${String(1 - split)} 1 0` }}>
          {selected === null ? (
            <p className="files-status">Select a file.</p>
          ) : (
            <>
              <div className="files-preview-head">
                <span className="files-preview-path" title={selected.path}>
                  {selected.path}
                </span>
                {bytes !== null ? (
                  <span className="files-preview-size">{formatBytes(bytes)}</span>
                ) : null}
                <div className="files-preview-actions">
                  <button
                    type="button"
                    className="btn btn-quiet btn-compact"
                    aria-pressed={mode === 'file'}
                    onClick={() => setPreview('file')}
                  >
                    File
                  </button>
                  {dirty ? (
                    <button
                      type="button"
                      className="btn btn-quiet btn-compact"
                      aria-pressed={mode === 'changes'}
                      onClick={() => setPreview('changes')}
                    >
                      Changes
                    </button>
                  ) : null}
                  <button
                    type="button"
                    className="btn btn-quiet btn-compact"
                    disabled={opening}
                    onClick={() => {
                      void openSelected()
                    }}
                  >
                    Open
                  </button>
                </div>
              </div>
              {openError !== null ? (
                <p className="error-text" role="alert">
                  {openError}
                </p>
              ) : null}
              <div className="files-preview-body">
                {mode === 'changes' ? (
                  <DiffBlock text={shownDiff} />
                ) : (
                  <FileBody content={shownContent} />
                )}
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  )
}

function FileBody({ content }: { content: FileContent | null }): React.JSX.Element {
  if (content === null) return <p className="files-status">Loading…</p>
  if (content.kind === 'missing') return <p className="files-status">This file is missing.</p>
  if (content.kind === 'binary') return <p className="files-status">This file is binary.</p>
  if (content.kind === 'too-large')
    return <p className="files-status">This file is too large to show.</p>
  const lines = content.text.split(/\r?\n/)
  const shown = lines.slice(0, FILE_LINE_CAP)
  const hidden = lines.length - shown.length
  return (
    <div className="files-text">
      {shown.map((line, index) => (
        <div key={index} className="files-text-line">
          <span className="files-text-n">{index + 1}</span>
          <span className="files-text-src">{line.length === 0 ? ' ' : line}</span>
        </div>
      ))}
      {hidden > 0 ? (
        <p className="files-status">{`First ${String(FILE_LINE_CAP)} lines shown. ${String(hidden)} more.`}</p>
      ) : null}
    </div>
  )
}

export function DiffBlock({ text }: { text: string | null }): React.JSX.Element {
  if (text === null) return <p className="files-status">Loading…</p>
  if (text === '') return <p className="files-status">No changes.</p>
  const lines = parseDiff(text)
  const shown = lines.slice(0, DIFF_LINE_CAP)
  const hidden = lines.length - shown.length
  return (
    <div className="diff-block">
      {shown.map((line, index) => (
        <div key={index} className={`diff-line is-${line.kind}`}>
          {line.text.length === 0 ? ' ' : line.text}
        </div>
      ))}
      {hidden > 0 ? (
        <div className="diff-line is-meta">{`… ${String(hidden)} more lines not shown`}</div>
      ) : null}
    </div>
  )
}

export default FilesPanel
