type ProjectInfo = Awaited<ReturnType<Window['api']['getProject']>>

function folderName(path: string): string {
  const trimmed = path.replace(/[/\\]+$/, '')
  const index = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'))
  if (index === -1) return trimmed
  return trimmed.slice(index + 1)
}

function repoNote(project: ProjectInfo): string | null {
  if (!project.isRepo) return ' · not a git repository'
  if (project.branch === null) return null
  return ` · ${project.branch}`
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
  const note = project === null ? null : repoNote(project)

  return (
    <div className="project-bar">
      {project === null ? null : (
        <p className="project-line">
          <span className="project-name" title={project.path}>
            {folderName(project.path)}
          </span>
          {note !== null ? <span className="project-note">{note}</span> : null}
        </p>
      )}
      <button
        type="button"
        className="btn btn-quiet"
        aria-label="Change project folder"
        onClick={onChange}
      >
        Change…
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
