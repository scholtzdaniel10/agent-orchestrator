import { useEffect, useMemo, useRef, useState } from 'react'
import { graphRows, type GraphLine, type GraphRow } from './commit-graph'
import { DiffBlock } from './FilesPanel'

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

export function ago(time: number, now: number): string {
  const minutes = Math.floor(Math.max(0, now - time) / 60_000)
  if (minutes < 1) return 'now'
  if (minutes < 60) return `${String(minutes)}m`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${String(hours)}h`
  const days = Math.floor(hours / 24)
  if (days < 60) return `${String(days)}d`
  return `${String(Math.floor(days / 30))}mo`
}

/** "HEAD -> main" → main (current), "tag: v1" → v1 (tag), anything else as a branch. */
export function refBadges(
  refs: readonly string[]
): { name: string; kind: 'head' | 'tag' | 'branch' }[] {
  const out: { name: string; kind: 'head' | 'tag' | 'branch' }[] = []
  for (const ref of refs) {
    if (ref === 'HEAD') continue
    if (ref.startsWith('HEAD -> ')) out.push({ name: ref.slice(8), kind: 'head' })
    else if (ref.startsWith('tag: ')) out.push({ name: ref.slice(5), kind: 'tag' })
    else if (!ref.endsWith('/HEAD')) out.push({ name: ref, kind: 'branch' })
  }
  return out
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
  const [commits, setCommits] = useState<Commit[] | null>(null)
  const [more, setMore] = useState(false)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [selected, setSelected] = useState<string | null>(null)
  const [detail, setDetail] = useState<CommitDetail | null>(null)
  const [file, setFile] = useState<string | null>(null)
  const [diff, setDiff] = useState<string | null>(null)
  const [nonce, setNonce] = useState(0)
  const [now, setNow] = useState(() => Date.now())
  const live = useRef(0)

  useEffect(() => {
    const ticket = ++live.current
    setCommits(null)
    setSelected(null)
    setError(null)
    setLoading(true)
    window.api
      .commitLog(where, 0)
      .then((page) => {
        if (live.current !== ticket) return
        setCommits(page.commits)
        setMore(page.more)
        setNow(Date.now())
      })
      .catch((err: unknown) => {
        if (live.current === ticket) setError(errorText(err))
      })
      .finally(() => {
        if (live.current === ticket) setLoading(false)
      })
  }, [where, nonce])

  useEffect(() => {
    setDetail(null)
    setFile(null)
    setDiff(null)
    if (selected === null) return
    let on = true
    window.api
      .commitDetail(where, selected)
      .then((value) => {
        if (on) setDetail(value)
      })
      .catch((err: unknown) => {
        if (on) setError(errorText(err))
      })
    return () => {
      on = false
    }
  }, [where, selected])

  useEffect(() => {
    setDiff(null)
    if (selected === null || file === null) return
    let on = true
    window.api
      .commitFileDiff(where, selected, file)
      .then((value) => {
        if (on) setDiff(value)
      })
      .catch((err: unknown) => {
        if (on) setError(errorText(err))
      })
    return () => {
      on = false
    }
  }, [where, selected, file])

  const rows = useMemo(() => graphRows(commits ?? []), [commits])
  const width = rows.reduce((max, row) => Math.max(max, row.width), 1)

  function loadMore(): void {
    if (commits === null || loading) return
    const ticket = live.current
    setLoading(true)
    window.api
      .commitLog(where, commits.length)
      .then((page) => {
        if (live.current !== ticket) return
        setCommits([...commits, ...page.commits])
        setMore(page.more)
      })
      .catch((err: unknown) => {
        setError(errorText(err))
      })
      .finally(() => {
        setLoading(false)
      })
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
              {loading ? 'Loading…' : 'Load more'}
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
