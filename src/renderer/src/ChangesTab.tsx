import { useEffect, useRef, useState, type KeyboardEvent } from 'react'
import { RayBurst } from './Star'
import { parseDiff } from './diff-lines'

type ChangeSet = Awaited<ReturnType<Window['api']['listChanges']>>[number]
type ChangedFile = ChangeSet['files'][number]
type JobRecord = Awaited<ReturnType<Window['api']['listJobs']>>[number]
type ProviderId = Parameters<Window['api']['openTerminal']>[0]

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

function titleFromPrompt(prompt: string): string {
  const line = prompt.split(/\r?\n/, 1)[0] ?? ''
  return line.trim().slice(0, 72)
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
  onSelect,
  claudeAvailable,
  cursorAvailable,
  opening,
  onOpenHere
}: {
  changes: ChangeSet[]
  jobs: JobRecord[]
  selectedId: string | null
  onSelect: (id: string | null) => void
  claudeAvailable: boolean
  cursorAvailable: boolean
  opening: boolean
  onOpenHere: (provider: ProviderId, changeId: string) => void
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
  const [githubOn, setGithubOn] = useState(false)
  const [mergedOk, setMergedOk] = useState(false)
  const [prForm, setPrForm] = useState(false)
  const [prTitle, setPrTitle] = useState('')
  const [prBody, setPrBody] = useState('')
  const [prOpening, setPrOpening] = useState(false)
  const [prUrl, setPrUrl] = useState<string | null>(null)
  const [prError, setPrError] = useState<string | null>(null)
  const busyRef = useRef(false)
  const titleRef = useRef<HTMLInputElement>(null)

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
    if (githubOn && mergedOk) return
    const timer = window.setTimeout(() => setNotice(null), 5000)
    return () => window.clearTimeout(timer)
  }, [notice, githubOn, mergedOk])

  useEffect(() => {
    let active = true
    void window.api.githubAvailable().then(
      (ok) => {
        if (active) setGithubOn(ok)
      },
      () => {
        if (active) setGithubOn(false)
      }
    )
    return () => {
      active = false
    }
  }, [])

  useEffect(() => {
    if (!prForm) return
    titleRef.current?.focus()
  }, [prForm])

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
    const prompt = jobs.find((job) => job.change === id)?.prompt ?? ''
    setArmedId(null)
    void run(id, async () => {
      const result = await window.api.mergeChange(id)
      setNotice({ id, tone: result.ok ? 'ok' : 'bad', text: result.message })
      if (result.ok) {
        setMergedOk(true)
        setPrForm(false)
        setPrUrl(null)
        setPrError(null)
        setPrBody('')
        setPrTitle(titleFromPrompt(prompt))
      }
    })
  }

  function onCancelPr(): void {
    if (prOpening) return
    setPrForm(false)
    setPrError(null)
  }

  async function onOpenPr(): Promise<void> {
    if (prOpening) return
    if (prTitle.length < 1 || prTitle.length > 120) return
    setPrOpening(true)
    setPrError(null)
    try {
      const result = await window.api.githubOpenPr(prTitle, prBody)
      setPrUrl(result.url)
      setPrForm(false)
    } catch (err: unknown) {
      setPrError(errorText(err))
    } finally {
      setPrOpening(false)
    }
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
      {prUrl !== null ? (
        <p className="change-ok" role="status">
          {'Pull request opened '}
          <a href={prUrl} target="_blank" rel="noreferrer">
            {prUrl}
          </a>
        </p>
      ) : null}
      {githubOn && mergedOk && prUrl === null && !prForm ? (
        <div className="change-pr">
          <button
            type="button"
            className="btn btn-primary"
            disabled={busy || prOpening}
            onClick={() => setPrForm(true)}
          >
            Open pull request
          </button>
        </div>
      ) : null}
      {prForm && prUrl === null ? (
        <form
          className="change-pr-form"
          onSubmit={(event) => {
            event.preventDefault()
            void onOpenPr()
          }}
          onKeyDown={(event) => {
            if (event.key === 'Escape') {
              event.preventDefault()
              onCancelPr()
            }
          }}
        >
          <div className="change-pr-field">
            <label htmlFor="pr-title">Title</label>
            <input
              ref={titleRef}
              id="pr-title"
              type="text"
              value={prTitle}
              maxLength={120}
              disabled={prOpening}
              onChange={(event) => setPrTitle(event.target.value)}
            />
          </div>
          <div className="change-pr-field">
            <label htmlFor="pr-body">Description</label>
            <textarea
              id="pr-body"
              value={prBody}
              maxLength={4000}
              rows={4}
              disabled={prOpening}
              onChange={(event) => setPrBody(event.target.value)}
            />
          </div>
          <div className="change-actions">
            <button
              type="submit"
              className="btn btn-primary"
              disabled={prOpening || prTitle.length < 1 || prTitle.length > 120}
            >
              Open pull request
            </button>
            <button
              type="button"
              className="btn btn-quiet"
              disabled={prOpening}
              onClick={onCancelPr}
            >
              Cancel
            </button>
          </div>
          {prOpening ? <p className="change-help">Opening pull request…</p> : null}
          {prError !== null ? (
            <p className="field-error" role="alert">
              {prError}
            </p>
          ) : null}
        </form>
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
                  {item.files.length === 0 ? (
                    <span className="change-stats">No changes yet</span>
                  ) : (
                    <ChangeStats
                      files={item.files.length}
                      insertions={item.insertions}
                      deletions={item.deletions}
                    />
                  )}
                </div>
              )
            })}
          </div>
          {selected === null ? null : (
            <div className="change-detail">
              <div className="change-head">
                <span className="change-branch">{selected.branch}</span>
                {selected.files.length === 0 ? (
                  <span className="change-stats">No changes yet</span>
                ) : (
                  <ChangeStats
                    files={selected.files.length}
                    insertions={selected.insertions}
                    deletions={selected.deletions}
                  />
                )}
              </div>
              <div className="change-actions">
                {selected.files.length > 0 ? (
                  <button
                    type="button"
                    className="btn btn-primary"
                    disabled={busy}
                    onClick={onMerge}
                  >
                    Merge
                  </button>
                ) : null}
                <button
                  type="button"
                  className={confirming ? 'btn btn-quiet is-danger-label' : 'btn btn-quiet'}
                  disabled={busy}
                  onClick={onDiscard}
                >
                  {confirming ? 'Discard for good?' : 'Discard'}
                </button>
                {claudeAvailable ? (
                  <button
                    type="button"
                    className="btn btn-quiet btn-compact"
                    disabled={opening || busy}
                    onClick={() => {
                      onOpenHere('claude', selected.id)
                    }}
                  >
                    Claude here
                  </button>
                ) : null}
                {cursorAvailable ? (
                  <button
                    type="button"
                    className="btn btn-quiet btn-compact"
                    disabled={opening || busy}
                    onClick={() => {
                      onOpenHere('cursor', selected.id)
                    }}
                  >
                    Cursor here
                  </button>
                ) : null}
              </div>
              {selected.files.length > 0 ? (
                <p className="change-help">
                  Merge applies this to your project as staged changes. Nothing is committed.
                </p>
              ) : null}
              {badNotice !== null ? (
                <p className="field-error" role="alert">
                  {badNotice}
                </p>
              ) : null}
              {selected.files.length > 0 ? (
                <>
                  <ul className="change-files">
                    {selected.files.map((file) => (
                      <li key={file.path} className="change-file">
                        <span className="change-path">{file.path}</span>
                        <FileStat file={file} />
                      </li>
                    ))}
                  </ul>
                  <DiffPane key={`${selected.id}:${listTick}`} id={selected.id} />
                </>
              ) : null}
            </div>
          )}
        </div>
      )}
    </div>
  )
}

export default ChangesTab
