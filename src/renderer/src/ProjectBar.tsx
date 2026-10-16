import { useEffect, useId, useRef, useState, type KeyboardEvent, type MouseEvent } from 'react'
import BotAvatar from './BotAvatar'
import SidebarMenu, { type MenuEntry } from './SidebarMenu'
import { Star } from './Star'
import {
  arrangeCards,
  cardStatus,
  readOptions,
  type Card,
  type CardInput,
  type CardStatus,
  type SidebarOptions
} from './sidebar-cards'
import { worktreeLabel, worktreeNameFromTask } from './worktree-label'

type ProjectEntry = Awaited<ReturnType<Window['api']['listProjects']>>[number]
type LeadChat = Awaited<ReturnType<Window['api']['listLeadChats']>>[number]
type GithubRepo = Awaited<ReturnType<Window['api']['githubRepos']>>[number]
type ChangeSet = Awaited<ReturnType<Window['api']['listChanges']>>[number]
type TerminalInfo = Awaited<ReturnType<Window['api']['listTerminals']>>[number]
type JobRecord = Awaited<ReturnType<Window['api']['listJobs']>>[number]
type ProviderId = Parameters<Window['api']['openTerminal']>[0]
type WorktreeId = string | null
type WorktreeSelectMode = 'only' | 'toggle' | 'range'

function worktreeKeys(changes: readonly ChangeSet[]): WorktreeId[] {
  return [null, ...changes.map((item) => item.id)]
}

function clickSelectMode(event: {
  shiftKey: boolean
  ctrlKey: boolean
  metaKey: boolean
}): WorktreeSelectMode {
  if (event.shiftKey) return 'range'
  if (event.ctrlKey || event.metaKey) return 'toggle'
  return 'only'
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

/** Upstream distance, uncommitted files and the lines waiting in changes; nothing when all zero. */
function ProjectStatus({ entry }: { entry: ProjectEntry }): React.JSX.Element | null {
  const upstream = entry.ahead > 0 || entry.behind > 0
  const lines = entry.insertions > 0 || entry.deletions > 0
  if (!entry.isRepo || (!upstream && entry.dirty === 0 && !lines)) return null
  const said: string[] = []
  if (entry.ahead > 0) said.push(`${String(entry.ahead)} ahead`)
  if (entry.behind > 0) said.push(`${String(entry.behind)} behind`)
  if (entry.dirty > 0) said.push(`${String(entry.dirty)} uncommitted`)
  if (lines)
    said.push(`${String(entry.insertions)} lines added, ${String(entry.deletions)} removed`)
  return (
    <span className="project-status" aria-label={said.join(', ')}>
      {upstream ? (
        <span className="project-status-item" title="Commits ahead of and behind upstream">
          {entry.ahead > 0 ? `↑${String(entry.ahead)}` : null}
          {entry.ahead > 0 && entry.behind > 0 ? ' ' : null}
          {entry.behind > 0 ? `↓${String(entry.behind)}` : null}
        </span>
      ) : null}
      {entry.dirty > 0 ? (
        <span className="project-status-item" title="Uncommitted files in the project folder">
          {`${String(entry.dirty)} uncommitted`}
        </span>
      ) : null}
      {lines ? (
        <span className="project-status-item" title="Lines in changes waiting for review">
          <span className="stat-add">{`+${String(entry.insertions)}`}</span>{' '}
          <span className="stat-del">{`−${String(entry.deletions)}`}</span>
        </span>
      ) : null}
    </span>
  )
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
        {chat.direct !== undefined ? (
          <BotAvatar bot={chat.direct} state="idle" size={14} title={`Just ${chat.direct}`} />
        ) : null}
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

function terminalsIn(terminals: readonly TerminalInfo[], change: string | null): TerminalInfo[] {
  return terminals.filter((info) =>
    change === null ? info.change === undefined : info.change === change
  )
}

function worktreeJobRunning(jobs: readonly JobRecord[], id: string | null): boolean {
  return jobs.some(
    (job) =>
      (id === null ? job.change === undefined : job.change === id) &&
      (job.status === 'queued' || job.status === 'running')
  )
}

const STATUS_TEXT: Record<CardStatus, string> = {
  working: 'agent working',
  unread: 'new output since you looked',
  idle: 'quiet'
}

function readJson(key: string): unknown {
  try {
    const raw = window.localStorage.getItem(key)
    return raw === null ? null : (JSON.parse(raw) as unknown)
  } catch {
    return null
  }
}

function writeJson(key: string, value: unknown): void {
  try {
    window.localStorage.setItem(key, JSON.stringify(value))
  } catch {
    // Storage blocked: the choice lasts for this session only.
  }
}

/** Pins and "last looked at" times, per project, kept in this browser's storage. */
interface WorktreeMarks {
  pinned: string[]
  seen: Record<string, number>
}

function readMarks(project: string): WorktreeMarks {
  const raw = readJson(`legate.sidebar.marks:${project}`)
  const value = typeof raw === 'object' && raw !== null ? (raw as Record<string, unknown>) : {}
  const pinned = Array.isArray(value.pinned)
    ? value.pinned.filter((item): item is string => typeof item === 'string')
    : []
  const seen: Record<string, number> = {}
  if (typeof value.seen === 'object' && value.seen !== null) {
    for (const [key, at] of Object.entries(value.seen as Record<string, unknown>)) {
      if (typeof at === 'number') seen[key] = at
    }
  }
  return { pinned, seen }
}

/** Storage key of a worktree: its id, or "" for the project folder. */
function markKey(key: string | null): string {
  return key ?? ''
}

function MoreGlyph(): React.JSX.Element {
  return (
    <svg width="14" height="14" viewBox="0 0 16 16" aria-hidden="true" focusable="false">
      <path fill="currentColor" d="M3 7h2v2H3zM7 7h2v2H7zM11 7h2v2h-2z" />
    </svg>
  )
}

function WorktreeCard({
  card,
  rowId,
  compact,
  terminals,
  stats,
  menuLabel,
  onSelect,
  onKeyDown,
  onMenu
}: {
  card: Card
  rowId: string
  compact: boolean
  terminals: readonly TerminalInfo[]
  stats: React.JSX.Element | null
  menuLabel: string
  onSelect: (event: MouseEvent<HTMLButtonElement>) => void
  onKeyDown: (event: KeyboardEvent<HTMLButtonElement>) => void
  onMenu: (at: { x: number; y: number }) => void
}): React.JSX.Element {
  const agents = terminals.filter((info) => info.status === 'running')
  return (
    <li
      className={`project-worktree-item is-${card.status}${compact ? ' is-compact' : ''}`}
      onContextMenu={(event) => {
        event.preventDefault()
        onMenu({ x: event.clientX, y: event.clientY })
      }}
    >
      <button
        id={rowId}
        type="button"
        className={`project-worktree-row${card.selected ? ' is-selected' : ''}`}
        title={card.key === null ? 'Project folder' : card.title}
        aria-current={card.selected ? 'true' : undefined}
        onClick={onSelect}
        onKeyDown={onKeyDown}
      >
        <span
          className={`worktree-dot is-${card.status}`}
          role="img"
          aria-label={STATUS_TEXT[card.status]}
          title={STATUS_TEXT[card.status]}
        />
        <span className="project-worktree-body">
          <span className="project-worktree-line">
            <span
              className={
                card.key !== null && card.title === card.key
                  ? 'project-worktree-id'
                  : 'project-worktree-name'
              }
            >
              {card.title}
            </span>
            {card.pinned ? (
              <span className="worktree-pin" aria-label="pinned" title="Pinned">
                ▲
              </span>
            ) : null}
            {agents.length > 0 ? (
              <span className="worktree-agents" aria-label={`${String(agents.length)} agents open`}>
                {agents.slice(0, 3).map((info) =>
                  info.provider === 'claude' || info.provider === 'cursor' ? (
                    <BotAvatar
                      key={info.id}
                      bot={info.provider}
                      state={card.status === 'working' ? 'working' : 'idle'}
                      size={14}
                    />
                  ) : (
                    <span key={info.id} className="worktree-shell" aria-hidden="true">
                      $
                    </span>
                  )
                )}
              </span>
            ) : null}
          </span>
          {!compact ? (
            <span className="project-worktree-meta">
              {card.key === null ? <span>project folder</span> : null}
              {card.branch !== null ? (
                <span className="project-worktree-branch">{card.branch}</span>
              ) : null}
              {stats}
            </span>
          ) : null}
        </span>
      </button>
      <button
        type="button"
        className="worktree-more"
        aria-label={menuLabel}
        title="More actions"
        aria-haspopup="menu"
        onClick={(event) => {
          const rect = event.currentTarget.getBoundingClientRect()
          onMenu({ x: rect.left, y: rect.bottom })
        }}
      >
        <MoreGlyph />
      </button>
    </li>
  )
}

function WorktreeRenameRow({
  title,
  currentName,
  onDone
}: {
  title: string
  currentName: string
  onDone: (name: string | null) => void
}): React.JSX.Element {
  const [draft, setDraft] = useState(currentName)
  const inputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    inputRef.current?.focus()
    inputRef.current?.select()
  }, [])

  function save(): void {
    const next = draft.trim()
    onDone(next === '' || next === currentName ? null : next)
  }

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
        onBlur={save}
        onKeyDown={(event: KeyboardEvent<HTMLInputElement>) => {
          if (event.key === 'Enter') {
            event.preventDefault()
            save()
          } else if (event.key === 'Escape') {
            event.preventDefault()
            onDone(null)
          }
        }}
      />
    </li>
  )
}

function WorktreesSection({
  project,
  branch,
  changes,
  jobs,
  terminals,
  activity,
  options,
  filter,
  selectedWorktrees,
  claudeAvailable,
  cursorAvailable,
  onSelect,
  onSelectAll,
  onCreate,
  onRename,
  onDiscard,
  onOpen,
  onReveal
}: {
  project: string
  branch: string | null
  changes: ChangeSet[]
  jobs: readonly JobRecord[]
  terminals: readonly TerminalInfo[]
  activity: Readonly<Record<string, number>>
  options: SidebarOptions
  filter: string
  selectedWorktrees: ReadonlySet<WorktreeId>
  claudeAvailable: boolean
  cursorAvailable: boolean
  onSelect: (id: WorktreeId, mode?: WorktreeSelectMode) => void
  onSelectAll: () => void
  onCreate: (
    name: string | undefined,
    count: number,
    start: ProviderId | null,
    prompt?: string
  ) => void
  onRename: (id: string, name: string) => void
  onDiscard: (id: string) => void
  onOpen: (provider: ProviderId, worktree: string | null) => void
  onReveal: (worktree: string | null) => void
}): React.JSX.Element {
  const [creating, setCreating] = useState(false)
  const [draft, setDraft] = useState('')
  const [task, setTask] = useState('')
  const [howMany, setHowMany] = useState(1)
  const [start, setStart] = useState<'' | ProviderId>('')
  const [marks, setMarks] = useState<WorktreeMarks>(() => readMarks(project))
  const [marksFor, setMarksFor] = useState(project)
  const [menu, setMenu] = useState<{ key: WorktreeId; at: { x: number; y: number } } | null>(null)
  const [renaming, setRenaming] = useState<string | null>(null)
  const [confirming, setConfirming] = useState<string | null>(null)
  const [clock, setClock] = useState(() => Date.now())
  const createRef = useRef<HTMLInputElement>(null)
  const canTask = start === 'claude' || start === 'cursor'

  // Another project's pins: reload while rendering, the pattern React recommends over an effect.
  if (marksFor !== project) {
    setMarksFor(project)
    setMarks(readMarks(project))
  }

  useEffect(() => {
    if (!creating) return
    createRef.current?.focus()
  }, [creating])

  // Re-read the clock so "working" turns back to quiet once output stops.
  useEffect(() => {
    const timer = window.setInterval(() => {
      setClock(Date.now())
    }, 2000)
    return () => {
      window.clearInterval(timer)
    }
  }, [])

  function saveMarks(next: WorktreeMarks): void {
    setMarks(next)
    writeJson(`legate.sidebar.marks:${project}`, next)
  }

  function markSeen(keys: Iterable<WorktreeId>): void {
    const seen = { ...marks.seen }
    // Seen up to its latest output; the clock covers worktrees with none.
    for (const key of keys) {
      const card = cards.find((entry) => entry.key === key)
      seen[markKey(key)] = Math.max(clock, card?.lastActive ?? 0)
    }
    saveMarks({ ...marks, seen })
  }

  function select(id: WorktreeId, mode: WorktreeSelectMode): void {
    markSeen([...selectedWorktrees, id])
    onSelect(id, mode)
  }

  function finishCreate(save: boolean): void {
    const next = draft.trim()
    const count = Math.min(6, Math.max(1, Math.round(howMany)))
    const agent = start === '' ? null : start
    const taskText = task.trim()
    const named = next !== '' ? next : taskText !== '' ? worktreeNameFromTask(taskText) : undefined
    const prompt = agent !== null && taskText !== '' ? taskText : undefined
    setCreating(false)
    setDraft('')
    setTask('')
    setHowMany(1)
    setStart('')
    if (save)
      onCreate(named === undefined || named === '' ? undefined : named, count, agent, prompt)
  }

  const inputs: CardInput[] = [
    {
      key: null,
      title: branch ?? 'HEAD',
      branch: null,
      terminalIds: terminalsIn(terminals, null).map((info) => info.id),
      jobRunning: worktreeJobRunning(jobs, null),
      hasChanges: false,
      createdIndex: 0,
      pinned: marks.pinned.includes(''),
      seenAt: marks.seen[''] ?? 0,
      selected: selectedWorktrees.has(null)
    },
    ...changes.map((item, index) => ({
      key: item.id,
      title: worktreeLabel(item),
      branch: item.branch,
      terminalIds: terminalsIn(terminals, item.id).map((info) => info.id),
      jobRunning: worktreeJobRunning(jobs, item.id),
      hasChanges: item.files.length > 0,
      createdIndex: index + 1,
      pinned: marks.pinned.includes(item.id),
      seenAt: marks.seen[item.id] ?? 0,
      selected: selectedWorktrees.has(item.id)
    }))
  ]
  const cards = inputs.map((input) => cardStatus(input, activity, clock))
  const groups = arrangeCards(cards, options, filter)
  const order = groups.flatMap((group) => group.cards)
  const keys = worktreeKeys(changes)
  const allSelected = keys.every((key) => selectedWorktrees.has(key))
  const shownCount = selectedWorktrees.size
  const working = cards.filter((card) => card.status === 'working').length

  function rowId(key: WorktreeId): string {
    return key === null ? 'worktree-row-project' : `worktree-row-${key}`
  }

  function onRowKeyDown(event: KeyboardEvent<HTMLButtonElement>, index: number): void {
    const card = order[index]
    if (card === undefined) return
    if (event.key === 'Enter') {
      event.preventDefault()
      select(card.key, 'only')
      return
    }
    if (event.key === ' ') {
      event.preventDefault()
      select(card.key, 'toggle')
      return
    }
    if (event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) {
      event.preventDefault()
      const rect = event.currentTarget.getBoundingClientRect()
      setMenu({ key: card.key, at: { x: rect.left + 20, y: rect.bottom } })
      return
    }
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
    event.preventDefault()
    const next = order[event.key === 'ArrowDown' ? index + 1 : index - 1]
    if (next === undefined) return
    const element = document.getElementById(rowId(next.key))
    if (element instanceof HTMLButtonElement) element.focus()
  }

  function statsFor(card: Card): React.JSX.Element | null {
    const item = card.key === null ? undefined : changes.find((change) => change.id === card.key)
    const open = terminalsIn(terminals, card.key).length
    const parts: React.JSX.Element[] = []
    if (item?.setup === 'running') parts.push(<span key="setup">setting up…</span>)
    if (item?.setup === 'failed')
      parts.push(
        <span key="setup" className="setup-failed">
          setup failed
        </span>
      )
    if (item !== undefined) {
      parts.push(
        item.files.length === 0 ? (
          <span key="diff">no changes yet</span>
        ) : (
          <span key="diff">
            <span className="stat-add">{`+${String(item.insertions)}`}</span>{' '}
            <span className="stat-del">{`−${String(item.deletions)}`}</span>
          </span>
        )
      )
    }
    if (open > 0)
      parts.push(
        <span key="terms" className="project-worktree-terms">
          {open === 1 ? '1 terminal' : `${String(open)} terminals`}
        </span>
      )
    return parts.length === 0 ? null : <>{parts}</>
  }

  function menuEntries(key: WorktreeId): MenuEntry[] {
    const item = key === null ? undefined : changes.find((change) => change.id === key)
    const path = item?.path ?? project
    const pinned = marks.pinned.includes(markKey(key))
    const card = cards.find((entry) => entry.key === key)
    const blocked = key !== null && worktreeJobRunning(jobs, key)
    const entries: MenuEntry[] = [
      {
        label: 'Open Claude here',
        disabled: !claudeAvailable,
        onSelect: () => {
          select(key, 'only')
          onOpen('claude', key)
        }
      },
      {
        label: 'Open Cursor here',
        disabled: !cursorAvailable,
        onSelect: () => {
          select(key, 'only')
          onOpen('cursor', key)
        }
      },
      {
        label: pinned ? 'Unpin' : 'Pin to top',
        separator: true,
        onSelect: () => {
          const id = markKey(key)
          saveMarks({
            ...marks,
            pinned: pinned ? marks.pinned.filter((entry) => entry !== id) : [...marks.pinned, id]
          })
        }
      },
      card?.status === 'unread'
        ? {
            label: 'Mark as read',
            onSelect: () => {
              markSeen([key])
            }
          }
        : {
            label: 'Mark as unread',
            disabled: card === undefined || card.lastActive === 0,
            onSelect: () => {
              saveMarks({ ...marks, seen: { ...marks.seen, [markKey(key)]: 0 } })
            }
          },
      {
        label: 'Open folder',
        separator: true,
        onSelect: () => {
          onReveal(key)
        }
      },
      {
        label: 'Copy path',
        onSelect: () => {
          void navigator.clipboard.writeText(path)
        }
      }
    ]
    if (item !== undefined) {
      entries.push(
        {
          label: 'Copy branch name',
          onSelect: () => {
            void navigator.clipboard.writeText(item.branch)
          }
        },
        {
          label: 'Rename…',
          separator: true,
          onSelect: () => {
            setRenaming(item.id)
          }
        },
        {
          label: blocked ? 'Discard (a job is running)' : 'Discard worktree…',
          danger: true,
          disabled: blocked,
          onSelect: () => {
            setConfirming(item.id)
          }
        }
      )
    }
    return entries
  }

  function renderCard(card: Card, index: number): React.JSX.Element {
    const item = card.key === null ? undefined : changes.find((change) => change.id === card.key)
    if (item !== undefined && renaming === item.id) {
      return (
        <WorktreeRenameRow
          key={item.id}
          title={card.title}
          currentName={item.name ?? ''}
          onDone={(name) => {
            setRenaming(null)
            if (name !== null) onRename(item.id, name)
          }}
        />
      )
    }
    if (item !== undefined && confirming === item.id) {
      return (
        <li key={item.id} className="project-worktree-item">
          <div className="project-chat-confirm" role="group" aria-label={`Discard ${card.title}?`}>
            <span className="project-chat-confirm-text">Discard this worktree?</span>
            <button
              type="button"
              className="btn btn-quiet btn-compact is-danger-label"
              autoFocus
              onClick={() => {
                setConfirming(null)
                onDiscard(item.id)
              }}
            >
              Discard
            </button>
            <button
              type="button"
              className="btn btn-quiet btn-compact"
              onClick={() => {
                setConfirming(null)
              }}
            >
              Cancel
            </button>
          </div>
        </li>
      )
    }
    return (
      <WorktreeCard
        key={card.key ?? ''}
        card={card}
        rowId={rowId(card.key)}
        compact={options.layout === 'compact'}
        terminals={terminalsIn(terminals, card.key)}
        stats={statsFor(card)}
        menuLabel={`Actions for ${card.title}`}
        onSelect={(event) => {
          select(card.key, clickSelectMode(event))
        }}
        onKeyDown={(event) => {
          onRowKeyDown(event, index)
        }}
        onMenu={(at) => {
          setMenu({ key: card.key, at })
        }}
      />
    )
  }

  const position = new Map(order.map((card, at) => [card.key, at]))
  return (
    <div className="project-worktrees">
      <div className="project-chats-head">
        <span className="project-chats-label">
          Worktrees
          {working > 0 ? (
            <span className="project-worktree-shown">{`${String(working)} working`}</span>
          ) : null}
          {shownCount > 1 ? (
            <span className="project-worktree-shown">{`${String(shownCount)} shown`}</span>
          ) : null}
        </span>
        <span className="head-actions">
          <button
            type="button"
            className="btn btn-quiet btn-compact"
            title="Show every worktree side by side"
            aria-pressed={allSelected}
            onClick={onSelectAll}
          >
            All
          </button>
          <button
            type="button"
            className="btn btn-quiet btn-compact"
            title="New worktree"
            aria-label="New worktree"
            onClick={() => {
              setDraft('')
              setTask('')
              setHowMany(1)
              setStart('')
              setCreating(true)
            }}
          >
            +
          </button>
        </span>
      </div>
      {creating ? (
        <form
          className="project-worktree-create"
          onSubmit={(event) => {
            event.preventDefault()
            finishCreate(true)
          }}
          onKeyDown={(event) => {
            if (event.key === 'Escape') {
              event.preventDefault()
              finishCreate(false)
            }
          }}
        >
          <label className="project-worktree-field">
            Name (optional)
            <input
              ref={createRef}
              className="project-worktree-rename"
              value={draft}
              aria-label="New worktree name"
              maxLength={40}
              onChange={(event) => {
                setDraft(event.target.value)
              }}
            />
          </label>
          <label className="project-worktree-field">
            Task
            <textarea
              className="project-worktree-task"
              value={task}
              rows={2}
              maxLength={8000}
              disabled={!canTask}
              placeholder={canTask ? 'What should the agent do? (optional)' : undefined}
              aria-label="Task"
              onChange={(event) => {
                setTask(event.target.value)
              }}
              onKeyDown={(event) => {
                if (event.key === 'Enter' && event.ctrlKey) {
                  event.preventDefault()
                  finishCreate(true)
                }
              }}
            />
            {!canTask ? (
              <span className="project-worktree-task-hint">
                Choose an agent under Start to give it a task.
              </span>
            ) : null}
          </label>
          <label className="project-worktree-how">
            How many
            <input
              type="number"
              min={1}
              max={6}
              value={howMany}
              aria-label="How many"
              onChange={(event) => {
                const value = Number(event.target.value)
                if (!Number.isFinite(value)) return
                setHowMany(value)
              }}
            />
          </label>
          <label className="project-worktree-start">
            Start
            <select
              value={start}
              aria-label="Start"
              onChange={(event) => {
                const value = event.target.value
                if (value === 'claude' || value === 'cursor') setStart(value)
                else setStart('')
              }}
            >
              <option value="">nothing</option>
              {claudeAvailable ? <option value="claude">Claude</option> : null}
              {cursorAvailable ? <option value="cursor">Cursor</option> : null}
            </select>
          </label>
        </form>
      ) : null}
      <div className="project-worktrees-list">
        {groups.length === 0 ? <p className="project-empty">No worktrees match.</p> : null}
        {groups.map((group) => (
          <div key={group.label ?? 'all'} role="group" aria-label={group.label ?? 'Worktrees'}>
            {group.label !== null ? (
              <div className="worktree-group-label">
                {group.label}
                <span className="worktree-group-count">{group.cards.length}</span>
              </div>
            ) : null}
            <ul className="worktree-group-list">
              {group.cards.map((card) => renderCard(card, position.get(card.key) ?? 0))}
            </ul>
          </div>
        ))}
      </div>
      {menu !== null ? (
        <SidebarMenu
          label="Worktree actions"
          at={menu.at}
          entries={menuEntries(menu.key)}
          onClose={() => {
            setMenu(null)
          }}
        />
      ) : null}
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
  jobs,
  terminals,
  selectedWorktrees,
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
  onSelectAllWorktrees,
  onCreateWorktree,
  onRenameWorktree,
  onDiscardWorktree,
  onOpenInWorktree,
  activity,
  onReveal,
  onRevealProject,
  onOpenSettings
}: {
  projects: ProjectEntry[]
  chats: LeadChat[]
  busy: boolean
  leadBusy: boolean
  now: number
  error: string | null
  githubAvailable: boolean
  changes: ChangeSet[]
  jobs: readonly JobRecord[]
  terminals: readonly TerminalInfo[]
  selectedWorktrees: ReadonlySet<WorktreeId>
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
  onSelectWorktree: (id: WorktreeId, mode?: WorktreeSelectMode) => void
  onSelectAllWorktrees: () => void
  onCreateWorktree: (
    name: string | undefined,
    count: number,
    start: ProviderId | null,
    prompt?: string
  ) => void
  onRenameWorktree: (id: string, name: string) => void
  onDiscardWorktree: (id: string) => void
  onOpenInWorktree: (provider: ProviderId, worktree: string | null) => void
  /** Terminal id → epoch ms of its latest output. */
  activity: Readonly<Record<string, number>>
  /** Open the active project's folder (null) or a worktree's folder in the file manager. */
  onReveal: (worktree: string | null) => void
  /** Open a listed project's folder in the file manager. */
  onRevealProject: (path: string) => void
  onOpenSettings: () => void
}): React.JSX.Element {
  const [search, setSearch] = useState('')
  const [options, setOptions] = useState<SidebarOptions>(() =>
    readOptions(readJson('legate.sidebar.options'))
  )
  const [optionsAt, setOptionsAt] = useState<{ x: number; y: number } | null>(null)
  const [projectMenu, setProjectMenu] = useState<{
    path: string
    at: { x: number; y: number }
  } | null>(null)
  const searchRef = useRef<HTMLInputElement>(null)
  const needle = search.trim().toLowerCase()
  const shownProjects =
    needle === ''
      ? projects
      : projects.filter((entry) => entry.active || entry.path.toLowerCase().includes(needle))
  const shown = chats
    .filter((chat) => needle === '' || chat.title.toLowerCase().includes(needle))
    .slice(0, 30)

  useEffect(() => {
    const onKey = (event: globalThis.KeyboardEvent): void => {
      if (!(event.ctrlKey || event.metaKey) || event.shiftKey || event.altKey) return
      if (event.key.toLowerCase() !== 'k') return
      event.preventDefault()
      searchRef.current?.focus()
      searchRef.current?.select()
    }
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('keydown', onKey)
    }
  }, [])

  function setOption(patch: Partial<SidebarOptions>): void {
    const next = { ...options, ...patch }
    setOptions(next)
    writeJson('legate.sidebar.options', next)
  }

  function optionEntries(): MenuEntry[] {
    return [
      { label: 'Group by', heading: true },
      {
        label: 'Nothing',
        checked: options.groupBy === 'none',
        onSelect: () => setOption({ groupBy: 'none' })
      },
      {
        label: 'Status',
        checked: options.groupBy === 'status',
        onSelect: () => setOption({ groupBy: 'status' })
      },
      { label: 'Sort by', heading: true, separator: true },
      {
        label: 'Created',
        checked: options.sortBy === 'created',
        onSelect: () => setOption({ sortBy: 'created' })
      },
      {
        label: 'Recent activity',
        checked: options.sortBy === 'recent',
        onSelect: () => setOption({ sortBy: 'recent' })
      },
      {
        label: 'Name',
        checked: options.sortBy === 'name',
        onSelect: () => setOption({ sortBy: 'name' })
      },
      { label: 'Cards', heading: true, separator: true },
      {
        label: 'Detailed',
        checked: options.layout === 'detailed',
        onSelect: () => setOption({ layout: 'detailed' })
      },
      {
        label: 'Compact',
        checked: options.layout === 'compact',
        onSelect: () => setOption({ layout: 'compact' })
      },
      {
        label: 'Hide quiet worktrees',
        separator: true,
        checked: options.hideClean,
        title: 'Hide worktrees with no changes, no terminals and nothing happening',
        onSelect: () => setOption({ hideClean: !options.hideClean })
      }
    ]
  }

  function projectEntries(path: string): MenuEntry[] {
    const entry = projects.find((item) => item.path === path)
    const active = entry?.active === true
    return [
      {
        label: active ? 'Open folder' : 'Switch to this project',
        onSelect: () => {
          if (active) onReveal(null)
          else onSwitch(path)
        }
      },
      ...(active
        ? []
        : [
            {
              label: 'Open folder',
              onSelect: () => {
                onRevealProject(path)
              }
            }
          ]),
      {
        label: 'Copy path',
        onSelect: () => {
          void navigator.clipboard.writeText(path)
        }
      },
      {
        label: 'Remove from list',
        separator: true,
        danger: true,
        disabled: active,
        title: active
          ? 'Switch to another project first'
          : 'Removes it from this list. The folder is not deleted.',
        onSelect: () => {
          onRemove(path)
        }
      }
    ]
  }
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
      <div className="project-search-row">
        <input
          ref={searchRef}
          className="project-search"
          type="search"
          value={search}
          placeholder="Search  Ctrl+K"
          aria-label="Search projects, worktrees and chats"
          onChange={(event) => {
            setSearch(event.target.value)
          }}
          onKeyDown={(event) => {
            if (event.key === 'Escape' && search !== '') {
              event.preventDefault()
              event.stopPropagation()
              setSearch('')
            }
          }}
        />
        <button
          type="button"
          className="btn btn-quiet btn-compact project-options"
          aria-label="View options"
          title="Group, sort and card options"
          aria-haspopup="menu"
          aria-expanded={optionsAt !== null}
          onClick={(event) => {
            const rect = event.currentTarget.getBoundingClientRect()
            setOptionsAt({ x: rect.left, y: rect.bottom })
          }}
        >
          <svg width="14" height="14" viewBox="0 0 16 16" aria-hidden="true" focusable="false">
            <path
              fill="currentColor"
              d="M1 3h8v1.5H1zM12 3h3v1.5h-3zM9.5 1.5H11v4H9.5zM1 11.5h3V13H1zM7 11.5h8V13H7zM4.5 10H6v4H4.5zM1 7.25h11v1.5H1zM13.5 6.25H15v3.5h-1.5z"
            />
          </svg>
        </button>
      </div>
      {optionsAt !== null ? (
        <SidebarMenu
          label="View options"
          at={optionsAt}
          entries={optionEntries()}
          onClose={() => {
            setOptionsAt(null)
          }}
        />
      ) : null}
      {projectMenu !== null ? (
        <SidebarMenu
          label="Project actions"
          at={projectMenu.at}
          entries={projectEntries(projectMenu.path)}
          onClose={() => {
            setProjectMenu(null)
          }}
        />
      ) : null}
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
        {shownProjects.map((entry) => {
          const name = folderName(entry.path)
          const active = entry.active
          return (
            <li
              key={entry.path}
              className={active ? 'project-item is-open' : 'project-item'}
              onContextMenu={(event) => {
                if (
                  event.target instanceof Element &&
                  event.target.closest('.project-worktrees, .project-chats')
                )
                  return
                event.preventDefault()
                setProjectMenu({ path: entry.path, at: { x: event.clientX, y: event.clientY } })
              }}
            >
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
                <span className="project-chevron" aria-hidden="true">
                  {active ? '▾' : '▸'}
                </span>
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
                  <ProjectStatus entry={entry} />
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
              <button
                type="button"
                className="project-remove"
                aria-label={`Actions for ${name}`}
                title="More actions"
                aria-haspopup="menu"
                onClick={(event) => {
                  const rect = event.currentTarget.getBoundingClientRect()
                  setProjectMenu({ path: entry.path, at: { x: rect.left, y: rect.bottom } })
                }}
              >
                <MoreGlyph />
              </button>
              {active ? (
                <div className="project-children">
                  {entry.isRepo ? (
                    <WorktreesSection
                      project={entry.path}
                      activity={activity}
                      options={options}
                      filter={search}
                      onReveal={onReveal}
                      branch={entry.branch}
                      changes={changes}
                      jobs={jobs}
                      terminals={terminals}
                      selectedWorktrees={selectedWorktrees}
                      claudeAvailable={claudeAvailable}
                      cursorAvailable={cursorAvailable}
                      onSelect={onSelectWorktree}
                      onSelectAll={onSelectAllWorktrees}
                      onCreate={onCreateWorktree}
                      onRename={onRenameWorktree}
                      onDiscard={onDiscardWorktree}
                      onOpen={onOpenInWorktree}
                    />
                  ) : null}
                  {
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
                  }
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
      <div className="project-foot">
        <button
          type="button"
          className="project-foot-button"
          title="Settings (Ctrl+,)"
          onClick={onOpenSettings}
        >
          <svg width="14" height="14" viewBox="0 0 16 16" aria-hidden="true" focusable="false">
            <path
              fill="currentColor"
              d="M7 1h2l.4 1.9 1.3.6 1.7-1 1.4 1.4-1 1.7.6 1.3L15 7v2l-1.9.4-.6 1.3 1 1.7-1.4 1.4-1.7-1-1.3.6L9 15H7l-.4-1.9-1.3-.6-1.7 1-1.4-1.4 1-1.7-.6-1.3L1 9V7l1.9-.4.6-1.3-1-1.7 1.4-1.4 1.7 1 1.3-.6zm1 4.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5z"
            />
          </svg>
          <span className="project-foot-label">Settings</span>
        </button>
      </div>
    </aside>
  )
}

export default ProjectBar
