import { useEffect, useState, type FormEvent } from 'react'

type JobRecord = Awaited<ReturnType<Window['api']['listJobs']>>[number]
type JobType = Parameters<Window['api']['submitJob']>[0]
type JobAccess = 'read' | 'edit' | 'full'
type Routine = Awaited<ReturnType<Window['api']['listRoutines']>>[number]
type RoutineWorker = Routine['worker']

const JOB_TYPES: readonly JobType[] = ['planning', 'debugging', 'review', 'refactor', 'boilerplate']

function errorText(err: unknown): string {
  if (err instanceof Error) {
    return err.message.replace(/^Error invoking remote method '[^']+': (Error: )?/, '')
  }
  if (typeof err === 'string') return err
  return 'Request failed'
}

function asJobType(value: string): JobType {
  if (
    value === 'planning' ||
    value === 'debugging' ||
    value === 'review' ||
    value === 'refactor' ||
    value === 'boilerplate'
  ) {
    return value
  }
  return 'planning'
}

function asWorker(value: string): RoutineWorker {
  if (value === 'claude' || value === 'cursor') return value
  return 'auto'
}

function asAccess(value: string): JobAccess {
  if (value === 'edit' || value === 'full') return value
  return 'read'
}

function afterLabel(after: Routine['after']): string {
  if (after === undefined) return ''
  return ` · after every ${after.jobType} job`
}

function emptyDraft(): {
  id: string | null
  name: string
  prompt: string
  type: JobType
  worker: RoutineWorker
  access: JobAccess
  after: JobType | ''
} {
  return {
    id: null,
    name: '',
    prompt: '',
    type: 'planning',
    worker: 'auto',
    access: 'read',
    after: ''
  }
}

function Routines({
  project,
  onRun
}: {
  project: string
  onRun: (job: JobRecord) => void
}): React.JSX.Element {
  const [commands, setCommands] = useState('')
  const [routines, setRoutines] = useState<Routine[]>([])
  const [draft, setDraft] = useState<ReturnType<typeof emptyDraft> | null>(null)
  const [formError, setFormError] = useState<string | null>(null)
  const [setupError, setSetupError] = useState<string | null>(null)
  const [armedId, setArmedId] = useState<string | null>(null)
  const [runError, setRunError] = useState<string | null>(null)

  useEffect(() => {
    let active = true
    void window.api.getProjectSetup().then(
      (list) => {
        if (active) setCommands(list.join('\n'))
      },
      () => {
        if (active) setCommands('')
      }
    )
    void window.api.listRoutines().then(
      (list) => {
        if (active) setRoutines(list)
      },
      () => {
        if (active) setRoutines([])
      }
    )
    return () => {
      active = false
    }
  }, [project])

  useEffect(() => {
    if (armedId === null) return
    const timer = window.setTimeout(() => setArmedId(null), 4000)
    return () => window.clearTimeout(timer)
  }, [armedId])

  async function saveSetup(): Promise<void> {
    const lines = commands
      .split('\n')
      .map((line) => line.trim())
      .filter((line) => line !== '')
      .slice(0, 5)
    try {
      await window.api.setProjectSetup(lines)
      setCommands(lines.join('\n'))
      setSetupError(null)
    } catch (err: unknown) {
      setSetupError(errorText(err))
    }
  }

  async function refreshRoutines(): Promise<void> {
    const list = await window.api.listRoutines()
    setRoutines(list)
  }

  async function saveDraft(event: FormEvent): Promise<void> {
    event.preventDefault()
    if (draft === null) return
    try {
      const routine: Routine = {
        id: draft.id ?? crypto.randomUUID(),
        name: draft.name,
        prompt: draft.prompt,
        type: draft.type,
        worker: draft.worker,
        access: draft.access
      }
      if (draft.after !== '') routine.after = { jobType: draft.after }
      await window.api.saveRoutine(routine)
      await refreshRoutines()
      setDraft(null)
      setFormError(null)
    } catch (err: unknown) {
      setFormError(errorText(err))
    }
  }

  async function run(id: string): Promise<void> {
    try {
      const job = await window.api.runRoutine(id)
      setRunError(null)
      onRun(job)
    } catch (err: unknown) {
      setRunError(errorText(err))
    }
  }

  async function remove(id: string): Promise<void> {
    if (armedId !== id) {
      setArmedId(id)
      return
    }
    setArmedId(null)
    try {
      await window.api.removeRoutine(id)
      await refreshRoutines()
      setRunError(null)
    } catch (err: unknown) {
      setRunError(errorText(err))
    }
  }

  function startEdit(routine: Routine): void {
    setDraft({
      id: routine.id,
      name: routine.name,
      prompt: routine.prompt,
      type: routine.type,
      worker: routine.worker,
      access: routine.access,
      after: routine.after?.jobType ?? ''
    })
    setFormError(null)
  }

  const setupLines = commands.split('\n')
  const setupTooLong = setupLines.length > 5

  return (
    <div className="routines-pane">
      <section className="routines-setup" aria-label="Worktree setup">
        <h3 className="routines-heading">Worktree setup</h3>
        <label className="sr-only" htmlFor="setup-commands">
          Setup commands
        </label>
        <textarea
          id="setup-commands"
          rows={5}
          value={commands}
          placeholder="pnpm install"
          onChange={(event) => setCommands(event.target.value)}
        />
        <div className="routines-setup-foot">
          <p className="hint">
            Runs in each worktree you create by hand, in a terminal you can watch.
          </p>
          <button type="button" className="btn btn-primary" onClick={() => void saveSetup()}>
            Save
          </button>
        </div>
        {setupTooLong ? (
          <p className="field-error" role="alert">
            At most 5 commands, one per line.
          </p>
        ) : null}
        {setupError !== null ? (
          <p className="field-error" role="alert">
            {setupError}
          </p>
        ) : null}
      </section>

      <section className="routines-list" aria-label="Routines">
        {routines.length === 0 && draft === null ? (
          <p className="hint">
            A routine is a saved job you run with one click, or after a job of a chosen type
            finishes.
          </p>
        ) : (
          routines.map((routine) => (
            <div key={routine.id} className="job-row">
              <div className="job-main">
                <span className="job-title">{routine.name}</span>
              </div>
              <div className="job-meta">
                {`${routine.type} · ${routine.worker} · ${routine.access}${afterLabel(routine.after)}`}
              </div>
              <div className="routines-row-actions">
                <button
                  type="button"
                  className="btn btn-quiet btn-compact"
                  onClick={() => void run(routine.id)}
                >
                  Run
                </button>
                <button
                  type="button"
                  className="btn btn-quiet btn-compact"
                  onClick={() => startEdit(routine)}
                >
                  Edit
                </button>
                <button
                  type="button"
                  className={
                    armedId === routine.id
                      ? 'btn btn-quiet btn-compact is-danger-label'
                      : 'btn btn-quiet btn-compact'
                  }
                  onClick={() => void remove(routine.id)}
                >
                  {armedId === routine.id ? 'Delete?' : 'Delete'}
                </button>
              </div>
            </div>
          ))
        )}
        {runError !== null ? (
          <p className="field-error" role="alert">
            {runError}
          </p>
        ) : null}
        {draft === null ? (
          <button
            type="button"
            className="btn btn-quiet"
            onClick={() => {
              setDraft(emptyDraft())
              setFormError(null)
            }}
          >
            New routine
          </button>
        ) : (
          <form className="routines-form" onSubmit={(event) => void saveDraft(event)}>
            <label htmlFor="routine-name">Name</label>
            <input
              id="routine-name"
              type="text"
              value={draft.name}
              maxLength={40}
              onChange={(event) => setDraft({ ...draft, name: event.target.value })}
            />
            <label htmlFor="routine-prompt">Prompt</label>
            <textarea
              id="routine-prompt"
              rows={4}
              value={draft.prompt}
              maxLength={8000}
              onChange={(event) => setDraft({ ...draft, prompt: event.target.value })}
            />
            <div className="composer-tools">
              <label htmlFor="routine-type">Job type</label>
              <select
                id="routine-type"
                className="job-type"
                value={draft.type}
                onChange={(event) => setDraft({ ...draft, type: asJobType(event.target.value) })}
              >
                {JOB_TYPES.map((type) => (
                  <option key={type} value={type}>
                    {type}
                  </option>
                ))}
              </select>
              <label htmlFor="routine-worker">Worker</label>
              <select
                id="routine-worker"
                className="plan-choice"
                value={draft.worker}
                onChange={(event) => setDraft({ ...draft, worker: asWorker(event.target.value) })}
              >
                <option value="auto">auto</option>
                <option value="claude">claude</option>
                <option value="cursor">cursor</option>
              </select>
              <label htmlFor="routine-access">Access</label>
              <select
                id="routine-access"
                className="job-access"
                value={draft.access}
                onChange={(event) => setDraft({ ...draft, access: asAccess(event.target.value) })}
              >
                <option value="read">Read only</option>
                <option value="edit">Edit files</option>
                <option value="full">Full access</option>
              </select>
              <label htmlFor="routine-after">Run automatically</label>
              <select
                id="routine-after"
                value={draft.after}
                onChange={(event) => {
                  const value = event.target.value
                  setDraft({
                    ...draft,
                    after: value === '' ? '' : asJobType(value)
                  })
                }}
              >
                <option value="">only when I click Run</option>
                {JOB_TYPES.map((type) => (
                  <option key={type} value={type}>
                    {`after every ${type} job`}
                  </option>
                ))}
              </select>
            </div>
            <div className="routines-form-actions">
              <button type="submit" className="btn btn-primary">
                Save
              </button>
              <button
                type="button"
                className="btn btn-quiet"
                onClick={() => {
                  setDraft(null)
                  setFormError(null)
                }}
              >
                Cancel
              </button>
            </div>
            {formError !== null ? (
              <p className="field-error" role="alert">
                {formError}
              </p>
            ) : null}
          </form>
        )}
      </section>
    </div>
  )
}

export default Routines
