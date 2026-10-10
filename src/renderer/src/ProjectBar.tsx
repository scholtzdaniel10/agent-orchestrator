import { useEffect, useId, useRef, useState, type KeyboardEvent } from 'react'
import { Star } from './Star'
import { worktreeLabel } from './worktree-label'

type ProjectEntry = Awaited<ReturnType<Window['api']['listProjects']>>[number]
type LeadChat = Awaited<ReturnType<Window['api']['listLeadChats']>>[number]
type GithubRepo = Awaited<ReturnType<Window['api']['githubRepos']>>[number]
type ChangeSet = Awaited<ReturnType<Window['api']['listChanges']>>[number]
type TerminalInfo = Awaited<ReturnType<Window['api']['listTerminals']>>[number]
type ProviderId = Parameters<Window['api']['openTerminal']>[0]

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

function countTerminals(terminals: readonly TerminalInfo[], change: string | null): number {
  let n = 0
  for (const info of terminals) {
    if (change === null) {
      if (info.change === undefined) n += 1
    } else if (info.change === change) {
      n += 1
    }
  }
  return n
}

function worktreeRowId(index: number, changes: readonly ChangeSet[]): string {
  if (index === 0) return 'worktree-row-project'
  const item = changes[index - 1]
  return item === undefined ? 'worktree-row-project' : `worktree-row-${item.id}`
}

function WorktreeRow({
  rowId,
  selected,
  title,
  titleMono,
  subtitle,
  stats,
  terminals,
  claudeAvailable,
  cursorAvailable,
  canRename,
  canDiscard,
  currentName,
  onSelect,
  onKeyDown,
  onOpen,
  onRename,
  onDiscard
}: {
  rowId: string
  selected: boolean
  title: string
  titleMono: boolean
  subtitle: string | null
  stats: React.JSX.Element | null
  terminals: number
  claudeAvailable: boolean
  cursorAvailable: boolean
  canRename: boolean
  canDiscard: boolean
  currentName: string
  onSelect: () => void
  onKeyDown: (event: KeyboardEvent<HTMLButtonElement>) => void
  onOpen: (provider: ProviderId) => void
  onRename?: (name: string) => void
  onDiscard?: () => void
}): React.JSX.Element {
  const [mode, setMode] = useState<'idle' | 'rename' | 'confirm'>('idle')
  const [draft, setDraft] = useState(currentName)
  const inputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    if (mode !== 'rename') return
    inputRef.current?.focus()
    inputRef.current?.select()
  }, [mode])

  function saveRename(): void {
    const next = draft.trim()
    setMode('idle')
    if (next === '' || next === currentName) {
      setDraft(currentName)
      return
    }
    onRename?.(next)
  }

  if (mode === 'confirm') {
    return (
      <li className="project-worktree-item">
        <div className="project-chat-confirm" role="group" aria-label={`Discard ${title}?`}>
          <span className="project-chat-confirm-text">Discard this worktree?</span>
          <button
            type="button"
            className="btn btn-quiet btn-compact is-danger-label"
            onClick={() => {
              setMode('idle')
              onDiscard?.()
            }}
          >
            Discard
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
      <li className="project-worktree-item">
        <input
          ref={inputRef}
          className="project-worktree-rename"
          value={draft}
          aria-label={`Rename ${title}`}
          maxLength={40}
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
              setDraft(currentName)
              setMode('idle')
            }
          }}
        />
      </li>
    )
  }

  return (
    <li className="project-worktree-item">
      <button
        id={rowId}
        type="button"
        className={`project-worktree-row${selected ? ' is-selected' : ''}`}
        title={title}
        aria-current={selected ? 'true' : undefined}
        onClick={onSelect}
        onKeyDown={onKeyDown}
      >
        <span className="project-worktree-body">
          <span className={titleMono ? 'project-worktree-id' : 'project-worktree-name'}>
            {title}
          </span>
          {subtitle !== null ? <span className="project-worktree-sub">{subtitle}</span> : null}
          {stats !== null || terminals > 0 ? (
            <span className="project-worktree-meta">
              {stats}
              {terminals > 0 ? (
                <span className="project-worktree-terms">
                  {terminals === 1 ? '1 terminal' : `${String(terminals)} terminals`}
                </span>
              ) : null}
            </span>
          ) : null}
        </span>
      </button>
      <div className="project-worktree-actions">
        {claudeAvailable ? (
          <button
            type="button"
            className="btn btn-quiet btn-compact"
            title="Open Claude here"
            onClick={() => {
              onOpen('claude')
            }}
          >
            + Claude
          </button>
        ) : null}
        {cursorAvailable ? (
          <button
            type="button"
            className="btn btn-quiet btn-compact"
            title="Open Cursor here"
            onClick={() => {
              onOpen('cursor')
            }}
          >
            + Cursor
          </button>
        ) : null}
        {canRename ? (
          <button
            type="button"
            className="btn btn-quiet btn-compact"
            title="Rename"
            onClick={() => {
              setDraft(currentName)
              setMode('rename')
            }}
          >
            Rename
          </button>
        ) : null}
        {canDiscard ? (
          <button
            type="button"
            className="btn btn-quiet btn-compact"
            title="Discard"
            onClick={() => {
              setMode('confirm')
            }}
          >
            Discard
          </button>
        ) : null}
      </div>
    </li>
  )
}

function WorktreesSection({
  branch,
  changes,
  terminals,
  selectedWorktree,
  claudeAvailable,
  cursorAvailable,
  onSelect,
  onCreate,
  onRename,
  onDiscard,
  onOpen
}: {
  branch: string | null
  changes: ChangeSet[]
  terminals: readonly TerminalInfo[]
  selectedWorktree: string | null
  claudeAvailable: boolean
  cursorAvailable: boolean
  onSelect: (id: string | null) => void
  onCreate: (name?: string) => void
  onRename: (id: string, name: string) => void
  onDiscard: (id: string) => void
  onOpen: (provider: ProviderId, worktree: string | null) => void
}): React.JSX.Element {
  const [creating, setCreating] = useState(false)
  const [draft, setDraft] = useState('')
  const createRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    if (!creating) return
    createRef.current?.focus()
  }, [creating])

  function finishCreate(save: boolean): void {
    const next = draft.trim()
    setCreating(false)
    setDraft('')
    if (save) onCreate(next === '' ? undefined : next)
  }

  function onRowKeyDown(event: KeyboardEvent<HTMLButtonElement>, index: number): void {
    if (event.key === 'Enter') {
      event.preventDefault()
      onSelect(index === 0 ? null : (changes[index - 1]?.id ?? null))
      return
    }
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
    event.preventDefault()
    const next = event.key === 'ArrowDown' ? index + 1 : index - 1
    if (next < 0 || next > changes.length) return
    const element = document.getElementById(worktreeRowId(next, changes))
    if (element instanceof HTMLButtonElement) element.focus()
  }

  const folderLabel = branch ?? 'HEAD'

  return (
    <div className="project-worktrees">
      <div className="project-chats-head">
        <span className="project-chats-label">Worktrees</span>
        <button
          type="button"
          className="btn btn-quiet btn-compact"
          title="New worktree"
          aria-label="New worktree"
          onClick={() => {
            setDraft('')
            setCreating(true)
          }}
        >
          +
        </button>
      </div>
      {creating ? (
        <input
          ref={createRef}
          className="project-worktree-rename"
          value={draft}
          aria-label="New worktree name"
          placeholder="Name (optional)"
          maxLength={40}
          onChange={(event) => {
            setDraft(event.target.value)
          }}
          onBlur={() => {
            finishCreate(false)
          }}
          onKeyDown={(event: KeyboardEvent<HTMLInputElement>) => {
            if (event.key === 'Enter') {
              event.preventDefault()
              finishCreate(true)
            } else if (event.key === 'Escape') {
              event.preventDefault()
              finishCreate(false)
            }
          }}
        />
      ) : null}
      <ul className="project-worktrees-list">
        <WorktreeRow
          rowId={worktreeRowId(0, changes)}
          selected={selectedWorktree === null}
          title={folderLabel}
          titleMono={false}
          subtitle="project folder"
          stats={null}
          terminals={countTerminals(terminals, null)}
          claudeAvailable={claudeAvailable}
          cursorAvailable={cursorAvailable}
          canRename={false}
          canDiscard={false}
          currentName=""
          onSelect={() => {
            onSelect(null)
          }}
          onKeyDown={(event) => {
            onRowKeyDown(event, 0)
          }}
          onOpen={(provider) => {
            onSelect(null)
            onOpen(provider, null)
          }}
        />
        {changes.map((item, index) => {
          const label = worktreeLabel(item)
          const named = typeof item.name === 'string' && item.name !== ''
          const noChanges = item.files.length === 0
          return (
            <WorktreeRow
              key={item.id}
              rowId={worktreeRowId(index + 1, changes)}
              selected={selectedWorktree === item.id}
              title={label}
              titleMono={!named}
              subtitle={null}
              stats={
                noChanges ? (
                  <span>no changes yet</span>
                ) : (
                  <span>
                    <span className="stat-add">{`+${String(item.insertions)}`}</span>{' '}
                    <span className="stat-del">{`−${String(item.deletions)}`}</span>
                  </span>
                )
              }
              terminals={countTerminals(terminals, item.id)}
              claudeAvailable={claudeAvailable}
              cursorAvailable={cursorAvailable}
              canRename={true}
              canDiscard={true}
              currentName={named ? (item.name ?? '') : ''}
              onSelect={() => {
                onSelect(item.id)
              }}
              onKeyDown={(event) => {
                onRowKeyDown(event, index + 1)
              }}
              onOpen={(provider) => {
                onSelect(item.id)
                onOpen(provider, item.id)
              }}
              onRename={(name) => {
                onRename(item.id, name)
              }}
              onDiscard={() => {
                onDiscard(item.id)
              }}
            />
          )
        })}
      </ul>
    </div>
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
  changes,
  terminals,
  selectedWorktree,
  claudeAvailable,
  cursorAvailable,
  onAdd,
  onGithubClone,
  onSwitch,
  onRemove,
  onOpenChat,
  onNewChat,
  onRenameChat,
  onRemoveChat,
  onSelectWorktree,
  onCreateWorktree,
  onRenameWorktree,
  onDiscardWorktree,
  onOpenInWorktree
}: {
  projects: ProjectEntry[]
  chats: LeadChat[]
  busy: boolean
  leadBusy: boolean
  now: number
  error: string | null
  githubAvailable: boolean
  changes: ChangeSet[]
  terminals: readonly TerminalInfo[]
  selectedWorktree: string | null
  claudeAvailable: boolean
  cursorAvailable: boolean
  onAdd: () => void
  onGithubClone: (nameWithOwner: string) => Promise<boolean>
  onSwitch: (path: string) => void
  onRemove: (path: string) => void
  onOpenChat: (id: string) => void
  onNewChat: () => void
  onRenameChat: (id: string, title: string) => void
  onRemoveChat: (id: string) => void
  onSelectWorktree: (id: string | null) => void
  onCreateWorktree: (name?: string) => void
  onRenameWorktree: (id: string, name: string) => void
  onDiscardWorktree: (id: string) => void
  onOpenInWorktree: (provider: ProviderId, worktree: string | null) => void
}): React.JSX.Element {
  const shown = chats.slice(0, 30)
  const [githubOpen, setGithubOpen] = useState(false)
  const [repos, setRepos] = useState<GithubRepo[]>([])
  const [reposLoading, setReposLoading] = useState(false)
  const [filter, setFilter] = useState('')
  const [cloning, setCloning] = useState<string | null>(null)

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

  async function cloneRepo(nameWithOwner: string): Promise<void> {
    if (cloning !== null) return
    setCloning(nameWithOwner)
    try {
      const ok = await onGithubClone(nameWithOwner)
      if (ok) {
        setGithubOpen(false)
        setFilter('')
      }
    } finally {
      setCloning(null)
    }
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
      {githubOpen ? (
        <GithubPanel
          repos={repos}
          loading={reposLoading}
          cloning={cloning}
          filter={filter}
          now={now}
          onFilter={setFilter}
          onClone={(name) => {
            void cloneRepo(name)
          }}
          onClose={() => {
            if (cloning !== null) return
            setGithubOpen(false)
            setFilter('')
          }}
        />
      ) : null}
      <ul className="project-list">
        {projects.map((entry) => {
          const name = folderName(entry.path)
          const active = entry.active
          return (
            <li key={entry.path} className="project-item">
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
                <FolderGlyph />
                <span className="project-row-body">
                  <span className="project-name">{name}</span>
                  <span className="project-row-meta">
                    {entry.isRepo && entry.branch !== null ? (
                      <span className="project-branch">{entry.branch}</span>
                    ) : (
                      <span className="project-note">not a git repository</span>
                    )}
                  </span>
                  {active && busy ? (
                    <span className="status status-running project-row-run">
                      <span className="status-dot" aria-hidden="true" />
                      running
                    </span>
                  ) : null}
                </span>
                {entry.changes > 0 ? (
                  <span
                    className="project-row-changes"
                    aria-label={`${String(entry.changes)} changes waiting`}
                  >
                    {entry.changes}
                  </span>
                ) : null}
              </button>
              {!active ? (
                <button
                  type="button"
                  className="project-remove"
                  aria-label={`Remove ${name} from the list`}
                  title="Remove from this list (does not delete the folder)"
                  onClick={() => {
                    onRemove(entry.path)
                  }}
                >
                  ×
                </button>
              ) : null}
              {active && entry.isRepo ? (
                <WorktreesSection
                  branch={entry.branch}
                  changes={changes}
                  terminals={terminals}
                  selectedWorktree={selectedWorktree}
                  claudeAvailable={claudeAvailable}
                  cursorAvailable={cursorAvailable}
                  onSelect={onSelectWorktree}
                  onCreate={onCreateWorktree}
                  onRename={onRenameWorktree}
                  onDiscard={onDiscardWorktree}
                  onOpen={onOpenInWorktree}
                />
              ) : null}
              {active ? (
                <div className="project-chats">
                  <div className="project-chats-head">
                    <span className="project-chats-label">Chats</span>
                    <button
                      type="button"
                      className="btn btn-quiet btn-compact"
                      disabled={leadBusy || !chats.some((chat) => chat.active)}
                      onClick={onNewChat}
                    >
                      New chat
                    </button>
                  </div>
                  <ul className="project-chat-list">
                    {shown.map((chat) => (
                      <ChatRow
                        key={chat.id}
                        chat={chat}
                        leadBusy={leadBusy}
                        now={now}
                        onOpen={onOpenChat}
                        onRename={onRenameChat}
                        onRemove={onRemoveChat}
                      />
                    ))}
                  </ul>
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
    </aside>
  )
}

export default ProjectBar
