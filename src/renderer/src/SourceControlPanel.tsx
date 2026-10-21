import { useEffect, useMemo, useState } from 'react'
import { graphRows } from './commit-graph'
import { Graph } from './HistoryPanel'
import { ago, refBadges } from './history-format'

type BranchStatus = Awaited<ReturnType<Window['api']['branchStatus']>>
type CommitPage = Awaited<ReturnType<Window['api']['commitLog']>>

const GRAPH_ROWS = 50

function errorText(err: unknown): string {
  if (!(err instanceof Error)) return String(err)
  return err.message.replace(/^Error invoking remote method '[^']+': (Error: )?/, '')
}

function fileName(path: string): string {
  return path.slice(path.lastIndexOf('/') + 1)
}

function folderOf(path: string): string {
  const cut = path.lastIndexOf('/')
  return cut < 0 ? '' : path.slice(0, cut)
}

/**
 * The selected folder's branch, as VS Code's Source Control shows it: where it stands against its
 * upstream and base, a message box that commits everything, publish or push, a pull request, the
 * files the branch changes, and the commit graph under them.
 */
function SourceControlPanel({
  where,
  refreshKey = '',
  onOpenFile
}: {
  where: string | null
  /** Changes when work lands, so the status reads again. */
  refreshKey?: string
  /** Opens a file of the folder in the editor. */
  onOpenFile?: (path: string) => void
}): React.JSX.Element {
  const [nonce, setNonce] = useState(0)
  const key = `${where ?? ''}|${String(nonce)}|${refreshKey}`
  const [status, setStatus] = useState<{ key: string; value: BranchStatus } | null>(null)
  const [log, setLog] = useState<{ key: string; page: CommitPage; at: number } | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [message, setMessage] = useState('')
  const [busy, setBusy] = useState<'commit' | 'push' | 'pr' | null>(null)
  const [filesOpen, setFilesOpen] = useState(true)
  const [graphOpen, setGraphOpen] = useState(true)

  useEffect(() => {
    let on = true
    window.api.branchStatus(where).then(
      (value) => {
        if (on) setStatus({ key, value })
      },
      (err: unknown) => {
        if (on) setError(errorText(err))
      }
    )
    window.api.commitLog(where, 0).then(
      (page) => {
        if (on) setLog({ key, page, at: Date.now() })
      },
      () => {}
    )
    return () => {
      on = false
    }
  }, [where, key])

  useEffect(() => {
    const onFocus = (): void => {
      setNonce((n) => n + 1)
    }
    window.addEventListener('focus', onFocus)
    return () => {
      window.removeEventListener('focus', onFocus)
    }
  }, [])

  const branch = status?.key === key ? status.value : (status?.value ?? null)
  const commits = useMemo(() => (log?.page.commits ?? []).slice(0, GRAPH_ROWS), [log])
  const rows = useMemo(() => graphRows(commits), [commits])
  const width = rows.reduce((max, row) => Math.max(max, row.width), 1)
  const now = log?.at ?? 0

  function act(kind: 'commit' | 'push' | 'pr', task: () => Promise<unknown>): void {
    setBusy(kind)
    setError(null)
    task()
      .then(() => {
        if (kind === 'commit') setMessage('')
      })
      .catch((err: unknown) => {
        setError(errorText(err))
      })
      .finally(() => {
        setBusy(null)
        setNonce((n) => n + 1)
      })
  }

  const dirty = (branch?.uncommitted.length ?? 0) > 0
  const published = branch?.upstream !== null && branch !== null
  const primary = dirty
    ? { label: 'Commit', kind: 'commit' as const, disabled: message.trim() === '' }
    : published
      ? {
          label: branch.ahead > 0 ? `Push ${String(branch.ahead)}` : 'Up to date',
          kind: 'push' as const,
          disabled: branch.ahead === 0
        }
      : { label: 'Publish branch', kind: 'push' as const, disabled: branch === null }

  return (
    <div className="sc-panel">
      <div className="sc-head">
        <button
          type="button"
          className="btn btn-quiet btn-compact"
          disabled={busy !== null || !published || branch.base === null || branch.commits === 0}
          title="Open a pull request from this branch into its base"
          onClick={() => {
            act('pr', () => window.api.createBranchPr(where))
          }}
        >
          {busy === 'pr' ? 'Opening…' : 'Create PR'}
        </button>
        <button
          type="button"
          className="btn btn-quiet btn-compact"
          onClick={() => {
            setNonce((n) => n + 1)
          }}
        >
          Refresh
        </button>
      </div>
      {branch !== null ? (
        <div className="sc-branch">
          <span className="sc-branch-name mono">{branch.branch}</span>
          <span className="sc-branch-totals">
            <span className="diff-add">{`+${String(branch.insertions)}`}</span>
            <span className="diff-del">{`−${String(branch.deletions)}`}</span>
          </span>
          <span className="sc-branch-base mono">
            {branch.base === null ? 'no remote' : `→ ${branch.base}`}
          </span>
          <span className="sc-branch-sync">
            {branch.upstream === null
              ? 'not published'
              : `↑${String(branch.ahead)} ↓${String(branch.behind)}`}
          </span>
        </div>
      ) : null}
      <textarea
        className="sc-message"
        placeholder={dirty ? 'Message' : 'Nothing to commit'}
        aria-label="Commit message"
        rows={3}
        value={message}
        disabled={!dirty || busy !== null}
        onChange={(event) => {
          setMessage(event.target.value)
        }}
        onKeyDown={(event) => {
          if (event.key === 'Enter' && event.ctrlKey && !primary.disabled && busy === null) {
            event.preventDefault()
            act('commit', () => window.api.commitAll(where, message))
          }
        }}
      />
      <button
        type="button"
        className="btn sc-primary"
        disabled={primary.disabled || busy !== null}
        onClick={() => {
          act(primary.kind, () =>
            primary.kind === 'commit'
              ? window.api.commitAll(where, message)
              : window.api.pushBranch(where)
          )
        }}
      >
        {busy === 'commit' ? 'Committing…' : busy === 'push' ? 'Pushing…' : primary.label}
      </button>
      {error !== null ? (
        <p className="error-text" role="alert">
          {error}
        </p>
      ) : null}
      {dirty && branch !== null ? (
        <section className="sc-section" aria-label="Changes">
          <h3 className="sc-section-head">
            <span>Changes</span>
            <span className="tab-badge">{branch.uncommitted.length}</span>
          </h3>
          {branch.uncommitted.map((file) => (
            <button
              key={file.path}
              type="button"
              className="sc-file"
              onClick={() => onOpenFile?.(file.path)}
            >
              <span className="sc-file-name">{fileName(file.path)}</span>
              <span className="sc-file-dir">{folderOf(file.path)}</span>
              <span className="sc-file-mark mono">{file.status}</span>
            </button>
          ))}
        </section>
      ) : null}
      {branch !== null && branch.base !== null ? (
        <section className="sc-section" aria-label="Committed on branch">
          <button
            type="button"
            className="sc-section-head"
            aria-expanded={filesOpen}
            onClick={() => {
              setFilesOpen((open) => !open)
            }}
          >
            <span>Committed on branch</span>
            <span className="tab-badge">{branch.commits}</span>
          </button>
          {filesOpen
            ? branch.files.map((file) => (
                <button
                  key={file.path}
                  type="button"
                  className="sc-file"
                  onClick={() => onOpenFile?.(file.path)}
                >
                  <span className="sc-file-name">{fileName(file.path)}</span>
                  <span className="sc-file-dir">{folderOf(file.path)}</span>
                  <span className="sc-file-stat">
                    {file.added === null ? (
                      'binary'
                    ) : (
                      <>
                        <span className="diff-add">{`+${String(file.added)}`}</span>{' '}
                        <span className="diff-del">{`−${String(file.removed ?? 0)}`}</span>
                      </>
                    )}
                  </span>
                </button>
              ))
            : null}
          {filesOpen && branch.files.length === 0 ? (
            <p className="files-status">{`Nothing committed since ${branch.base}.`}</p>
          ) : null}
        </section>
      ) : null}
      <section className="sc-section sc-graph" aria-label="Commits">
        <button
          type="button"
          className="sc-section-head"
          aria-expanded={graphOpen}
          onClick={() => {
            setGraphOpen((open) => !open)
          }}
        >
          <span>Commits</span>
          <span className="tab-badge">{`${String(commits.length)}${log?.page.more === true || (log?.page.commits.length ?? 0) > GRAPH_ROWS ? '+' : ''}`}</span>
        </button>
        {graphOpen ? (
          <ol className="history-list">
            {commits.map((commit, index) => {
              const row = rows[index]
              const isHead = commit.refs.some((ref) => ref === 'HEAD' || ref.startsWith('HEAD -> '))
              return (
                <li key={commit.hash}>
                  <div
                    className="history-row"
                    title={`${commit.subject}\n${commit.author} · ${new Date(commit.time).toLocaleString()}`}
                  >
                    {row !== undefined ? <Graph row={row} width={width} head={isHead} /> : null}
                    <span className="history-text">
                      <span className="history-subject">
                        {commit.subject}
                        {refBadges(commit.refs).map((badge) => (
                          <span
                            key={`${badge.kind}-${badge.name}`}
                            className={`history-ref is-${badge.kind}`}
                          >
                            {badge.name}
                          </span>
                        ))}
                      </span>
                    </span>
                    <span className="history-meta">
                      <span className="history-age">{ago(commit.time, now)}</span>
                    </span>
                  </div>
                </li>
              )
            })}
          </ol>
        ) : null}
      </section>
    </div>
  )
}

export default SourceControlPanel
