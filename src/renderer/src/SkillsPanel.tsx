import { useEffect, useState } from 'react'
import type { SkillsState } from '../../preload/api-types'
import { Star } from './Star'

const GIVEN_TO_LABEL = {
  'every agent': 'every agent',
  workers: 'workers & terminals',
  lead: 'lead'
} as const

/** Skills loaded into the agents Legate starts: Legate's own, and folders the person adds. */
function SkillsPanel(): React.JSX.Element {
  const [state, setState] = useState<SkillsState | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let live = true
    window.api
      .skillsGet()
      .then((next) => {
        if (live) setState(next)
      })
      .catch((err: unknown) => {
        if (live) setError(err instanceof Error ? err.message : String(err))
      })
    return () => {
      live = false
    }
  }, [])

  async function run(action: () => Promise<SkillsState>): Promise<void> {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      setState(await action())
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <section className="skills" aria-labelledby="skills-heading">
      <div className="panel-head">
        <div className="panel-title">
          <Star size={12} />
          <h2 id="skills-heading">Skills</h2>
        </div>
        <div className="skills-actions">
          <button
            type="button"
            className="btn btn-quiet btn-compact"
            disabled={busy || state === null || state.folders.length === 0}
            title="Copy your skills again after editing one"
            onClick={() => void run(() => window.api.skillsReload())}
          >
            Reload
          </button>
          <button
            type="button"
            className="btn btn-compact"
            disabled={busy}
            onClick={() => void run(() => window.api.skillsAdd())}
          >
            Add folder
          </button>
        </div>
      </div>
      <p className="hint">
        Loaded only into agents Legate starts. Your other Claude and Cursor sessions do not get
        them.
      </p>
      {error !== null ? <p className="skills-problem">{error}</p> : null}
      {state === null ? (
        <p className="hint">Loading skills…</p>
      ) : (
        <>
          <h3 className="skills-subhead">Yours</h3>
          {state.folders.length === 0 ? (
            <p className="hint">
              None yet. Add a folder with a SKILL.md, or a folder of skill folders.
            </p>
          ) : (
            <ul className="skills-list">
              {state.folders.map((folder) => (
                <li key={folder.folder} className="skills-row">
                  <div className="skills-row-main">
                    <span className="skills-path" title={folder.folder}>
                      {folder.folder}
                    </span>
                    {folder.problem !== null ? (
                      <span className="skills-problem">{folder.problem}</span>
                    ) : (
                      <span className="skills-names">
                        {folder.skills.map((skill) => skill.name).join(', ')}
                      </span>
                    )}
                  </div>
                  <button
                    type="button"
                    className="btn btn-quiet btn-compact"
                    disabled={busy}
                    aria-label={`Remove ${folder.folder}`}
                    onClick={() => void run(() => window.api.skillsRemove(folder.folder))}
                  >
                    Remove
                  </button>
                </li>
              ))}
            </ul>
          )}
          <h3 className="skills-subhead">Legate&apos;s</h3>
          {state.builtIn.length === 0 ? (
            <p className="skills-problem">Legate&apos;s own skills were not found in this build.</p>
          ) : (
            <ul className="skills-list">
              {state.builtIn.map((skill) => (
                <li key={`${skill.givenTo}-${skill.name}`} className="skills-row">
                  <span className="skills-name">{skill.name}</span>
                  <span className="skills-tag">{GIVEN_TO_LABEL[skill.givenTo]}</span>
                </li>
              ))}
            </ul>
          )}
        </>
      )}
    </section>
  )
}

export default SkillsPanel
