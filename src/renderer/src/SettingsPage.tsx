import { useEffect, useId, useRef, useState } from 'react'
import BotAvatar from './BotAvatar'
import ModelPicker from './ModelPicker'
import { Star } from './Star'

type AppInfo = Awaited<ReturnType<Window['api']['getAppInfo']>>
type Prefs = Awaited<ReturnType<Window['api']['getPrefs']>>
type PlanStatus = Awaited<ReturnType<Window['api']['listPlans']>>[number]
type ProjectEntry = Awaited<ReturnType<Window['api']['listProjects']>>[number]
type ProviderId = PlanStatus['id']
type PlanChoice = 'auto' | ProviderId
type Theme = Prefs['theme']
type Section = 'general' | 'plans' | 'projects' | 'terminal' | 'about'

const SECTIONS: { id: Section; label: string }[] = [
  { id: 'general', label: 'General' },
  { id: 'plans', label: 'Plans' },
  { id: 'projects', label: 'Projects' },
  { id: 'terminal', label: 'Terminal' },
  { id: 'about', label: 'About' }
]

const SHORTCUTS: { keys: string; action: string }[] = [
  { keys: 'Ctrl+,', action: 'Open settings' },
  { keys: 'Ctrl+B', action: 'Hide or show the projects sidebar' },
  { keys: 'Ctrl+K', action: 'Filter projects' },
  { keys: 'Ctrl+Enter', action: 'Send to the lead or submit a job' },
  { keys: 'Ctrl+1…9', action: 'Switch to a project by position' }
]

function errorText(err: unknown): string {
  if (err instanceof Error) {
    return err.message.replace(/^Error invoking remote method '[^']+': (Error: )?/, '')
  }
  if (typeof err === 'string') return err
  return 'Request failed'
}

function folderName(path: string): string {
  const trimmed = path.replace(/[/\\]+$/, '')
  const index = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'))
  if (index === -1) return trimmed
  return trimmed.slice(index + 1)
}

function asPlanChoice(value: string): PlanChoice {
  if (value === 'claude' || value === 'cursor') return value
  return 'auto'
}

function planAvailable(plans: readonly PlanStatus[] | null, id: ProviderId): boolean {
  return plans !== null && plans.some((plan) => plan.id === id && plan.available)
}

function problemNotice(plan: PlanStatus): string | null {
  if (plan.problem === 'not-installed') {
    return plan.id === 'claude'
      ? 'Claude Code is not installed. Install it, then run `claude` once in a terminal to sign in.'
      : 'The Cursor CLI is not installed. Install it, then run `agent login` in a terminal.'
  }
  if (plan.problem === 'signed-out') {
    return plan.id === 'claude'
      ? 'Claude Code is signed out. Run `claude` in a terminal and sign in.'
      : 'The Cursor CLI is signed out. Run `agent login` in a terminal.'
  }
  return null
}

function planStatusWord(plan: PlanStatus): 'Ready' | 'Not installed' | 'Signed out' {
  if (plan.problem === 'not-installed') return 'Not installed'
  if (plan.problem === 'signed-out' || !plan.available) return 'Signed out'
  return 'Ready'
}

function planUnusable(plan: PlanStatus): boolean {
  return !plan.available || plan.problem === 'not-installed' || plan.problem === 'signed-out'
}

function Stepper({
  id,
  value,
  min,
  max,
  disabled,
  onChange
}: {
  id: string
  value: number
  min: number
  max: number
  disabled?: boolean
  onChange: (next: number) => void
}): React.JSX.Element {
  return (
    <div className="settings-stepper">
      <button
        type="button"
        className="btn btn-quiet btn-compact settings-stepper-btn"
        aria-label="Decrease"
        disabled={disabled === true || value <= min}
        onClick={() => {
          onChange(Math.max(min, value - 1))
        }}
      >
        −
      </button>
      <input
        id={id}
        className="settings-stepper-input"
        type="number"
        min={min}
        max={max}
        value={value}
        disabled={disabled === true}
        onChange={(event) => {
          const next = Number(event.target.value)
          if (!Number.isFinite(next)) return
          onChange(Math.min(max, Math.max(min, Math.round(next))))
        }}
      />
      <button
        type="button"
        className="btn btn-quiet btn-compact settings-stepper-btn"
        aria-label="Increase"
        disabled={disabled === true || value >= max}
        onClick={() => {
          onChange(Math.min(max, value + 1))
        }}
      >
        +
      </button>
    </div>
  )
}

function SettingsPage({
  prefs,
  appInfo,
  plans,
  projects,
  leadPlan,
  section: initialSection = 'general',
  onPrefs,
  onLeadPlan,
  onProjects,
  onClose
}: {
  prefs: Prefs
  appInfo: AppInfo
  plans: PlanStatus[] | null
  projects: ProjectEntry[]
  leadPlan: PlanChoice
  section?: Section
  onPrefs: (next: Prefs) => void
  onLeadPlan: (next: PlanChoice) => void
  onProjects: (next: ProjectEntry[]) => void
  onClose: () => void
}): React.JSX.Element {
  const [section, setSection] = useState<Section>(initialSection)
  const [error, setError] = useState<string | null>(null)
  const [checking, setChecking] = useState(false)
  const closeRef = useRef<HTMLButtonElement>(null)
  const checkingRef = useRef(false)
  const parallelId = useId()
  const fontId = useId()
  const notifyId = useId()
  const leadId = useId()
  const parallelValue = prefs.maxParallel ?? appInfo.rulesMaxParallel

  useEffect(() => {
    closeRef.current?.focus()
  }, [])

  useEffect(() => {
    function onKey(event: globalThis.KeyboardEvent): void {
      if (event.key !== 'Escape') return
      event.preventDefault()
      onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('keydown', onKey)
    }
  }, [onClose])

  async function patchPrefs(patch: Partial<Prefs>): Promise<void> {
    setError(null)
    try {
      const next = await window.api.setPrefs(patch)
      onPrefs(next)
    } catch (err: unknown) {
      setError(errorText(err))
    }
  }

  async function changeLead(value: PlanChoice): Promise<void> {
    setError(null)
    try {
      await window.api.setLeadPlan(value === 'auto' ? null : value)
      onLeadPlan(value)
    } catch (err: unknown) {
      setError(errorText(err))
    }
  }

  async function checkAgain(): Promise<void> {
    if (checkingRef.current) return
    checkingRef.current = true
    setChecking(true)
    setError(null)
    try {
      await window.api.recheckPlans()
    } catch (err: unknown) {
      setError(errorText(err))
    } finally {
      checkingRef.current = false
      setChecking(false)
    }
  }

  async function chooseClone(): Promise<void> {
    setError(null)
    try {
      const next = await window.api.chooseCloneDir()
      if (next !== null) onPrefs(next)
    } catch (err: unknown) {
      setError(errorText(err))
    }
  }

  async function removeProject(path: string): Promise<void> {
    setError(null)
    try {
      await window.api.removeProject(path)
      onProjects(await window.api.listProjects())
    } catch (err: unknown) {
      setError(errorText(err))
    }
  }

  async function reveal(path: string): Promise<void> {
    setError(null)
    try {
      await window.api.revealProject(path)
    } catch (err: unknown) {
      setError(errorText(err))
    }
  }

  async function openData(): Promise<void> {
    setError(null)
    try {
      await window.api.openDataDir()
    } catch (err: unknown) {
      setError(errorText(err))
    }
  }

  return (
    <div className="settings-page" role="dialog" aria-labelledby="settings-heading">
      <div className="settings-header">
        <div className="panel-title">
          <Star size={12} />
          <h2 id="settings-heading">Settings</h2>
        </div>
        <button
          ref={closeRef}
          type="button"
          className="btn btn-quiet"
          onClick={onClose}
        >
          Close
        </button>
      </div>
      <div className="settings-body">
        <nav className="settings-nav" aria-label="Settings sections">
          {SECTIONS.map((item) => (
            <button
              key={item.id}
              type="button"
              className={`settings-nav-row${section === item.id ? ' is-selected' : ''}`}
              aria-current={section === item.id ? 'true' : undefined}
              onClick={() => {
                setSection(item.id)
                setError(null)
              }}
            >
              {item.label}
            </button>
          ))}
        </nav>
        <div className="settings-content">
          {error !== null ? (
            <p className="field-error" role="alert">
              {error}
            </p>
          ) : null}

          {section === 'general' ? (
            <div className="settings-section">
              <div className="settings-row">
                <div className="settings-row-copy">
                  <span className="settings-row-label">Theme</span>
                  <p className="settings-row-help">Colour scheme for the window.</p>
                </div>
                <div className="settings-segment" role="group" aria-label="Theme">
                  {(
                    [
                      ['system', 'System'],
                      ['dark', 'Dark'],
                      ['light', 'Light']
                    ] as const
                  ).map(([value, label]) => (
                    <button
                      key={value}
                      type="button"
                      className={`settings-segment-btn${prefs.theme === value ? ' is-selected' : ''}`}
                      aria-pressed={prefs.theme === value}
                      onClick={() => {
                        void patchPrefs({ theme: value satisfies Theme })
                      }}
                    >
                      {label}
                    </button>
                  ))}
                </div>
              </div>
              <div className="settings-row">
                <div className="settings-row-copy">
                  <label className="settings-row-label" htmlFor={leadId}>
                    Lead runs on
                  </label>
                  <p className="settings-row-help">
                    Switching plans starts a fresh lead session.
                  </p>
                </div>
                <select
                  id={leadId}
                  className="plan-choice"
                  value={leadPlan}
                  onChange={(event) => {
                    void changeLead(asPlanChoice(event.target.value))
                  }}
                >
                  <option value="auto">auto</option>
                  <option value="claude" disabled={!planAvailable(plans, 'claude')}>
                    claude
                  </option>
                  <option value="cursor" disabled={!planAvailable(plans, 'cursor')}>
                    cursor
                  </option>
                </select>
              </div>
              <div className="settings-row">
                <div className="settings-row-copy">
                  <label className="settings-row-label" htmlFor={parallelId}>
                    Jobs at once per plan
                  </label>
                  <p className="settings-row-help">
                    How many jobs one plan runs side by side. More is faster and uses the allowance
                    faster.
                  </p>
                </div>
                <div className="settings-row-control">
                  <Stepper
                    id={parallelId}
                    value={parallelValue}
                    min={1}
                    max={8}
                    onChange={(next) => {
                      void patchPrefs({ maxParallel: next })
                    }}
                  />
                  <button
                    type="button"
                    className="btn btn-quiet btn-compact"
                    disabled={prefs.maxParallel === null}
                    onClick={() => {
                      void patchPrefs({ maxParallel: null })
                    }}
                  >
                    Use default ({appInfo.rulesMaxParallel})
                  </button>
                </div>
              </div>
              <div className="settings-row">
                <div className="settings-row-copy">
                  <label className="settings-row-label" htmlFor={notifyId}>
                    Notifications
                  </label>
                  <p className="settings-row-help">Desktop alerts for finished jobs.</p>
                </div>
                <label className="edit-toggle settings-check" htmlFor={notifyId}>
                  <input
                    id={notifyId}
                    type="checkbox"
                    checked={prefs.notify}
                    onChange={(event) => {
                      void patchPrefs({ notify: event.target.checked })
                    }}
                  />
                  Tell me when a job ends while the window is in the background
                </label>
              </div>
            </div>
          ) : null}

          {section === 'plans' ? (
            <div className="settings-section">
              <p className="settings-note">
                Each plan is the CLI signed in on this computer. The app never reads or stores your
                logins.
              </p>
              {(plans ?? []).map((plan) => {
                const word = planStatusWord(plan)
                const notice = problemNotice(plan)
                return (
                  <div key={plan.id} className="settings-plan">
                    <div className="settings-plan-head">
                      <BotAvatar
                        bot={plan.id}
                        state={planUnusable(plan) ? 'resting' : 'idle'}
                        size={28}
                      />
                      <span className="settings-plan-name">{plan.id}</span>
                    </div>
                    <div className="settings-row">
                      <div className="settings-row-copy">
                        <span className="settings-row-label">Status</span>
                        <p className="settings-row-help">{notice ?? word}</p>
                      </div>
                      <div className="settings-row-control">
                        <span className="settings-plan-status">{word}</span>
                        {notice !== null ? (
                          <button
                            type="button"
                            className="btn btn-compact"
                            disabled={checking}
                            onClick={() => {
                              void checkAgain()
                            }}
                          >
                            Check again
                          </button>
                        ) : null}
                      </div>
                    </div>
                    <ModelPicker plan={plan} disabled={planUnusable(plan)} />
                  </div>
                )
              })}
            </div>
          ) : null}

          {section === 'projects' ? (
            <div className="settings-section">
              <div className="settings-row">
                <div className="settings-row-copy">
                  <span className="settings-row-label">Clone folder</span>
                  <p className="settings-row-help">
                    Where repositories cloned from GitHub are put.
                  </p>
                </div>
                <div className="settings-row-control">
                  <span className="settings-mono">
                    {prefs.cloneDir === null ? 'Ask every time' : prefs.cloneDir}
                  </span>
                  <button
                    type="button"
                    className="btn btn-quiet btn-compact"
                    onClick={() => {
                      void chooseClone()
                    }}
                  >
                    Choose…
                  </button>
                  {prefs.cloneDir !== null ? (
                    <button
                      type="button"
                      className="btn btn-quiet btn-compact"
                      onClick={() => {
                        void patchPrefs({ cloneDir: null })
                      }}
                    >
                      Ask every time
                    </button>
                  ) : null}
                </div>
              </div>
              <div className="settings-block">
                <span className="settings-row-label">Remembered projects</span>
                <ul className="settings-project-list">
                  {projects.map((entry) => {
                    const name = folderName(entry.path)
                    return (
                      <li key={entry.path} className="settings-project-item">
                        <div className="settings-project-info">
                          <span className="settings-project-name">{name}</span>
                          <span className="settings-mono">{entry.path}</span>
                          {entry.isRepo && entry.branch !== null ? (
                            <span className="settings-mono">{entry.branch}</span>
                          ) : null}
                        </div>
                        <div className="settings-row-control">
                          <button
                            type="button"
                            className="btn btn-quiet btn-compact"
                            onClick={() => {
                              void reveal(entry.path)
                            }}
                          >
                            Open folder
                          </button>
                          <button
                            type="button"
                            className="btn btn-quiet btn-compact"
                            disabled={entry.active}
                            title={
                              entry.active
                                ? 'Switch to another project before removing this one'
                                : 'Remove from this list (does not delete the folder)'
                            }
                            onClick={() => {
                              void removeProject(entry.path)
                            }}
                          >
                            Remove
                          </button>
                        </div>
                      </li>
                    )
                  })}
                </ul>
              </div>
              <div className="settings-row">
                <div className="settings-row-copy">
                  <span className="settings-row-label">Worktrees folder</span>
                  <p className="settings-row-help">
                    Editing jobs work in copies here until you merge or discard them.
                  </p>
                </div>
                <span className="settings-mono">{appInfo.worktreesDir}</span>
              </div>
            </div>
          ) : null}

          {section === 'terminal' ? (
            <div className="settings-section">
              <div className="settings-row">
                <div className="settings-row-copy">
                  <label className="settings-row-label" htmlFor={fontId}>
                    Text size
                  </label>
                  <p className="settings-row-help">Pixel size for terminal text.</p>
                </div>
                <Stepper
                  id={fontId}
                  value={prefs.terminalFontSize}
                  min={10}
                  max={22}
                  onChange={(next) => {
                    void patchPrefs({ terminalFontSize: next })
                  }}
                />
              </div>
            </div>
          ) : null}

          {section === 'about' ? (
            <div className="settings-section">
              <div className="settings-row">
                <div className="settings-row-copy">
                  <span className="settings-row-label">App</span>
                  <p className="settings-row-help">agent-orchestrator</p>
                </div>
                <span className="settings-about-version">Version {appInfo.version}</span>
              </div>
              <div className="settings-row">
                <div className="settings-row-copy">
                  <span className="settings-row-label">Data folder</span>
                  <p className="settings-row-help">
                    Settings and saved chats. Nothing here leaves this computer.
                  </p>
                </div>
                <div className="settings-row-control">
                  <span className="settings-mono">{appInfo.dataDir}</span>
                  <button
                    type="button"
                    className="btn btn-quiet btn-compact"
                    onClick={() => {
                      void openData()
                    }}
                  >
                    Open folder
                  </button>
                </div>
              </div>
              <div className="settings-block">
                <span className="settings-row-label">Shortcuts</span>
                <dl className="settings-shortcuts">
                  {SHORTCUTS.map((item) => (
                    <div key={item.keys} className="settings-shortcut">
                      <dt>{item.keys}</dt>
                      <dd>{item.action}</dd>
                    </div>
                  ))}
                </dl>
              </div>
            </div>
          ) : null}
        </div>
      </div>
    </div>
  )
}

export default SettingsPage
