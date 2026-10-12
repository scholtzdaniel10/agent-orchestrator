import {
  useEffect,
  useId,
  useRef,
  useState,
  type KeyboardEvent,
  type RefObject
} from 'react'
import { Star } from './Star'

type ProjectEntry = Awaited<ReturnType<Window['api']['listProjects']>>[number]
type LeadChat = Awaited<ReturnType<Window['api']['listLeadChats']>>[number]
type GithubRepo = Awaited<ReturnType<Window['api']['githubRepos']>>[number]

const CHATS_OPEN_KEY = 'ao.chats-open'

function readStoredBool(key: string, fallback: boolean): boolean {
  try {
    const raw = localStorage.getItem(key)
    if (raw === null) return fallback
    return raw === '1' || raw === 'true'
  } catch {
    return fallback
  }
}

function writeStoredBool(key: string, value: boolean): void {
  try {
    localStorage.setItem(key, value ? '1' : '0')
  } catch {
    // Ignore quota / private-mode failures.
  }
}

function folderName(path: string): string {
  const trimmed = path.replace(/[/\\]+$/, '')
  const index = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'))
  if (index === -1) return trimmed
  return trimmed.slice(index + 1)
}

function relativeAge(updatedAt: number, now: number): string {
  const delta = Math.max(0, now - updatedAt)
  const minutes = Math.floor(delta / 60_000)
  if (minutes < 60) return `${String(Math.max(minutes, 0))}m`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${String(hours)}h`
  return `${String(Math.floor(hours / 24))}d`
}

function relativeUpdated(iso: string, now: number): string {
  const then = Date.parse(iso)
  if (!Number.isFinite(then)) return ''
  return relativeAge(then, now)
}

function FolderGlyph(): React.JSX.Element {
  return (
    <svg
      className="project-folder"
      width="14"
      height="14"
      viewBox="0 0 16 16"
      aria-hidden="true"
      focusable="false"
    >
      <path
        fill="currentColor"
        d="M1.5 3.5h4.2l1.3 1.5H14.5v7.5H1.5V3.5zm1 1v7h11v-5.5H6.6L5.3 4.5H2.5z"
      />
    </svg>
  )
}

function GearGlyph(): React.JSX.Element {
  return (
    <svg
      className="project-gear"
      width="14"
      height="14"
      viewBox="0 0 16 16"
      aria-hidden="true"
      focusable="false"
    >
      <path
        fill="currentColor"
        d="M6.3 1.2h3.4l.3 1.6a5 5 0 0 1 1.2.7l1.5-.6 1.7 1.7-.6 1.5c.3.4.5.8.7 1.2l1.6.3v3.4l-1.6.3a5 5 0 0 1-.7 1.2l.6 1.5-1.7 1.7-1.5-.6a5 5 0 0 1-1.2.7l-.3 1.6H6.3l-.3-1.6a5 5 0 0 1-1.2-.7l-1.5.6-1.7-1.7.6-1.5a5 5 0 0 1-.7-1.2L.2 8.7V5.3l1.6-.3c.2-.4.4-.8.7-1.2l-.6-1.5L3.6 1.6l1.5.6c.4-.3.8-.5 1.2-.7l.3-1.6ZM8 5.5A2.5 2.5 0 1 0 8 10.5 2.5 2.5 0 0 0 8 5.5Z"
      />
    </svg>
  )
}

function LockGlyph(): React.JSX.Element {
  return (
    <svg
      className="github-lock"
      width="10"
      height="12"
      viewBox="0 0 10 12"
      aria-hidden="true"
      focusable="false"
    >
      <path
        fill="currentColor"
        d="M5 0a3 3 0 0 0-3 3v2H1v7h8V5H8V3a3 3 0 0 0-3-3zm-2 3a2 2 0 1 1 4 0v2H3V3zm1 5h2v2H4V8z"
      />
    </svg>
  )
}

function StatusDot({
  running,
  waiting
}: {
  running: boolean
  waiting: boolean
}): React.JSX.Element | null {
  if (running) {
    return (
      <span
        className="project-status-dot is-running"
        aria-label="jobs running"
        title="jobs running"
      />
    )
  }
  if (waiting) {
    return (
      <span
        className="project-status-dot is-waiting"
        aria-label="changes waiting"
        title="changes waiting"
      />
    )
  }
  return null
}

function ProjectMeta({ entry }: { entry: ProjectEntry }): React.JSX.Element {
  if (!entry.isRepo) {
    return <span className="project-note">not a git repository</span>
  }
  const parts: React.JSX.Element[] = []
  if (entry.branch !== null) {
    parts.push(
      <span key="branch" className="project-branch">
        {entry.branch}
      </span>
    )
  }
  if (entry.ahead > 0 || entry.behind > 0) {
    const bits: string[] = []
    if (entry.ahead > 0) bits.push(`↑${String(entry.ahead)}`)
    if (entry.behind > 0) bits.push(`↓${String(entry.behind)}`)
    parts.push(
      <span key="ab" className="project-ab">
        {bits.join(' ')}
      </span>
    )
  }
  if (entry.dirty > 0) {
    parts.push(
      <span
        key="dirty"
        className="project-dirty"
        title={`${String(entry.dirty)} uncommitted files`}
        aria-label={`${String(entry.dirty)} uncommitted files`}
      >
        ●{String(entry.dirty)} edited
      </span>
    )
  }
  if (entry.changes > 0) {
    parts.push(
      <span
        key="changes"
        className="project-diff"
        aria-label={`${String(entry.changes)} changes waiting, ${String(entry.insertions)} added, ${String(entry.deletions)} removed`}
      >
        <span className="stat-add">+{String(entry.insertions)}</span>{' '}
        <span className="stat-del">−{String(entry.deletions)}</span>
      </span>
    )
  }
  if (!entry.active && entry.chats > 0) {
    parts.push(
      <span key="chats" className="project-chats-count">
        {entry.chats === 1 ? '1 chat' : `${String(entry.chats)} chats`}
      </span>
    )
  }
  if (parts.length === 0) {
    return <span className="project-branch">{entry.branch ?? ''}</span>
  }
  return <>{parts}</>
}

function ChatRow({
  chat,
  leadBusy,
  now,
  onOpen,
  onRename,
  onRemove
}: {
  chat: LeadChat
  leadBusy: boolean
  now: number
  onOpen: (id: string) => void
  onRename: (id: string, title: string) => void
  onRemove: (id: string) => void
}): React.JSX.Element {
  const [mode, setMode] = useState<'idle' | 'rename' | 'confirm'>('idle')
  const [draft, setDraft] = useState(chat.title)
  const inputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    if (mode !== 'rename') return
    inputRef.current?.focus()
    inputRef.current?.select()
  }, [mode])

  function saveRename(): void {
    const next = draft.trim()
    setMode('idle')
    if (next === '' || next === chat.title) {
      setDraft(chat.title)
      return
    }
    onRename(chat.id, next)
  }

  if (mode === 'confirm') {
    return (
      <li className="project-chat-item">
        <div className="project-chat-confirm" role="group" aria-label={`Delete ${chat.title}?`}>
          <span className="project-chat-confirm-text">Delete this chat?</span>
          <button
            type="button"
            className="btn btn-quiet btn-compact is-danger-label"
            onClick={() => {
              setMode('idle')
              onRemove(chat.id)
            }}
          >
            Delete
          </button>
          <button
            type="button"
            className="btn btn-quiet btn-compact"
            onClick={() => {
              setMode('idle')
            }}
          >
            Cancel
          </button>
        </div>
      </li>
    )
  }

  if (mode === 'rename') {
    return (
      <li className="project-chat-item">
        <input
          ref={inputRef}
          className="project-chat-rename"
          value={draft}
          aria-label={`Rename ${chat.title}`}
          maxLength={80}
          onChange={(event) => {
            setDraft(event.target.value)
          }}
          onBlur={() => {
            saveRename()
          }}
          onKeyDown={(event: KeyboardEvent<HTMLInputElement>) => {
            if (event.key === 'Enter') {
              event.preventDefault()
              saveRename()
            } else if (event.key === 'Escape') {
              event.preventDefault()
              setDraft(chat.title)
              setMode('idle')
            }
          }}
        />
      </li>
    )
  }

  return (
    <li className="project-chat-item">
      <button
        type="button"
        className={`project-chat-row${chat.active ? ' is-selected' : ''}`}
        title={chat.title}
        aria-current={chat.active ? 'true' : undefined}
        disabled={leadBusy}
        onClick={() => {
          onOpen(chat.id)
        }}
      >
        <span className="project-chat-title">{chat.title}</span>
        <span className="project-chat-meta">
          {chat.pending === true ? (
            <span className="status status-running project-chat-run">
              <span className="status-dot" aria-hidden="true" />
              running
            </span>
          ) : null}
          <span className="project-chat-age">{relativeAge(chat.updatedAt, now)}</span>
        </span>
      </button>
      <div className="project-chat-actions">
        <button
          type="button"
          className="project-chat-action"
          aria-label={`Rename ${chat.title}`}
          title="Rename"
          disabled={leadBusy}
          onClick={() => {
            setDraft(chat.title)
            setMode('rename')
          }}
        >
          ✎
        </button>
        <button
          type="button"
          className="project-chat-action"
          aria-label={`Delete ${chat.title}`}
          title="Delete"
          disabled={leadBusy}
          onClick={() => {
            setMode('confirm')
          }}
        >
          ×
        </button>
      </div>
    </li>
  )
}

function GithubPanel({
  repos,
  loading,
  cloning,
  filter,
  now,
  onFilter,
  onClone,
  onClose
}: {
  repos: GithubRepo[]
  loading: boolean
  cloning: string | null
  filter: string
  now: number
  onFilter: (value: string) => void
  onClone: (nameWithOwner: string) => void
  onClose: () => void
}): React.JSX.Element {
  const filterId = useId()
  const needle = filter.trim().toLowerCase()
  const shown =
    needle === ''
      ? repos
      : repos.filter(
          (repo) =>
            repo.nameWithOwner.toLowerCase().includes(needle) ||
            repo.description.toLowerCase().includes(needle)
        )

  return (
    <div
      className="github-panel"
      role="region"
      aria-label="Clone from GitHub"
      onKeyDown={(event) => {
        if (event.key === 'Escape') {
          event.stopPropagation()
          onClose()
        }
      }}
    >
      <div className="github-panel-head">
        <label className="github-filter-label" htmlFor={filterId}>
          Filter
        </label>
        <input
          id={filterId}
          className="github-filter"
          type="search"
          value={filter}
          placeholder="Filter repos"
          autoFocus
          disabled={cloning !== null}
          onChange={(event) => {
            onFilter(event.target.value)
          }}
        />
        <button
          type="button"
          className="btn btn-quiet btn-compact"
          disabled={cloning !== null}
          onClick={onClose}
        >
          Close
        </button>
      </div>
      {loading ? <p className="github-status">Loading…</p> : null}
      {!loading && shown.length === 0 ? <p className="github-status">No repositories</p> : null}
      {!loading && shown.length > 0 ? (
        <ul className="github-list">
          {shown.map((repo) => {
            const busy = cloning === repo.nameWithOwner
            const age = relativeUpdated(repo.updatedAt, now)
            return (
              <li key={repo.nameWithOwner}>
                <button
                  type="button"
                  className="github-row"
                  disabled={cloning !== null}
                  aria-label={
                    busy
                      ? `Cloning ${repo.nameWithOwner}`
                      : `Clone ${repo.nameWithOwner}${repo.isPrivate ? ', private' : ''}`
                  }
                  onClick={() => {
                    onClone(repo.nameWithOwner)
                  }}
                >
                  <span className="github-row-name">
                    <span className="github-name">{repo.nameWithOwner}</span>
                    {repo.isPrivate ? <LockGlyph /> : null}
                  </span>
                  <span className="github-row-meta">
                    {busy ? (
                      <span className="github-cloning">Cloning…</span>
                    ) : age !== '' ? (
                      <span className="github-updated">{age}</span>
                    ) : null}
                  </span>
                </button>
              </li>
            )
          })}
        </ul>
      ) : null}
    </div>
  )
}

function ProjectBar({
  projects,
  chats,
  busy,
  leadBusy,
  now,
  error,
  githubAvailable,
  collapsed,
  settingsButtonRef,
  onCollapsed,
  onAdd,
  onGithubClone,
  onSwitch,
  onRemove,
  onReveal,
  onOpenSettings,
  onOpenChat,
  onNewChat,
  onRenameChat,
  onRemoveChat
}: {
  projects: ProjectEntry[]
  chats: LeadChat[]
  busy: boolean
  leadBusy: boolean
  now: number
  error: string | null
  githubAvailable: boolean
  collapsed: boolean
  settingsButtonRef: RefObject<HTMLButtonElement | null>
  onCollapsed: (next: boolean) => void
  onAdd: () => void
  onGithubClone: (nameWithOwner: string) => Promise<boolean>
  onSwitch: (path: string) => void
  onRemove: (path: string) => void
  onReveal: (path: string) => void
  onOpenSettings: () => void
  onOpenChat: (id: string) => void
  onNewChat: () => void
  onRenameChat: (id: string, title: string) => void
  onRemoveChat: (id: string) => void
}): React.JSX.Element {
  const [githubOpen, setGithubOpen] = useState(false)
  const [repos, setRepos] = useState<GithubRepo[]>([])
  const [reposLoading, setReposLoading] = useState(false)
  const [githubFilter, setGithubFilter] = useState('')
  const [cloning, setCloning] = useState<string | null>(null)
  const [filter, setFilter] = useState('')
  const [chatsOpen, setChatsOpen] = useState(() => readStoredBool(CHATS_OPEN_KEY, true))
  const filterRef = useRef<HTMLInputElement>(null)
  const filterId = useId()
  const needle = filter.trim().toLowerCase()

  const filteredChats =
    needle === ''
      ? chats.slice(0, 30)
      : chats.filter((chat) => chat.title.toLowerCase().includes(needle)).slice(0, 30)

  const filteredProjects =
    needle === ''
      ? projects
      : projects.filter((entry) => {
          const name = folderName(entry.path).toLowerCase()
          const path = entry.path.toLowerCase()
          if (name.includes(needle) || path.includes(needle)) return true
          return entry.active && filteredChats.length > 0
        })

  const showEmpty = needle !== '' && filteredProjects.length === 0

  useEffect(() => {
    if (!githubOpen) return
    let active = true
    void window.api.githubRepos().then(
      (list) => {
        if (!active) return
        setRepos(list)
        setReposLoading(false)
      },
      () => {
        if (!active) return
        setRepos([])
        setReposLoading(false)
      }
    )
    return () => {
      active = false
    }
  }, [githubOpen])

  useEffect(() => {
    function onKey(event: globalThis.KeyboardEvent): void {
      const mod = event.ctrlKey || event.metaKey
      if (mod && event.key.toLowerCase() === 'k') {
        event.preventDefault()
        if (collapsed) onCollapsed(false)
        filterRef.current?.focus()
        filterRef.current?.select()
        return
      }
      if (mod && event.key >= '1' && event.key <= '9') {
        const index = Number(event.key) - 1
        const entry = projects[index]
        if (entry === undefined) return
        event.preventDefault()
        if (!entry.active) onSwitch(entry.path)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('keydown', onKey)
    }
  }, [collapsed, onCollapsed, onSwitch, projects])

  async function cloneRepo(nameWithOwner: string): Promise<void> {
    if (cloning !== null) return
    setCloning(nameWithOwner)
    try {
      const ok = await onGithubClone(nameWithOwner)
      if (ok) {
        setGithubOpen(false)
        setGithubFilter('')
      }
    } finally {
      setCloning(null)
    }
  }

  function setCollapsed(next: boolean): void {
    onCollapsed(next)
  }

  function toggleChats(): void {
    setChatsOpen((current) => {
      const next = !current
      writeStoredBool(CHATS_OPEN_KEY, next)
      return next
    })
  }

  if (collapsed) {
    return (
      <aside className="project-bar is-collapsed" aria-label="Projects">
        <div className="project-rail-head">
          <button
            type="button"
            className="project-rail-toggle"
            aria-label="Show projects sidebar"
            title="Show sidebar"
            onClick={() => {
              setCollapsed(false)
            }}
          >
            »
          </button>
        </div>
        <ul className="project-rail-list">
          {projects.map((entry) => {
            const name = folderName(entry.path)
            const letter = name.charAt(0).toUpperCase()
            const running = entry.active && busy
            const waiting = entry.changes > 0
            return (
              <li key={entry.path}>
                <button
                  type="button"
                  className={`project-rail-btn${entry.active ? ' is-selected' : ''}`}
                  title={name}
                  aria-label={name}
                  aria-current={entry.active ? 'true' : undefined}
                  onClick={() => {
                    if (!entry.active) onSwitch(entry.path)
                  }}
                >
                  <span className="project-rail-letter">{letter}</span>
                  <StatusDot running={running} waiting={waiting} />
                </button>
              </li>
            )
          })}
        </ul>
        <button
          ref={settingsButtonRef}
          type="button"
          className="project-settings-btn is-rail"
          aria-label="Settings"
          title="Settings"
          onClick={onOpenSettings}
        >
          <GearGlyph />
        </button>
      </aside>
    )
  }

  return (
    <aside className="project-bar" aria-labelledby="projects-heading">
      <div className="panel-head">
        <div className="panel-title">
          <Star size={12} />
          <h2 id="projects-heading">Projects</h2>
        </div>
        <div className="head-actions">
          <button
            type="button"
            className="project-collapse"
            aria-label="Hide projects sidebar"
            title="Hide sidebar"
            onClick={() => {
              setCollapsed(true)
            }}
          >
            «
          </button>
          <button
            type="button"
            className="btn btn-quiet btn-compact project-add"
            onClick={onAdd}
            aria-label="Add folder"
            title="Add folder"
          >
            <span className="project-add-label">Add folder</span>
            <span className="project-add-glyph" aria-hidden="true">
              +
            </span>
          </button>
          {githubAvailable ? (
            <button
              type="button"
              className="btn btn-quiet btn-compact project-github"
              onClick={() => {
                setReposLoading(!githubOpen)
                setGithubOpen(!githubOpen)
              }}
              aria-label="From GitHub"
              title="From GitHub"
              aria-expanded={githubOpen}
            >
              <span className="project-github-label">From GitHub</span>
              <span className="project-github-glyph" aria-hidden="true">
                ↓
              </span>
            </button>
          ) : null}
        </div>
      </div>
      <div className="project-filter-wrap">
        <label className="visually-hidden" htmlFor={filterId}>
          Filter projects
        </label>
        <input
          ref={filterRef}
          id={filterId}
          className="project-filter"
          type="search"
          value={filter}
          placeholder="Filter projects"
          onChange={(event) => {
            setFilter(event.target.value)
          }}
          onKeyDown={(event) => {
            if (event.key === 'Escape') {
              event.preventDefault()
              setFilter('')
            }
          }}
        />
      </div>
      {githubOpen ? (
        <GithubPanel
          repos={repos}
          loading={reposLoading}
          cloning={cloning}
          filter={githubFilter}
          now={now}
          onFilter={setGithubFilter}
          onClone={(name) => {
            void cloneRepo(name)
          }}
          onClose={() => {
            if (cloning !== null) return
            setGithubOpen(false)
            setGithubFilter('')
          }}
        />
      ) : null}
      <ul className="project-list">
        {showEmpty ? <li className="project-empty">No matches</li> : null}
        {filteredProjects.map((entry) => {
          const name = folderName(entry.path)
          const active = entry.active
          const running = active && busy
          const waiting = entry.changes > 0
          return (
            <li key={entry.path} className="project-item">
              <div className="project-row-wrap">
                <button
                  type="button"
                  className={`project-row${active ? ' is-selected' : ''}`}
                  title={entry.path}
                  aria-label={name}
                  aria-current={active ? 'true' : undefined}
                  onClick={() => {
                    if (!active) onSwitch(entry.path)
                  }}
                >
                  <span className="project-mark">
                    <FolderGlyph />
                    <StatusDot running={running} waiting={waiting} />
                  </span>
                  <span className="project-row-body">
                    <span className="project-name">{name}</span>
                    <span className="project-row-meta">
                      <ProjectMeta entry={entry} />
                    </span>
                  </span>
                </button>
                <div className="project-row-actions">
                  <button
                    type="button"
                    className="project-chat-action"
                    aria-label={`Open folder ${name}`}
                    title="Open folder"
                    onClick={() => {
                      onReveal(entry.path)
                    }}
                  >
                    ⌂
                  </button>
                  {!active ? (
                    <button
                      type="button"
                      className="project-chat-action"
                      aria-label={`Remove ${name} from the list`}
                      title="Remove from this list (does not delete the folder)"
                      onClick={() => {
                        onRemove(entry.path)
                      }}
                    >
                      ×
                    </button>
                  ) : null}
                </div>
              </div>
              {active ? (
                <div className="project-chats">
                  <div className="project-chats-head">
                    <button
                      type="button"
                      className="project-chats-toggle"
                      aria-expanded={chatsOpen}
                      onClick={toggleChats}
                    >
                      <span className="project-chats-chevron" aria-hidden="true">
                        {chatsOpen ? '▾' : '▸'}
                      </span>
                      <span className="project-chats-label">
                        Chats{' '}
                        <span className="project-chats-n">({String(chats.length)})</span>
                      </span>
                    </button>
                    <button
                      type="button"
                      className="btn btn-quiet btn-compact"
                      disabled={leadBusy || !chats.some((chat) => chat.active)}
                      onClick={onNewChat}
                    >
                      New chat
                    </button>
                  </div>
                  {chatsOpen ? (
                    <ul className="project-chat-list">
                      {needle !== '' && filteredChats.length === 0 ? (
                        <li className="project-empty">No matches</li>
                      ) : (
                        filteredChats.map((chat) => (
                          <ChatRow
                            key={chat.id}
                            chat={chat}
                            leadBusy={leadBusy}
                            now={now}
                            onOpen={onOpenChat}
                            onRename={onRenameChat}
                            onRemove={onRemoveChat}
                          />
                        ))
                      )}
                    </ul>
                  ) : null}
                </div>
              ) : null}
            </li>
          )
        })}
      </ul>
      {error !== null ? (
        <p className="field-error project-error" role="alert">
          {error}
        </p>
      ) : null}
      <button
        ref={settingsButtonRef}
        type="button"
        className="project-settings-btn"
        onClick={onOpenSettings}
      >
        <GearGlyph />
        <span>Settings</span>
      </button>
    </aside>
  )
}

export default ProjectBar
