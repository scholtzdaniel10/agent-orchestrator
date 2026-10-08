type ProjectInfo = Awaited<ReturnType<Window['api']['getProject']>>

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

function ChevronGlyph(): React.JSX.Element {
  return (
    <svg
      className="project-chevron"
      width="12"
      height="12"
      viewBox="0 0 12 12"
      aria-hidden="true"
      focusable="false"
    >
      <path
        d="M3.25 4.5 6 7.25 8.75 4.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  )
}

function ProjectBar({
  project,
  error,
  onChange
}: {
  project: ProjectInfo | null
  error: string | null
  onChange: () => void
}): React.JSX.Element {
  return (
    <div className="project-bar">
      <button
        type="button"
        className="project-chip"
        aria-label="Change project folder"
        title={project === null ? undefined : project.path}
        onClick={onChange}
      >
        <FolderGlyph />
        {project === null ? null : (
          <>
            <span className="project-name">{folderName(project.path)}</span>
            {project.isRepo && project.branch !== null ? (
              <span className="project-branch">{project.branch}</span>
            ) : null}
            {!project.isRepo ? (
              <span className="project-note">not a git repository</span>
            ) : null}
          </>
        )}
        <ChevronGlyph />
      </button>
      {error !== null ? (
        <p className="field-error project-error" role="alert">
          {error}
        </p>
      ) : null}
    </div>
  )
}

export default ProjectBar
