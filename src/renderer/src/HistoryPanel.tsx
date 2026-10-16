import { useEffect, useMemo, useState } from 'react'
import { graphRows, type GraphLine, type GraphRow } from './commit-graph'
import { DiffBlock } from './FilesPanel'
import { ago, refBadges } from './history-format'

type CommitPage = Awaited<ReturnType<Window['api']['commitLog']>>
type Commit = CommitPage['commits'][number]
type CommitDetail = Awaited<ReturnType<Window['api']['commitDetail']>>

const ROW = 30
const LANE = 12
const PAD = 8
const LANE_COLORS = [
  'var(--accent)',
  'var(--bot-claude)',
  'var(--bot-cursor)',
  'var(--signal)',
  '#6ba8ff',
  '#c45ad6'
]

function laneColor(lane: number): string {
  return LANE_COLORS[lane % LANE_COLORS.length] ?? 'var(--accent)'
}

function x(lane: number): number {
  return PAD + lane * LANE
}

function errorText(err: unknown): string {
  if (!(err instanceof Error)) return String(err)
  return err.message.replace(/^Error invoking remote method '[^']+': (Error: )?/, '')
}

function curve(line: GraphLine, y1: number, y2: number): string {
  const x1 = x(line.from)
  const x2 = x(line.to)
  if (x1 === x2) return `M${String(x1)} ${String(y1)}V${String(y2)}`
  const mid = (y1 + y2) / 2
  return `M${String(x1)} ${String(y1)}C${String(x1)} ${String(mid)} ${String(x2)} ${String(mid)} ${String(x2)} ${String(y2)}`
}

function Graph({
  row,
  width,
  head
}: {
  row: GraphRow
  width: number
  head: boolean
}): React.JSX.Element {
  const mid = ROW / 2
  return (
    <svg
      className="history-graph"
      width={PAD * 2 + (width - 1) * LANE}
      height={ROW}
      aria-hidden="true"
    >
      {row.top.map((line, index) => (
        <path key={`t${String(index)}`} d={curve(line, 0, mid)} stroke={laneColor(line.lane)} />
      ))}
      {row.bottom.map((line, index) => (
        <path key={`b${String(index)}`} d={curve(line, mid, ROW)} stroke={laneColor(line.lane)} />
      ))}
      <circle
        cx={x(row.col)}
        cy={mid}
        r={head ? 4.5 : 3.5}
        fill={head ? 'var(--bg)' : laneColor(row.col)}
        stroke={laneColor(row.col)}
        strokeWidth={head ? 2 : 0}
      />
    </svg>
  )
}

/** Commits of the selected project or worktree, with a branch graph, like VS Code's. */
function HistoryPanel({
  where,
  label
}: {
  where: string | null
  label: string
}): React.JSX.Element {
  // Each result remembers the request it answers, so a stale one is ignored instead of reset.
  const [nonce, setNonce] = useState(0)
  const key = `${where ?? ''}|${String(nonce)}`
  const [log, setLog] = useState<{
    key: string
    commits: Commit[]
    more: boolean
    at: number
  } | null>(null)
  const [failure, setFailure] = useState<{ key: string; text: string } | null>(null)
  const [paging, setPaging] = useState(false)
  const [pick, setPick] = useState<{ key: string; hash: string } | null>(null)
  const [loaded, setLoaded] = useState<CommitDetail | null>(null)
  const [fileSel, setFileSel] = useState<{ hash: string; path: string } | null>(null)
  const [diffSel, setDiffSel] = useState<{ hash: string; path: string; text: string } | null>(null)

  const commits = log?.key === key ? log.commits : null
  const more = log?.key === key && log.more
  const now = log?.at ?? 0
  const error = failure?.key === key ? failure.text : null
  const selected = pick?.key === key ? pick.hash : null
  const detail = selected !== null && loaded?.hash.startsWith(selected) === true ? loaded : null
  const file = fileSel !== null && fileSel.hash === selected ? fileSel.path : null
  const diff =
    diffSel !== null && diffSel.hash === selected && diffSel.path === file ? diffSel.text : null

  useEffect(() => {
    let on = true
    window.api.commitLog(where, 0).then(
      (page) => {
        if (on) setLog({ key, commits: page.commits, more: page.more, at: Date.now() })
      },
      (err: unknown) => {
        if (on) setFailure({ key, text: errorText(err) })
      }
    )
    return () => {
      on = false
    }
  }, [where, key])

  useEffect(() => {
    if (selected === null) return
    let on = true
    window.api.commitDetail(where, selected).then(
      (value) => {
        if (on) setLoaded(value)
      },
      (err: unknown) => {
        if (on) setFailure({ key, text: errorText(err) })
      }
    )
    return () => {
      on = false
    }
  }, [where, selected, key])

  useEffect(() => {
    if (selected === null || file === null) return
    let on = true
    window.api.commitFileDiff(where, selected, file).then(
      (text) => {
        if (on) setDiffSel({ hash: selected, path: file, text })
      },
      (err: unknown) => {
        if (on) setFailure({ key, text: errorText(err) })
      }
    )
    return () => {
      on = false
    }
  }, [where, selected, file, key])

  const rows = useMemo(() => graphRows(commits ?? []), [commits])
  const width = rows.reduce((max, row) => Math.max(max, row.width), 1)
  const loading = commits === null || paging

  function loadMore(): void {
    if (commits === null || paging) return
    const asked = key
    setPaging(true)
    window.api
      .commitLog(where, commits.length)
      .then((page) => {
        setLog((prev) =>
          prev !== null && prev.key === asked
            ? { ...prev, commits: [...prev.commits, ...page.commits], more: page.more }
            : prev
        )
      })
      .catch((err: unknown) => {
        setFailure({ key: asked, text: errorText(err) })
      })
      .finally(() => {
        setPaging(false)
      })
  }

  function setSelected(hash: string | null): void {
    setPick(hash === null ? null : { key, hash })
  }

  function setFile(path: string | null): void {
    setFileSel(path === null || selected === null ? null : { hash: selected, path })
  }

  return (
    <div className="history-panel">
      <div className="files-head">
        <span className="files-label">{label}</span>
        <span className="files-count">
          {commits === null ? '' : `${String(commits.length)}${more ? '+' : ''} commits`}
        </span>
        <button
          type="button"
          className="btn btn-quiet btn-compact"
          disabled={loading}
          onClick={() => {
            setNonce((n) => n + 1)
          }}
        >
          Refresh
        </button>
      </div>
      {error !== null ? <p className="files-error">{error}</p> : null}
      {commits === null && error === null ? <p className="files-status">Loading history…</p> : null}
      {commits !== null && commits.length === 0 ? (
        <p className="files-status">No commits yet.</p>
      ) : null}
      <ol className={selected === null ? 'history-list' : 'history-list is-split'}>
        {(commits ?? []).map((commit, index) => {
          const row = rows[index]
          const badges = refBadges(commit.refs)
          const isHead = commit.refs.some((ref) => ref === 'HEAD' || ref.startsWith('HEAD -> '))
          const isSelected = commit.hash === selected
          return (
            <li key={commit.hash}>
              <button
                type="button"
                className={isSelected ? 'history-row is-selected' : 'history-row'}
                aria-pressed={isSelected}
                title={`${commit.subject}\n${commit.author} · ${new Date(commit.time).toLocaleString()}\n${commit.hash}`}
                onClick={() => {
                  setSelected(isSelected ? null : commit.hash)
                }}
              >
                {row !== undefined ? <Graph row={row} width={width} head={isHead} /> : null}
                <span className="history-text">
                  <span className="history-subject">
                    {badges.map((badge) => (
                      <span
                        key={`${badge.kind}-${badge.name}`}
                        className={`history-ref is-${badge.kind}`}
                      >
                        {badge.name}
                      </span>
                    ))}
                    {commit.subject}
                  </span>
                </span>
                <span className="history-meta">
                  <span className="history-author">{commit.author}</span>
                  <span className="history-age">{ago(commit.time, now)}</span>
                </span>
              </button>
            </li>
          )
        })}
        {more ? (
          <li className="history-more">
            <button
              type="button"
              className="btn btn-quiet btn-compact"
              disabled={loading}
              onClick={loadMore}
            >
              {paging ? 'Loading…' : 'Load more'}
            </button>
          </li>
        ) : null}
      </ol>
      {selected !== null ? (
        <div className="history-detail">
          {detail === null ? (
            <p className="files-status">Loading commit…</p>
          ) : (
            <>
              <div className="history-detail-head">
                <span className="history-hash">{detail.hash.slice(0, 10)}</span>
                <span className="history-who">
                  {detail.author} · {new Date(detail.time).toLocaleString()}
                </span>
                <button
                  type="button"
                  className="btn btn-quiet btn-compact"
                  onClick={() => {
                    void navigator.clipboard.writeText(detail.hash)
                  }}
                >
                  Copy id
                </button>
              </div>
              <pre className="history-message">{detail.message}</pre>
              <ul className="history-files">
                {detail.files.length === 0 ? (
                  <li className="files-status">No file changes.</li>
                ) : null}
                {detail.files.map((entry) => (
                  <li key={entry.path}>
                    <button
                      type="button"
                      className={entry.path === file ? 'history-file is-selected' : 'history-file'}
                      onClick={() => {
                        setFile(entry.path === file ? null : entry.path)
                      }}
                    >
                      <span className="history-file-path">{entry.path}</span>
                      {entry.added === null || entry.removed === null ? (
                        <span className="history-file-stat">binary</span>
                      ) : (
                        <span className="history-file-stat">
                          <span className="stat-add">{`+${String(entry.added)}`}</span>{' '}
                          <span className="stat-del">{`−${String(entry.removed)}`}</span>
                        </span>
                      )}
                    </button>
                  </li>
                ))}
              </ul>
              {file !== null ? <DiffBlock text={diff} /> : null}
            </>
          )}
        </div>
      ) : null}
    </div>
  )
}

export default HistoryPanel
