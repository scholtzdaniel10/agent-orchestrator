import { useEffect, useRef, useState } from 'react'
import { DiffBlock } from './FilesPanel'
import { worktreeLabel } from './worktree-label'

type ChangeSet = Awaited<ReturnType<Window['api']['listChanges']>>[number]
type ChangedFile = ChangeSet['files'][number]

function errorText(err: unknown): string {
  if (err instanceof Error) {
    return err.message.replace(/^Error invoking remote method '[^']+': (Error: )?/, '')
  }
  if (typeof err === 'string') return err
  return 'Request failed'
}

function diffTarget(path: string): string {
  const marker = ' => '
  const at = path.lastIndexOf(marker)
  if (at === -1) return path
  return path.slice(at + marker.length)
}

function headerMatches(text: string, path: string): boolean {
  const target = diffTarget(path)
  return text.endsWith(`b/${target}`) || text.endsWith(`b/${target}"`)
}

function FileStat({ file }: { file: ChangedFile }): React.JSX.Element {
  if (file.insertions === null || file.deletions === null) {
    return <span className="diff-binary">binary</span>
  }
  return (
    <span className="change-stats">
      <span className="stat-add">{`+${String(file.insertions)}`}</span>{' '}
      <span className="stat-del">{`−${String(file.deletions)}`}</span>
    </span>
  )
}

function ReviewPane({
  change,
  onClose,
  onDone
}: {
  change: ChangeSet
  onClose: () => void
  onDone: () => void
}): React.JSX.Element {
  const [payload, setPayload] = useState<{
    id: string
    text: string | null
    error: string | null
  } | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)
  const [mergeMessage, setMergeMessage] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [confirming, setConfirming] = useState(false)
  const busyRef = useRef(false)
  const diffRef = useRef<HTMLDivElement>(null)
  const shown = payload !== null && payload.id === change.id ? payload : null

  useEffect(() => {
    let active = true
    const id = change.id
    void window.api.changeDiff(id).then(
      (next) => {
        if (!active) return
        setPayload({ id, text: next, error: null })
      },
      (err: unknown) => {
        if (!active) return
        setPayload({ id, text: null, error: errorText(err) })
      }
    )
    return () => {
      active = false
    }
  }, [change.id])

  function scrollToFile(path: string): void {
    const root = diffRef.current
    if (root === null) return
    const lines = root.querySelectorAll('.diff-line.is-file')
    for (let index = 0; index < lines.length; index += 1) {
      const line = lines.item(index)
      if (line === null) continue
      if (!headerMatches(line.textContent ?? '', path)) continue
      line.scrollIntoView({ block: 'start' })
      return
    }
  }

  async function onMerge(): Promise<void> {
    if (busyRef.current) return
    busyRef.current = true
    setBusy(true)
    setActionError(null)
    setMergeMessage(null)
    try {
      const result = await window.api.mergeChange(change.id)
      const message =
        result.message === '' ? 'Could not merge this worktree. Try again.' : result.message
      if (!result.ok) {
        setActionError(message)
        return
      }
      setMergeMessage(message)
      onDone()
    } catch (err: unknown) {
      setActionError(errorText(err))
    } finally {
      busyRef.current = false
      setBusy(false)
    }
  }

  async function onDiscard(): Promise<void> {
    if (busyRef.current) return
    busyRef.current = true
    setBusy(true)
    setActionError(null)
    try {
      await window.api.discardChange(change.id)
      onDone()
    } catch (err: unknown) {
      setActionError(errorText(err))
    } finally {
      busyRef.current = false
      setBusy(false)
    }
  }

  return (
    <div className="review-pane" role="region" aria-label="Review">
      <header className="review-head">
        <div className="review-summary">
          <div className="review-title">
            <span className="change-branch">{worktreeLabel(change)}</span>
            <span className="review-branch">{change.branch}</span>
            <span className="change-stats">
              <span className="stat-add">{`+${String(change.insertions)}`}</span>{' '}
              <span className="stat-del">{`−${String(change.deletions)}`}</span>
            </span>
          </div>
          <ul className="change-files">
            {change.files.map((file) => (
              <li key={file.path} className="change-file">
                <button
                  type="button"
                  className="review-file"
                  onClick={() => scrollToFile(file.path)}
                >
                  <span className="change-path">{file.path}</span>
                  <FileStat file={file} />
                </button>
              </li>
            ))}
          </ul>
        </div>
        <div className="review-actions">
          <button
            type="button"
            className="btn btn-primary"
            disabled={busy}
            onClick={() => {
              void onMerge()
            }}
          >
            Merge
          </button>
          {confirming ? (
            <div className="review-confirm">
              <span className="review-confirm-copy">Discard this worktree and its changes?</span>
              <button
                type="button"
                className="btn btn-quiet is-danger-label"
                disabled={busy}
                onClick={() => {
                  void onDiscard()
                }}
              >
                Yes
              </button>
              <button
                type="button"
                className="btn btn-quiet"
                disabled={busy}
                onClick={() => setConfirming(false)}
              >
                No
              </button>
            </div>
          ) : (
            <button
              type="button"
              className="btn btn-quiet"
              disabled={busy}
              onClick={() => setConfirming(true)}
            >
              Discard
            </button>
          )}
          <button type="button" className="btn btn-quiet" disabled={busy} onClick={onClose}>
            Back to terminals
          </button>
        </div>
      </header>
      {mergeMessage !== null ? (
        <p className="change-ok" role="status">
          {mergeMessage}
        </p>
      ) : null}
      {actionError !== null ? (
        <p className="error-text" role="alert">
          {actionError}
        </p>
      ) : null}
      <div className="review-diff" ref={diffRef} aria-busy={shown === null}>
        {shown === null ? <p className="diff-status">Loading diff…</p> : null}
        {shown !== null && shown.error !== null ? (
          <p className="error-text" role="alert">
            {shown.error}
          </p>
        ) : null}
        {shown !== null && shown.error === null ? <DiffBlock text={shown.text} /> : null}
      </div>
    </div>
  )
}

export default ReviewPane
