import { Star } from './Star'

type ProjectEntry = Awaited<ReturnType<Window['api']['listProjects']>>[number]
type LeadChat = Awaited<ReturnType<Window['api']['listLeadChats']>>[number]

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

function ProjectBar({
  projects,
  chats,
  busy,
  leadBusy,
  now,
  error,
  onAdd,
  onSwitch,
  onRemove,
  onOpenChat,
  onNewChat
}: {
  projects: ProjectEntry[]
  chats: LeadChat[]
  busy: boolean
  leadBusy: boolean
  now: number
  error: string | null
  onAdd: () => void
  onSwitch: (path: string) => void
  onRemove: (path: string) => void
  onOpenChat: (id: string) => void
  onNewChat: () => void
}): React.JSX.Element {
  const shown = chats.slice(0, 30)
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
        </div>
      </div>
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
              {active ? (
                <div className="project-chats">
                  <div className="project-chats-head">
                    <span className="project-chats-label">Chats</span>
                    <button
                      type="button"
                      className="btn btn-quiet btn-compact"
                      disabled={leadBusy}
                      onClick={onNewChat}
                    >
                      New chat
                    </button>
                  </div>
                  <ul className="project-chat-list">
                    {shown.map((chat) => (
                      <li key={chat.id}>
                        <button
                          type="button"
                          className={`project-chat-row${chat.active ? ' is-selected' : ''}`}
                          title={chat.title}
                          aria-current={chat.active ? 'true' : undefined}
                          disabled={leadBusy}
                          onClick={() => {
                            onOpenChat(chat.id)
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
                            <span className="project-chat-age">
                              {relativeAge(chat.updatedAt, now)}
                            </span>
                          </span>
                        </button>
                      </li>
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
