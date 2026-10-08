import { Star } from './Star'

type ProjectEntry = Awaited<ReturnType<Window['api']['listProjects']>>[number]

function folderName(path: string): string {
  const trimmed = path.replace(/[/\\]+$/, '')
  const index = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'))
  if (index === -1) return trimmed
  return trimmed.slice(index + 1)
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
  busy,
  error,
  onAdd,
  onSwitch,
  onRemove
}: {
  projects: ProjectEntry[]
  busy: boolean
  error: string | null
  onAdd: () => void
  onSwitch: (path: string) => void
  onRemove: (path: string) => void
}): React.JSX.Element {
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
