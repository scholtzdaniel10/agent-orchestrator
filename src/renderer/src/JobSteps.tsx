import { useEffect, useRef, useState, type UIEvent } from 'react'
import { parseDiff, type DiffLine } from './diff-lines'
import { editedFiles, editedFilesSummary } from './job-steps'

type JobRecord = Awaited<ReturnType<Window['api']['listJobs']>>[number]
type JobStep = NonNullable<JobRecord['steps']>[number]

const DIFF_LINE_CAP = 5000
const stepsOpenChoice = new Map<string, boolean>()

const TOOL_WORD: Record<JobStep['tool'], string> = {
  read: 'READ',
  edit: 'EDIT',
  shell: 'RUN',
  search: 'SEARCH',
  web: 'WEB',
  mcp: 'MCP',
  say: 'SAY',
  think: 'THINK',
  other: 'TOOL'
}

function splitPath(path: string): { name: string; folder: string } {
  const norm = path.replace(/\\/g, '/')
  const index = norm.lastIndexOf('/')
  if (index < 0) return { name: norm, folder: '' }
  return { name: norm.slice(index + 1), folder: norm.slice(0, index) }
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${String(ms)}ms`
  const seconds = ms / 1000
  if (seconds < 10) return `${seconds.toFixed(1)}s`
  return `${String(Math.round(seconds))}s`
}

function fileLabel(count: number): string {
  return count === 1 ? '1 file' : `${String(count)} files`
}

function diffForFile(text: string, filePath: string): DiffLine[] {
  const lines = parseDiff(text)
  const wanted = filePath.replace(/\\/g, '/')
  const out: DiffLine[] = []
  let keep = false
  for (const line of lines) {
    if (line.kind === 'file') keep = gitHeaderMatches(line.text, wanted)
    if (keep) out.push(line)
  }
  return out
}

function gitHeaderMatches(header: string, filePath: string): boolean {
  const match = /^diff --git a\/(.+) b\/(.+)$/.exec(header)
  if (match === null) return header.includes(filePath)
  return pathMatches(filePath, match[1] ?? '') || pathMatches(filePath, match[2] ?? '')
}

function pathMatches(filePath: string, gitPath: string): boolean {
  const a = filePath.replace(/\\/g, '/')
  const b = gitPath.replace(/\\/g, '/')
  return a === b || a.endsWith(`/${b}`) || b.endsWith(`/${a}`)
}

function FileDiff({ text, filePath }: { text: string; filePath: string }): React.JSX.Element {
  const lines = diffForFile(text, filePath)
  if (lines.length === 0) {
    return <p className="job-edits-gone">The diff is no longer available.</p>
  }
  const shown = lines.slice(0, DIFF_LINE_CAP)
  const hidden = lines.length - shown.length
  return (
    <div className="diff-block">
      {shown.map((line, index) => (
        <div key={index} className={`diff-line is-${line.kind}`}>
          {line.text.length === 0 ? ' ' : line.text}
        </div>
      ))}
      {hidden > 0 ? (
        <div className="diff-line is-meta">{`… ${String(hidden)} more lines not shown`}</div>
      ) : null}
    </div>
  )
}

function EditedFilesCard({ job }: { job: JobRecord }): React.JSX.Element | null {
  const files = editedFiles(job.steps ?? [])
  const [openPath, setOpenPath] = useState<string | null>(null)
  const changeId = job.change
  const [diffText, setDiffText] = useState<string | null>(null)
  const [diffFailed, setDiffFailed] = useState(false)
  const [diffLoading, setDiffLoading] = useState(changeId !== undefined)

  useEffect(() => {
    if (changeId === undefined) return
    let active = true
    void window.api.changeDiff(changeId).then(
      (next) => {
        if (!active) return
        setDiffText(next)
        setDiffLoading(false)
      },
      () => {
        if (!active) return
        setDiffFailed(true)
        setDiffLoading(false)
      }
    )
    return () => {
      active = false
    }
  }, [changeId])

  if (files.length === 0) return null
  const summary = editedFilesSummary(job)
  if (summary === null) return null

  function fileBody(filePath: string): React.JSX.Element {
    if (changeId === undefined || diffFailed) {
      return <p className="job-edits-gone">The diff is no longer available.</p>
    }
    if (diffLoading || diffText === null) return <p className="diff-status">Loading diff…</p>
    return <FileDiff text={diffText} filePath={filePath} />
  }

  return (
    <div className="job-edits">
      <div className="job-edits-head">
        <span className="job-edits-title">{`Edited ${fileLabel(summary.files)}`}</span>
        <span className="change-stats">
          <span className="stat-add">{`+${String(summary.added)}`}</span>{' '}
          <span className="stat-del">{`−${String(summary.removed)}`}</span>
        </span>
      </div>
      <ul className="job-edits-list">
        {files.map((file) => {
          const { name, folder } = splitPath(file.path)
          const open = openPath === file.path
          return (
            <li key={file.path} className="job-edits-item">
              <button
                type="button"
                className="job-edits-row"
                aria-expanded={open}
                onClick={() => setOpenPath(open ? null : file.path)}
              >
                <span className="job-edits-name">{name}</span>
                {folder !== '' ? <span className="job-edits-folder">{folder}</span> : null}
                <span className="change-stats">
                  <span className="stat-add">{`+${String(file.added)}`}</span>{' '}
                  <span className="stat-del">{`−${String(file.removed)}`}</span>
                </span>
                <span className={open ? 'job-edits-chevron is-open' : 'job-edits-chevron'}>
                  {'>'}
                </span>
              </button>
              {open ? fileBody(file.path) : null}
            </li>
          )
        })}
      </ul>
    </div>
  )
}

function StepRow({ step }: { step: JobStep }): React.JSX.Element {
  const [open, setOpen] = useState(false)
  const duration =
    step.endedAt === undefined ? '' : formatDuration(Math.max(0, step.endedAt - step.startedAt))
  const hasDetail = step.detail !== undefined && step.detail !== ''
  const body = (
    <>
      <span className="job-step-tool">{TOOL_WORD[step.tool]}</span>
      <span className="job-step-title" title={step.title}>
        {step.title}
      </span>
      {duration !== '' ? <span className="job-step-time">{duration}</span> : null}
      {step.status === 'running' ? (
        <span className="status status-running">
          <span className="status-dot" aria-hidden="true" />
        </span>
      ) : null}
      {step.status === 'failed' ? <span className="job-step-failed">failed</span> : null}
    </>
  )
  if (!hasDetail) {
    return <div className="job-step">{body}</div>
  }
  return (
    <div className="job-step-wrap">
      <button
        type="button"
        className="job-step"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        {body}
      </button>
      {open ? <pre className="job-step-detail">{step.detail}</pre> : null}
    </div>
  )
}

function StepsList({ job }: { job: JobRecord }): React.JSX.Element | null {
  const steps = job.steps ?? []
  const dropped = job.stepsDropped ?? 0
  const running = job.status === 'running' || job.status === 'queued'
  const [open, setOpen] = useState(() => stepsOpenChoice.get(job.id) ?? running)
  const [follow, setFollow] = useState(true)
  const listRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open || !follow || !running) return
    const list = listRef.current
    if (list === null) return
    list.scrollTop = list.scrollHeight
  }, [open, follow, running, steps.length])

  if (steps.length === 0 && dropped === 0) return null

  function toggle(): void {
    setOpen((value) => {
      const next = !value
      stepsOpenChoice.set(job.id, next)
      return next
    })
  }

  function onScroll(event: UIEvent<HTMLDivElement>): void {
    const list = event.currentTarget
    const atBottom = list.scrollHeight - list.scrollTop - list.clientHeight < 24
    setFollow(atBottom)
  }

  return (
    <div className="job-steps">
      <button type="button" className="job-steps-head" aria-expanded={open} onClick={toggle}>
        {`Steps (${String(steps.length)})`}
      </button>
      {open ? (
        <div ref={listRef} className="job-steps-list" onScroll={onScroll}>
          {dropped > 0 ? (
            <p className="job-steps-dropped">{`${String(dropped)} earlier steps not kept.`}</p>
          ) : null}
          {steps.map((item) => (
            <StepRow key={item.id} step={item} />
          ))}
        </div>
      ) : null}
    </div>
  )
}

function JobSteps({ job }: { job: JobRecord }): React.JSX.Element | null {
  const steps = job.steps ?? []
  const dropped = job.stepsDropped ?? 0
  if (steps.length === 0 && dropped === 0) return null
  return (
    <div className="job-steps-block">
      <EditedFilesCard job={job} />
      <StepsList key={job.id} job={job} />
    </div>
  )
}

export default JobSteps
