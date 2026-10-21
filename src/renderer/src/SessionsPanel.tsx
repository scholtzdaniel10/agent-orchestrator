import { useCallback, useEffect, useState } from 'react'
import BotAvatar from './BotAvatar'
import Select from './Select'

type SessionScan = Awaited<ReturnType<Window['api']['listSessions']>>
type AgentSession = SessionScan['sessions'][number]
type Scope = 'workspace' | 'project' | 'all'
type Sort = 'updated' | 'messages'

const PAGE = 20

function errorText(err: unknown): string {
  if (!(err instanceof Error)) return String(err)
  return err.message.replace(/^Error invoking remote method '[^']+': (Error: )?/, '')
}

/** Folder paths compared the way Windows does: any slash, any case, no trailing slash. */
function samePath(a: string, b: string): boolean {
  const fold = (value: string): string =>
    value.replace(/\\/g, '/').replace(/\/+$/, '').toLowerCase()
  return fold(a) === fold(b)
}

function ago(at: number, now: number): string {
  const minutes = Math.max(0, Math.round((now - at) / 60_000))
  if (minutes < 1) return 'just now'
  if (minutes < 60) return `${String(minutes)}m ago`
  const hours = Math.round(minutes / 60)
  if (hours < 24) return `${String(hours)}h ago`
  return `${String(Math.round(hours / 24))}d ago`
}

function folderName(path: string): string {
  const trimmed = path.replace(/[/\\]+$/, '')
  return trimmed.slice(Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\')) + 1)
}

/**
 * Past Claude and Cursor sessions on this machine, read from the CLIs' own logs: the open
 * folder's, the project's (its worktrees too), or all of them. A click opens one again.
 */
function SessionsPanel({
  workspace,
  projectFolders
}: {
  /** The folder the terminals work in now: the project or the selected worktree. */
  workspace: string
  /** The project folder and every worktree folder of it. */
  projectFolders: readonly string[]
}): React.JSX.Element {
  const [scan, setScan] = useState<(SessionScan & { at: number }) | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [scope, setScope] = useState<Scope>('workspace')
  const [sort, setSort] = useState<Sort>('updated')
  const [query, setQuery] = useState('')
  const [shown, setShown] = useState(PAGE)
  const [opening, setOpening] = useState<string | null>(null)

  const fetchSessions = useCallback((): void => {
    window.api.listSessions().then(
      (next) => {
        setScan({ ...next, at: Date.now() })
        setLoading(false)
      },
      (err: unknown) => {
        setError(errorText(err))
        setLoading(false)
      }
    )
  }, [])

  useEffect(fetchSessions, [fetchSessions])

  function load(): void {
    setLoading(true)
    setError(null)
    fetchSessions()
  }

  // Ages read against the moment the list arrived, so a render never asks the clock.
  const now = scan?.at ?? 0
  const all = scan?.sessions ?? []
  const inScope = all.filter((session) => {
    if (scope === 'all') return true
    if (session.cwd === null) return false
    if (scope === 'workspace') return samePath(session.cwd, workspace)
    return projectFolders.some((folder) => samePath(session.cwd ?? '', folder))
  })
  const words = query.trim().toLowerCase()
  const matching = inScope
    .filter(
      (session) => words === '' || `${session.title} ${session.last}`.toLowerCase().includes(words)
    )
    .sort((a, b) =>
      sort === 'messages'
        ? (b.messages ?? Infinity) - (a.messages ?? Infinity) || b.updatedAt - a.updatedAt
        : b.updatedAt - a.updatedAt
    )
  const groups = new Map<string, AgentSession[]>()
  for (const session of matching.slice(0, shown)) {
    const name = session.cwd === null ? 'Unknown folder' : folderName(session.cwd)
    groups.set(name, [...(groups.get(name) ?? []), session])
  }

  function resume(session: AgentSession): void {
    setOpening(session.id)
    setError(null)
    window.api
      .resumeSession(session.id, 100, 30)
      .catch((err: unknown) => {
        setError(errorText(err))
      })
      .finally(() => {
        setOpening(null)
      })
  }

  return (
    <div className="sessions-panel">
      <div className="sessions-head">
        <span className="sessions-title">Agent session history</span>
        <button
          type="button"
          className="btn btn-quiet btn-compact"
          disabled={loading}
          onClick={load}
        >
          Refresh
        </button>
      </div>
      <div className="sessions-scopes" role="group" aria-label="Which sessions">
        {(['workspace', 'project', 'all'] as const).map((item) => (
          <button
            key={item}
            type="button"
            className="btn btn-quiet btn-compact"
            aria-pressed={scope === item}
            onClick={() => {
              setScope(item)
              setShown(PAGE)
            }}
          >
            {item === 'workspace' ? 'Workspace' : item === 'project' ? 'Project' : 'All'}
          </button>
        ))}
      </div>
      <input
        type="search"
        className="files-filter"
        placeholder="Search sessions"
        aria-label="Search sessions"
        value={query}
        onChange={(event) => {
          setQuery(event.target.value)
          setShown(PAGE)
        }}
      />
      <div className="sessions-meta">
        <span>
          {scan === null
            ? loading
              ? 'Scanning…'
              : ''
            : `Scanned ${String(scan.files)} files · ${ago(scan.scannedAt, now)}`}
        </span>
      </div>
      <div className="sessions-meta">
        <span>{`${String(Math.min(shown, matching.length))} of ${String(matching.length)} sessions`}</span>
        <Select
          className="sessions-sort"
          value={sort}
          aria-label="Sort sessions"
          onChange={(picked) => {
            setSort(picked === 'messages' ? 'messages' : 'updated')
          }}
        >
          <option value="updated">Last updated</option>
          <option value="messages">Most messages</option>
        </Select>
      </div>
      {error !== null ? (
        <p className="error-text" role="alert">
          {error}
        </p>
      ) : null}
      <div className="sessions-list">
        {scan !== null && matching.length === 0 ? (
          <p className="files-status">
            {scope === 'all' || words !== ''
              ? 'No sessions match.'
              : 'No sessions in this folder yet. All shows every session on this machine.'}
          </p>
        ) : null}
        {[...groups].map(([name, items]) => (
          <section key={name} className="sessions-group" aria-label={name}>
            <h3 className="sessions-group-head">
              <span>{name}</span>
              <span className="tab-badge">{items.length}</span>
            </h3>
            {items.map((session) => (
              <button
                key={`${session.provider}:${session.id}`}
                type="button"
                className="sessions-row"
                disabled={opening !== null}
                title={`Open again in a terminal${session.cwd === null ? '' : `, in ${session.cwd}`}`}
                onClick={() => {
                  resume(session)
                }}
              >
                <span className="sessions-row-title">{session.title}</span>
                <span className="sessions-row-last">
                  <span className="sessions-row-who">
                    {session.lastRole === 'you' ? 'You: ' : 'Agent: '}
                  </span>
                  {session.last}
                </span>
                <span className="sessions-row-facts">
                  <BotAvatar bot={session.provider} state="idle" size={14} />
                  <span>{session.provider === 'claude' ? 'Claude' : 'Cursor'}</span>
                  <span>
                    {session.messages === null
                      ? 'long session'
                      : `${String(session.messages)} msgs`}
                  </span>
                  <span>{ago(session.updatedAt, now)}</span>
                  {session.model !== null ? <span className="mono">{session.model}</span> : null}
                  {opening === session.id ? <span>Opening…</span> : null}
                </span>
              </button>
            ))}
          </section>
        ))}
        {matching.length > shown ? (
          <button
            type="button"
            className="btn btn-quiet sessions-more"
            onClick={() => {
              setShown((count) => count + PAGE)
            }}
          >
            Show more sessions
          </button>
        ) : null}
      </div>
    </div>
  )
}

export default SessionsPanel
