import { useEffect, useRef, useState, type KeyboardEvent } from 'react'
import { RayBurst } from './Star'
import { parseDiff } from './diff-lines'

type ChangeSet = Awaited<ReturnType<Window['api']['listChanges']>>[number]
type ChangedFile = ChangeSet['files'][number]
type JobRecord = Awaited<ReturnType<Window['api']['listJobs']>>[number]

const DIFF_LINE_CAP = 5000

function errorText(err: unknown): string {
  if (err instanceof Error) {
    // Electron prefixes errors from the main process; show only the message itself.
    return err.message.replace(/^Error invoking remote method '[^']+': (Error: )?/, '')
  }
  if (typeof err === 'string') return err
  return 'Request failed'
}

function jobTypeFor(jobs: readonly JobRecord[], id: string): string {
  const match = jobs.find((job) => job.change === id)
  return match === undefined ? 'earlier session' : match.type
}

function fileLabel(count: number): string {
  return count === 1 ? '1 file' : `${count} files`
}

function ChangeStats({
  files,
  insertions,
  deletions
}: {
  files: number
  insertions: number
  deletions: number
}): React.JSX.Element {
  return (
    <span className="change-stats">
      {`${fileLabel(files)} · `}
      <span className="stat-add">{`+${insertions}`}</span>{' '}
      <span className="stat-del">{`−${deletions}`}</span>
    </span>
  )
}

function FileStat({ file }: { file: ChangedFile }): React.JSX.Element {
  if (file.insertions === null || file.deletions === null) {
    return <span className="diff-binary">binary</span>
  }
  return (
    <span className="change-stats">
      <span className="stat-add">{`+${file.insertions}`}</span>{' '}
      <span className="stat-del">{`−${file.deletions}`}</span>
    </span>
  )
}

function DiffPane({ id }: { id: string }): React.JSX.Element {
  const [text, setText] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    let active = true
    void window.api.changeDiff(id).then(
      (next) => {
        if (!active) return
        setText(next)
        setLoading(false)
      },
      (err: unknown) => {
        if (!active) return
        setError(errorText(err))
        setLoading(false)
      }
    )
    return () => {
      active = false
    }
  }, [id])

  return (
    <div className="diff-block" role="region" aria-label="Diff" aria-busy={loading}>
      {loading ? <p className="diff-status">Loading diff…</p> : null}
      {!loading && error !== null ? (
        <p className="field-error" role="alert">
          {error}
        </p>
      ) : null}
      {!loading && error === null && text !== null ? <DiffBlock text={text} /> : null}
    </div>
  )
}

function DiffBlock({ text }: { text: string }): React.JSX.Element {
  const lines = parseDiff(text)
  const shown = lines.slice(0, DIFF_LINE_CAP)
  const hidden = lines.length - shown.length
  return (
    <>
      {shown.map((line, index) => (
        <div key={index} className={`diff-line is-${line.kind}`}>
          {line.text.length === 0 ? ' ' : line.text}
        </div>
      ))}
      {hidden > 0 ? (
        <div className="diff-line is-meta">{`… ${hidden} more lines not shown`}</div>
      ) : null}
    </>
  )
}

function ChangesTab({
  changes,
  jobs,
  selectedId,
  onSelect
}: {
  changes: ChangeSet[]
  jobs: JobRecord[]
  selectedId: string | null
  onSelect: (id: string | null) => void
}): React.JSX.Element {
  const [listSeen, setListSeen] = useState(changes)
  const [listTick, setListTick] = useState(0)
  const [busy, setBusy] = useState(false)
  const [armedId, setArmedId] = useState<string | null>(null)
  const [notice, setNotice] = useState<{
    id: string
    tone: 'ok' | 'bad'
    text: string
  } | null>(null)
  const busyRef = useRef(false)

  const resolvedId =
    selectedId !== null && changes.some((item) => item.id === selectedId)
      ? selectedId
      : (changes[0]?.id ?? null)
  const selected = changes.find((item) => item.id === resolvedId) ?? null
  if (listSeen !== changes) {
    setListSeen(changes)
    setListTick((tick) => tick + 1)
  }

  useEffect(() => {
    const present = selectedId !== null && changes.some((item) => item.id === selectedId)
    if (present) return
    onSelect(changes[0]?.id ?? null)
  }, [changes, selectedId, onSelect])

  useEffect(() => {
    if (armedId === null) return
    const timer = window.setTimeout(() => setArmedId(null), 4000)
    return () => window.clearTimeout(timer)
  }, [armedId])

  useEffect(() => {
    if (notice === null || notice.tone !== 'ok') return
    const timer = window.setTimeout(() => setNotice(null), 5000)
    return () => window.clearTimeout(timer)
  }, [notice])

  function onRowKeyDown(event: KeyboardEvent<HTMLDivElement>, index: number): void {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault()
      const item = changes[index]
      if (item) onSelect(item.id)
      return
    }
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
    event.preventDefault()
    const nextIndex = event.key === 'ArrowDown' ? index + 1 : index - 1
    const next = changes[nextIndex]
    if (!next) return
    onSelect(next.id)
    document.getElementById(`change-${next.id}`)?.focus()
  }

  async function run(id: string, action: () => Promise<void>): Promise<void> {
    if (busyRef.current) return
    busyRef.current = true
    setBusy(true)
    setNotice(null)
    try {
      await action()
    } catch (err: unknown) {
      setNotice({ id, tone: 'bad', text: errorText(err) })
    } finally {
      busyRef.current = false
      setBusy(false)
    }
  }

  function onMerge(): void {
    if (selected === null) return
    const id = selected.id
    setArmedId(null)
    void run(id, async () => {
      const result = await window.api.mergeChange(id)
      setNotice({ id, tone: result.ok ? 'ok' : 'bad', text: result.message })
    })
  }

  function onDiscard(): void {
    if (selected === null || busyRef.current) return
    if (armedId !== selected.id) {
      setArmedId(selected.id)
      return
    }
    const id = selected.id
    setArmedId(null)
    void run(id, async () => {
      await window.api.discardChange(id)
    })
  }

  const confirming = selected !== null && armedId === selected.id
  const okNotice = notice !== null && notice.tone === 'ok' ? notice.text : null
  const badNotice =
    notice !== null && notice.tone === 'bad' && selected !== null && notice.id === selected.id
      ? notice.text
      : null

  return (
    <div className="changes-pane">
      {okNotice !== null ? (
        <p className="change-ok" role="status">
          {okNotice}
        </p>
      ) : null}
      {changes.length === 0 || selected === null ? (
        <div className="changes-empty">
          <RayBurst />
          <p className="hint">
            {'No changes waiting. Tick "Let it edit files" on a job to get one.'}
          </p>
        </div>
      ) : (
        <div className="changes">
          <div role="listbox" aria-label="Changes" className="change-list">
            {changes.map((item, index) => {
              const isSelected = item.id === resolvedId
              return (
                <div
                  key={item.id}
                  id={`change-${item.id}`}
                  role="option"
                  aria-selected={isSelected}
                  tabIndex={isSelected ? 0 : -1}
                  className={isSelected ? 'job-row is-selected' : 'job-row'}
                  onClick={() => onSelect(item.id)}
                  onKeyDown={(event) => onRowKeyDown(event, index)}
                >
                  <span className="change-id">{item.id}</span>
                  <span className="change-origin">{jobTypeFor(jobs, item.id)}</span>
                  <ChangeStats
                    files={item.files.length}
                    insertions={item.insertions}
                    deletions={item.deletions}
                  />
                </div>
              )
            })}
          </div>
          {selected === null ? null : (
            <div className="change-detail">
              <div className="change-head">
                <span className="change-branch">{selected.branch}</span>
                <ChangeStats
                  files={selected.files.length}
                  insertions={selected.insertions}
                  deletions={selected.deletions}
                />
              </div>
              <div className="change-actions">
                <button type="button" className="btn btn-primary" disabled={busy} onClick={onMerge}>
                  Merge
                </button>
                <button
                  type="button"
                  className={confirming ? 'btn btn-quiet is-danger-label' : 'btn btn-quiet'}
                  disabled={busy}
                  onClick={onDiscard}
                >
                  {confirming ? 'Discard for good?' : 'Discard'}
                </button>
              </div>
              <p className="change-help">
                Merge applies this to your project as staged changes. Nothing is committed.
              </p>
              {badNotice !== null ? (
                <p className="field-error" role="alert">
                  {badNotice}
                </p>
              ) : null}
              <ul className="change-files">
                {selected.files.map((file) => (
                  <li key={file.path} className="change-file">
                    <span className="change-path">{file.path}</span>
                    <FileStat file={file} />
                  </li>
                ))}
              </ul>
              <DiffPane key={`${selected.id}:${listTick}`} id={selected.id} />
            </div>
          )}
        </div>
      )}
    </div>
  )
}

export default ChangesTab
